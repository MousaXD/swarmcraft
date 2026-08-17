use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::time::Duration;
use swarm_network::{
    generate_transport_key, load_or_create_transport_key, ConnectivityIssueKindV1, NetworkEvent, SwarmNode,
    TransportPeerId, WireRequest, WireResponse,
};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PeerId, PROTOCOL_VERSION};
use tempfile::tempdir;
use tokio::time::timeout;

fn signed_hello(key: &SigningKey, nonce: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [nonce; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

async fn wait_for_authentication(a: &mut SwarmNode, b: &mut SwarmNode, app_a: PeerId, app_b: PeerId) {
    let mut a_saw_b = false;
    let mut b_saw_a = false;
    timeout(Duration::from_secs(15), async {
        while !(a_saw_b && b_saw_a) {
            tokio::select! {
                event = a.next_event() => {
                    if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                        a_saw_b |= application_peer == app_b;
                    }
                }
                event = b.next_event() => {
                    if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                        b_saw_a |= application_peer == app_a;
                    }
                }
            }
        }
    })
    .await
    .expect("both peers should authenticate");
}

async fn assert_ping_round_trip(sender: &mut SwarmNode, receiver: &mut SwarmNode, receiver_transport: TransportPeerId) {
    const NONCE: u64 = 0x5A17_CAFE;
    let request_id = sender.send_request(&receiver_transport, WireRequest::Ping { nonce: NONCE }).unwrap();

    timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                event = receiver.next_event() => {
                    if let NetworkEvent::InboundRequest { request: WireRequest::Ping { nonce }, channel, .. } = event.unwrap() {
                        assert_eq!(nonce, NONCE);
                        receiver.respond(channel, WireResponse::Pong { nonce }).unwrap();
                    }
                }
                event = sender.next_event() => {
                    if let NetworkEvent::Response { request_id: observed, response: WireResponse::Pong { nonce }, .. } = event.unwrap() {
                        if observed == request_id {
                            assert_eq!(nonce, NONCE);
                            break;
                        }
                    }
                }
            }
        }
    })
    .await
    .expect("authenticated request should survive reconnect");
}

#[tokio::test]
async fn hard_reconnect_preserves_transport_identity_and_authenticated_requests() {
    let temp = tempdir().unwrap();
    let transport_a_path = temp.path().join("a/transport.key");
    let transport_b_path = temp.path().join("b/transport.key");

    let app_key_a = SigningKey::generate(&mut OsRng);
    let app_key_b = SigningKey::generate(&mut OsRng);
    let hello_a = signed_hello(&app_key_a, 1);
    let hello_b = signed_hello(&app_key_b, 2);
    let app_a = hello_a.peer_id;
    let app_b = hello_b.peer_id;

    let transport_a_key = load_or_create_transport_key(&transport_a_path).unwrap();
    let transport_b_key = load_or_create_transport_key(&transport_b_path).unwrap();
    let transport_a = transport_a_key.public().to_peer_id();
    let transport_b = transport_b_key.public().to_peer_id();

    let mut node_a = SwarmNode::new(transport_a_key, hello_a.clone()).unwrap();
    let mut node_b = SwarmNode::new(transport_b_key, hello_b.clone()).unwrap();
    node_a.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();

    let listen_address = timeout(Duration::from_secs(10), async {
        loop {
            if let NetworkEvent::Listening { address } = node_a.next_event().await.unwrap() {
                break address;
            }
        }
    })
    .await
    .expect("node A should listen");

    node_b.dial(listen_address.clone()).unwrap();
    wait_for_authentication(&mut node_a, &mut node_b, app_a, app_b).await;
    assert_ping_round_trip(&mut node_b, &mut node_a, transport_a).await;

    // Simulate a hard peer restart. Do not drain node A's disconnect event first: the
    // replacement connection is deliberately allowed to race the dead libp2p connection.
    drop(node_b);

    let replacement_key = load_or_create_transport_key(&transport_b_path).unwrap();
    assert_eq!(replacement_key.public().to_peer_id(), transport_b);
    let mut replacement_b = SwarmNode::new(replacement_key, signed_hello(&app_key_b, 3)).unwrap();
    replacement_b.dial(listen_address).unwrap();

    wait_for_authentication(&mut node_a, &mut replacement_b, app_a, app_b).await;
    assert_eq!(node_a.application_peer(&transport_b), Some(app_b));
    assert_ping_round_trip(&mut replacement_b, &mut node_a, transport_a).await;
}

#[tokio::test]
async fn invalid_bootstrap_address_is_rejected_and_diagnosed() {
    let app_key = SigningKey::generate(&mut OsRng);
    let hello = signed_hello(&app_key, 9);
    let mut node = SwarmNode::new(generate_transport_key(), hello).unwrap();
    let invalid_bootstrap = "/ip4/127.0.0.1/udp/4001/quic-v1".parse().unwrap();

    let error = node.add_bootstrap_address(invalid_bootstrap).unwrap_err();
    assert!(error.to_string().contains("/p2p/<peer-id>"));

    let diagnostics = node.connectivity_diagnostics();
    assert_eq!(
        diagnostics.recent_failures.last().map(|issue| issue.kind),
        Some(ConnectivityIssueKindV1::InvalidAddress)
    );
}
