//! Encrypted peer transport, discovery, authentication, and bounded replication messages.

mod node;
mod transport;
pub mod wire;

pub use node::{NetworkEvent, SwarmNode, WIRE_PROTOCOL};
pub use transport::{generate_transport_key, load_or_create_transport_key};
pub use wire::{
    BlobResumeV1, ReplicaAckV1, WireLimitError, WireRequest, WireResponse, MAX_BLOB_CHUNK,
    MAX_MISSING_BLOBS, MAX_WORLD_MEMBERS,
};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PROTOCOL_VERSION};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HandshakeError {
    #[error("peer ID does not match the presented public key")]
    PeerIdMismatch,
    #[error("peer does not support protocol {0}")]
    ProtocolMismatch(u16),
    #[error("peer hello signature is invalid")]
    SignatureInvalid,
    #[error("peer hello cannot be encoded")]
    EncodingFailed,
}

pub fn verify_peer_hello(hello: &PeerHelloV1) -> Result<(), HandshakeError> {
    if peer_id_from_public_key(&hello.public_key) != hello.peer_id {
        return Err(HandshakeError::PeerIdMismatch);
    }
    if !hello.protocol_versions.contains(&PROTOCOL_VERSION) {
        return Err(HandshakeError::ProtocolMismatch(PROTOCOL_VERSION));
    }
    let key = VerifyingKey::from_bytes(&hello.public_key).map_err(|_| HandshakeError::SignatureInvalid)?;
    let signature = Signature::from_slice(&hello.signature).map_err(|_| HandshakeError::SignatureInvalid)?;
    let message = hello.signing_bytes().map_err(|_| HandshakeError::EncodingFailed)?;
    key.verify(&message, &signature).map_err(|_| HandshakeError::SignatureInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;
    use std::time::Duration;
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

    #[test]
    fn signed_hello_authenticates_application_identity() {
        verify_peer_hello(&signed_hello()).unwrap();
    }

    #[tokio::test]
    async fn two_quic_nodes_authenticate_each_other() {
        let hello_a = signed_hello();
        let hello_b = signed_hello();
        let app_a = hello_a.peer_id;
        let app_b = hello_b.peer_id;
        let mut node_a = SwarmNode::new(generate_transport_key(), hello_a).unwrap();
        let mut node_b = SwarmNode::new(generate_transport_key(), hello_b).unwrap();
        node_a.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();

        let address = timeout(Duration::from_secs(10), async {
            loop {
                if let NetworkEvent::Listening { address } = node_a.next_event().await.unwrap() {
                    break address;
                }
            }
        })
        .await
        .expect("node A should listen");
        node_b.dial(address).unwrap();

        let mut a_saw_b = false;
        let mut b_saw_a = false;
        timeout(Duration::from_secs(15), async {
            while !(a_saw_b && b_saw_a) {
                tokio::select! {
                    event = node_a.next_event() => {
                        if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                            a_saw_b |= application_peer == app_b;
                        }
                    }
                    event = node_b.next_event() => {
                        if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                            b_saw_a |= application_peer == app_a;
                        }
                    }
                }
            }
        })
        .await
        .expect("both QUIC peers should authenticate");
    }
}
