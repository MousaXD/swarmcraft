//! Verifier-owned, connection-bound one-use V2 authority-action nonces.
//!
//! This is an additional *network replay precondition*, NOT a canonical
//! accepted-root, distributed lock, fenced lease or permission to start Java.
//! A verified session, accepted policy and current local resource readiness
//! still have to be checked at every runtime authority transition.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use rand_core::{OsRng, RngCore};
use swarm_protocol::{
    ExpectedHostingAuthorityHeadV2, Hash32, HostingAuthorityOperationV2, PeerId, SignedHostingAuthorityActionV2,
    WorldId,
};
use thiserror::Error;

use crate::authority_capability::{
    verify_v2_authority_action_session, V2AuthorityActionError, VerifiedAuthorityFeatureSession,
    MAX_AUTHORITY_SESSION_AGE_MS, MAX_V2_AUTHORITY_HEAP_MIB, MIN_V2_AUTHORITY_HEAP_MIB,
};

pub const MAX_PENDING_V2_AUTHORITY_ACTIONS: usize = 1024;
const ACTION_NONCE_LIFETIME: Duration = Duration::from_millis(MAX_AUTHORITY_SESSION_AGE_MS);

/// The expected action scope MUST come from an independently accepted current
/// canonical head, authenticated connection and a locally checked candidate.
/// A peer's received SignedHostingAuthorityActionV2 is NOT an acceptable source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V2AuthorityActionScope {
    pub realm_id: WorldId,
    pub connection_generation: u64,
    pub operation: HostingAuthorityOperationV2,
    pub candidate_peer_id: PeerId,
    pub expected_signer_peer_id: PeerId,
    pub committed_policy_hash: Hash32,
    pub committed_migration_hash: Hash32,
    pub accepted_membership_hash: Hash32,
    pub snapshot_hash: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub consent_revision: u64,
    pub selected_heap_mib: u32,
}

impl V2AuthorityActionScope {
    fn valid(self) -> bool {
        self.realm_id != WorldId::default()
            && self.connection_generation != 0
            && self.candidate_peer_id != PeerId::default()
            && self.expected_signer_peer_id != PeerId::default()
            && self.committed_policy_hash != Hash32::default()
            && self.committed_migration_hash != Hash32::default()
            && self.accepted_membership_hash != Hash32::default()
            && self.snapshot_hash != Hash32::default()
            && self.epoch != 0
            && self.fencing_token != 0
            && self.consent_revision != 0
            && (MIN_V2_AUTHORITY_HEAP_MIB..=MAX_V2_AUTHORITY_HEAP_MIB).contains(&self.selected_heap_mib)
    }

    fn matches_session(self, session: VerifiedAuthorityFeatureSession) -> bool {
        self.valid()
            && self.realm_id == session.world_id()
            && self.candidate_peer_id == session.peer_id()
            && self.committed_policy_hash == session.policy_hash()
            && self.committed_migration_hash == session.migration_hash()
            && self.accepted_membership_hash == session.membership_hash()
            && self.snapshot_hash == session.snapshot_hash()
            && self.epoch == session.epoch()
            && self.fencing_token == session.fencing_token()
            && self.consent_revision == session.consent_revision()
            && self.selected_heap_mib == session.selected_heap_mib()
    }

    fn expected_action(self, nonce: [u8; 32]) -> ExpectedHostingAuthorityHeadV2 {
        ExpectedHostingAuthorityHeadV2 {
            operation: self.operation,
            realm_id: self.realm_id,
            committed_policy_hash: self.committed_policy_hash,
            committed_migration_hash: self.committed_migration_hash,
            accepted_membership_hash: self.accepted_membership_hash,
            epoch: self.epoch,
            fencing_token: self.fencing_token,
            snapshot_hash: self.snapshot_hash,
            candidate_peer_id: self.candidate_peer_id,
            expected_consent_revision: self.consent_revision,
            expected_selected_heap_mib: self.selected_heap_mib,
            expected_signer_peer_id: self.expected_signer_peer_id,
            expected_nonce: nonce,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum V2AuthorityActionNonceError {
    #[error("authority action request is not bound to a current authenticated V2 session")]
    InvalidScopeOrSession,
    #[error("conflicting V2 successors or canonical heads are pending for one realm")]
    ConcurrentSuccessor,
    #[error("V2 authority action nonce capacity is exhausted")]
    CapacityExceeded,
    #[error("could not issue an unpredictable unused V2 authority action nonce")]
    EntropyUnavailable,
    #[error("missing, expired, replayed or mismatched V2 authority action nonce")]
    UnknownOrStaleNonce,
    #[error(transparent)]
    InvalidSignedAction(#[from] V2AuthorityActionError),
}

#[derive(Debug, Clone, Copy)]
struct PendingAction {
    scope: V2AuthorityActionScope,
    issued_at: Instant,
}

/// A task-owned verifier-side registry. If used concurrently, serialize
/// issue/consume/invalidate via one mutex. It is intentionally NOT durable:
/// process restart invalidates pending nonces but cannot commit an authority.
#[derive(Debug, Default)]
pub struct V2AuthorityActionNonceBook {
    pending: HashMap<[u8; 32], PendingAction>,
}

impl V2AuthorityActionNonceBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn expire(&mut self, now: Instant) {
        self.pending.retain(|_, entry| {
            now.checked_duration_since(entry.issued_at).is_some_and(|age| age <= ACTION_NONCE_LIFETIME)
        });
    }

    /// Reject two different candidate/head proposals for the same realm even
    /// before their signed action is verified. This is only local, provisional
    /// replay resistance; it CANNOT replace durable quorum-backed CAS/fencing
    /// between distinct devices.
    pub fn issue(
        &mut self,
        session: VerifiedAuthorityFeatureSession,
        scope: V2AuthorityActionScope,
        now_unix_ms: u64,
        now: Instant,
    ) -> Result<[u8; 32], V2AuthorityActionNonceError> {
        if !session.valid_at(now_unix_ms) || !scope.matches_session(session) {
            return Err(V2AuthorityActionNonceError::InvalidScopeOrSession);
        }
        self.expire(now);
        // One unresolved action per realm. Distinct operations, signer keys,
        // candidates or fenced generations must serialize through the
        // accepted-root coordinator; allowing multiple pending actions would
        // hide transfer/recovery or successor conflicts.
        if self.pending.values().any(|entry| entry.scope.realm_id == scope.realm_id && entry.scope != scope) {
            return Err(V2AuthorityActionNonceError::ConcurrentSuccessor);
        }
        // Reissuing one exact operation on one connection invalidates its
        // predecessor, rather than allowing duplicate signed completions.
        self.pending.retain(|_, entry| entry.scope != scope);
        if self.pending.len() >= MAX_PENDING_V2_AUTHORITY_ACTIONS {
            return Err(V2AuthorityActionNonceError::CapacityExceeded);
        }
        for _ in 0..8 {
            let mut nonce = [0_u8; 32];
            OsRng.fill_bytes(&mut nonce);
            if nonce != [0; 32] && !self.pending.contains_key(&nonce) {
                self.pending.insert(nonce, PendingAction { scope, issued_at: now });
                return Ok(nonce);
            }
        }
        Err(V2AuthorityActionNonceError::EntropyUnavailable)
    }

    /// Verify a signed V2 authority operation against a previously issued
    /// exact-scope nonce; consume the nonce inside the original cryptographic
    /// verifier only AFTER signature and expected-context checks succeed.
    pub fn verify_action(
        &mut self,
        session: VerifiedAuthorityFeatureSession,
        scope: V2AuthorityActionScope,
        action: &SignedHostingAuthorityActionV2,
        now_unix_ms: u64,
        now: Instant,
    ) -> Result<(), V2AuthorityActionNonceError> {
        if !session.valid_at(now_unix_ms) || !scope.matches_session(session) {
            return Err(V2AuthorityActionNonceError::InvalidScopeOrSession);
        }
        let nonce = action.nonce;
        let Some(pending) = self.pending.get(&nonce).copied() else {
            return Err(V2AuthorityActionNonceError::UnknownOrStaleNonce);
        };
        if pending.scope != scope
            || !now.checked_duration_since(pending.issued_at).is_some_and(|age| age <= ACTION_NONCE_LIFETIME)
        {
            self.pending.remove(&nonce);
            return Err(V2AuthorityActionNonceError::UnknownOrStaleNonce);
        }
        let expected = scope.expected_action(nonce);
        verify_v2_authority_action_session(session, action, &expected, now_unix_ms, |signed_nonce| {
            signed_nonce == &nonce && self.pending.remove(signed_nonce).is_some()
        })?;
        Ok(())
    }

    /// Mandatory on disconnect, transport replacement or authenticated
    /// identity change. Never reuse action nonces across connections.
    pub fn invalidate_connection(&mut self, connection_generation: u64) {
        self.pending.retain(|_, entry| entry.scope.connection_generation != connection_generation);
    }

    /// Mandatory on accepted realm, policy, membership, fencing or consent
    /// revision changes, including revoke -> re-opt-in at a higher revision.
    pub fn invalidate_realm(&mut self, realm_id: WorldId) {
        self.pending.retain(|_, entry| entry.scope.realm_id != realm_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use swarm_protocol::{peer_id_from_public_key, SignedHostingAuthorityActionV2, HOSTING_REALM_FORMAT_VERSION_V2};

    fn session() -> (VerifiedAuthorityFeatureSession, SigningKey) {
        use crate::authority_capability::{
            verify_authority_feature_session, ExpectedAuthorityFeatureV2, SignedAuthorityFeatureV2,
            MANDATORY_AUTHORITY_FEATURE_VERSION, MANDATORY_AUTHORITY_WIRE_PROTOCOL,
        };
        let key = SigningKey::from_bytes(&[7; 32]);
        let peer = peer_id_from_public_key(&key.verifying_key().to_bytes());
        let mut claim = SignedAuthorityFeatureV2 {
            feature_version: MANDATORY_AUTHORITY_FEATURE_VERSION,
            world_id: WorldId([1; 32]),
            peer_id: peer,
            public_key: key.verifying_key().to_bytes(),
            committed_policy_hash: Hash32([2; 32]),
            committed_migration_hash: Hash32([3; 32]),
            accepted_membership_hash: Hash32([4; 32]),
            snapshot_hash: Hash32([5; 32]),
            selected_heap_mib: 2048,
            epoch: 6,
            fencing_token: 7,
            consent_revision: 8,
            challenge: [9; 32],
            claimant_transport_peer: b"claimant".to_vec(),
            verifier_transport_peer: b"verifier".to_vec(),
            issued_unix_ms: 100_000,
            expires_unix_ms: 110_000,
            signature: Vec::new(),
        };
        claim.sign(&key).unwrap();
        let expected = ExpectedAuthorityFeatureV2 {
            world_id: claim.world_id,
            peer_id: claim.peer_id,
            policy_hash: claim.committed_policy_hash,
            migration_hash: claim.committed_migration_hash,
            membership_hash: claim.accepted_membership_hash,
            snapshot_hash: claim.snapshot_hash,
            selected_heap_mib: claim.selected_heap_mib,
            epoch: claim.epoch,
            fencing_token: claim.fencing_token,
            consent_revision: claim.consent_revision,
            challenge: claim.challenge,
            claimant_transport_peer: &claim.claimant_transport_peer,
            verifier_transport_peer: &claim.verifier_transport_peer,
            now_unix_ms: 105_000,
        };
        let verified =
            verify_authority_feature_session(MANDATORY_AUTHORITY_WIRE_PROTOCOL, &claim, &expected, || true).unwrap();
        (verified, key)
    }

    fn scope(session: VerifiedAuthorityFeatureSession) -> V2AuthorityActionScope {
        V2AuthorityActionScope {
            realm_id: session.world_id(),
            connection_generation: 15,
            operation: HostingAuthorityOperationV2::RecoveryBallot,
            candidate_peer_id: session.peer_id(),
            expected_signer_peer_id: session.peer_id(),
            committed_policy_hash: session.policy_hash(),
            committed_migration_hash: session.migration_hash(),
            accepted_membership_hash: session.membership_hash(),
            snapshot_hash: session.snapshot_hash(),
            epoch: session.epoch(),
            fencing_token: session.fencing_token(),
            consent_revision: session.consent_revision(),
            selected_heap_mib: session.selected_heap_mib(),
        }
    }

    fn sign_action(scope: V2AuthorityActionScope, nonce: [u8; 32], key: &SigningKey) -> SignedHostingAuthorityActionV2 {
        let mut action = SignedHostingAuthorityActionV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            operation: scope.operation,
            realm_id: scope.realm_id,
            committed_policy_hash: scope.committed_policy_hash,
            committed_migration_hash: scope.committed_migration_hash,
            accepted_membership_hash: scope.accepted_membership_hash,
            epoch: scope.epoch,
            fencing_token: scope.fencing_token,
            snapshot_hash: scope.snapshot_hash,
            candidate_peer_id: scope.candidate_peer_id,
            consent_revision: scope.consent_revision,
            selected_heap_mib: scope.selected_heap_mib,
            signer_peer_id: scope.expected_signer_peer_id,
            signer_public_key: key.verifying_key().to_bytes(),
            nonce,
            signature: Vec::new(),
        };
        action.signature = key.sign(&action.signing_bytes().unwrap()).to_bytes().to_vec();
        action
    }

    #[test]
    fn valid_operation_consumes_nonce_once() {
        let (session, key) = session();
        let now = Instant::now();
        let scope = scope(session);
        let mut book = V2AuthorityActionNonceBook::new();
        let nonce = book.issue(session, scope, 105_000, now).unwrap();
        let action = sign_action(scope, nonce, &key);
        assert_eq!(book.verify_action(session, scope, &action, 105_000, now), Ok(()));
        assert_eq!(book.pending_count(), 0);
        assert_eq!(
            book.verify_action(session, scope, &action, 105_000, now),
            Err(V2AuthorityActionNonceError::UnknownOrStaleNonce)
        );
    }

    #[test]
    fn forged_signature_and_cross_operation_action_do_not_consume_nonce() {
        let (session, key) = session();
        let now = Instant::now();
        let scope = scope(session);
        let mut book = V2AuthorityActionNonceBook::new();
        let nonce = book.issue(session, scope, 105_000, now).unwrap();
        let mut forged = sign_action(scope, nonce, &key);
        forged.signature[0] ^= 1;
        assert_eq!(
            book.verify_action(session, scope, &forged, 105_000, now),
            Err(V2AuthorityActionNonceError::InvalidSignedAction(V2AuthorityActionError::InvalidAction))
        );
        let mut different_operation = sign_action(scope, nonce, &key);
        different_operation.operation = HostingAuthorityOperationV2::Wake;
        different_operation.signature = key.sign(&different_operation.signing_bytes().unwrap()).to_bytes().to_vec();
        assert_eq!(
            book.verify_action(session, scope, &different_operation, 105_000, now),
            Err(V2AuthorityActionNonceError::InvalidSignedAction(V2AuthorityActionError::InvalidAction))
        );
        assert_eq!(book.pending_count(), 1);
        assert_eq!(book.verify_action(session, scope, &sign_action(scope, nonce, &key), 105_000, now), Ok(()));
    }

    #[test]
    fn conflicting_successor_and_reconnection_fail_closed() {
        let (session, _) = session();
        let now = Instant::now();
        let original = scope(session);
        let mut book = V2AuthorityActionNonceBook::new();
        let nonce = book.issue(session, original, 105_000, now).unwrap();
        let mut alternate = original;
        alternate.connection_generation += 1;
        assert_eq!(book.issue(session, alternate, 105_000, now), Err(V2AuthorityActionNonceError::ConcurrentSuccessor));
        let mut other_operation = original;
        other_operation.operation = HostingAuthorityOperationV2::TransferCommit;
        assert_eq!(
            book.issue(session, other_operation, 105_000, now),
            Err(V2AuthorityActionNonceError::ConcurrentSuccessor)
        );
        assert_eq!(
            book.verify_action(
                session,
                alternate,
                &sign_action(original, nonce, &SigningKey::from_bytes(&[7; 32])),
                105_000,
                now,
            ),
            Err(V2AuthorityActionNonceError::UnknownOrStaleNonce)
        );
        book.invalidate_connection(original.connection_generation);
        assert_eq!(book.pending_count(), 0);
        assert!(book.issue(session, alternate, 105_000, now).is_ok());
    }

    #[test]
    fn revoked_consent_stale_fencing_and_expiration_deny_actions() {
        let (session, key) = session();
        let now = Instant::now();
        let original = scope(session);
        let mut book = V2AuthorityActionNonceBook::new();
        let nonce = book.issue(session, original, 105_000, now).unwrap();
        let action = sign_action(original, nonce, &key);
        let mut revoked = original;
        revoked.consent_revision += 1;
        assert_eq!(
            book.verify_action(session, revoked, &action, 105_000, now),
            Err(V2AuthorityActionNonceError::InvalidScopeOrSession)
        );
        let mut successor = original;
        successor.fencing_token += 1;
        assert_eq!(
            book.issue(session, successor, 105_000, now),
            Err(V2AuthorityActionNonceError::InvalidScopeOrSession)
        );
        assert_eq!(
            book.verify_action(
                session,
                original,
                &action,
                105_000,
                now + ACTION_NONCE_LIFETIME + Duration::from_millis(1),
            ),
            Err(V2AuthorityActionNonceError::UnknownOrStaleNonce)
        );
        assert_eq!(
            book.issue(session, original, 110_001, now),
            Err(V2AuthorityActionNonceError::InvalidScopeOrSession)
        );
    }

    #[test]
    fn fresh_issue_replaces_same_operation_nonce_and_realm_invalidation_clears_all() {
        let (session, key) = session();
        let now = Instant::now();
        let original = scope(session);
        let mut book = V2AuthorityActionNonceBook::new();
        let first = book.issue(session, original, 105_000, now).unwrap();
        let second = book.issue(session, original, 105_000, now).unwrap();
        assert_ne!(first, second);
        assert_eq!(book.pending_count(), 1);
        assert_eq!(
            book.verify_action(session, original, &sign_action(original, first, &key), 105_000, now),
            Err(V2AuthorityActionNonceError::UnknownOrStaleNonce)
        );
        book.invalidate_realm(original.realm_id);
        assert_eq!(book.pending_count(), 0);
        assert_eq!(
            book.verify_action(session, original, &sign_action(original, second, &key), 105_000, now),
            Err(V2AuthorityActionNonceError::UnknownOrStaleNonce)
        );
    }
}
