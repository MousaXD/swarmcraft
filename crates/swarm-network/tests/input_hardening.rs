use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use futures::StreamExt;
use libp2p::{
    request_response::{self, cbor, ProtocolSupport},
    swarm::SwarmEvent,
    Multiaddr, PeerId as TransportPeerId, StreamProtocol, SwarmBuilder,
};
use rand_core::OsRng;
use swarm_network::{
    generate_transport_key, NetworkEvent, SwarmNode, WireRequest, WireResponse, MAX_BLOB_CHUNK, WIRE_PROTOCOL,
};
use swarm_protocol::{peer_id_from_public_key, BlobEncoding, Hash32, PeerHelloV1, WorldId, PROTOCOL_VERSION};
use tokio::time::timeout;

fn signed_hello() -> (PeerHelloV1, SigningKey) {
    let key = SigningKey::generate(&mut OsRng);
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [7; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    (hello, key)
}

fn new_node() -> SwarmNode {
    let (hello, signing_key) = signed_hello();
    SwarmNode::new(generate_transport_key(), hello, signing_key).unwrap()
}

async fn listen_address(node: &mut SwarmNode) -> Multiaddr {
    timeout(Duration::from_secs(10), async {
        loop {
            match node.next_event().await.unwrap() {
                NetworkEvent::Listening { address } => break address,
                _ => continue,
            }
        }
    })
    .await
    .expect("node should listen")
}

async fn authenticate_pair(
    client: &mut SwarmNode,
    server: &mut SwarmNode,
    server_address: Multiaddr,
) -> (TransportPeerId, TransportPeerId) {
    let client_peer = client.local_transport_peer_id();
    let server_peer = server.local_transport_peer_id();
    client.dial(server_address).unwrap();

    let mut client_authenticated = false;
    let mut server_authenticated = false;
    timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                event = client.next_event() => {
                    if let NetworkEvent::Authenticated { transport_peer, .. } = event.unwrap() {
                        if transport_peer == server_peer {
                            client_authenticated = true;
                        }
                    }
                }
                event = server.next_event() => {
                    if let NetworkEvent::Authenticated { transport_peer, .. } = event.unwrap() {
                        if transport_peer == client_peer {
                            server_authenticated = true;
                        }
                    }
                }
            }
            if client_authenticated && server_authenticated {
                break;
            }
        }
    })
    .await
    .expect("peers should mutually authenticate");

    (client_peer, server_peer)
}

async fn assert_ping_round_trip(client: &mut SwarmNode, server: &mut SwarmNode, nonce: u64) {
    let client_peer = client.local_transport_peer_id();
    let server_peer = server.local_transport_peer_id();
    client.send_request(&server_peer, WireRequest::Ping { nonce }).unwrap();

    let mut server_received = false;
    timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                event = server.next_event() => {
                    match event.unwrap() {
                        NetworkEvent::InboundRequest {
                            transport_peer,
                            request: WireRequest::Ping { nonce: received },
                            channel,
                        } if transport_peer == client_peer => {
                            assert_eq!(received, nonce);
                            server.respond(channel, WireResponse::Pong { nonce: received }).unwrap();
                            server_received = true;
                        }
                        _ => {}
                    }
                }
                event = client.next_event() => {
                    match event.unwrap() {
                        NetworkEvent::Response {
                            transport_peer,
                            response: WireResponse::Pong { nonce: received },
                            ..
                        } if transport_peer == server_peer => {
                            assert_eq!(received, nonce);
                            assert!(server_received);
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    })
    .await
    .expect("valid ping should succeed after hostile input");
}

#[tokio::test]
async fn oversized_pre_auth_request_is_rejected_and_valid_traffic_still_succeeds() {
    let mut victim = new_node();
    let victim_peer = victim.local_transport_peer_id();
    victim.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let address = listen_address(&mut victim).await;

    let behaviour = cbor::Behaviour::new(
        [(StreamProtocol::new(WIRE_PROTOCOL), ProtocolSupport::Full)],
        request_response::Config::default().with_request_timeout(Duration::from_secs(5)),
    );
    let mut attacker =
        SwarmBuilder::with_new_identity().with_tokio().with_quic().with_behaviour(|_| behaviour).unwrap().build();
    attacker.dial(address.clone()).unwrap();

    let mut request_sent = false;
    timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                victim_event = victim.next_event() => {
                    if let Err(error) = victim_event {
                        panic!("malicious request terminated the victim event loop: {error:#}");
                    }
                }
                attacker_event = attacker.select_next_some() => match attacker_event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == victim_peer && !request_sent => {
                        attacker.behaviour_mut().send_request(
                            &victim_peer,
                            WireRequest::BlobChunk {
                                world_id: WorldId([1; 32]),
                                hash: Hash32([2; 32]),
                                encoding: BlobEncoding::Zstd,
                                offset: 0,
                                data: vec![0; MAX_BLOB_CHUNK + 1],
                                finished: false,
                            },
                        );
                        request_sent = true;
                    }
                    SwarmEvent::Behaviour(request_response::Event::Message {
                        message: request_response::Message::Response { response, .. },
                        ..
                    }) => {
                        assert!(request_sent);
                        assert!(matches!(
                            response,
                            WireResponse::Error { code, .. } if code == "REQUEST_LIMIT_EXCEEDED"
                        ));
                        break;
                    }
                    _ => {}
                }
            }
        }
    })
    .await
    .expect("attacker should receive a bounded rejection while victim remains alive");

    let mut valid = new_node();
    authenticate_pair(&mut valid, &mut victim, address).await;
    assert_ping_round_trip(&mut valid, &mut victim, 0x5eed).await;
}

#[tokio::test]
async fn vanished_response_channel_is_peer_local_and_node_continues() {
    let mut victim = new_node();
    victim.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let address = listen_address(&mut victim).await;

    let mut requester = new_node();
    let (requester_peer, victim_peer) = authenticate_pair(&mut requester, &mut victim, address.clone()).await;
    requester.send_request(&victim_peer, WireRequest::Ping { nonce: 41 }).unwrap();

    let channel = timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                event = victim.next_event() => {
                    if let NetworkEvent::InboundRequest {
                        transport_peer,
                        request: WireRequest::Ping { nonce: 41 },
                        channel,
                    } = event.unwrap()
                    {
                        assert_eq!(transport_peer, requester_peer);
                        break channel;
                    }
                }
                event = requester.next_event() => {
                    event.unwrap();
                }
            }
        }
    })
    .await
    .expect("victim should receive the request before the requester disappears");

    drop(requester);
    timeout(Duration::from_secs(15), async {
        loop {
            if let NetworkEvent::Disconnected { transport_peer } = victim.next_event().await.unwrap() {
                if transport_peer == requester_peer {
                    break;
                }
            }
        }
    })
    .await
    .expect("victim should observe the requester disconnect");

    assert!(victim.respond(channel, WireResponse::Pong { nonce: 41 }).is_err());

    let mut valid = new_node();
    authenticate_pair(&mut valid, &mut victim, address).await;
    assert_ping_round_trip(&mut valid, &mut victim, 42).await;
}
