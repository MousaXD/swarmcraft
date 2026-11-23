//! A real encrypted/authenticated QUIC control exchange driven through the
//! bounded host controller, not a direct test-only network event handler.
//! The authority source deliberately refuses: no synthetic success path.

use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use swarm_network::{
    game_route::{FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError},
    game_route_control::{
        GameRouteControlRequestV1 as Request, GameRouteControlResponseV1 as Response, GameRouteRefusalV1,
    },
    game_route_host::AuthenticatedGameRouteHostSigner,
    game_route_host_controller::{
        GameRouteHostControlService, GameRouteHostReplyAction, MAX_PENDING_HOST_ROUTE_ACTIONS,
    },
    game_route_host_service::BoundedGameRouteHostResponder,
    generate_transport_key, NetworkEvent, SwarmNode,
};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tokio::{
    sync::{mpsc, watch},
    time::timeout,
};

fn authenticated_hello(key: &SigningKey, tag: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [tag; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

#[derive(Default)]
struct RefusingCanonicalAuthority {
    attempted: AtomicUsize,
}

impl FreshAuthoritySource for RefusingCanonicalAuthority {
    fn verify_fresh(
        &self,
        _world: WorldId,
        _requester: PeerId,
        _nonce: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError> {
        self.attempted.fetch_add(1, Ordering::SeqCst);
        Err(GameRouteError::UnverifiedAuthority)
    }
}

#[tokio::test]
async fn asynchronous_host_controller_only_returns_authenticated_fail_closed_route_replies() {
    timeout(Duration::from_secs(25), async {
        let host_key = SigningKey::generate(&mut OsRng);
        let player_key = SigningKey::generate(&mut OsRng);
        let host_app = peer_id_from_public_key(&host_key.verifying_key().to_bytes());
        let player_app = peer_id_from_public_key(&player_key.verifying_key().to_bytes());
        let mut host =
            SwarmNode::new(generate_transport_key(), authenticated_hello(&host_key, 1), host_key.clone()).unwrap();
        let mut player =
            SwarmNode::new(generate_transport_key(), authenticated_hello(&player_key, 2), player_key).unwrap();
        let host_transport = host.local_transport_peer_id();
        let player_transport = player.local_transport_peer_id();

        let events = host.take_game_route_control_events().unwrap();
        let source = Arc::new(RefusingCanonicalAuthority::default());
        let root = tempfile::tempdir().unwrap();
        let signer = Arc::new(AuthenticatedGameRouteHostSigner::new(
            host_key,
            host_transport,
            source.clone(),
            root.path().join("unreachable-host-sequencer"),
        ));
        let responder = Arc::new(BoundedGameRouteHostResponder::new(signer));
        let (reply_tx, mut replies) = mpsc::channel::<GameRouteHostReplyAction>(MAX_PENDING_HOST_ROUTE_ACTIONS);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let host_controller =
            tokio::spawn(GameRouteHostControlService::new(responder, events).run(reply_tx, shutdown_rx));

        host.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
        let address = loop {
            if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                break address;
            }
        };
        player.dial(address).unwrap();
        let mut host_authenticated = false;
        let mut player_authenticated = false;
        timeout(Duration::from_secs(12), async {
            while !host_authenticated || !player_authenticated {
                tokio::select! {
                    event = host.next_event() => {
                        if let NetworkEvent::Authenticated { transport_peer, application_peer } = event.unwrap() {
                            host_authenticated |= transport_peer == player_transport && application_peer == player_app;
                        }
                    }
                    event = player.next_event() => {
                        if let NetworkEvent::Authenticated { transport_peer, application_peer } = event.unwrap() {
                            player_authenticated |= transport_peer == host_transport && application_peer == host_app;
                        }
                    }
                }
            }
        })
        .await
        .expect("QUIC must authenticate the application identities");

        let rpc = player.game_route_rpc_client();
        let world_id = WorldId([37; 32]);
        let mut request = tokio::spawn({
            let rpc = rpc.clone();
            async move { rpc.request(host_transport, Request::Offer { world_id }).await }
        });
        let refusal = timeout(Duration::from_secs(12), async {
            loop {
                tokio::select! {
                    event = host.next_event() => { event.unwrap(); }
                    event = player.next_event() => { event.unwrap(); }
                    action = replies.recv() => {
                        let action = action.expect("host controller must remain available");
                        host.respond_game_route(action.channel, action.response).unwrap();
                    }
                    result = &mut request => { break result.unwrap().unwrap(); }
                }
            }
        })
        .await
        .expect("authenticated route RPC must receive a bounded denial");
        assert_eq!(refusal, Response::Refused(GameRouteRefusalV1::QuorumUnavailable));
        assert_eq!(source.attempted.load(Ordering::SeqCst), 1, "actual signer must consult the proof source");

        shutdown_tx.send(true).unwrap();
        timeout(Duration::from_secs(2), host_controller).await.unwrap().unwrap();
        let mut denied_after_shutdown =
            tokio::spawn(async move { rpc.request(host_transport, Request::Offer { world_id }).await });
        let denied = timeout(Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    event = host.next_event() => { event.unwrap(); }
                    event = player.next_event() => { event.unwrap(); }
                    result = &mut denied_after_shutdown => { break result.unwrap().unwrap(); }
                }
            }
        })
        .await
        .expect("closed route controller must fail closed");
        assert_eq!(denied, Response::Refused(GameRouteRefusalV1::Unavailable));
        assert_eq!(source.attempted.load(Ordering::SeqCst), 1, "shutdown must prevent additional signing");
    })
    .await
    .expect("real QUIC host-controller test must complete");
}

#[tokio::test]
async fn oversized_reply_queue_cannot_disable_host_route_action_bounds() {
    let source = Arc::new(RefusingCanonicalAuthority::default());
    let signer = Arc::new(AuthenticatedGameRouteHostSigner::new(
        SigningKey::generate(&mut OsRng),
        generate_transport_key().public().to_peer_id(),
        source.clone(),
        tempfile::tempdir().unwrap().path().join("unused-sequences"),
    ));
    let (_event_tx, events) = mpsc::channel(1);
    let (oversized, _unread) = mpsc::channel::<GameRouteHostReplyAction>(MAX_PENDING_HOST_ROUTE_ACTIONS + 1);
    let (_stop_tx, stop_rx) = watch::channel(false);
    let service = GameRouteHostControlService::new(Arc::new(BoundedGameRouteHostResponder::new(signer)), events);
    timeout(Duration::from_millis(250), service.run(oversized, stop_rx))
        .await
        .expect("oversized queues must be refused instead of retaining unbounded answers");
    assert_eq!(source.attempted.load(Ordering::SeqCst), 0);
}
