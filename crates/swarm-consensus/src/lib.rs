//! Preview-specific authority election and fencing rules.

use swarm_protocol::PeerId;
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
}

/// Selects one authority deterministically.
///
/// Eligibility is a hard gate. Ordering follows the preview plan:
/// highest accepted epoch, highest canonical sequence, then peer ID as a stable tie breaker.
/// Snapshot completeness is required rather than merely preferred because an authority must be able
/// to restore its claimed canonical state.
pub fn elect_authority(candidates: &[AuthorityCandidate]) -> Result<PeerId, ElectionError> {
    candidates
        .iter()
        .filter(|candidate| {
            candidate.compatible
                && candidate.authority_eligible
                && candidate.snapshot_complete
                && !candidate.banned
        })
        .max_by(|a, b| {
            a.accepted_epoch
                .cmp(&b.accepted_epoch)
                .then(a.canonical_sequence.cmp(&b.canonical_sequence))
                // Reverse the peer ordering inside max_by so the lexicographically lowest peer wins ties.
                .then_with(|| b.peer_id.cmp(&a.peer_id))
        })
        .map(|candidate| candidate.peer_id)
        .ok_or(ElectionError::NoEligibleCandidate)
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
            std::cmp::Ordering::Less => Err(FencingError::Stale { received: token, current: self.accepted_token }),
            std::cmp::Ordering::Greater => Err(FencingError::Future { received: token, current: self.accepted_token }),
            std::cmp::Ordering::Equal => Ok(()),
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

    #[test]
    fn election_uses_latest_complete_state_then_stable_tie_break() {
        let candidates = vec![
            AuthorityCandidate { peer_id: PeerId([9; 32]), accepted_epoch: 4, canonical_sequence: 10, snapshot_complete: true, compatible: true, authority_eligible: true, banned: false },
            AuthorityCandidate { peer_id: PeerId([2; 32]), accepted_epoch: 5, canonical_sequence: 3, snapshot_complete: true, compatible: true, authority_eligible: true, banned: false },
            AuthorityCandidate { peer_id: PeerId([1; 32]), accepted_epoch: 5, canonical_sequence: 3, snapshot_complete: true, compatible: true, authority_eligible: true, banned: false },
        ];
        assert_eq!(elect_authority(&candidates).unwrap(), PeerId([1; 32]));
    }

    #[test]
    fn stale_authority_writes_are_rejected_after_advance() {
        let mut fencing = FencingState::new(10);
        fencing.validate_write(10).unwrap();
        fencing.advance(11).unwrap();
        assert!(matches!(fencing.validate_write(10), Err(FencingError::Stale { .. })));
        fencing.validate_write(11).unwrap();
    }
}
