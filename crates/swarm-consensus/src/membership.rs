use std::collections::{BTreeMap, BTreeSet};

use swarm_protocol::{MembershipCertificateV1, MembershipProposalV1, MembershipVoteV1, PeerId, WorldMemberV1};
use thiserror::Error;

use crate::quorum_size;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MembershipConsensusError {
    #[error("membership proposal is malformed")]
    MalformedProposal,
    #[error("membership contains duplicate peer {0}")]
    DuplicateMember(PeerId),
    #[error("membership authority is not an active voter in both configurations")]
    AuthorityNotActive,
    #[error("membership vote does not match the proposed transition")]
    VoteMismatch,
    #[error("membership vote came from a peer outside the old/new voter union")]
    UnknownVoter,
    #[error("membership vote public key does not match the canonical member key")]
    VoterKeyMismatch,
    #[error("old membership quorum unavailable: votes={votes}, required={required}")]
    OldQuorumUnavailable { votes: usize, required: usize },
    #[error("new membership quorum unavailable: votes={votes}, required={required}")]
    NewQuorumUnavailable { votes: usize, required: usize },
}

fn active_voters(members: &[WorldMemberV1]) -> Result<BTreeMap<PeerId, [u8; 32]>, MembershipConsensusError> {
    let mut all = BTreeSet::new();
    let mut active = BTreeMap::new();
    for member in members {
        if !all.insert(member.peer_id) {
            return Err(MembershipConsensusError::DuplicateMember(member.peer_id));
        }
        if !member.banned {
            active.insert(member.peer_id, member.public_key);
        }
    }
    Ok(active)
}

pub fn validate_membership_proposal_shape(proposal: &MembershipProposalV1) -> Result<(), MembershipConsensusError> {
    if !proposal.validate_shape().unwrap_or(false) {
        return Err(MembershipConsensusError::MalformedProposal);
    }
    let old = active_voters(&proposal.previous.members)?;
    let new = active_voters(&proposal.proposed.members)?;
    let authority = proposal.previous.authority_peer_id;
    let key = proposal.previous.authority_public_key;
    if old.get(&authority) != Some(&key) || new.get(&authority) != Some(&key) {
        return Err(MembershipConsensusError::AuthorityNotActive);
    }
    Ok(())
}

/// Validate a joint-consensus membership certificate. Cryptographic signature
/// verification remains in swarm-core/daemon; this function owns voter-universe,
/// uniqueness and old+new quorum intersection rules.
pub fn validate_membership_certificate_shape(
    certificate: &MembershipCertificateV1,
) -> Result<(), MembershipConsensusError> {
    let proposal = &certificate.proposal;
    validate_membership_proposal_shape(proposal)?;
    let old = active_voters(&proposal.previous.members)?;
    let new = active_voters(&proposal.proposed.members)?;
    let mut seen = BTreeSet::new();
    let mut old_votes = 0usize;
    let mut new_votes = 0usize;

    for vote in &certificate.votes {
        if !vote.matches_proposal(proposal).unwrap_or(false) {
            return Err(MembershipConsensusError::VoteMismatch);
        }
        if !seen.insert(vote.voter_peer_id) {
            continue;
        }
        let old_key = old.get(&vote.voter_peer_id);
        let new_key = new.get(&vote.voter_peer_id);
        let expected = old_key.or(new_key).ok_or(MembershipConsensusError::UnknownVoter)?;
        if old_key.is_some_and(|key| key != &vote.voter_public_key)
            || new_key.is_some_and(|key| key != &vote.voter_public_key)
            || expected != &vote.voter_public_key
        {
            return Err(MembershipConsensusError::VoterKeyMismatch);
        }
        if old_key.is_some() {
            old_votes += 1;
        }
        if new_key.is_some() {
            new_votes += 1;
        }
    }

    let old_required = quorum_size(old.len());
    if old_votes < old_required {
        return Err(MembershipConsensusError::OldQuorumUnavailable { votes: old_votes, required: old_required });
    }
    let new_required = quorum_size(new.len());
    if new_votes < new_required {
        return Err(MembershipConsensusError::NewQuorumUnavailable { votes: new_votes, required: new_required });
    }
    Ok(())
}

pub fn membership_vote_for(
    proposal: &MembershipProposalV1,
    voter_peer_id: PeerId,
    voter_public_key: [u8; 32],
) -> Result<MembershipVoteV1, swarm_protocol::ProtocolError> {
    Ok(MembershipVoteV1 {
        protocol_version: swarm_protocol::PROTOCOL_VERSION,
        world_id: proposal.proposed.world_id,
        previous_membership_hash: proposal.previous.record_hash()?,
        proposed_membership_hash: proposal.proposed.record_hash()?,
        proposed_sequence: proposal.proposed.sequence,
        voter_peer_id,
        voter_public_key,
        signature: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{Hash32, MembershipRecordV1, WorldId, PROTOCOL_VERSION};

    fn member(id: u8) -> WorldMemberV1 {
        WorldMemberV1 { peer_id: PeerId([id; 32]), public_key: [id; 32], authority_eligible: true, banned: false }
    }

    fn proposal(old_ids: &[u8], new_ids: &[u8]) -> MembershipProposalV1 {
        let previous = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([9; 32]),
            epoch: 7,
            sequence: 4,
            previous_membership_hash: Some(Hash32([1; 32])),
            members: old_ids.iter().copied().map(member).collect(),
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            signature: vec![1],
        };
        let mut proposed = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: previous.world_id,
            epoch: previous.epoch,
            sequence: 5,
            previous_membership_hash: Some(previous.record_hash().unwrap()),
            members: new_ids.iter().copied().map(member).collect(),
            authority_peer_id: previous.authority_peer_id,
            authority_public_key: previous.authority_public_key,
            signature: vec![2],
        };
        proposed.members.sort_by_key(|value| value.peer_id);
        MembershipProposalV1 { previous, proposed }
    }

    fn votes(proposal: &MembershipProposalV1, ids: &[u8]) -> Vec<MembershipVoteV1> {
        ids.iter().map(|id| membership_vote_for(proposal, PeerId([*id; 32]), [*id; 32]).unwrap()).collect()
    }

    #[test]
    fn three_to_five_requires_old_and_new_majorities() {
        let p = proposal(&[1, 2, 3], &[1, 2, 3, 4, 5]);
        let invalid = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 4, 5]) };
        assert_eq!(
            validate_membership_certificate_shape(&invalid),
            Err(MembershipConsensusError::OldQuorumUnavailable { votes: 1, required: 2 })
        );

        let valid = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 2, 4]) };
        validate_membership_certificate_shape(&valid).unwrap();
        // The only old voter outside this committed joint quorum is peer 3, which
        // cannot form the stale old 2-of-3 majority after voters 1/2 lock the proposal.
        assert_eq!(quorum_size(3), 2);
    }

    #[test]
    fn five_to_three_removal_requires_intersecting_majorities() {
        let p = proposal(&[1, 2, 3, 4, 5], &[1, 2, 3]);
        let valid = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 2, 3]) };
        validate_membership_certificate_shape(&valid).unwrap();
        assert_eq!(quorum_size(5), 3);
        assert_eq!(quorum_size(3), 2);

        let invalid = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 4, 5]) };
        assert_eq!(
            validate_membership_certificate_shape(&invalid),
            Err(MembershipConsensusError::NewQuorumUnavailable { votes: 1, required: 2 })
        );
    }

    #[test]
    fn one_to_two_join_can_commit_with_old_authority_and_new_peer() {
        let p = proposal(&[1], &[1, 2]);
        let cert = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 2]) };
        validate_membership_certificate_shape(&cert).unwrap();
    }
}
