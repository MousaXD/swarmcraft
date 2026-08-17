use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use swarm_network::{verify_peer_hello, HandshakeError};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PROTOCOL_VERSION};

fn signed_hello(protocol_versions: Vec<u16>) -> PeerHelloV1 {
    let key = SigningKey::generate(&mut OsRng);
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions,
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [8; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

#[test]
fn signed_hello_cannot_downgrade_below_current_protocol() {
    let hello = signed_hello(vec![PROTOCOL_VERSION - 1]);
    assert_eq!(verify_peer_hello(&hello), Err(HandshakeError::ProtocolMismatch(PROTOCOL_VERSION)));
}

#[test]
fn signed_hello_rejects_replayed_signature_after_nonce_mutation() {
    let mut hello = signed_hello(vec![PROTOCOL_VERSION]);
    hello.nonce[0] ^= 0xff;
    assert_eq!(verify_peer_hello(&hello), Err(HandshakeError::SignatureInvalid));
}
