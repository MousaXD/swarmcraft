from pathlib import Path
import re


def read(path):
    return Path(path).read_text()


def write(path, text):
    Path(path).write_text(text)


def replace_once(path, old, new):
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one target, found {count}: {old[:100]!r}")
    write(path, text.replace(old, new, 1))


def regex_once(path, pattern, replacement):
    text = read(path)
    text, count = re.subn(pattern, replacement, text, count=1, flags=re.S)
    if count != 1:
        raise SystemExit(f"{path}: regex expected one target, found {count}: {pattern[:100]!r}")
    write(path, text)


# The network layer needs entropy for per-connection receiver challenges.
replace_once(
    "crates/swarm-network/Cargo.toml",
    "libp2p.workspace = true\nserde.workspace = true",
    "libp2p.workspace = true\nrand_core.workspace = true\nserde.workspace = true",
)
replace_once(
    "crates/swarm-network/Cargo.toml",
    "[dev-dependencies]\nrand_core.workspace = true\ntempfile.workspace = true",
    "[dev-dependencies]\ntempfile.workspace = true",
)

# PeerIdentity hands an opaque SigningKey clone to the in-process network
# layer. The secret bytes are never serialized or written by networking.
replace_once(
    "crates/swarm-core/src/lib.rs",
    """    pub fn sign(&self, bytes: &[u8]) -> Vec<u8> {
        self.signing_key.sign(bytes).to_bytes().to_vec()
    }

""",
    """    pub fn sign(&self, bytes: &[u8]) -> Vec<u8> {
        self.signing_key.sign(bytes).to_bytes().to_vec()
    }

    /// Clone the in-process application signing key for connection-bound
    /// network authentication. Networking never persists or serializes it.
    pub fn network_signing_key(&self) -> SigningKey {
        self.signing_key.clone()
    }

""",
)

# Append challenge/proof wire variants so all existing postcard enum
# discriminants remain stable.
replace_once(
    "crates/swarm-network/src/wire.rs",
    "pub const MAX_DISCOVERY_ANNOUNCEMENT_BYTES: usize = 16 * 1024;",
    "pub const MAX_DISCOVERY_ANNOUNCEMENT_BYTES: usize = 16 * 1024;\npub const MAX_HANDSHAKE_TRANSPORT_ID_BYTES: usize = 128;",
)
replace_once(
    "crates/swarm-network/src/wire.rs",
    """#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireRequest {
""",
    """#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHelloProofV1 {
    pub hello: PeerHelloV1,
    pub challenge: [u8; 32],
    pub claimant_transport_peer: Vec<u8>,
    pub receiver_transport_peer: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireRequest {
""",
)
replace_once(
    "crates/swarm-network/src/wire.rs",
    """    FriendPresence { expected_peer_id: PeerId, requester_peer_id: PeerId, nonce: [u8; 32] },
}
""",
    """    FriendPresence { expected_peer_id: PeerId, requester_peer_id: PeerId, nonce: [u8; 32] },
    // Connection-bound authentication extensions are append-only.
    HelloChallenge { challenge: [u8; 32] },
    HelloProof(Box<PeerHelloProofV1>),
}
""",
)
replace_once(
    "crates/swarm-network/src/wire.rs",
    """            | Self::DiscoveryResolve { .. }
            | Self::FriendPresence { .. } => None,
""",
    """            | Self::DiscoveryResolve { .. }
            | Self::FriendPresence { .. }
            | Self::HelloChallenge { .. }
            | Self::HelloProof(_) => None,
""",
)
replace_once(
    "crates/swarm-network/src/wire.rs",
    """            Self::DiscoveryPublic { filter } => validate_discovery_filter(filter),
            _ => Ok(()),
""",
    """            Self::DiscoveryPublic { filter } => validate_discovery_filter(filter),
            Self::HelloProof(proof)
                if proof.claimant_transport_peer.len() > MAX_HANDSHAKE_TRANSPORT_ID_BYTES
                    || proof.receiver_transport_peer.len() > MAX_HANDSHAKE_TRANSPORT_ID_BYTES =>
            {
                Err(WireLimitError::HandshakeTransportIdTooLarge)
            }
            _ => Ok(()),
""",
)
replace_once(
    "crates/swarm-network/src/wire.rs",
    """    FriendPresence(Option<FriendPresenceV1>),
}
""",
    """    FriendPresence(Option<FriendPresenceV1>),
    HelloChallengeAccepted,
}
""",
)
replace_once(
    "crates/swarm-network/src/wire.rs",
    """    #[error(\"world discovery announcement is {0} encoded bytes; maximum is {MAX_DISCOVERY_ANNOUNCEMENT_BYTES}\")]
    DiscoveryAnnouncementTooLarge(usize),
}
""",
    """    #[error(\"world discovery announcement is {0} encoded bytes; maximum is {MAX_DISCOVERY_ANNOUNCEMENT_BYTES}\")]
    DiscoveryAnnouncementTooLarge(usize),
    #[error(\"handshake transport peer identifier exceeds {MAX_HANDSHAKE_TRANSPORT_ID_BYTES} bytes\")]
    HandshakeTransportIdTooLarge,
}
""",
)

# Cryptographic application proof bound to a receiver challenge and both Noise
# transport identities.
replace_once(
    "crates/swarm-network/src/lib.rs",
    """pub use wire::{
    BlobResumeV1, HostCapabilityV1, HostRuntimeReadinessV1, ReplicaAckV1, ServerModsReadinessV1, WireLimitError,
""",
    """pub use wire::{
    BlobResumeV1, HostCapabilityV1, HostRuntimeReadinessV1, PeerHelloProofV1, ReplicaAckV1, ServerModsReadinessV1,
    WireLimitError,
""",
)
replace_once(
    "crates/swarm-network/src/lib.rs",
    "use ed25519_dalek::{Signature, Verifier, VerifyingKey};",
    "use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};",
)
replace_once(
    "crates/swarm-network/src/lib.rs",
    """    #[error(\"peer hello cannot be encoded\")]
    EncodingFailed,
}
""",
    """    #[error(\"peer hello cannot be encoded\")]
    EncodingFailed,
    #[error(\"connection authentication challenge does not match the live receiver challenge\")]
    ChallengeMismatch,
    #[error(\"application proof is bound to a different transport connection\")]
    TransportBindingMismatch,
    #[error(\"application signing key does not match the advertised peer identity\")]
    SigningKeyMismatch,
}
""",
)
replace_once(
    "crates/swarm-network/src/lib.rs",
    """pub fn verify_peer_hello(hello: &PeerHelloV1) -> Result<(), HandshakeError> {
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
""",
    """pub fn verify_peer_hello(hello: &PeerHelloV1) -> Result<(), HandshakeError> {
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

const PEER_HELLO_PROOF_DOMAIN: &[u8] = b\"swarmcraft/peer-hello-connection-proof/v1\\0\";

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
    key.verify(&peer_hello_proof_signing_bytes(proof)?, &signature)
        .map_err(|_| HandshakeError::SignatureInvalid)
}
""",
)

# Replace the small lib.rs test module so constructors carry the live signing
# key and add a direct three-transport replay regression.
regex_once(
    "crates/swarm-network/src/lib.rs",
    r"#\[cfg\(test\)\]\nmod tests \{.*\}\n$",
    """#[cfg(test)]
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
            capabilities: vec![\"snapshot-replication-v1\".into()],
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
        node_a.listen(\"/ip4/127.0.0.1/udp/0/quic-v1\".parse().unwrap()).unwrap();

        let address = timeout(Duration::from_secs(10), async {
            loop {
                if let NetworkEvent::Listening { address } = node_a.next_event().await.unwrap() {
                    break address;
                }
            }
        })
        .await
        .expect(\"node A should listen\");
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
        .expect(\"both QUIC peers should authenticate\");

        assert_eq!(node_a.connectivity_diagnostics().state, ConnectivityStateV1::DirectReachable);
        assert_eq!(node_b.connectivity_diagnostics().state, ConnectivityStateV1::DirectReachable);
    }
}
""",
)

# Main SwarmNode connection-bound handshake state.
replace_once(
    "crates/swarm-network/src/node.rs",
    """use crate::{
    verify_peer_hello, wire::WireRequest, wire::WireResponse, ConnectivityDiagnosticsV1, ConnectivityIssueKindV1,
    ConnectivityIssueV1, NatStatusV1,
};
""",
    """use crate::{
    build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, wire::WireRequest, wire::WireResponse,
    ConnectivityDiagnosticsV1, ConnectivityIssueKindV1, ConnectivityIssueV1, NatStatusV1,
};
""",
)
replace_once("crates/swarm-network/src/node.rs", "use anyhow::{anyhow, Context, Result};", "use anyhow::{anyhow, Context, Result};\nuse ed25519_dalek::SigningKey;")
replace_once(
    "crates/swarm-network/src/node.rs",
    """use std::{
    collections::{HashMap, HashSet},
    env,
    time::Duration,
};
""",
    """use rand_core::{OsRng, RngCore};
use std::{
    collections::{HashMap, HashSet},
    env,
    time::Duration,
};
""",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    """    local_hello: PeerHelloV1,
    authenticated: HashMap<TransportPeerId, PeerId>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
""",
    """    local_hello: PeerHelloV1,
    application_signing_key: SigningKey,
    authenticated: HashMap<TransportPeerId, (PeerId, ConnectionId)>,
    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32])>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    connection_counts: HashMap<TransportPeerId, usize>,
""",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    """    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1) -> Result<Self> {
        verify_peer_hello(&local_hello).context(\"local peer hello must be valid before networking starts\")?;

        let local_peer = transport_key.public().to_peer_id();
""",
    """    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1, application_signing_key: SigningKey) -> Result<Self> {
        verify_peer_hello(&local_hello).context(\"local peer hello must be valid before networking starts\")?;

        let local_peer = transport_key.public().to_peer_id();
        let self_test = build_peer_hello_proof(
            &local_hello,
            &application_signing_key,
            [0; 32],
            &local_peer,
            &local_peer,
        )?;
        verify_peer_hello_proof(&self_test, [0; 32], &local_peer, &local_peer)
            .context(\"application signing key must match the local PeerHello\")?;
""",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    """            swarm,
            local_hello,
            authenticated: HashMap::new(),
            active_connections: HashMap::new(),
            connection_paths: HashMap::new(),
""",
    """            swarm,
            local_hello,
            application_signing_key,
            authenticated: HashMap::new(),
            pending_challenges: HashMap::new(),
            active_connections: HashMap::new(),
            connection_counts: HashMap::new(),
            connection_paths: HashMap::new(),
""",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    """    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).copied()
    }
""",
    """    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).map(|(peer, _)| *peer)
    }
""",
)
regex_once(
    "crates/swarm-network/src/node.rs",
    r"                SwarmEvent::ConnectionEstablished \{ peer_id, connection_id, endpoint, num_established, \.\. \} => \{.*?                SwarmEvent::OutgoingConnectionError",
    """                SwarmEvent::ConnectionEstablished { peer_id, connection_id, endpoint, num_established, .. } => {
                    let path_kind = classify_connection_path(
                        self.bootstrap_peers.contains(&peer_id),
                        self.relay_peers.contains(&peer_id),
                        endpoint.is_relayed(),
                    );
                    self.connection_paths.insert(connection_id, path_kind);
                    self.refresh_connectivity_paths();
                    if matches!(
                        path_kind,
                        ConnectionPathKind::DirectApplication | ConnectionPathKind::RelayedApplication
                    ) {
                        self.relay_fallbacks.remove(&peer_id);
                    }
                    self.connection_counts.insert(peer_id, num_established.get() as usize);
                    self.authenticated.remove(&peer_id);
                    self.pending_challenges.remove(&peer_id);
                    debug!(transport_peer = %peer_id, %connection_id, %num_established, relayed = endpoint.is_relayed(), ?path_kind, \"peer connected\");

                    // request-response selects by peer ID. During replacement races,
                    // close the superseded route before issuing a fresh receiver
                    // challenge so proof traffic can only use the canonical connection.
                    let previous = self.active_connections.insert(peer_id, connection_id);
                    let defer_challenge = previous
                        .filter(|previous| *previous != connection_id && num_established.get() > 1)
                        .is_some_and(|previous| self.swarm.close_connection(previous));
                    if !defer_challenge {
                        self.issue_auth_challenge(peer_id, connection_id)?;
                    }
                    return Ok(NetworkEvent::Connected { transport_peer: peer_id });
                }
                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    self.connection_paths.remove(&connection_id);
                    self.refresh_connectivity_paths();
                    self.connection_counts.insert(peer_id, num_established as usize);
                    if self.authenticated.get(&peer_id).is_some_and(|(_, authenticated_connection)| *authenticated_connection == connection_id) {
                        self.authenticated.remove(&peer_id);
                    }
                    if self.pending_challenges.get(&peer_id).is_some_and(|(challenge_connection, _)| *challenge_connection == connection_id) {
                        self.pending_challenges.remove(&peer_id);
                    }
                    if num_established == 0 {
                        self.active_connections.remove(&peer_id);
                        self.connection_counts.remove(&peer_id);
                        self.authenticated.remove(&peer_id);
                        self.pending_challenges.remove(&peer_id);
                        return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });
                    }

                    if let Some(active) = self
                        .active_connections
                        .get(&peer_id)
                        .copied()
                        .filter(|active| *active != connection_id)
                    {
                        self.issue_auth_challenge(peer_id, active)?;
                    }
                    debug!(transport_peer = %peer_id, %connection_id, remaining_connections = num_established, \"peer connection closed; replacement requires fresh application proof\");
                }
                SwarmEvent::OutgoingConnectionError""",
)
regex_once(
    "crates/swarm-network/src/node.rs",
    r"                    request_response::Event::Message \{ peer, message, \.\. \} => match message \{.*?                    request_response::Event::OutboundFailure",
    """                    request_response::Event::Message { peer, connection_id, message } => match message {
                        request_response::Message::Request { request, channel, .. } => {
                            if let Err(error) = request.validate_limits() {
                                let response = WireResponse::Error {
                                    code: \"REQUEST_LIMIT_EXCEEDED\".into(),
                                    message: error.to_string(),
                                };
                                if let Err(response_error) = self.respond(channel, response) {
                                    warn!(
                                        transport_peer = %peer,
                                        error = %response_error,
                                        \"failed to send request limit error; continuing network loop\"
                                    );
                                }
                                continue;
                            }
                            match request {
                                WireRequest::Hello(_) => {
                                    let _ = self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: \"CONNECTION_PROOF_REQUIRED\".into(),
                                            message: \"reusable PeerHello is not an authentication proof; wait for a receiver challenge\".into(),
                                        },
                                    );
                                }
                                WireRequest::HelloChallenge { challenge } => {
                                    let canonical = self.active_connections.get(&peer).is_some_and(|active| *active == connection_id)
                                        && self.connection_counts.get(&peer).copied() == Some(1);
                                    if !canonical {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: \"AUTH_CONNECTION_RETRY\".into(),
                                                message: \"authentication challenge arrived on a superseded connection\".into(),
                                            },
                                        );
                                        continue;
                                    }
                                    let local_transport = *self.swarm.local_peer_id();
                                    let proof = build_peer_hello_proof(
                                        &self.local_hello,
                                        &self.application_signing_key,
                                        challenge,
                                        &local_transport,
                                        &peer,
                                    )?;
                                    self.respond(channel, WireResponse::HelloChallengeAccepted)?;
                                    self.swarm
                                        .behaviour_mut()
                                        .request_response
                                        .send_request(&peer, WireRequest::HelloProof(Box::new(proof)));
                                }
                                WireRequest::HelloProof(proof) => {
                                    let expected = self.pending_challenges.remove(&peer);
                                    let Some((challenge_connection, expected_challenge)) = expected else {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: \"PEER_AUTHENTICATION_FAILED\".into(),
                                                message: \"no live receiver challenge exists for this proof\".into(),
                                            },
                                        );
                                        continue;
                                    };
                                    if challenge_connection != connection_id
                                        || self.active_connections.get(&peer).is_none_or(|active| *active != connection_id)
                                        || self.connection_counts.get(&peer).copied() != Some(1)
                                    {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: \"PEER_AUTHENTICATION_FAILED\".into(),
                                                message: \"connection was replaced before proof verification\".into(),
                                            },
                                        );
                                        continue;
                                    }
                                    match verify_peer_hello_proof(
                                        &proof,
                                        expected_challenge,
                                        &peer,
                                        self.swarm.local_peer_id(),
                                    ) {
                                        Ok(()) => {
                                            self.authenticated.insert(peer, (proof.hello.peer_id, connection_id));
                                            if let Err(response_error) = self.respond(
                                                channel,
                                                WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                            ) {
                                                warn!(
                                                    transport_peer = %peer,
                                                    error = %response_error,
                                                    \"peer proof response channel closed; continuing network loop\"
                                                );
                                            }
                                            return Ok(NetworkEvent::Authenticated {
                                                transport_peer: peer,
                                                application_peer: proof.hello.peer_id,
                                            });
                                        }
                                        Err(error) => {
                                            let _ = self.respond(
                                                channel,
                                                WireResponse::Error {
                                                    code: \"PEER_AUTHENTICATION_FAILED\".into(),
                                                    message: error.to_string(),
                                                },
                                            );
                                        }
                                    }
                                }
                                request if self.authenticated.get(&peer).is_some_and(|(_, authenticated_connection)| *authenticated_connection == connection_id) => {
                                    return Ok(NetworkEvent::InboundRequest { transport_peer: peer, request, channel });
                                }
                                _ => {
                                    if let Err(response_error) = self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: \"HANDSHAKE_REQUIRED\".into(),
                                            message: \"complete the connection-bound application proof before other requests\".into(),
                                        },
                                    ) {
                                        warn!(
                                            transport_peer = %peer,
                                            error = %response_error,
                                            \"handshake-required response channel closed; continuing network loop\"
                                        );
                                    }
                                }
                            }
                        }
                        request_response::Message::Response { request_id, response } => {
                            return Ok(NetworkEvent::Response { transport_peer: peer, request_id, response });
                        }
                    },
                    request_response::Event::OutboundFailure""",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    """    fn refresh_connectivity_paths(&mut self) {
""",
    """    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        let mut challenge = [0_u8; 32];
        OsRng.fill_bytes(&mut challenge);
        self.authenticated.remove(&peer);
        self.pending_challenges.insert(peer, (connection_id, challenge));
        self.swarm
            .behaviour_mut()
            .request_response
            .send_request(&peer, WireRequest::HelloChallenge { challenge });
        Ok(())
    }

    fn refresh_connectivity_paths(&mut self) {
""",
)

# DiscoveryNode uses the same connection-bound proof semantics.
replace_once("crates/swarm-network/src/discovery.rs", "use anyhow::{anyhow, Context, Result};", "use anyhow::{anyhow, Context, Result};\nuse ed25519_dalek::SigningKey;")
replace_once(
    "crates/swarm-network/src/discovery.rs",
    """    swarm::{dial_opts::DialOpts, NetworkBehaviour, SwarmEvent},
""",
    """    swarm::{dial_opts::DialOpts, ConnectionId, NetworkBehaviour, SwarmEvent},
""",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "use std::{collections::HashMap, env, time::Duration};",
    "use rand_core::{OsRng, RngCore};\nuse std::{collections::HashMap, env, time::Duration};",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "use crate::{verify_peer_hello, WireRequest, WireResponse, BOOTSTRAP_ENV};",
    "use crate::{build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, WireRequest, WireResponse, BOOTSTRAP_ENV};",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    """    local_hello: PeerHelloV1,
    authenticated: HashMap<TransportPeerId, PeerId>,
}

impl DiscoveryNode {
    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1) -> Result<Self> {
        verify_peer_hello(&local_hello).context(\"local discovery PeerHello must be valid\")?;
        let local_peer = transport_key.public().to_peer_id();
""",
    """    local_hello: PeerHelloV1,
    application_signing_key: SigningKey,
    authenticated: HashMap<TransportPeerId, (PeerId, ConnectionId)>,
    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32])>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    connection_counts: HashMap<TransportPeerId, usize>,
}

impl DiscoveryNode {
    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1, application_signing_key: SigningKey) -> Result<Self> {
        verify_peer_hello(&local_hello).context(\"local discovery PeerHello must be valid\")?;
        let local_peer = transport_key.public().to_peer_id();
        let self_test = build_peer_hello_proof(
            &local_hello,
            &application_signing_key,
            [0; 32],
            &local_peer,
            &local_peer,
        )?;
        verify_peer_hello_proof(&self_test, [0; 32], &local_peer, &local_peer)
            .context(\"application signing key must match the local discovery PeerHello\")?;
""",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    """        let mut node = Self { swarm, local_hello, authenticated: HashMap::new() };
""",
    """        let mut node = Self {
            swarm,
            local_hello,
            application_signing_key,
            authenticated: HashMap::new(),
            pending_challenges: HashMap::new(),
            active_connections: HashMap::new(),
            connection_counts: HashMap::new(),
        };
""",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    """    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).copied()
    }
""",
    """    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).map(|(peer, _)| *peer)
    }
""",
)
regex_once(
    "crates/swarm-network/src/discovery.rs",
    r"                SwarmEvent::ConnectionEstablished \{ peer_id, \.\. \} => \{.*?                SwarmEvent::Behaviour\(DiscoveryBehaviourEvent::Mdns",
    """                SwarmEvent::ConnectionEstablished { peer_id, connection_id, num_established, .. } => {
                    self.connection_counts.insert(peer_id, num_established.get() as usize);
                    self.authenticated.remove(&peer_id);
                    self.pending_challenges.remove(&peer_id);
                    let previous = self.active_connections.insert(peer_id, connection_id);
                    let defer_challenge = previous
                        .filter(|previous| *previous != connection_id && num_established.get() > 1)
                        .is_some_and(|previous| self.swarm.close_connection(previous));
                    if !defer_challenge {
                        self.issue_auth_challenge(peer_id, connection_id)?;
                    }
                }
                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    self.connection_counts.insert(peer_id, num_established as usize);
                    if self.authenticated.get(&peer_id).is_some_and(|(_, authenticated_connection)| *authenticated_connection == connection_id) {
                        self.authenticated.remove(&peer_id);
                    }
                    if self.pending_challenges.get(&peer_id).is_some_and(|(challenge_connection, _)| *challenge_connection == connection_id) {
                        self.pending_challenges.remove(&peer_id);
                    }
                    if num_established == 0 {
                        self.active_connections.remove(&peer_id);
                        self.connection_counts.remove(&peer_id);
                        let application_peer = self.authenticated.remove(&peer_id).map(|(peer, _)| peer);
                        self.pending_challenges.remove(&peer_id);
                        return Ok(DiscoveryNetworkEvent::Disconnected { transport_peer: peer_id, application_peer });
                    }
                    if let Some(active) = self.active_connections.get(&peer_id).copied().filter(|active| *active != connection_id) {
                        self.issue_auth_challenge(peer_id, active)?;
                    }
                }
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Mdns""",
)
regex_once(
    "crates/swarm-network/src/discovery.rs",
    r"                    request_response::Event::Message \{ peer, message, \.\. \} => match message \{.*?                    request_response::Event::OutboundFailure",
    """                    request_response::Event::Message { peer, connection_id, message } => match message {
                        request_response::Message::Request { request, channel, .. } => {
                            if let Err(error) = request.validate_limits() {
                                let _ = self.respond(
                                    channel,
                                    WireResponse::Error {
                                        code: \"REQUEST_LIMIT_EXCEEDED\".into(),
                                        message: error.to_string(),
                                    },
                                );
                                continue;
                            }
                            match request {
                                WireRequest::Hello(_) => {
                                    let _ = self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: \"CONNECTION_PROOF_REQUIRED\".into(),
                                            message: \"reusable PeerHello is not an authentication proof\".into(),
                                        },
                                    );
                                }
                                WireRequest::HelloChallenge { challenge } => {
                                    let canonical = self.active_connections.get(&peer).is_some_and(|active| *active == connection_id)
                                        && self.connection_counts.get(&peer).copied() == Some(1);
                                    if !canonical {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: \"AUTH_CONNECTION_RETRY\".into(),
                                                message: \"authentication challenge arrived on a superseded connection\".into(),
                                            },
                                        );
                                        continue;
                                    }
                                    let local_transport = *self.swarm.local_peer_id();
                                    let proof = build_peer_hello_proof(
                                        &self.local_hello,
                                        &self.application_signing_key,
                                        challenge,
                                        &local_transport,
                                        &peer,
                                    )?;
                                    self.respond(channel, WireResponse::HelloChallengeAccepted)?;
                                    self.swarm
                                        .behaviour_mut()
                                        .request_response
                                        .send_request(&peer, WireRequest::HelloProof(Box::new(proof)));
                                }
                                WireRequest::HelloProof(proof) => {
                                    let Some((challenge_connection, expected_challenge)) = self.pending_challenges.remove(&peer) else {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: \"PEER_AUTHENTICATION_FAILED\".into(),
                                                message: \"no live receiver challenge exists for this proof\".into(),
                                            },
                                        );
                                        continue;
                                    };
                                    if challenge_connection != connection_id
                                        || self.active_connections.get(&peer).is_none_or(|active| *active != connection_id)
                                        || self.connection_counts.get(&peer).copied() != Some(1)
                                    {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: \"PEER_AUTHENTICATION_FAILED\".into(),
                                                message: \"connection was replaced before proof verification\".into(),
                                            },
                                        );
                                        continue;
                                    }
                                    match verify_peer_hello_proof(
                                        &proof,
                                        expected_challenge,
                                        &peer,
                                        self.swarm.local_peer_id(),
                                    ) {
                                        Ok(()) => {
                                            self.authenticated.insert(peer, (proof.hello.peer_id, connection_id));
                                            self.respond(
                                                channel,
                                                WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                            )?;
                                            return Ok(DiscoveryNetworkEvent::Authenticated {
                                                transport_peer: peer,
                                                application_peer: proof.hello.peer_id,
                                            });
                                        }
                                        Err(error) => {
                                            let _ = self.respond(
                                                channel,
                                                WireResponse::Error {
                                                    code: \"PEER_AUTHENTICATION_FAILED\".into(),
                                                    message: error.to_string(),
                                                },
                                            );
                                        }
                                    }
                                }
                                request => {
                                    if let Some((application_peer, _)) = self
                                        .authenticated
                                        .get(&peer)
                                        .copied()
                                        .filter(|(_, authenticated_connection)| *authenticated_connection == connection_id)
                                    {
                                        return Ok(DiscoveryNetworkEvent::InboundRequest {
                                            transport_peer: peer,
                                            application_peer,
                                            request,
                                            channel,
                                        });
                                    }
                                    let _ = self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: \"HANDSHAKE_REQUIRED\".into(),
                                            message: \"complete the connection-bound application proof before discovery requests\".into(),
                                        },
                                    );
                                }
                            }
                        }
                        request_response::Message::Response { request_id, response } => {
                            if let Err(error) = response.validate_limits() {
                                return Ok(DiscoveryNetworkEvent::OutboundFailure {
                                    transport_peer: peer,
                                    request_id,
                                    error: error.to_string(),
                                });
                            }
                            return Ok(DiscoveryNetworkEvent::Response { transport_peer: peer, request_id, response });
                        }
                    },
                    request_response::Event::OutboundFailure""",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    """}

pub fn public_directory_key() -> kad::RecordKey {
""",
    """    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        let mut challenge = [0_u8; 32];
        OsRng.fill_bytes(&mut challenge);
        self.authenticated.remove(&peer);
        self.pending_challenges.insert(peer, (connection_id, challenge));
        self.swarm
            .behaviour_mut()
            .request_response
            .send_request(&peer, WireRequest::HelloChallenge { challenge });
        Ok(())
    }
}

pub fn public_directory_key() -> kad::RecordKey {
""",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    """    use super::*;
    use swarm_protocol::{peer_id_from_public_key, PeerHelloV1};
""",
    """    use super::*;
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;
    use swarm_protocol::{peer_id_from_public_key, PeerHelloV1};
""",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    """        assert!(DiscoveryNode::new(Keypair::generate_ed25519(), hello).is_err());
""",
    """        assert!(DiscoveryNode::new(Keypair::generate_ed25519(), hello, SigningKey::generate(&mut OsRng)).is_err());
""",
)

# Production constructors now pass the live application signing key.
replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "let mut node = SwarmNode::new(transport_key, hello)?;",
    "let mut node = SwarmNode::new(transport_key, hello, identity.network_signing_key())?;",
)
replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "\"transport connected; waiting for signed PeerHello\"",
    "\"transport connected; waiting for connection-bound application proof\"",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "let mut node = DiscoveryNode::new(transport_key, hello)?;",
    "let mut node = DiscoveryNode::new(transport_key, hello, identity.network_signing_key())?;",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "let mut node = DiscoveryNode::new(generate_transport_key(), hello)?;",
    "let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key())?;",
)

# Example constructor.
replace_once(
    "crates/swarm-network/examples/connectivity_probe.rs",
    "fn signed_probe_hello() -> PeerHelloV1 {",
    "fn signed_probe_hello() -> (PeerHelloV1, SigningKey) {",
)
replace_once(
    "crates/swarm-network/examples/connectivity_probe.rs",
    """    hello.signature = key.sign(&hello.signing_bytes().expect(\"probe hello should encode\")).to_bytes().to_vec();
    hello
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut node = SwarmNode::new(generate_transport_key(), signed_probe_hello())?;
""",
    """    hello.signature = key.sign(&hello.signing_bytes().expect(\"probe hello should encode\")).to_bytes().to_vec();
    (hello, key)
}

#[tokio::main]
async fn main() -> Result<()> {
    let (hello, signing_key) = signed_probe_hello();
    let mut node = SwarmNode::new(generate_transport_key(), hello, signing_key)?;
""",
)

# Hostile input tests use a helper that retains the application key.
replace_once(
    "crates/swarm-network/tests/input_hardening.rs",
    "fn signed_hello() -> PeerHelloV1 {",
    "fn signed_hello() -> (PeerHelloV1, SigningKey) {",
)
replace_once(
    "crates/swarm-network/tests/input_hardening.rs",
    """    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

async fn listen_address""",
    """    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    (hello, key)
}

fn new_node() -> SwarmNode {
    let (hello, signing_key) = signed_hello();
    SwarmNode::new(generate_transport_key(), hello, signing_key).unwrap()
}

async fn listen_address""",
)
text = read("crates/swarm-network/tests/input_hardening.rs")
text = text.replace("SwarmNode::new(generate_transport_key(), signed_hello()).unwrap()", "new_node()")
write("crates/swarm-network/tests/input_hardening.rs", text)

# Reconnect acceptance test retains keys explicitly across restart.
for old, new in [
    ("SwarmNode::new(transport_a_key, hello_a.clone()).unwrap()", "SwarmNode::new(transport_a_key, hello_a.clone(), app_key_a.clone()).unwrap()"),
    ("SwarmNode::new(transport_b_key, hello_b.clone()).unwrap()", "SwarmNode::new(transport_b_key, hello_b.clone(), app_key_b.clone()).unwrap()"),
    ("SwarmNode::new(replacement_key, signed_hello(&app_key_b, 3)).unwrap()", "SwarmNode::new(replacement_key, signed_hello(&app_key_b, 3), app_key_b.clone()).unwrap()"),
    ("SwarmNode::new(generate_transport_key(), hello).unwrap()", "SwarmNode::new(generate_transport_key(), hello, app_key.clone()).unwrap()"),
]:
    replace_once("crates/swarm-network/tests/peer_networking_acceptance.rs", old, new)

# Transfer soak retains sender/receiver application keys already.
replace_once(
    "crates/swarm-network/tests/network_transfer_soak.rs",
    "SwarmNode::new(transport_key, signed_hello(app_key, nonce)).unwrap()",
    "SwarmNode::new(transport_key, signed_hello(app_key, nonce), app_key.clone()).unwrap()",
)
replace_once(
    "crates/swarm-network/tests/network_transfer_soak.rs",
    "SwarmNode::new(receiver_key, receiver_hello).unwrap()",
    "SwarmNode::new(receiver_key, receiver_hello, receiver_app_key.clone()).unwrap()",
)
