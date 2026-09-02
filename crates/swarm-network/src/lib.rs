//! Encrypted peer transport, discovery, authentication, and bounded replication messages.

mod diagnostics;
mod discovery;
mod invite_connectivity;
mod node;
mod transport;
pub mod wire;

pub use diagnostics::{
    ConnectivityDiagnosticsV1, ConnectivityIssueKindV1, ConnectivityIssueV1, ConnectivityStateV1, HolePunchStateV1,
    NatStatusV1, CONNECTIVITY_DIAGNOSTICS_JSON_ENV, CONNECTIVITY_DIAGNOSTICS_SNAPSHOT_FILE, MAX_CONNECTIVITY_FAILURES,
};
pub use discovery::{
    friend_presence_key, public_directory_key, world_discovery_key, DiscoveryNetworkEvent, DiscoveryNode,
    DISCOVERY_WIRE_PROTOCOL,
};
pub use invite_connectivity::{
    invite_connectivity_from_snapshot, validate_invite_addresses, InviteConnectivityError, InviteConnectivityV1,
    InviteReachabilityV1, DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE, MAX_CONNECTIVITY_SNAPSHOT_BYTES,
    MAX_INVITE_ADDRESSES, MAX_INVITE_ADDRESS_CHARS,
};
pub use libp2p::request_response::ResponseChannel;
pub use libp2p::PeerId as TransportPeerId;
pub use node::{NetworkEvent, SwarmNode, BOOTSTRAP_ENV, RELAY_ENV, WIRE_PROTOCOL};
pub use transport::{generate_transport_key, load_or_create_transport_key};
pub use wire::{
    BlobResumeV1, HostCapabilityV1, HostRuntimeReadinessV1, PeerHelloProofV1, ReplicaAckV1, ServerModsReadinessV1,
    WireLimitError, WireRequest, WireResponse, MAX_BLOB_CHUNK, MAX_DISCOVERY_ANNOUNCEMENT_BYTES,
    MAX_DISCOVERY_QUERY_BYTES, MAX_DISCOVERY_RESULTS, MAX_DISCOVERY_TAGS, MAX_MISSING_BLOBS, MAX_RECOVERY_VOTES,
    MAX_WORLD_ARTIFACTS, MAX_WORLD_MEMBERS,
};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
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
    #[error("connection authentication challenge does not match the live receiver challenge")]
    ChallengeMismatch,
    #[error("application proof is bound to a different transport connection")]
    TransportBindingMismatch,
    #[error("application signing key does not match the advertised peer identity")]
    SigningKeyMismatch,
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

const PEER_HELLO_PROOF_DOMAIN: &[u8] = b"swarmcraft/peer-hello-connection-proof/v1\0";

fn push_len_prefixed(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), HandshakeError> {
    let len = u32::try_from(value.len()).map_err(|_| HandshakeError::EncodingFailed)?;
    bytes.extend_from_slice(&len.to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

fn peer_hello_proof_signing_bytes(proof: &PeerHelloProofV1) -> Result<Vec<u8>, HandshakeError> {
    let hello_bytes = proof.hello.signing_bytes().map_err(|_| HandshakeError::EncodingFailed)?;
    let mut bytes = Vec::with_capacity(
        PEER_HELLO_PROOF_DOMAIN.len()
            + hello_bytes.len()
            + proof.hello.signature.len()
            + proof.claimant_transport_peer.len()
            + proof.receiver_transport_peer.len()
            + 64,
    );
    bytes.extend_from_slice(PEER_HELLO_PROOF_DOMAIN);
    push_len_prefixed(&mut bytes, &hello_bytes)?;
    push_len_prefixed(&mut bytes, &proof.hello.signature)?;
    bytes.extend_from_slice(&proof.challenge);
    push_len_prefixed(&mut bytes, &proof.claimant_transport_peer)?;
    push_len_prefixed(&mut bytes, &proof.receiver_transport_peer)?;
    Ok(bytes)
}

pub fn build_peer_hello_proof(
    hello: &PeerHelloV1,
    signing_key: &SigningKey,
    challenge: [u8; 32],
    claimant_transport_peer: &TransportPeerId,
    receiver_transport_peer: &TransportPeerId,
) -> Result<PeerHelloProofV1, HandshakeError> {
    verify_peer_hello(hello)?;
    if signing_key.verifying_key().to_bytes() != hello.public_key {
        return Err(HandshakeError::SigningKeyMismatch);
    }
    let mut proof = PeerHelloProofV1 {
        hello: hello.clone(),
        challenge,
        claimant_transport_peer: claimant_transport_peer.to_bytes(),
        receiver_transport_peer: receiver_transport_peer.to_bytes(),
        signature: Vec::new(),
    };
    proof.signature = signing_key.sign(&peer_hello_proof_signing_bytes(&proof)?).to_bytes().to_vec();
    Ok(proof)
}

pub fn verify_peer_hello_proof(
    proof: &PeerHelloProofV1,
    expected_challenge: [u8; 32],
    expected_claimant_transport_peer: &TransportPeerId,
    local_receiver_transport_peer: &TransportPeerId,
) -> Result<(), HandshakeError> {
    verify_peer_hello(&proof.hello)?;
    if proof.challenge != expected_challenge {
        return Err(HandshakeError::ChallengeMismatch);
    }
    if proof.claimant_transport_peer != expected_claimant_transport_peer.to_bytes()
        || proof.receiver_transport_peer != local_receiver_transport_peer.to_bytes()
    {
        return Err(HandshakeError::TransportBindingMismatch);
    }
    let key = VerifyingKey::from_bytes(&proof.hello.public_key).map_err(|_| HandshakeError::SignatureInvalid)?;
    let signature = Signature::from_slice(&proof.signature).map_err(|_| HandshakeError::SignatureInvalid)?;
    key.verify(&peer_hello_proof_signing_bytes(proof)?, &signature).map_err(|_| HandshakeError::SignatureInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;
    use std::time::Duration;
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

    #[test]
    fn signed_hello_authenticates_application_identity() {
        verify_peer_hello(&signed_hello().0).unwrap();
    }

    #[test]
    fn captured_proof_cannot_move_to_attacker_transport() {
        let (hello_b, key_b) = signed_hello();
        let transport_a = generate_transport_key().public().to_peer_id();
        let transport_b = generate_transport_key().public().to_peer_id();
        let transport_c = generate_transport_key().public().to_peer_id();
        let challenge = [0xA5; 32];
        let captured = build_peer_hello_proof(&hello_b, &key_b, challenge, &transport_b, &transport_a).unwrap();
        verify_peer_hello_proof(&captured, challenge, &transport_b, &transport_a).unwrap();
        assert_eq!(
            verify_peer_hello_proof(&captured, challenge, &transport_c, &transport_a),
            Err(HandshakeError::TransportBindingMismatch)
        );
    }

    #[tokio::test]
    async fn two_quic_nodes_authenticate_each_other() {
        let (hello_a, key_a) = signed_hello();
        let (hello_b, key_b) = signed_hello();
        let app_a = hello_a.peer_id;
        let app_b = hello_b.peer_id;
        let mut node_a = SwarmNode::new(generate_transport_key(), hello_a, key_a).unwrap();
        let mut node_b = SwarmNode::new(generate_transport_key(), hello_b, key_b).unwrap();
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

        assert_eq!(node_a.connectivity_diagnostics().state, ConnectivityStateV1::DirectReachable);
        assert_eq!(node_b.connectivity_diagnostics().state, ConnectivityStateV1::DirectReachable);
    }
}
