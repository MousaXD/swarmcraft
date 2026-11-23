//! Real QUIC transport exercising asynchronous route-RPC mailbox dispatch.
//! The network driver retains exclusive SwarmNode ownership throughout.
//! Refusal is a refusal: no synthetic signatures or authority are invented.

use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::time::Duration;
use swarm_network::{
    game_route_control::{
        GameRouteControlEvent, GameRouteControlRequestV1 as Request, GameRouteControlResponseV1 as Response,
        GameRouteRefusalV1 as Refusal, GameRouteRpcError,
    },
    generate_transport_key, NetworkEvent, SwarmNode,
};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, WorldId, PROTOCOL_VERSION};
use tokio::time::timeout;

fn node() -> SwarmNode {
    let key = SigningKey::generate(&mut OsRng);
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".to_owned()],
        nonce: [42; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    SwarmNode::new(generate_transport_key(), hello, key).unwrap()
}

#[tokio::test]
async fn mailbox_cannot_request_routes_before_connection_bound_application_auth() {
    let mut player = node();
    let rpc = player.game_route_rpc_client();
    let nonexistent_host = generate_transport_key().public().to_peer_id();
    let driver = tokio::spawn(async move {
        loop {
            player.next_event().await.unwrap();
        }
    });
    let result =
        timeout(Duration::from_secs(2), rpc.request(nonexistent_host, Request::Offer { world_id: WorldId([2; 32]) }))
            .await
            .expect("local authentication rejection must be prompt");
    assert!(matches!(result, Err(GameRouteRpcError::Unavailable)));
    driver.abort();
    assert!(matches!(
        rpc.request(nonexistent_host, Request::Offer { world_id: WorldId::default() }).await,
        Err(GameRouteRpcError::InvalidRequest)
    ));
}

#[tokio::test]
async fn mailbox_routes_authorized_quic_requests_and_refuses_when_host_handler_is_removed() {
    let mut host = node();
    let mut player = node();
    let host_transport = host.local_transport_peer_id();
    let player_transport = player.local_transport_peer_id();
    let rpc = player.game_route_rpc_client();
    let mut host_route_events = host.take_game_route_control_events().unwrap();
    host.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let address = timeout(Duration::from_secs(10), async {
        loop {
            if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                break address;
            }
        }
    })
    .await
    .unwrap();
    player.dial(address).unwrap();
    let mut host_authenticated = false;
    let mut player_authenticated = false;
    timeout(Duration::from_secs(12), async {
        while !host_authenticated || !player_authenticated {
            tokio::select! {
                event = host.next_event() => {
                    if let NetworkEvent::Authenticated { transport_peer, .. } = event.unwrap() {
                        host_authenticated |= transport_peer == player_transport;
                    }
                }
                event = player.next_event() => {
                    if let NetworkEvent::Authenticated { transport_peer, .. } = event.unwrap() {
                        player_authenticated |= transport_peer == host_transport;
                    }
                }
            }
        }
    })
    .await
    .expect("both peers must authenticate before route RPC");

    let world = WorldId([19; 32]);
    let mut request = tokio::spawn({
        let rpc = rpc.clone();
        async move { rpc.request(host_transport, Request::Offer { world_id: world }).await }
    });
    timeout(Duration::from_secs(8), async {
        loop {
            tokio::select! {
                event = host.next_event() => { event.unwrap(); }
                event = player.next_event() => { event.unwrap(); }
                event = host_route_events.recv() => {
                    if let Some(GameRouteControlEvent::Inbound {
                        transport_peer, application_peer, request: Request::Offer { world_id }, channel,
                    }) = event {
                        assert_eq!(transport_peer, player_transport);
                        assert_eq!(world_id, world);
                        assert_ne!(application_peer, swarm_protocol::PeerId::default());
                        host.respond_game_route(channel, Response::Refused(Refusal::QuorumUnavailable)).unwrap();
                        break;
                    }
                }
            }
        }
    })
    .await
    .expect("host must receive authenticated route request");
    let response = timeout(Duration::from_secs(8), async {
        loop {
            tokio::select! {
                event = host.next_event() => { event.unwrap(); }
                event = player.next_event() => { event.unwrap(); }
                result = &mut request => {
                    return result.expect("rpc task must not panic");
                }
            }
        }
    })
    .await
    .expect("the asynchronous requester must get a bounded refusal");
    assert_eq!(response.unwrap(), Response::Refused(Refusal::QuorumUnavailable));

    // An authenticated SwarmCraft transport still has no route authority if
    // the application controller unsubscribes or crashes.
    drop(host_route_events);
    let mut request =
        tokio::spawn(async move { rpc.request(host_transport, Request::Offer { world_id: world }).await });
    let failed = timeout(Duration::from_secs(8), async {
        loop {
            tokio::select! {
                event = host.next_event() => { event.unwrap(); }
                event = player.next_event() => { event.unwrap(); }
                response = &mut request => { return response.unwrap(); }
            }
        }
    })
    .await
    .expect("unsubscribed route handler must not leave requester hanging");
    assert_eq!(failed.unwrap(), Response::Refused(Refusal::Unavailable));
}
