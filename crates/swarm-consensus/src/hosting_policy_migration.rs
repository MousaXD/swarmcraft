//! Candidate migration checks for a future canonically committed hosting policy.
//!
//! These helpers DO NOT accept a V1 sidecar as an activation record and are
//! intentionally not wired into the legacy daemon's authority paths. Storage
//! verification, signature checks and a mandatory V2 authority transport must
//! exist before the caller may supply an Active policy view.

use std::collections::BTreeSet;
use swarm_protocol::{Hash32, PeerId, WorldId};
use thiserror::Error;

/// Proposed commitment into a NEW versioned canonical extension family.
/// This is NOT a WorldConfigV1 or a substitute for a durable accepted record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyCommitmentIntent {
    pub world_id: WorldId,
    pub config_hash: Hash32,
    pub membership_hash: Hash32,
    pub policy_hash: Hash32,
    pub policy_sequence: u64,
    pub previous_policy_hash: Option<Hash32>,
    pub epoch: u64,
    pub fencing_token: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MigrationGuardError {
    #[error("a canonical V1 policy sidecar cannot activate hosting")]
    NotCanonicallyActivated,
    #[error("the policy commitment is not bound to the accepted canonical head")]
    CanonicalBaseMismatch,
    #[error("policy sequence or previous policy hash is invalid")]
    InvalidPolicyChain,
    #[error("a policy transition cannot reuse or roll back authority fencing")]
    StaleFencing,
    #[error("empty or duplicate canonical voter set")]
    InvalidVoterSet,
    #[error("vote from unknown or duplicate voter")]
    InvalidVote,
    #[error("canonical quorum unavailable")]
    QuorumUnavailable,
    #[error("the candidate is not an active authority-eligible member")]
    IneligibleCandidate,
    #[error("a mandatory policy authority session is absent or unsupported")]
    UnsupportedAuthoritySession,
    #[error("authority session uses stale policy, generation, or peer identity")]
    StaleAuthoritySession,
    #[error("the local device has not retained and refreshed explicit hosting consent")]
    MissingLocalConsent,
    #[error("snapshot, runtime readiness, quorum, or fencing proof is missing")]
    MissingAuthorityProof,
}

/// Check a prospective bridge using already accepted V1 config and membership
/// hashes and generation. Does NOT verify signatures, establish a quorum,
/// atomically persist the bridge, or make it active.
pub fn validate_policy_commitment_intent(
    accepted_world: WorldId,
    accepted_config_hash: Hash32,
    accepted_membership_hash: Hash32,
    accepted_epoch: u64,
    accepted_fencing_token: u64,
    previous: Option<&PolicyCommitmentIntent>,
    next: &PolicyCommitmentIntent,
) -> Result<(), MigrationGuardError> {
    if next.world_id != accepted_world
        || next.config_hash != accepted_config_hash
        || next.membership_hash != accepted_membership_hash
    {
        return Err(MigrationGuardError::CanonicalBaseMismatch);
    }
    if next.epoch <= accepted_epoch || next.fencing_token <= accepted_fencing_token {
        return Err(MigrationGuardError::StaleFencing);
    }
    match previous {
        None if next.policy_sequence == 1 && next.previous_policy_hash.is_none() => Ok(()),
        Some(old)
            if old.world_id == next.world_id
                && old.config_hash == next.config_hash
                && old.membership_hash == next.membership_hash
                && old.policy_sequence.checked_add(1) == Some(next.policy_sequence)
                && next.previous_policy_hash == Some(old.policy_hash)
                && next.epoch > old.epoch
                && next.fencing_token > old.fencing_token =>
        {
            Ok(())
        }
        _ => Err(MigrationGuardError::InvalidPolicyChain),
    }
}

/// Validate only the voter-universe/majority *shape*. The integration MUST
/// independently verify every signed ballot and a matching canonical digest,
/// and atomically persist the resulting certificate before activation.
/// Membership transitions require the existing joint old/new quorum rules.
pub fn validate_policy_vote_quorum_shape(
    canonical_voters: &[PeerId],
    approvers: &[PeerId],
) -> Result<(), MigrationGuardError> {
    let voters: BTreeSet<_> = canonical_voters.iter().copied().collect();
    if voters.is_empty() || voters.len() != canonical_voters.len() {
        return Err(MigrationGuardError::InvalidVoterSet);
    }
    let mut seen = BTreeSet::new();
    for voter in approvers {
        if !voters.contains(voter) || !seen.insert(*voter) {
            return Err(MigrationGuardError::InvalidVote);
        }
    }
    if seen.len() <= voters.len() / 2 {
        return Err(MigrationGuardError::QuorumUnavailable);
    }
    Ok(())
}

/// Describes the result of an external, durable, cryptographically verified
/// canonical migration. V1 worlds MUST always be represented as LegacyV1.
/// Supplying Active merely from a signed sidecar is a security bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalPolicyView {
    LegacyV1,
    PreparedNotCommitted,
    Active {
        policy_hash: Hash32,
        epoch: u64,
        fencing_token: u64,
    },
}

/// Facts verified by the authoritative membership, runtime, network and
/// storage layers. This is an additional *negative gate*, not authority
/// issuance or proof that these facts are true. It never grants a lease.
#[derive(Debug, Clone, Copy)]
pub struct AuthorityPolicyFacts {
    pub candidate: PeerId,
    pub canonical_member: bool,
    pub authority_eligible: bool,
    pub banned: bool,
    pub mandatory_v2_session: bool,
    pub session_peer: Option<PeerId>,
    pub session_policy_hash: Option<Hash32>,
    pub session_epoch: u64,
    pub session_fencing_token: u64,
    pub local_consent_enabled: bool,
    pub local_consent_fresh: bool,
    pub runtime_budget_verified: bool,
    pub snapshot_complete: bool,
    pub signed_quorum_verified: bool,
    pub fencing_verified: bool,
}

/// Must be invoked on EVERY authority route (launch, grant, renewal, recovery,
/// manual transfer, solo/wake and incoming authority records) once activated.
/// Non-hosting voter and snapshot operations do not pass through this gate.
pub fn check_policy_authority_gate(
    canonical: CanonicalPolicyView,
    facts: &AuthorityPolicyFacts,
) -> Result<(), MigrationGuardError> {
    let (policy_hash, epoch, fencing_token) = match canonical {
        CanonicalPolicyView::Active { policy_hash, epoch, fencing_token } => {
            (policy_hash, epoch, fencing_token)
        }
        CanonicalPolicyView::LegacyV1 | CanonicalPolicyView::PreparedNotCommitted => {
            return Err(MigrationGuardError::NotCanonicallyActivated);
        }
    };
    if !facts.canonical_member || !facts.authority_eligible || facts.banned {
        return Err(MigrationGuardError::IneligibleCandidate);
    }
    if !facts.mandatory_v2_session {
        return Err(MigrationGuardError::UnsupportedAuthoritySession);
    }
    if facts.session_peer != Some(facts.candidate)
        || facts.session_policy_hash != Some(policy_hash)
        || facts.session_epoch != epoch
        || facts.session_fencing_token != fencing_token
    {
        return Err(MigrationGuardError::StaleAuthoritySession);
    }
    if !facts.local_consent_enabled || !facts.local_consent_fresh {
        return Err(MigrationGuardError::MissingLocalConsent);
    }
    if !facts.runtime_budget_verified
        || !facts.snapshot_complete
        || !facts.signed_quorum_verified
        || !facts.fencing_verified
    {
        return Err(MigrationGuardError::MissingAuthorityProof);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(sequence: u64, previous: Option<Hash32>) -> PolicyCommitmentIntent {
        PolicyCommitmentIntent {
            world_id: WorldId([1; 32]),
            config_hash: Hash32([2; 32]),
            membership_hash: Hash32([3; 32]),
            policy_hash: Hash32([sequence as u8; 32]),
            policy_sequence: sequence,
            previous_policy_hash: previous,
            epoch: 10 + sequence,
            fencing_token: 20 + sequence,
        }
    }

    fn check(previous: Option<&PolicyCommitmentIntent>, next: &PolicyCommitmentIntent) -> Result<(), MigrationGuardError> {
        validate_policy_commitment_intent(WorldId([1; 32]), Hash32([2; 32]), Hash32([3; 32]), 10, 20, previous, next)
    }

    fn facts() -> AuthorityPolicyFacts {
        AuthorityPolicyFacts {
            candidate: PeerId([5; 32]),
            canonical_member: true,
            authority_eligible: true,
            banned: false,
            mandatory_v2_session: true,
            session_peer: Some(PeerId([5; 32])),
            session_policy_hash: Some(Hash32([1; 32])),
            session_epoch: 11,
            session_fencing_token: 21,
            local_consent_enabled: true,
            local_consent_fresh: true,
            runtime_budget_verified: true,
            snapshot_complete: true,
            signed_quorum_verified: true,
            fencing_verified: true,
        }
    }

    fn active() -> CanonicalPolicyView {
        CanonicalPolicyView::Active { policy_hash: Hash32([1; 32]), epoch: 11, fencing_token: 21 }
    }

    #[test]
    fn bridge_keeps_v1_identity_and_requires_fenced_chain() {
        let first = intent(1, None);
        assert_eq!(check(None, &first), Ok(()));
        assert_eq!(check(Some(&first), &intent(2, Some(first.policy_hash))), Ok(()));
        let mut bad = first;
        bad.world_id = WorldId([9; 32]);
        assert_eq!(check(None, &bad), Err(MigrationGuardError::CanonicalBaseMismatch));
        bad = first;
        bad.config_hash = Hash32([9; 32]);
        assert_eq!(check(None, &bad), Err(MigrationGuardError::CanonicalBaseMismatch));
        bad = first;
        bad.membership_hash = Hash32([9; 32]);
        assert_eq!(check(None, &bad), Err(MigrationGuardError::CanonicalBaseMismatch));
        bad = first;
        bad.epoch = 10;
        assert_eq!(check(None, &bad), Err(MigrationGuardError::StaleFencing));
        bad = first;
        bad.fencing_token = 20;
        assert_eq!(check(None, &bad), Err(MigrationGuardError::StaleFencing));
        bad = intent(2, Some(Hash32([99; 32])));
        assert_eq!(check(Some(&first), &bad), Err(MigrationGuardError::InvalidPolicyChain));
        bad = intent(2, Some(first.policy_hash));
        bad.fencing_token = first.fencing_token;
        assert_eq!(check(Some(&first), &bad), Err(MigrationGuardError::InvalidPolicyChain));
        assert_eq!(check(None, &intent(2, None)), Err(MigrationGuardError::InvalidPolicyChain));
    }

    #[test]
    fn only_signed_canonical_majority_shape_is_eligible() {
        let voters = [PeerId([1; 32]), PeerId([2; 32]), PeerId([3; 32])];
        assert_eq!(validate_policy_vote_quorum_shape(&voters, &voters[..2]), Ok(()));
        assert_eq!(validate_policy_vote_quorum_shape(&voters, &voters[..1]), Err(MigrationGuardError::QuorumUnavailable));
        assert_eq!(validate_policy_vote_quorum_shape(&voters, &[voters[0], voters[0]]), Err(MigrationGuardError::InvalidVote));
        assert_eq!(validate_policy_vote_quorum_shape(&voters, &[voters[0], PeerId([9; 32])]), Err(MigrationGuardError::InvalidVote));
        assert_eq!(validate_policy_vote_quorum_shape(&[], &[]), Err(MigrationGuardError::InvalidVoterSet));
        assert_eq!(validate_policy_vote_quorum_shape(&[voters[0], voters[0]], &[voters[0]]), Err(MigrationGuardError::InvalidVoterSet));
    }

    #[test]
    fn old_v1_and_uncommitted_sidecars_never_enable_authority() {
        for state in [CanonicalPolicyView::LegacyV1, CanonicalPolicyView::PreparedNotCommitted] {
            assert_eq!(check_policy_authority_gate(state, &facts()), Err(MigrationGuardError::NotCanonicallyActivated));
        }
    }

    #[test]
    fn old_authority_client_cannot_bypass_required_v2_session() {
        let mut f = facts();
        f.mandatory_v2_session = false;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::UnsupportedAuthoritySession));
        f = facts();
        f.session_policy_hash = Some(Hash32([2; 32]));
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::StaleAuthoritySession));
        f = facts();
        f.session_epoch = 10;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::StaleAuthoritySession));
        f = facts();
        f.session_fencing_token = 20;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::StaleAuthoritySession));
        f = facts();
        f.session_peer = Some(PeerId([8; 32]));
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::StaleAuthoritySession));
    }

    #[test]
    fn revoked_consent_minorities_partitions_and_recovery_are_denied() {
        let mut f = facts();
        f.local_consent_enabled = false;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::MissingLocalConsent));
        f = facts();
        f.local_consent_fresh = false;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::MissingLocalConsent));
        f = facts();
        f.signed_quorum_verified = false;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::MissingAuthorityProof));
        f = facts();
        f.fencing_verified = false;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::MissingAuthorityProof));
        f = facts();
        f.snapshot_complete = false;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::MissingAuthorityProof));
        f = facts();
        f.runtime_budget_verified = false;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::MissingAuthorityProof));
        f = facts();
        f.canonical_member = false;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::IneligibleCandidate));
        f = facts();
        f.banned = true;
        assert_eq!(check_policy_authority_gate(active(), &f), Err(MigrationGuardError::IneligibleCandidate));
        assert_eq!(check_policy_authority_gate(active(), &facts()), Ok(()));
    }
}
