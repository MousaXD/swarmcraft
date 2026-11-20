//! Non-authorizing, fail-closed V2 authority challenge registry.
//!
//! Used only after an actually negotiated /swarmcraft/hosting-authority/2
//! connection is authenticated and the expected canonical head is independently
//! loaded. In-memory challenges deliberately do NOT survive a process crash.
//! This registry is not a lease, V2 realm activation or hosting permit.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use rand_core::{OsRng, RngCore};
use swarm_protocol::{Hash32, PeerId, WorldId};
use thiserror::Error;

use crate::authority_capability::{
    MAX_AUTHORITY_SESSION_AGE_MS, MAX_AUTHORITY_TRANSPORT_ID_BYTES, MAX_V2_AUTHORITY_HEAP_MIB,
    MIN_V2_AUTHORITY_HEAP_MIB,
};

/// Bound independently of libp2p's request limits. A connected peer can hold
/// at most one challenge per realm and connection generation.
pub const MAX_OUTSTANDING_V2_CHALLENGES: usize = 1024;
const CHALLENGE_LIFETIME: Duration = Duration::from_millis(MAX_AUTHORITY_SESSION_AGE_MS);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2ChallengeScope {
    pub realm_id: WorldId,
    pub claimant_peer_id: PeerId,
    pub claimant_transport_peer: Vec<u8>,
    pub verifier_transport_peer: Vec<u8>,
    /// Opaque LOCAL connection generation. A reconnect must use a different
    /// generation even if the remote libp2p identity has not changed.
    pub connection_generation: u64,
    pub committed_policy_hash: Hash32,
    pub committed_migration_hash: Hash32,
    pub accepted_membership_hash: Hash32,
    pub snapshot_hash: Hash32,
    pub selected_heap_mib: u32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub consent_revision: u64,
}

impl V2ChallengeScope {
    fn validate(&self) -> bool {
        self.realm_id != WorldId::default()
            && self.claimant_peer_id != PeerId::default()
            && self.connection_generation != 0
            && !self.claimant_transport_peer.is_empty()
            && !self.verifier_transport_peer.is_empty()
            && self.claimant_transport_peer.len() <= MAX_AUTHORITY_TRANSPORT_ID_BYTES
            && self.verifier_transport_peer.len() <= MAX_AUTHORITY_TRANSPORT_ID_BYTES
            && self.committed_policy_hash != Hash32::default()
            && self.committed_migration_hash != Hash32::default()
            && self.accepted_membership_hash != Hash32::default()
            && self.snapshot_hash != Hash32::default()
            && (MIN_V2_AUTHORITY_HEAP_MIB..=MAX_V2_AUTHORITY_HEAP_MIB).contains(&self.selected_heap_mib)
            && self.epoch != 0
            && self.fencing_token != 0
            && self.consent_revision != 0
    }

    fn same_connection(&self, other: &Self) -> bool {
        self.realm_id == other.realm_id
            && self.claimant_peer_id == other.claimant_peer_id
            && self.connection_generation == other.connection_generation
            && self.claimant_transport_peer == other.claimant_transport_peer
            && self.verifier_transport_peer == other.verifier_transport_peer
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum V2ChallengeError {
    #[error("invalid canonical head, runtime selection or authenticated connection binding")]
    InvalidScope,
    #[error("too many outstanding V2 authority challenges")]
    CapacityExceeded,
    #[error("could not generate a fresh unique authority challenge")]
    EntropyUnavailable,
}

#[derive(Debug)]
struct PendingChallenge {
    scope: V2ChallengeScope,
    issued_at: Instant,
}

/// All mutations are exclusive to the owning network task. When shared across
/// tasks this registry MUST be protected by one mutex whose critical section
/// includes the nonce consumption; never check and delete in separate locks.
#[derive(Debug, Default)]
pub struct V2AuthorityChallengeBook {
    outstanding: HashMap<[u8; 32], PendingChallenge>,
}

impl V2AuthorityChallengeBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn outstanding_count(&self) -> usize {
        self.outstanding.len()
    }

    /// Prune expired challenges. The caller supplies monotonic time, never a
    /// remote timestamp. A backwards clock value does not extend issuance.
    pub fn expire(&mut self, now: Instant) {
        self.outstanding.retain(|_, entry| {
            now.checked_duration_since(entry.issued_at)
                .is_some_and(|age| age <= CHALLENGE_LIFETIME)
        });
    }

    /// Issue a verifier-owned unpredictable challenge. A replacement request
    /// for the same connection and realm removes any prior unconsumed value.
    /// Repeated issuance must not accumulate challenges for one connection.
    pub fn issue(
        &mut self,
        scope: V2ChallengeScope,
        now: Instant,
    ) -> Result<[u8; 32], V2ChallengeError> {
        if !scope.validate() {
            return Err(V2ChallengeError::InvalidScope);
        }
        self.expire(now);
        self.outstanding.retain(|_, entry| !entry.scope.same_connection(&scope));
        if self.outstanding.len() >= MAX_OUTSTANDING_V2_CHALLENGES {
            return Err(V2ChallengeError::CapacityExceeded);
        }
        for _ in 0..8 {
            let mut challenge = [0_u8; 32];
            OsRng.fill_bytes(&mut challenge);
            if challenge != [0; 32] && !self.outstanding.contains_key(&challenge) {
                self.outstanding.insert(challenge, PendingChallenge { scope, issued_at: now });
                return Ok(challenge);
            }
        }
        Err(V2ChallengeError::EntropyUnavailable)
    }

    /// Consume the exact outstanding challenge ONCE, after signature
    /// verification. Incorrect scope also consumes the nonce to prevent
    /// repeated authentication attempts with an exposed challenge.
    pub fn consume(
        &mut self,
        challenge: &[u8; 32],
        expected_scope: &V2ChallengeScope,
        now: Instant,
    ) -> bool {
        let Some(entry) = self.outstanding.remove(challenge) else {
            return false;
        };
        expected_scope.validate()
            && &entry.scope == expected_scope
            && now.checked_duration_since(entry.issued_at)
                .is_some_and(|age| age <= CHALLENGE_LIFETIME)
    }

    /// Invoke on every transport disconnection, connection replacement or
    /// authenticated application identity change. No nonce may cross routes.
    pub fn invalidate_connection(&mut self, connection_generation: u64) {
        self.outstanding
            .retain(|_, entry| entry.scope.connection_generation != connection_generation);
    }

    /// Invoke when an accepted realm, membership, policy, fencing generation
    /// or consent revision changes (including revocation/re-opt-in). The
    /// verifier must issue and validate a new challenge against the new head.
    pub fn invalidate_realm(&mut self, realm: WorldId) {
        self.outstanding.retain(|_, entry| entry.scope.realm_id != realm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> V2ChallengeScope {
        V2ChallengeScope {
            realm_id: WorldId([1; 32]),
            claimant_peer_id: PeerId([2; 32]),
            claimant_transport_peer: b"claimant".to_vec(),
            verifier_transport_peer: b"verifier".to_vec(),
            connection_generation: 7,
            committed_policy_hash: Hash32([3; 32]),
            committed_migration_hash: Hash32([4; 32]),
            accepted_membership_hash: Hash32([5; 32]),
            snapshot_hash: Hash32([6; 32]),
            selected_heap_mib: 4096,
            epoch: 8,
            fencing_token: 9,
            consent_revision: 10,
        }
    }

    #[test]
    fn unique_unpredictable_challenges_are_one_use() {
        let mut store = V2AuthorityChallengeBook::new();
        let now = Instant::now();
        let a = store.issue(scope(), now).unwrap();
        assert_ne!(a, [0; 32]);
        let b = store.issue(scope(), now).unwrap();
        assert_ne!(b, [0; 32]);
        assert_ne!(a, b);
        assert_eq!(store.outstanding_count(), 1);
        assert!(!store.consume(&a, &scope(), now));
        assert!(store.consume(&b, &scope(), now));
        assert!(!store.consume(&b, &scope(), now));
        assert_eq!(store.outstanding_count(), 0);
    }

    #[test]
    fn stale_or_replaced_connections_cannot_replay_authority() {
        let mut store = V2AuthorityChallengeBook::new();
        let now = Instant::now();
        let nonce = store.issue(scope(), now).unwrap();
        let mut foreign = scope();
        foreign.connection_generation += 1;
        assert!(!store.consume(&nonce, &foreign, now));
        assert!(!store.consume(&nonce, &scope(), now));

        let nonce = store.issue(scope(), now).unwrap();
        store.invalidate_connection(7);
        assert!(!store.consume(&nonce, &scope(), now));

        let nonce = store.issue(scope(), now).unwrap();
        assert!(!store.consume(&nonce, &scope(), now + CHALLENGE_LIFETIME + Duration::from_millis(1)));
    }

    #[test]
    fn policy_fencing_membership_and_consent_mutation_fail_closed() {
        let changes: [fn(&mut V2ChallengeScope); 7] = [
            |s| s.committed_policy_hash = Hash32([99; 32]),
            |s| s.committed_migration_hash = Hash32([99; 32]),
            |s| s.accepted_membership_hash = Hash32([99; 32]),
            |s| s.snapshot_hash = Hash32([99; 32]),
            |s| s.epoch += 1,
            |s| s.fencing_token += 1,
            |s| s.consent_revision += 1,
        ];
        let now = Instant::now();
        for change in changes {
            let mut store = V2AuthorityChallengeBook::new();
            let nonce = store.issue(scope(), now).unwrap();
            let mut changed = scope();
            change(&mut changed);
            assert!(!store.consume(&nonce, &changed, now));
            assert!(!store.consume(&nonce, &scope(), now));
        }
    }

    #[test]
    fn invalid_scopes_rejected_without_minting_challenges() {
        let mut store = V2AuthorityChallengeBook::new();
        let now = Instant::now();
        let mut invalid = scope();
        invalid.selected_heap_mib = 0;
        assert_eq!(store.issue(invalid, now), Err(V2ChallengeError::InvalidScope));
        let mut invalid = scope();
        invalid.claimant_transport_peer = Vec::new();
        assert_eq!(store.issue(invalid, now), Err(V2ChallengeError::InvalidScope));
        let mut invalid = scope();
        invalid.realm_id = WorldId::default();
        assert_eq!(store.issue(invalid, now), Err(V2ChallengeError::InvalidScope));
        let mut invalid = scope();
        invalid.connection_generation = 0;
        assert_eq!(store.issue(invalid, now), Err(V2ChallengeError::InvalidScope));
        assert_eq!(store.outstanding_count(), 0);
    }

    #[test]
    fn realm_invalidation_clears_all_pending_authority_challenges() {
        let mut store = V2AuthorityChallengeBook::new();
        let now = Instant::now();
        let one = store.issue(scope(), now).unwrap();
        let mut other = scope();
        other.realm_id = WorldId([66; 32]);
        let two = store.issue(other.clone(), now).unwrap();
        store.invalidate_realm(scope().realm_id);
        assert!(!store.consume(&one, &scope(), now));
        assert!(store.consume(&two, &other, now));
    }

    #[test]
    fn capacity_is_bounded_across_distinct_connections() {
        let mut store = V2AuthorityChallengeBook::new();
        let now = Instant::now();
        for connection_generation in 1..=MAX_OUTSTANDING_V2_CHALLENGES as u64 {
            let mut s = scope();
            s.connection_generation = connection_generation;
            store.issue(s, now).unwrap();
        }
        let mut excess = scope();
        excess.connection_generation = MAX_OUTSTANDING_V2_CHALLENGES as u64 + 1;
        assert_eq!(store.issue(excess, now), Err(V2ChallengeError::CapacityExceeded));
        store.expire(now + CHALLENGE_LIFETIME + Duration::from_millis(1));
        assert_eq!(store.outstanding_count(), 0);
        assert!(store.issue(scope(), now + CHALLENGE_LIFETIME + Duration::from_millis(2)).is_ok());
    }
}
