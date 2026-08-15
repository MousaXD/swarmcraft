use std::collections::BTreeSet;

use swarm_protocol::{PeerId, RecoveryBallotV1, RecoveryCertificateV1};
use thiserror::Error;

use crate::quorum_size;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryBallotDecision {
    Accept,
    Idempotent,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecoveryBallotError {
    #[error("recovery ballot generation is malformed")]
    MalformedGeneration,
    #[error("recovery ballot round {received} is stale; highest durable round is {highest}")]
    StaleRound { received: u64, highest: u64 },
    #[error("recovery round {round} is already durably promised to a different ballot")]
    ConflictingSameRound { round: u64 },
    #[error("new recovery round changed its canonical base")]
    CanonicalBaseChanged,
}

/// Apply the Paxos-style promise rule used for crash-recovery ballots.
///
/// A peer may move its durable promise to a strictly higher round only when the
/// canonical base is unchanged. It never forgets a promise because time passed.
/// This lets a later successor supersede an abandoned candidate without allowing
/// the abandoned candidate to complete after a quorum has moved forward.
pub fn evaluate_recovery_ballot(
    durable: Option<&RecoveryBallotV1>,
    proposed: &RecoveryBallotV1,
) -> Result<RecoveryBallotDecision, RecoveryBallotError> {
    if !proposed.generation_is_well_formed() {
        return Err(RecoveryBallotError::MalformedGeneration);
    }
    let Some(existing) = durable else {
        return Ok(RecoveryBallotDecision::Accept);
    };
    if proposed.round < existing.round {
        return Err(RecoveryBallotError::StaleRound { received: proposed.round, highest: existing.round });
    }
    if proposed.round == existing.round {
        if proposed.ballot_hash().ok() == existing.ballot_hash().ok() {
            return Ok(RecoveryBallotDecision::Idempotent);
        }
        return Err(RecoveryBallotError::ConflictingSameRound { round: proposed.round });
    }
    if !same_canonical_base(existing, proposed) {
        return Err(RecoveryBallotError::CanonicalBaseChanged);
    }
    Ok(RecoveryBallotDecision::Accept)
}

pub fn same_canonical_base(a: &RecoveryBallotV1, b: &RecoveryBallotV1) -> bool {
    a.world_id == b.world_id
        && a.base_epoch == b.base_epoch
        && a.base_fencing_token == b.base_fencing_token
        && a.target_epoch == b.target_epoch
        && a.target_fencing_token == b.target_fencing_token
        && a.base_snapshot_hash == b.base_snapshot_hash
        && a.base_state_hash == b.base_state_hash
        && a.membership_hash == b.membership_hash
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecoveryCertificateError {
    #[error("recovery ballot generation is malformed")]
    MalformedBallot,
    #[error("recovery certificate contains a vote that does not match its ballot")]
    VoteMismatch,
    #[error("recovery certificate contains a vote from a non-member")]
    NonMemberVote,
    #[error("recovery certificate has only {votes} unique canonical votes; quorum requires {required}")]
    QuorumUnavailable { votes: usize, required: usize },
}

/// Validate quorum shape. Cryptographic signature checks are performed by
/// `swarm-core`; this function enforces ballot identity, membership, uniqueness,
/// and majority intersection.
pub fn validate_recovery_certificate_shape(
    certificate: &RecoveryCertificateV1,
    canonical_members: &[PeerId],
) -> Result<(), RecoveryCertificateError> {
    if !certificate.ballot.generation_is_well_formed() {
        return Err(RecoveryCertificateError::MalformedBallot);
    }
    let members = canonical_members.iter().copied().collect::<BTreeSet<_>>();
    let mut voters = BTreeSet::new();
    for vote in &certificate.votes {
        if !vote.matches_ballot(&certificate.ballot).unwrap_or(false) {
            return Err(RecoveryCertificateError::VoteMismatch);
        }
        if !members.contains(&vote.voter_peer_id) {
            return Err(RecoveryCertificateError::NonMemberVote);
        }
        voters.insert(vote.voter_peer_id);
    }
    let required = quorum_size(members.len());
    if voters.len() < required {
        return Err(RecoveryCertificateError::QuorumUnavailable { votes: voters.len(), required });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{Hash32, RecoveryVoteV1, WorldId, PROTOCOL_VERSION};

    fn ballot(candidate: u8, round: u64) -> RecoveryBallotV1 {
        RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            base_epoch: 7,
            base_fencing_token: 11,
            target_epoch: 8,
            target_fencing_token: 12,
            round,
            candidate_peer_id: PeerId([candidate; 32]),
            candidate_public_key: [candidate; 32],
            base_snapshot_hash: Hash32([2; 32]),
            base_state_hash: Hash32([3; 32]),
            membership_hash: Hash32([4; 32]),
            signature: Vec::new(),
        }
    }

    fn vote(ballot: &RecoveryBallotV1, voter: u8) -> RecoveryVoteV1 {
        RecoveryVoteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: ballot.world_id,
            ballot_hash: ballot.ballot_hash().unwrap(),
            base_epoch: ballot.base_epoch,
            target_epoch: ballot.target_epoch,
            round: ballot.round,
            candidate_peer_id: ballot.candidate_peer_id,
            voter_peer_id: PeerId([voter; 32]),
            voter_public_key: [voter; 32],
            signature: Vec::new(),
        }
    }

    #[test]
    fn successor_can_supersede_abandoned_candidate_with_higher_round() {
        let bob = ballot(2, 1);
        let charlie = ballot(3, 2);
        assert_eq!(evaluate_recovery_ballot(Some(&bob), &charlie), Ok(RecoveryBallotDecision::Accept));
    }

    #[test]
    fn stale_candidate_cannot_return_after_higher_round_promise() {
        let bob = ballot(2, 1);
        let charlie = ballot(3, 2);
        assert!(matches!(
            evaluate_recovery_ballot(Some(&charlie), &bob),
            Err(RecoveryBallotError::StaleRound { received: 1, highest: 2 })
        ));
    }

    #[test]
    fn same_round_cannot_be_promised_to_two_candidates() {
        let bob = ballot(2, 4);
        let charlie = ballot(3, 4);
        assert_eq!(
            evaluate_recovery_ballot(Some(&bob), &charlie),
            Err(RecoveryBallotError::ConflictingSameRound { round: 4 })
        );
    }

    #[test]
    fn higher_round_cannot_switch_canonical_base() {
        let bob = ballot(2, 1);
        let mut charlie = ballot(3, 2);
        charlie.base_state_hash = Hash32([9; 32]);
        assert_eq!(
            evaluate_recovery_ballot(Some(&bob), &charlie),
            Err(RecoveryBallotError::CanonicalBaseChanged)
        );
    }

    #[test]
    fn quorum_certificate_requires_unique_canonical_members() {
        let b = ballot(2, 3);
        let members = vec![PeerId([1; 32]), PeerId([2; 32]), PeerId([3; 32])];
        let certificate = RecoveryCertificateV1 { ballot: b.clone(), votes: vec![vote(&b, 1), vote(&b, 2)] };
        validate_recovery_certificate_shape(&certificate, &members).unwrap();

        let duplicate = RecoveryCertificateV1 { ballot: b.clone(), votes: vec![vote(&b, 1), vote(&b, 1)] };
        assert!(matches!(
            validate_recovery_certificate_shape(&duplicate, &members),
            Err(RecoveryCertificateError::QuorumUnavailable { votes: 1, required: 2 })
        ));
    }

    #[test]
    fn partitioned_majorities_cannot_form_for_different_rounds_without_intersection_peer_moving_forward() {
        let bob = ballot(2, 1);
        let charlie = ballot(3, 2);
        let mut promises = [None, None, None];
        for index in [0usize, 1] {
            assert!(evaluate_recovery_ballot(promises[index].as_ref(), &bob).is_ok());
            promises[index] = Some(bob.clone());
        }
        for index in [1usize, 2] {
            assert!(evaluate_recovery_ballot(promises[index].as_ref(), &charlie).is_ok());
            promises[index] = Some(charlie.clone());
        }
        assert!(evaluate_recovery_ballot(promises[1].as_ref(), &bob).is_err());
    }
}
