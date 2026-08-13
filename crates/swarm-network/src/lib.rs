//! Encrypted peer transport, discovery, authentication, and bounded replication messages.

mod node;
mod transport;
pub mod wire;

pub use node::{NetworkEvent, SwarmNode, WIRE_PROTOCOL};
pub use transport::{generate_transport_key, load_or_create_transport_key};
pub use wire::{ReplicaAckV1, WireLimitError, WireRequest, WireResponse, MAX_BLOB_CHUNK, MAX_MISSING_BLOBS};

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
