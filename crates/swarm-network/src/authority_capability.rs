//! Inert, signed authority-capability session foundation for policy-enabled worlds.
//!
//! V1 PeerHello capabilities and HostCapabilityV1 are NOT authority proofs.
//! This module defines a separate mandatory V2 authority transport transcript
//! for future integration; the live node still offers only /swarmcraft/1.
//! No legacy authority path is modified or permitted by this module alone.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerId, WorldId};
use thiserror::Error;

/// This MUST be negotiated as a real independent wire protocol and must not
/// be inferred from PeerHelloV1.capabilities or an arbitrary string.
pub const MANDATORY_AUTHORITY_WIRE_PROTOCOL: &str = "/swarmcraft/hosting-authority/2";
pub const MANDATORY_AUTHORITY_FEATURE_VERSION: u16 = 2;
pub const MAX_AUTHORITY_SESSION_AGE_MS: u64 = 30_000;
pub const MAX_AUTHORITY_TRANSPORT_ID_BYTES: usize = 128;
/// These limits must match the canonical Agent 02 hosting policy before V2 activation.
pub const MIN_V2_AUTHORITY_HEAP_MIB: u32 = 512;
pub const MAX_V2_AUTHORITY_HEAP_MIB: u32 = 65_536;
const AUTHORITY_FEATURE_SIGN_DOMAIN: &[u8] = b"swarmcraft/authority-feature-session/v2\0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthorityCapabilityError {
    #[error("V1 transport or capability advertisement is not mandatory authority negotiation")]
    UnsupportedTransport,
    #[error("unsupported authority feature version")]
    UnsupportedFeature,
    #[error("authority candidate identity is invalid")]
    InvalidIdentity,
    #[error("invalid or unbounded authority session transcript")]
    InvalidTranscript,
    #[error("the signed authority session does not match the current canonical head and live connection")]
    WrongContext,
    #[error("authority challenge is expired or not current")]
    StaleChallenge,
    #[error("authority capability signature is invalid")]
    InvalidSignature,
    #[error("authority challenge was already consumed or was never issued")]
    ReplayOrUnknownChallenge,
}

/// Proposed transcript for a *separate* wire protocol. It is NOT appended to
/// the legacy postcard WireRequest/WireResponse enums, and is not runnable
/// until the V2 transport and canonical policy integration are implemented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAuthorityFeatureV2 {
    pub feature_version: u16,
    pub world_id: WorldId,
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub committed_policy_hash: Hash32,
    pub committed_migration_hash: Hash32,
    pub accepted_membership_hash: Hash32,
    pub snapshot_hash: Hash32,
    pub selected_heap_mib: u32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub consent_revision: u64,
    pub challenge: [u8; 32],
    pub claimant_transport_peer: Vec<u8>,
    pub verifier_transport_peer: Vec<u8>,
    pub issued_unix_ms: u64,
    pub expires_unix_ms: u64,
    pub signature: Vec<u8>,
}

impl SignedAuthorityFeatureV2 {
    fn validate_transcript(&self) -> Result<(), AuthorityCapabilityError> {
        if self.feature_version != MANDATORY_AUTHORITY_FEATURE_VERSION {
            return Err(AuthorityCapabilityError::UnsupportedFeature);
        }
        if peer_id_from_public_key(&self.public_key) != self.peer_id {
            return Err(AuthorityCapabilityError::InvalidIdentity);
        }
        if self.world_id == WorldId([0; 32])
            || self.claimant_transport_peer.is_empty()
            || self.verifier_transport_peer.is_empty()
            || self.claimant_transport_peer.len() > MAX_AUTHORITY_TRANSPORT_ID_BYTES
            || self.verifier_transport_peer.len() > MAX_AUTHORITY_TRANSPORT_ID_BYTES
            || self.committed_policy_hash == Hash32::default()
            || self.committed_migration_hash == Hash32::default()
            || self.accepted_membership_hash == Hash32::default()
            || self.snapshot_hash == Hash32::default()
            || !(MIN_V2_AUTHORITY_HEAP_MIB..=MAX_V2_AUTHORITY_HEAP_MIB)
                .contains(&self.selected_heap_mib)
            || self.challenge == [0; 32]
            || self.epoch == 0
            || self.fencing_token == 0
            || self.consent_revision == 0
            || self.expires_unix_ms <= self.issued_unix_ms
            || self.expires_unix_ms - self.issued_unix_ms > MAX_AUTHORITY_SESSION_AGE_MS
        {
            return Err(AuthorityCapabilityError::InvalidTranscript);
        }
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, AuthorityCapabilityError> {
        self.validate_transcript()?;
        let mut bytes = Vec::with_capacity(AUTHORITY_FEATURE_SIGN_DOMAIN.len() + 448);
        bytes.extend_from_slice(AUTHORITY_FEATURE_SIGN_DOMAIN);
        bytes.extend_from_slice(&self.feature_version.to_be_bytes());
        bytes.extend_from_slice(&self.world_id.0);
        bytes.extend_from_slice(&self.peer_id.0);
        bytes.extend_from_slice(&self.public_key);
        bytes.extend_from_slice(&self.committed_policy_hash.0);
        bytes.extend_from_slice(&self.committed_migration_hash.0);
        bytes.extend_from_slice(&self.accepted_membership_hash.0);
        bytes.extend_from_slice(&self.snapshot_hash.0);
        bytes.extend_from_slice(&self.selected_heap_mib.to_be_bytes());
        bytes.extend_from_slice(&self.epoch.to_be_bytes());
        bytes.extend_from_slice(&self.fencing_token.to_be_bytes());
        bytes.extend_from_slice(&self.consent_revision.to_be_bytes());
        bytes.extend_from_slice(&self.challenge);
        for peer in [&self.claimant_transport_peer, &self.verifier_transport_peer] {
            let len = u16::try_from(peer.len()).map_err(|_| AuthorityCapabilityError::InvalidTranscript)?;
            bytes.extend_from_slice(&len.to_be_bytes());
            bytes.extend_from_slice(peer);
        }
        bytes.extend_from_slice(&self.issued_unix_ms.to_be_bytes());
        bytes.extend_from_slice(&self.expires_unix_ms.to_be_bytes());
        Ok(bytes)
    }

    pub fn sign(&mut self, local_signing_key: &SigningKey) -> Result<(), AuthorityCapabilityError> {
        if local_signing_key.verifying_key().to_bytes() != self.public_key {
            return Err(AuthorityCapabilityError::InvalidIdentity);
        }
        self.signature = local_signing_key.sign(&self.signing_bytes()?).to_bytes().to_vec();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExpectedAuthorityFeatureV2<'a> {
    pub world_id: WorldId,
    pub peer_id: PeerId,
    /// Obtained from an independently verified *durable canonical* commitment,
    /// not from discovery, a sidecar or the remote peer.
    pub policy_hash: Hash32,
    pub migration_hash: Hash32,
    pub membership_hash: Hash32,
    pub snapshot_hash: Hash32,
    pub selected_heap_mib: u32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub consent_revision: u64,
    pub challenge: [u8; 32],
    pub claimant_transport_peer: &'a [u8],
    pub verifier_transport_peer: &'a [u8],
    pub now_unix_ms: u64,
}

/// Opaque result of verifying a single authenticated V2 authority transcript.
/// The caller MUST still validate canonical policy, local consent, fresh
/// resource readiness, complete snapshot, quorum, lease and fencing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedAuthorityFeatureSession {
    peer_id: PeerId,
    world_id: WorldId,
    policy_hash: Hash32,
    migration_hash: Hash32,
    membership_hash: Hash32,
    snapshot_hash: Hash32,
    selected_heap_mib: u32,
    epoch: u64,
    fencing_token: u64,
    consent_revision: u64,
    issued_unix_ms: u64,
    expires_unix_ms: u64,
}

impl VerifiedAuthorityFeatureSession {
    pub fn peer_id(self) -> PeerId {
        self.peer_id
    }
    pub fn world_id(self) -> WorldId {
        self.world_id
    }
    pub fn policy_hash(self) -> Hash32 {
        self.policy_hash
    }
    pub fn migration_hash(self) -> Hash32 {
        self.migration_hash
    }
    pub fn membership_hash(self) -> Hash32 {
        self.membership_hash
    }
    pub fn snapshot_hash(self) -> Hash32 {
        self.snapshot_hash
    }
    pub fn selected_heap_mib(self) -> u32 {
        self.selected_heap_mib
    }
    pub fn epoch(self) -> u64 {
        self.epoch
    }
    pub fn fencing_token(self) -> u64 {
        self.fencing_token
    }
    pub fn consent_revision(self) -> u64 {
        self.consent_revision
    }
    pub fn valid_at(self, now_unix_ms: u64) -> bool {
        now_unix_ms >= self.issued_unix_ms && now_unix_ms <= self.expires_unix_ms
    }
}

/// Must be called only for an actually negotiated V2 stream. The verifier
/// provides a connection-bound unpredictable challenge and atomically consumes
/// it exactly once after signature validation. A successful claim is NOT proof
/// of available RAM, enforcement by unknown binary code, or local consent.
pub fn verify_authority_feature_session(
    negotiated_wire_protocol: &str,
    claim: &SignedAuthorityFeatureV2,
    expected: &ExpectedAuthorityFeatureV2<'_>,
    consume_verifier_challenge: impl FnOnce() -> bool,
) -> Result<VerifiedAuthorityFeatureSession, AuthorityCapabilityError> {
    if negotiated_wire_protocol != MANDATORY_AUTHORITY_WIRE_PROTOCOL {
        return Err(AuthorityCapabilityError::UnsupportedTransport);
    }
    claim.validate_transcript()?;
    if claim.world_id != expected.world_id
        || claim.peer_id != expected.peer_id
        || claim.committed_policy_hash != expected.policy_hash
        || claim.committed_migration_hash != expected.migration_hash
        || claim.accepted_membership_hash != expected.membership_hash
        || claim.snapshot_hash != expected.snapshot_hash
        || claim.selected_heap_mib != expected.selected_heap_mib
        || claim.epoch != expected.epoch
        || claim.fencing_token != expected.fencing_token
        || claim.consent_revision != expected.consent_revision
        || claim.challenge != expected.challenge
        || claim.claimant_transport_peer != expected.claimant_transport_peer
        || claim.verifier_transport_peer != expected.verifier_transport_peer
    {
        return Err(AuthorityCapabilityError::WrongContext);
    }
    if expected.now_unix_ms < claim.issued_unix_ms || expected.now_unix_ms > claim.expires_unix_ms {
        return Err(AuthorityCapabilityError::StaleChallenge);
    }
    let verifying_key =
        VerifyingKey::from_bytes(&claim.public_key).map_err(|_| AuthorityCapabilityError::InvalidSignature)?;
    let signature = Signature::from_slice(&claim.signature).map_err(|_| AuthorityCapabilityError::InvalidSignature)?;
    verifying_key
        .verify(&claim.signing_bytes()?, &signature)
        .map_err(|_| AuthorityCapabilityError::InvalidSignature)?;
    if !consume_verifier_challenge() {
        return Err(AuthorityCapabilityError::ReplayOrUnknownChallenge);
    }
    Ok(VerifiedAuthorityFeatureSession {
        peer_id: claim.peer_id,
        world_id: claim.world_id,
        policy_hash: claim.committed_policy_hash,
        migration_hash: claim.committed_migration_hash,
        membership_hash: claim.accepted_membership_hash,
        snapshot_hash: claim.snapshot_hash,
        selected_heap_mib: claim.selected_heap_mib,
        epoch: claim.epoch,
        fencing_token: claim.fencing_token,
        consent_revision: claim.consent_revision,
        issued_unix_ms: claim.issued_unix_ms,
        expires_unix_ms: claim.expires_unix_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn claim() -> (SignedAuthorityFeatureV2, SigningKey) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let mut v = SignedAuthorityFeatureV2 {
            feature_version: MANDATORY_AUTHORITY_FEATURE_VERSION,
            world_id: WorldId([1; 32]),
            peer_id: peer_id_from_public_key(&key.verifying_key().to_bytes()),
            public_key: key.verifying_key().to_bytes(),
            committed_policy_hash: Hash32([2; 32]),
            committed_migration_hash: Hash32([10; 32]),
            accepted_membership_hash: Hash32([11; 32]),
            snapshot_hash: Hash32([12; 32]),
            selected_heap_mib: 4096,
            epoch: 3,
            fencing_token: 4,
            consent_revision: 5,
            challenge: [6; 32],
            claimant_transport_peer: b"claimant".to_vec(),
            verifier_transport_peer: b"verifier".to_vec(),
            issued_unix_ms: 100_000,
            expires_unix_ms: 110_000,
            signature: Vec::new(),
        };
        v.sign(&key).unwrap();
        (v, key)
    }

    fn expected<'a>(v: &SignedAuthorityFeatureV2) -> ExpectedAuthorityFeatureV2<'a> {
        ExpectedAuthorityFeatureV2 {
            world_id: v.world_id,
            peer_id: v.peer_id,
            policy_hash: v.committed_policy_hash,
            migration_hash: v.committed_migration_hash,
            membership_hash: v.accepted_membership_hash,
            snapshot_hash: v.snapshot_hash,
            selected_heap_mib: v.selected_heap_mib,
            epoch: v.epoch,
            fencing_token: v.fencing_token,
            consent_revision: v.consent_revision,
            challenge: v.challenge,
            claimant_transport_peer: b"claimant",
            verifier_transport_peer: b"verifier",
            now_unix_ms: 105_000,
        }
    }

    #[test]
    fn v1_and_capability_strings_cannot_grant_v2_authority() {
        let (v, _) = claim();
        for protocol in ["/swarmcraft/1", "swarmcraft/hosting-policy/v1", ""] {
            assert_eq!(
                verify_authority_feature_session(protocol, &v, &expected(&v), || true),
                Err(AuthorityCapabilityError::UnsupportedTransport)
            );
        }
    }

    #[test]
    fn valid_signed_context_consumes_single_use_challenge() {
        let (v, _) = claim();
        let consumed = Cell::new(false);
        let validate = || {
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &expected(&v), || {
                if consumed.get() {
                    false
                } else {
                    consumed.set(true);
                    true
                }
            })
        };
        let verified = validate().unwrap();
        assert_eq!(verified.peer_id(), v.peer_id);
        assert_eq!(verified.policy_hash(), v.committed_policy_hash);
        assert_eq!(verified.migration_hash(), v.committed_migration_hash);
        assert_eq!(verified.membership_hash(), v.accepted_membership_hash);
        assert_eq!(verified.snapshot_hash(), v.snapshot_hash);
        assert_eq!(verified.selected_heap_mib(), 4096);
        assert!(!verified.valid_at(99_999));
        assert!(verified.valid_at(110_000));
        assert!(!verified.valid_at(110_001));
        assert_eq!(validate(), Err(AuthorityCapabilityError::ReplayOrUnknownChallenge));
    }

    #[test]
    fn rejects_unsigned_tampering_and_foreign_keys() {
        let (v, _) = claim();
        let mut bad = v.clone();
        bad.committed_policy_hash = Hash32([3; 32]);
        let mut ex = expected(&v);
        ex.policy_hash = bad.committed_policy_hash;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &bad, &ex, || true),
            Err(AuthorityCapabilityError::InvalidSignature)
        );
        bad = v.clone();
        bad.public_key = SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes();
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &bad, &expected(&v), || true),
            Err(AuthorityCapabilityError::InvalidIdentity)
        );
        bad = v.clone();
        bad.signature = Vec::new();
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &bad, &expected(&v), || true),
            Err(AuthorityCapabilityError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_replayed_wrong_world_stale_generation_and_consent() {
        let (v, _) = claim();
        let mut ex = expected(&v);
        ex.world_id = WorldId([9; 32]);
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
        ex = expected(&v);
        ex.epoch = 4;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
        ex = expected(&v);
        ex.fencing_token = 5;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
        ex = expected(&v);
        ex.consent_revision = 6;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
        ex = expected(&v);
        ex.challenge = [0xAA; 32];
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
        ex = expected(&v);
        ex.verifier_transport_peer = b"another connection";
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
    }

    #[test]
    fn rejects_stale_migration_membership_snapshot_or_runtime_heap() {
        let (v, _) = claim();
        let mut ex = expected(&v);
        ex.migration_hash = Hash32([99; 32]);
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
        ex = expected(&v);
        ex.membership_hash = Hash32([99; 32]);
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
        ex = expected(&v);
        ex.snapshot_hash = Hash32([99; 32]);
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );
        ex = expected(&v);
        ex.selected_heap_mib = 2048;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::WrongContext)
        );

        let mut tampered = v.clone();
        tampered.snapshot_hash = Hash32([88; 32]);
        let ex = expected(&tampered);
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &tampered, &ex, || true),
            Err(AuthorityCapabilityError::InvalidSignature)
        );
        tampered = v.clone();
        tampered.selected_heap_mib = 0;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &tampered, &expected(&v), || true),
            Err(AuthorityCapabilityError::InvalidTranscript)
        );
    }

    #[test]
    fn rejects_empty_target_realm_even_with_a_valid_signer() {
        let (mut signed, key) = claim();
        signed.world_id = WorldId([0; 32]);
        assert_eq!(signed.sign(&key), Err(AuthorityCapabilityError::InvalidTranscript));
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &signed, &expected(&signed), || true),
            Err(AuthorityCapabilityError::InvalidTranscript)
        );
    }

    #[test]
    fn rejects_expired_future_and_unsupported_claims() {
        let (v, _) = claim();
        let mut ex = expected(&v);
        ex.now_unix_ms = 110_001;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::StaleChallenge)
        );
        ex.now_unix_ms = 99_999;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &v, &ex, || true),
            Err(AuthorityCapabilityError::StaleChallenge)
        );
        let mut bad = v.clone();
        bad.feature_version = 99;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &bad, &expected(&v), || true),
            Err(AuthorityCapabilityError::UnsupportedFeature)
        );
        bad = v.clone();
        bad.expires_unix_ms = bad.issued_unix_ms + MAX_AUTHORITY_SESSION_AGE_MS + 1;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &bad, &expected(&v), || true),
            Err(AuthorityCapabilityError::InvalidTranscript)
        );
        bad = v.clone();
        bad.consent_revision = 0;
        assert_eq!(
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &bad, &expected(&v), || true),
            Err(AuthorityCapabilityError::InvalidTranscript)
        );
    }
}
