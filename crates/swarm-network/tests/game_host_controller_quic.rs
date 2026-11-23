//! A real encrypted/authenticated QUIC control exchange driven through the
//! bounded host controller, not a direct test-only network event handler.
//! The authority sources are TEST-ONLY; signed success here does not certify
//! real canonical quorum or live Fabric runtime readiness.

use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use swarm_network::{
    game_route::{FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError, RouteVerifier},
    game_route_control::{
        GameRouteControlRequestV1 as Request, GameRouteControlResponseV1 as Response, GameRouteRefusalV1,
        GameRouteRpcClient,
    },
    game_route_host::AuthenticatedGameRouteHostSigner,
    game_route_host_controller::{
        GameRouteHostControlService, GameRouteHostReplyAction, MAX_PENDING_HOST_ROUTE_ACTIONS,
    },
    game_route_host_service::BoundedGameRouteHostResponder,
    generate_transport_key, NetworkEvent, SwarmNode, TransportPeerId,
};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
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

/// Do NOT use outside tests: all quorum and runtime fields are synthetic.
struct SyntheticCurrentHost {
    world: WorldId,
    requester: PeerId,
    host_transport: TransportPeerId,
    host_public_key: [u8; 32],
    ready: AtomicBool,
}

impl FreshAuthoritySource for SyntheticCurrentHost {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError> {
        if world != self.world || requester != self.requester {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        Ok(FreshAuthorityEvidence {
            world_id: world,
            realm_hash: Hash32([2; 32]),
            authority_peer_id: peer_id_from_public_key(&self.host_public_key),
            authority_public_key: self.host_public_key,
            authority_transport_peer: self.host_transport,
            game_server_port: 25565,
            requester_is_member: true,
            authority_is_member: true,
            conflict_free: true,
            hosting_consent_valid: true,
            runtime_ready_verified: self.ready.load(Ordering::SeqCst),
            membership_hash: Hash32([3; 32]),
            quorum_certificate_hash: Hash32([4; 32]),
            compatibility_fingerprint: Hash32([5; 32]),
            checkpoint_hash: Hash32([6; 32]),
            state_root: Hash32([7; 32]),
            epoch: 4,
            fencing_token: 9,
            consent_revision: 2,
            runtime_instance_nonce: [8; 32],
            runtime_ready_proof_hash: Hash32([9; 32]),
            challenge_nonce: nonce,
            verified_at_unix_ms: now,
            valid_until_unix_ms: now + 5_000,
        })
    }
}

async fn request_with_polled_network_driver(
    host: &mut SwarmNode,
    player: &mut SwarmNode,
    replies: &mut mpsc::Receiver<GameRouteHostReplyAction>,
    rpc: &GameRouteRpcClient,
    host_transport: TransportPeerId,
    request: Request,
) -> Response {
    let rpc = rpc.clone();
    let mut result = tokio::spawn(async move { rpc.request(host_transport, request).await });
    timeout(Duration::from_secs(12), async {
        loop {
            tokio::select! {
                event = host.next_event() => { event.unwrap(); }
                event = player.next_event() => { event.unwrap(); }
                action = replies.recv() => {
                    let action = action.expect("host controller reply channel must remain open");
                    host.respond_game_route(action.channel, action.response).unwrap();
                }
                answer = &mut result => { break answer.unwrap().unwrap(); }
            }
        }
    })
    .await
    .expect("signed route exchange must complete over authenticated QUIC")
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

#[tokio::test]
async fn signed_offers_and_single_use_challenges_cross_the_real_host_controller() {
    timeout(Duration::from_secs(25), async {
        let host_key = SigningKey::generate(&mut OsRng);
        let player_key = SigningKey::generate(&mut OsRng);
        let host_app = peer_id_from_public_key(&host_key.verifying_key().to_bytes());
        let player_app = peer_id_from_public_key(&player_key.verifying_key().to_bytes());
        let mut host =
            SwarmNode::new(generate_transport_key(), authenticated_hello(&host_key, 3), host_key.clone()).unwrap();
        let mut player =
            SwarmNode::new(generate_transport_key(), authenticated_hello(&player_key, 4), player_key).unwrap();
        let host_transport = host.local_transport_peer_id();
        let player_transport = player.local_transport_peer_id();
        let events = host.take_game_route_control_events().unwrap();
        let world = WorldId([42; 32]);
        let source = Arc::new(SyntheticCurrentHost {
            world,
            requester: player_app,
            host_transport,
            host_public_key: host_key.verifying_key().to_bytes(),
            ready: AtomicBool::new(true),
        });
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let signer = Arc::new(AuthenticatedGameRouteHostSigner::new(
            host_key,
            host_transport,
            source.clone(),
            root.path().join("host-offer-sequences"),
        ));
        let responder = Arc::new(BoundedGameRouteHostResponder::new(signer));
        let (reply_tx, mut replies) = mpsc::channel::<GameRouteHostReplyAction>(MAX_PENDING_HOST_ROUTE_ACTIONS);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let controller = tokio::spawn(GameRouteHostControlService::new(responder, events).run(reply_tx, shutdown_rx));

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
        .expect("both peers must authenticate before game route requests");

        let rpc = player.game_route_rpc_client();
        let offered = request_with_polled_network_driver(
            &mut host,
            &mut player,
            &mut replies,
            &rpc,
            host_transport,
            Request::Offer { world_id: world },
        )
        .await;
        let Response::Offer(offer) = offered else {
            panic!("test-only current authority must sign an offer");
        };
        assert_eq!(offer.verify_signature().unwrap(), host_transport);
        assert_eq!(offer.authority_peer_id, host_app);
        assert_eq!(offer.world_id, world);
        let verifier = RouteVerifier::new(root.path().join("accepted-generation-floors"));
        let challenge = verifier.issue_challenge(&offer, player_app, player_transport).unwrap();
        let answered = request_with_polled_network_driver(
            &mut host,
            &mut player,
            &mut replies,
            &rpc,
            host_transport,
            Request::Challenge(Box::new(challenge.clone())),
        )
        .await;
        let Response::ChallengeResponse(response) = answered else {
            panic!("the host must answer an exact one-time challenge");
        };
        assert_eq!(response.challenge, challenge);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let verified = verifier.verify(&offer, &response, world, now, source.as_ref()).unwrap();
        assert!(verified.is_fresh());
        assert_eq!(verified.transport_peer(), host_transport);

        // One accepted signed response must not be reusable for another game
        // join, even while the host's synthetic readiness remains true.
        let replay = request_with_polled_network_driver(
            &mut host,
            &mut player,
            &mut replies,
            &rpc,
            host_transport,
            Request::Challenge(Box::new(challenge)),
        )
        .await;
        assert_eq!(replay, Response::Refused(GameRouteRefusalV1::ExpiredOrMalformed));

        source.ready.store(false, Ordering::SeqCst);
        let refused = request_with_polled_network_driver(
            &mut host,
            &mut player,
            &mut replies,
            &rpc,
            host_transport,
            Request::Offer { world_id: world },
        )
        .await;
        assert_eq!(refused, Response::Refused(GameRouteRefusalV1::HostUnready));
        shutdown_tx.send(true).unwrap();
        timeout(Duration::from_secs(2), controller).await.unwrap().unwrap();
    })
    .await
    .expect("real QUIC signed host route-control path must finish");
}
