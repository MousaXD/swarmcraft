use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use futures::StreamExt;
use libp2p::{
    request_response::{self, cbor, ProtocolSupport},
    swarm::SwarmEvent,
    StreamProtocol, SwarmBuilder,
};
use rand_core::OsRng;
use swarm_network::{
    generate_transport_key, NetworkEvent, SwarmNode, WireRequest, WireResponse, MAX_BLOB_CHUNK, WIRE_PROTOCOL,
};
use swarm_protocol::{peer_id_from_public_key, BlobEncoding, Hash32, PeerHelloV1, WorldId, PROTOCOL_VERSION};
use tokio::time::timeout;

fn signed_hello() -> PeerHelloV1 {
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
    hello
}

#[tokio::test]
async fn oversized_request_is_rejected_then_valid_request_still_succeeds() {
    let mut victim = SwarmNode::new(generate_transport_key(), signed_hello()).unwrap();
    let victim_peer = victim.local_transport_peer_id();
    victim.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();

    let address = timeout(Duration::from_secs(10), async {
        loop {
            match victim.next_event().await.unwrap() {
                NetworkEvent::Listening { address } => break address,
                _ => continue,
            }
        }
    })
    .await
    .expect("victim should listen");

    let behaviour = cbor::Behaviour::new(
        [(StreamProtocol::new(WIRE_PROTOCOL), ProtocolSupport::Full)],
        request_response::Config::default().with_request_timeout(Duration::from_secs(5)),
    );
    let mut attacker =
        SwarmBuilder::with_new_identity().with_tokio().with_quic().with_behaviour(|_| behaviour).unwrap().build();
    attacker.dial(address).unwrap();

    let valid_hello = signed_hello();
    let expected_application_peer = valid_hello.peer_id;
    let mut oversized_sent = false;
    let mut oversized_rejected = false;
    let mut valid_sent = false;
    let mut valid_response_received = false;
    let mut victim_authenticated = false;

    timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                victim_event = victim.next_event() => {
                    match victim_event {
                        Err(error) => panic!("malicious request terminated the victim event loop: {error:#}"),
                        Ok(NetworkEvent::Authenticated { application_peer, .. }) => {
                            assert_eq!(application_peer, expected_application_peer);
                            assert!(oversized_rejected, "valid request was processed only after oversized rejection");
                            victim_authenticated = true;
                        }
                        Ok(_) => {}
                    }
                }
                attacker_event = attacker.select_next_some() => match attacker_event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == victim_peer && !oversized_sent => {
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
                        oversized_sent = true;
                    }
                    SwarmEvent::Behaviour(request_response::Event::Message {
                        message: request_response::Message::Response { response, .. },
                        ..
                    }) if !oversized_rejected => {
                        assert!(oversized_sent);
                        assert!(matches!(
                            response,
                            WireResponse::Error { code, .. } if code == "REQUEST_LIMIT_EXCEEDED"
                        ));
                        oversized_rejected = true;
                        attacker.behaviour_mut().send_request(&victim_peer, WireRequest::Hello(valid_hello.clone()));
                        valid_sent = true;
                    }
                    SwarmEvent::Behaviour(request_response::Event::Message {
                        message: request_response::Message::Response { response, .. },
                        ..
                    }) if valid_sent => {
                        assert!(matches!(response, WireResponse::HelloAccepted { protocol_version } if protocol_version == PROTOCOL_VERSION));
                        valid_response_received = true;
                    }
                    _ => {}
                }
            }

            if victim_authenticated && valid_response_received {
                break;
            }
        }
    })
    .await
    .expect("victim should reject oversized input and then accept a valid request on the same connection");

    assert!(oversized_rejected);
    assert!(victim_authenticated);
    assert!(valid_response_received);
}
