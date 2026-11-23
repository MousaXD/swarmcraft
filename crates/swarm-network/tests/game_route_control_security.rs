//! Network-level fail-closed tests for route control BEFORE application auth.
//! A libp2p Noise/QUIC connection is not sufficient to request an authority
//! route or enter Minecraft, even if a peer knows the target WorldId.

use ed25519_dalek::{Signer, SigningKey};
use futures::StreamExt;
use libp2p::{
    request_response::{self, cbor, ProtocolSupport},
    swarm::SwarmEvent,
    PeerId as TransportPeerId, StreamProtocol, SwarmBuilder,
};
use rand_core::OsRng;
use std::time::Duration;
use swarm_network::{
    game_route_control::{
        GameRouteControlEvent, GameRouteControlRequestV1 as Request, GameRouteControlResponseV1 as Response,
        GameRouteRefusalV1 as Refusal, GAME_ROUTE_CONTROL_PROTOCOL,
    },
    generate_transport_key, NetworkEvent, SwarmNode,
};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, WorldId, PROTOCOL_VERSION};
use tokio::time::timeout;

fn new_node() -> SwarmNode {
    let key = SigningKey::generate(&mut OsRng);
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [3; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    SwarmNode::new(generate_transport_key(), hello, key).unwrap()
}

async fn deny_raw_route_request_before_auth(request: Request, expected: Refusal) {
    let mut victim = new_node();
    let victim_transport: TransportPeerId = victim.local_transport_peer_id();
    victim.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let address = timeout(Duration::from_secs(10), async {
        loop {
            if let NetworkEvent::Listening { address } = victim.next_event().await.unwrap() {
                break address;
            }
        }
    })
    .await
    .expect("victim must listen");

    // This fake client speaks the ROUTE control protocol but never completes
    // SwarmCraft's connection-bound PeerHello application proof.
    let behaviour = cbor::Behaviour::<Request, Response>::new(
        [(StreamProtocol::new(GAME_ROUTE_CONTROL_PROTOCOL), ProtocolSupport::Full)],
        request_response::Config::default().with_request_timeout(Duration::from_secs(5)),
    );
    let mut attacker =
        SwarmBuilder::with_new_identity().with_tokio().with_quic().with_behaviour(|_| behaviour).unwrap().build();
    attacker.dial(address).unwrap();
    let mut sent = false;
    timeout(Duration::from_secs(12), async {
        loop {
            tokio::select! {
                event = victim.next_event() => {
                    event.expect("unauthenticated request must not kill the network driver");
                }
                event = attacker.select_next_some() => match event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == victim_transport && !sent => {
                        attacker.behaviour_mut().send_request(&victim_transport, request.clone());
                        sent = true;
                    }
                    SwarmEvent::Behaviour(request_response::Event::Message {
                        message: request_response::Message::Response { response, .. },
                        ..
                    }) => {
                        assert!(sent);
                        assert_eq!(response, Response::Refused(expected));
                        assert_eq!(victim.application_peer(attacker.local_peer_id()), None);
                        break;
                    }
                    _ => {}
                }
            }
        }
    })
    .await
    .expect("unauthenticated QUIC route request must receive a bounded refusal");
}

#[tokio::test]
async fn route_offers_cannot_be_requested_over_transport_without_application_auth() {
    deny_raw_route_request_before_auth(Request::Offer { world_id: WorldId([4; 32]) }, Refusal::Unauthorized).await;
}

#[tokio::test]
async fn malformed_raw_route_requests_are_never_authority_proofs() {
    deny_raw_route_request_before_auth(Request::Offer { world_id: WorldId::default() }, Refusal::Unauthorized).await;
}

#[tokio::test]
async fn authenticated_members_without_a_quorum_handler_are_refused() {
    let mut host = new_node();
    let mut player = new_node();
    let host_id = host.local_transport_peer_id();
    let player_id = player.local_transport_peer_id();
    let mut controller_events = player.take_game_route_control_events().unwrap();
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
                        host_authenticated |= transport_peer == player_id;
                    }
                }
                event = player.next_event() => {
                    if let NetworkEvent::Authenticated { transport_peer, .. } = event.unwrap() {
                        player_authenticated |= transport_peer == host_id;
                    }
                }
            }
        }
    })
    .await
    .unwrap();

    let id = player.send_game_route_request(&host_id, Request::Offer { world_id: WorldId([5; 32]) }).unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                event = host.next_event() => { event.unwrap(); }
                event = player.next_event() => { event.unwrap(); }
                event = controller_events.recv() => {
                    if let Some(GameRouteControlEvent::Response { request_id, response, .. }) = event {
                        if request_id == id {
                            assert_eq!(response, Response::Refused(Refusal::Unavailable));
                            break;
                        }
                    }
                }
            }
        }
    })
    .await
    .expect("no route controller must be a bounded refusal rather than a trusted offer");
}
