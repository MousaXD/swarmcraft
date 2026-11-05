//! Preview-specific authority election, leases, divergence checks, and fencing rules.

use std::cmp::Ordering;
use swarm_protocol::{Hash32, PeerId};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityCandidate {
    pub peer_id: PeerId,
    pub accepted_epoch: u64,
    pub canonical_sequence: u64,
    pub snapshot_complete: bool,
    pub compatible: bool,
    pub authority_eligible: bool,
    pub banned: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ElectionError {
    #[error("no eligible authority candidate has a complete compatible state")]
    NoEligibleCandidate,
    #[error("automatic crash election requires a majority: visible={visible}, members={members}")]
    QuorumUnavailable { visible: usize, members: usize },
}

/// Selects one authority deterministically.
pub fn elect_authority(candidates: &[AuthorityCandidate]) -> Result<PeerId, ElectionError> {
    candidates
        .iter()
        .filter(|candidate| {
            candidate.compatible && candidate.authority_eligible && candidate.snapshot_complete && !candidate.banned
        })
        .max_by(|a, b| {
            a.accepted_epoch
                .cmp(&b.accepted_epoch)
                .then(a.canonical_sequence.cmp(&b.canonical_sequence))
                .then_with(|| b.peer_id.cmp(&a.peer_id))
        })
        .map(|candidate| candidate.peer_id)
        .ok_or(ElectionError::NoEligibleCandidate)
}

/// Automatic crash recovery needs a majority of the canonical membership.
/// Clean sleep/wake can still enter solo mode because the previous authority explicitly relinquished the world.
pub fn elect_authority_with_quorum(
    canonical_member_count: usize,
    visible_candidates: &[AuthorityCandidate],
) -> Result<PeerId, ElectionError> {
    if !has_quorum(canonical_member_count, visible_candidates.len()) {
        return Err(ElectionError::QuorumUnavailable {
            visible: visible_candidates.len(),
            members: canonical_member_count,
        });
    }
    elect_authority(visible_candidates)
}

pub const fn quorum_size(member_count: usize) -> usize {
    if member_count == 0 {
        0
    } else {
        member_count / 2 + 1
    }
}

pub const fn has_quorum(member_count: usize, visible_member_count: usize) -> bool {
    member_count != 0 && visible_member_count >= quorum_size(member_count)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthorityGeneration {
    pub epoch: u64,
    pub fencing_token: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GenerationError {
    #[error("authority epoch exhausted at u64::MAX")]
    EpochExhausted,
    #[error("authority fencing token exhausted at u64::MAX")]
    FencingTokenExhausted,
}

impl AuthorityGeneration {
    /// Return the unique next authority generation, failing closed on counter exhaustion.
    pub fn checked_next(self) -> Result<Self, GenerationError> {
        let epoch = self.epoch.checked_add(1).ok_or(GenerationError::EpochExhausted)?;
        let fencing_token = self.fencing_token.checked_add(1).ok_or(GenerationError::FencingTokenExhausted)?;
        Ok(Self { epoch, fencing_token })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedLease {
    pub generation: AuthorityGeneration,
    /// Milliseconds on a caller-supplied monotonic clock. This is never a wall-clock timestamp.
    pub expires_at_millis: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LeaseTracker {
    observed: Option<ObservedLease>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("authority lease duration must be non-zero")]
    ZeroDuration,
    #[error("stale authority lease generation: received {received:?}, accepted {accepted:?}")]
    StaleGeneration { received: AuthorityGeneration, accepted: AuthorityGeneration },
}

impl LeaseTracker {
    pub fn observe(
        &mut self,
        generation: AuthorityGeneration,
        lease_duration_ms: u64,
        monotonic_now_ms: u64,
    ) -> Result<ObservedLease, LeaseError> {
        if lease_duration_ms == 0 {
            return Err(LeaseError::ZeroDuration);
        }
        if let Some(current) = self.observed {
            if generation < current.generation {
                return Err(LeaseError::StaleGeneration { received: generation, accepted: current.generation });
            }
        }
        let observed =
            ObservedLease { generation, expires_at_millis: monotonic_now_ms.saturating_add(lease_duration_ms) };
        self.observed = Some(observed);
        Ok(observed)
    }

    pub const fn observed(&self) -> Option<ObservedLease> {
        self.observed
    }

    pub fn is_expired(&self, monotonic_now_ms: u64) -> bool {
        match self.observed {
            Some(lease) => monotonic_now_ms >= lease.expires_at_millis,
            None => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalState {
    pub epoch: u64,
    pub sequence: u64,
    pub state_hash: Hash32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HistoryError {
    #[error("world history diverged at epoch {epoch} sequence {sequence}: local={local_hash}, remote={remote_hash}")]
    Diverged { epoch: u64, sequence: u64, local_hash: Hash32, remote_hash: Hash32 },
}

pub fn compare_canonical_state(local: CanonicalState, remote: CanonicalState) -> Result<Ordering, HistoryError> {
    if local.epoch == remote.epoch && local.sequence == remote.sequence && local.state_hash != remote.state_hash {
        return Err(HistoryError::Diverged {
            epoch: local.epoch,
            sequence: local.sequence,
            local_hash: local.state_hash,
            remote_hash: remote.state_hash,
        });
    }
    Ok((local.epoch, local.sequence).cmp(&(remote.epoch, remote.sequence)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FencingState {
    accepted_token: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FencingError {
    #[error("stale fencing token {received}; current token is {current}")]
    Stale { received: u64, current: u64 },
    #[error("future fencing token {received}; current token is {current}")]
    Future { received: u64, current: u64 },
    #[error("new fencing token {new} must be greater than current token {current}")]
    NonMonotonic { new: u64, current: u64 },
}

impl FencingState {
    pub fn new(accepted_token: u64) -> Self {
        Self { accepted_token }
    }

    pub fn accepted_token(&self) -> u64 {
        self.accepted_token
    }

    pub fn validate_write(&self, token: u64) -> Result<(), FencingError> {
        match token.cmp(&self.accepted_token) {
            Ordering::Less => Err(FencingError::Stale { received: token, current: self.accepted_token }),
            Ordering::Greater => Err(FencingError::Future { received: token, current: self.accepted_token }),
            Ordering::Equal => Ok(()),
        }
    }

    pub fn advance(&mut self, new_token: u64) -> Result<(), FencingError> {
        if new_token <= self.accepted_token {
            return Err(FencingError::NonMonotonic { new: new_token, current: self.accepted_token });
        }
        self.accepted_token = new_token;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: u8, epoch: u64, sequence: u64) -> AuthorityCandidate {
        AuthorityCandidate {
            peer_id: PeerId([id; 32]),
            accepted_epoch: epoch,
            canonical_sequence: sequence,
            snapshot_complete: true,
            compatible: true,
            authority_eligible: true,
            banned: false,
        }
    }

    #[test]
    fn election_uses_latest_complete_state_then_stable_tie_break() {
        let candidates = vec![candidate(9, 4, 10), candidate(2, 5, 3), candidate(1, 5, 3)];
        assert_eq!(elect_authority(&candidates).unwrap(), PeerId([1; 32]));
    }

    #[test]
    fn automatic_crash_election_requires_majority() {
        let candidates = vec![candidate(2, 7, 50)];
        assert_eq!(
            elect_authority_with_quorum(3, &candidates),
            Err(ElectionError::QuorumUnavailable { visible: 1, members: 3 })
        );
        let candidates = vec![candidate(2, 7, 50), candidate(3, 7, 50)];
        assert_eq!(elect_authority_with_quorum(3, &candidates).unwrap(), PeerId([2; 32]));
    }

    #[test]
    fn authority_generation_fails_closed_at_counter_exhaustion() {
        let max = AuthorityGeneration { epoch: u64::MAX - 1, fencing_token: u64::MAX - 1 }.checked_next().unwrap();
        assert_eq!(max, AuthorityGeneration { epoch: u64::MAX, fencing_token: u64::MAX });
        assert_eq!(max.checked_next(), Err(GenerationError::EpochExhausted));
        assert_eq!(
            AuthorityGeneration { epoch: 7, fencing_token: u64::MAX }.checked_next(),
            Err(GenerationError::FencingTokenExhausted)
        );
    }

    #[test]
    fn lease_uses_monotonic_deadline_and_rejects_stale_generation() {
        let mut tracker = LeaseTracker::default();
        let generation = AuthorityGeneration { epoch: 4, fencing_token: 9 };
        tracker.observe(generation, 5_000, 10_000).unwrap();
        assert!(!tracker.is_expired(14_999));
        assert!(tracker.is_expired(15_000));
        tracker.observe(generation, 5_000, 20_000).unwrap();
        assert!(!tracker.is_expired(24_999));
        assert_eq!(
            tracker.observe(AuthorityGeneration { epoch: 3, fencing_token: 8 }, 5_000, 21_000),
            Err(LeaseError::StaleGeneration {
                received: AuthorityGeneration { epoch: 3, fencing_token: 8 },
                accepted: generation,
            })
        );
    }

    #[test]
    fn same_generation_different_hash_is_a_hard_conflict() {
        let local = CanonicalState { epoch: 15, sequence: 200, state_hash: Hash32([0xaa; 32]) };
        let remote = CanonicalState { epoch: 15, sequence: 200, state_hash: Hash32([0xbb; 32]) };
        assert!(matches!(compare_canonical_state(local, remote), Err(HistoryError::Diverged { .. })));
    }

    #[test]
    fn stale_authority_writes_are_rejected_after_advance() {
        let mut fencing = FencingState::new(10);
        fencing.validate_write(10).unwrap();
        fencing.advance(11).unwrap();
        assert!(matches!(fencing.validate_write(10), Err(FencingError::Stale { .. })));
        fencing.validate_write(11).unwrap();
    }

    #[derive(Clone)]
    struct FakePeer {
        candidate: AuthorityCandidate,
        online: bool,
        partition: u8,
        lease: LeaseTracker,
    }

    struct ChaosCluster {
        peers: Vec<FakePeer>,
        authority: Option<usize>,
        generation: AuthorityGeneration,
        now_ms: u64,
        rng: u64,
    }

    impl ChaosCluster {
        fn new() -> Self {
            let generation = AuthorityGeneration { epoch: 1, fencing_token: 1 };
            let mut peers = vec![1, 2, 3]
                .into_iter()
                .map(|id| FakePeer {
                    candidate: candidate(id, generation.epoch, 1),
                    online: true,
                    partition: 0,
                    lease: LeaseTracker::default(),
                })
                .collect::<Vec<_>>();
            for peer in &mut peers {
                peer.lease.observe(generation, 3_000, 0).unwrap();
            }
            Self { peers, authority: Some(0), generation, now_ms: 0, rng: 0x7ac5_1d33_9e37_79b9 }
        }

        fn random(&mut self) -> u64 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            self.rng
        }

        fn step(&mut self) {
            self.now_ms = self.now_ms.saturating_add(250);
            let event = self.random() % 10;
            let peer = (self.random() as usize) % self.peers.len();
            match event {
                0 => self.peers[peer].online = false,
                1 => self.peers[peer].online = true,
                2 => self.peers[peer].partition = 1,
                3 => self.peers[peer].partition = 0,
                _ => {}
            }

            if let Some(authority) = self.authority {
                if !self.peers[authority].online {
                    self.authority = None;
                } else {
                    let partition = self.peers[authority].partition;
                    let visible = self
                        .peers
                        .iter()
                        .enumerate()
                        .filter(|(_, peer)| peer.online && peer.partition == partition)
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    if has_quorum(self.peers.len(), visible.len()) {
                        for index in visible {
                            self.peers[index].lease.observe(self.generation, 3_000, self.now_ms).unwrap();
                        }
                    } else if self.peers[authority].lease.is_expired(self.now_ms) {
                        self.authority = None;
                    }
                }
            }

            if self.authority.is_none() {
                for partition in [0, 1] {
                    let visible = self
                        .peers
                        .iter()
                        .enumerate()
                        .filter(|(_, peer)| {
                            peer.online && peer.partition == partition && peer.lease.is_expired(self.now_ms)
                        })
                        .map(|(index, peer)| (index, peer.candidate.clone()))
                        .collect::<Vec<_>>();
                    let candidates = visible.iter().map(|(_, candidate)| candidate.clone()).collect::<Vec<_>>();
                    if let Ok(winner) = elect_authority_with_quorum(self.peers.len(), &candidates) {
                        let winner_index = visible
                            .iter()
                            .find(|(_, candidate)| candidate.peer_id == winner)
                            .map(|(index, _)| *index)
                            .unwrap();
                        self.generation =
                            self.generation.checked_next().expect("chaos simulation exhausted authority generation");
                        for peer in &mut self.peers {
                            peer.candidate.accepted_epoch = self.generation.epoch;
                        }
                        self.authority = Some(winner_index);
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn randomized_crash_partition_reconnect_simulation_never_elects_two_authorities() {
        for seed_offset in 0..64 {
            let mut cluster = ChaosCluster::new();
            cluster.rng ^= seed_offset;
            for _ in 0..2_000 {
                cluster.step();
                assert!(cluster.authority.into_iter().count() <= 1);
                if let Some(authority) = cluster.authority {
                    assert!(cluster.peers[authority].online);
                }
            }
        }
    }
}
