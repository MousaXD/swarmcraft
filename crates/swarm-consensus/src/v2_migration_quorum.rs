//! Cryptographic V2 migration-vote quorum verification (NOT activation).
//!
//! A valid signed vote set is only one prerequisite for a *distinct* V2 realm.
//! Trusted source pins must come from independently verified, durable V1
//! history, never a peer's proposal. No function in this module creates an
//! Active canonical policy, a lease, or an authority permission.

use std::collections::BTreeSet;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use swarm_protocol::{
    peer_id_from_public_key, Hash32, MembershipRecordV1, PeerId, SignedHostingMigrationProposalV2,
    SignedHostingMigrationVoteV2, SignedRealmHostingPolicyV2, WorldId,
};
use thiserror::Error;

const MAX_MIGRATION_VOTERS: usize = 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum V2MigrationQuorumError {
    #[error("source history is not the expected independently accepted V1 head")]
    UntrustedSourceHead,
    #[error("pending V1 membership transition requires an independently certified joint quorum")]
    PendingMembershipTransition,
    #[error("source membership record is invalid or is not canonically ordered")]
    InvalidMembership,
    #[error("source authority is not an active, eligible member with the accepted key")]
    InvalidSourceAuthority,
    #[error("source membership or migration proposal signature is invalid")]
    InvalidSourceSignature,
    #[error("migration proposal is malformed, stale or uses the source world identity")]
    InvalidProposal,
    #[error("initial V2 hosting policy is unsigned, stale, invalid or not bound to the proposal")]
    InvalidPolicy,
    #[error("migration vote set is empty, oversized, or contains a duplicate voter")]
    InvalidVoterSet,
    #[error("migration vote is not from a canonical active voter")]
    InvalidVoter,
    #[error("migration vote signature or proposal binding is invalid")]
    InvalidVoteSignature,
    #[error("V2 migration has insufficient signed canonical votes: {votes} of {required} required")]
    NoQuorum { votes: usize, required: usize },
}

/// Source pins taken from independently VERIFIED durable V1 state and its
/// accepted snapshot/epoch. A caller MUST prove there is no pending membership
/// transition, or supply its joint certificate to an eventual extended verifier.
/// These inputs are not authenticated by their mere presence here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedSourceV1Pins {
    pub world_id: WorldId,
    pub config_sequence: u64,
    pub config_hash: Hash32,
    pub membership_sequence: u64,
    pub membership_hash: Hash32,
    pub snapshot_hash: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub pending_membership_transition: bool,
}

/// Opaque outcome of verifying an individual stable-source vote set.
/// There is intentionally NO conversion to CanonicalPolicyView::Active or a
/// V2 hosting permit: atomic durable acceptance and V2 network exclusion are
/// independent and currently missing gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedV2MigrationVoteSet {
    target_realm_id: WorldId,
    migration_proposal_hash: Hash32,
    source_membership_hash: Hash32,
    activation_epoch: u64,
    activation_fencing_token: u64,
}

impl VerifiedV2MigrationVoteSet {
    pub fn target_realm_id(self) -> WorldId {
        self.target_realm_id
    }
    pub fn migration_proposal_hash(self) -> Hash32 {
        self.migration_proposal_hash
    }
    pub fn source_membership_hash(self) -> Hash32 {
        self.source_membership_hash
    }
    pub fn activation_epoch(self) -> u64 {
        self.activation_epoch
    }
    pub fn activation_fencing_token(self) -> u64 {
        self.activation_fencing_token
    }
}

fn verify_signature(public_key: &[u8; 32], bytes: &[u8], signature: &[u8]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    key.verify(bytes, &signature).is_ok()
}

/// Verify an actual Ed25519-signed V2 migration proposal and strict majority
/// of the independently pinned, stable V1 canonical voter set.
///
/// This does NOT verify the full historic chain, original source snapshot,
/// outstanding V1 lease/fence, or durable uniqueness of target realm adoption.
/// Those checks MUST happen in the source-of-truth storage and consensus
/// runtime before V2 can be activated. In particular, a successful result
/// cannot be used as a runtime hosting permit or V1 authority downgrade.
pub fn verify_v2_migration_votes(
    accepted_source: AcceptedSourceV1Pins,
    accepted_membership: &MembershipRecordV1,
    proposal: &SignedHostingMigrationProposalV2,
    initial_policy: &SignedRealmHostingPolicyV2,
    votes: &[SignedHostingMigrationVoteV2],
) -> Result<VerifiedV2MigrationVoteSet, V2MigrationQuorumError> {
    if accepted_source.pending_membership_transition {
        return Err(V2MigrationQuorumError::PendingMembershipTransition);
    }
    let source = proposal.realm_genesis.source;
    if accepted_source.world_id == WorldId::default()
        || accepted_source.config_hash == Hash32::default()
        || accepted_source.membership_hash == Hash32::default()
        || accepted_source.snapshot_hash == Hash32::default()
        || accepted_source.epoch == 0
        || accepted_source.fencing_token == 0
        || source.source_world_id != accepted_source.world_id
        || source.source_config_sequence != accepted_source.config_sequence
        || source.source_config_hash != accepted_source.config_hash
        || source.source_membership_sequence != accepted_source.membership_sequence
        || source.source_membership_hash != accepted_source.membership_hash
        || source.source_snapshot_hash != accepted_source.snapshot_hash
        || source.source_epoch != accepted_source.epoch
        || source.source_fencing_token != accepted_source.fencing_token
        || accepted_membership.world_id != accepted_source.world_id
        || accepted_membership.sequence != accepted_source.membership_sequence
        || accepted_membership.record_hash().ok() != Some(accepted_source.membership_hash)
        || accepted_membership.epoch > accepted_source.epoch
    {
        return Err(V2MigrationQuorumError::UntrustedSourceHead);
    }

    accepted_membership
        .validate_semantics()
        .map_err(|_| V2MigrationQuorumError::InvalidMembership)?;
    if accepted_membership.members.len() > MAX_MIGRATION_VOTERS
        || accepted_membership.members.iter().any(|member| {
            member.peer_id != peer_id_from_public_key(&member.public_key)
        })
    {
        return Err(V2MigrationQuorumError::InvalidMembership);
    }

    let source_authority = accepted_membership
        .members
        .iter()
        .find(|member| member.peer_id == accepted_membership.authority_peer_id);
    if source_authority.is_none_or(|member| {
        member.banned
            || !member.authority_eligible
            || member.public_key != accepted_membership.authority_public_key
            || member.peer_id != proposal.source_authority_peer_id
            || member.public_key != proposal.source_authority_public_key
    }) {
        return Err(V2MigrationQuorumError::InvalidSourceAuthority);
    }
    if !verify_signature(
        &accepted_membership.authority_public_key,
        &accepted_membership
            .signing_bytes()
            .map_err(|_| V2MigrationQuorumError::InvalidMembership)?,
        &accepted_membership.signature,
    ) {
        return Err(V2MigrationQuorumError::InvalidSourceSignature);
    }

    proposal
        .validate_shape()
        .map_err(|_| V2MigrationQuorumError::InvalidProposal)?;
    if proposal.target_realm_id == accepted_source.world_id {
        return Err(V2MigrationQuorumError::InvalidProposal);
    }
    if initial_policy.sequence != 1
        || initial_policy.previous_policy_hash.is_some()
        || initial_policy.record_hash().ok() != Some(proposal.initial_policy_hash)
        || initial_policy
            .verify_for_realm(&proposal.realm_genesis, proposal.source_authority_peer_id, verify_signature)
            .is_err()
    {
        return Err(V2MigrationQuorumError::InvalidPolicy);
    }
    if !verify_signature(
        &proposal.source_authority_public_key,
        &proposal
            .signing_bytes()
            .map_err(|_| V2MigrationQuorumError::InvalidProposal)?,
        &proposal.signature,
    ) {
        return Err(V2MigrationQuorumError::InvalidSourceSignature);
    }
    let migration_proposal_hash =
        proposal.proposal_hash().map_err(|_| V2MigrationQuorumError::InvalidProposal)?;

    if votes.is_empty() || votes.len() > MAX_MIGRATION_VOTERS {
        return Err(V2MigrationQuorumError::InvalidVoterSet);
    }
    let mut seen = BTreeSet::<PeerId>::new();
    for vote in votes {
        if !seen.insert(vote.voter_peer_id) {
            return Err(V2MigrationQuorumError::InvalidVoterSet);
        }
        if !accepted_membership.members.iter().any(|member| {
            !member.banned
                && member.peer_id == vote.voter_peer_id
                && member.public_key == vote.voter_public_key
        }) {
            return Err(V2MigrationQuorumError::InvalidVoter);
        }
        vote.verify_for_proposal(proposal, accepted_membership, verify_signature)
            .map_err(|_| V2MigrationQuorumError::InvalidVoteSignature)?;
    }
    // Player-only or storage-only members retain their V1 voting rights.
    // Authority eligibility is deliberately not used to shrink the quorum.
    let active_voters = accepted_membership.members.iter().filter(|member| !member.banned).count();
    if active_voters == 0 {
        return Err(V2MigrationQuorumError::InvalidMembership);
    }
    let required = active_voters / 2 + 1;
    if seen.len() < required {
        return Err(V2MigrationQuorumError::NoQuorum { votes: seen.len(), required });
    }

    Ok(VerifiedV2MigrationVoteSet {
        target_realm_id: proposal.target_realm_id,
        migration_proposal_hash,
        source_membership_hash: accepted_source.membership_hash,
        activation_epoch: proposal.activation_epoch,
        activation_fencing_token: proposal.activation_fencing_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use swarm_protocol::{
        ProposedHostingRealmGenesisV2, SourceHistoryAnchorV2, WorldHostingPolicyV1, WorldMemberV1,
        HOSTING_POLICY_FORMAT_VERSION, HOSTING_REALM_FORMAT_VERSION_V2, HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2,
        PROTOCOL_VERSION,
    };

    struct Fixture {
        pins: AcceptedSourceV1Pins,
        membership: MembershipRecordV1,
        policy: SignedRealmHostingPolicyV2,
        proposal: SignedHostingMigrationProposalV2,
        keys: Vec<SigningKey>,
    }

    impl Fixture {
        fn new() -> Self {
            let keys: Vec<_> = [1_u8, 2, 3]
                .iter()
                .map(|byte| SigningKey::from_bytes(&[*byte; 32]))
                .collect();
            let mut members: Vec<_> = keys
                .iter()
                .map(|key| {
                    let public_key = key.verifying_key().to_bytes();
                    WorldMemberV1 {
                        peer_id: peer_id_from_public_key(&public_key),
                        public_key,
                        authority_eligible: false,
                        banned: false,
                    }
                })
                .collect();
            let authority_key = &keys[0];
            let authority_pub = authority_key.verifying_key().to_bytes();
            let authority_id = peer_id_from_public_key(&authority_pub);
            members.iter_mut().find(|m| m.peer_id == authority_id).unwrap().authority_eligible = true;
            members.sort_by_key(|m| m.peer_id);
            let mut membership = MembershipRecordV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: WorldId([7; 32]),
                epoch: 2,
                sequence: 3,
                previous_membership_hash: Some(Hash32([21; 32])),
                members,
                authority_peer_id: authority_id,
                authority_public_key: authority_pub,
                signature: Vec::new(),
            };
            membership.signature = authority_key.sign(&membership.signing_bytes().unwrap()).to_bytes().to_vec();
            let pins = AcceptedSourceV1Pins {
                world_id: membership.world_id,
                config_sequence: 4,
                config_hash: Hash32([4; 32]),
                membership_sequence: membership.sequence,
                membership_hash: membership.record_hash().unwrap(),
                snapshot_hash: Hash32([5; 32]),
                epoch: 6,
                fencing_token: 9,
                pending_membership_transition: false,
            };
            let realm_genesis = ProposedHostingRealmGenesisV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                source: SourceHistoryAnchorV2 {
                    source_world_id: pins.world_id,
                    source_config_sequence: pins.config_sequence,
                    source_config_hash: pins.config_hash,
                    source_membership_sequence: pins.membership_sequence,
                    source_membership_hash: pins.membership_hash,
                    source_snapshot_hash: pins.snapshot_hash,
                    source_epoch: pins.epoch,
                    source_fencing_token: pins.fencing_token,
                },
                migration_nonce: [17; 32],
                required_authority_feature_version: HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2,
            };
            let mut policy = SignedRealmHostingPolicyV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                realm_id: realm_genesis.realm_id().unwrap(),
                sequence: 1,
                previous_policy_hash: None,
                policy: WorldHostingPolicyV1 {
                    format_version: HOSTING_POLICY_FORMAT_VERSION,
                    allow_member_contributions: true,
                    minimum_server_heap_mib: 1024,
                },
                signer_peer_id: authority_id,
                signer_public_key: authority_pub,
                signature: Vec::new(),
            };
            policy.signature = authority_key.sign(&policy.signing_bytes().unwrap()).to_bytes().to_vec();
            let mut proposal = SignedHostingMigrationProposalV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                target_realm_id: realm_genesis.realm_id().unwrap(),
                realm_genesis,
                initial_policy_hash: policy.record_hash().unwrap(),
                activation_epoch: 7,
                activation_fencing_token: 10,
                source_authority_peer_id: authority_id,
                source_authority_public_key: authority_pub,
                signature: Vec::new(),
            };
            proposal.signature = authority_key.sign(&proposal.signing_bytes().unwrap()).to_bytes().to_vec();
            Self { pins, membership, policy, proposal, keys }
        }

        fn vote(&self, index: usize) -> SignedHostingMigrationVoteV2 {
            let key = &self.keys[index];
            let public_key = key.verifying_key().to_bytes();
            let mut vote = SignedHostingMigrationVoteV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                target_realm_id: self.proposal.target_realm_id,
                migration_proposal_hash: self.proposal.proposal_hash().unwrap(),
                source_membership_hash: self.membership.record_hash().unwrap(),
                voter_peer_id: peer_id_from_public_key(&public_key),
                voter_public_key: public_key,
                signature: Vec::new(),
            };
            vote.signature = key.sign(&vote.signing_bytes().unwrap()).to_bytes().to_vec();
            vote
        }

        fn verify(
            &self,
            votes: &[SignedHostingMigrationVoteV2],
        ) -> Result<VerifiedV2MigrationVoteSet, V2MigrationQuorumError> {
            verify_v2_migration_votes(self.pins, &self.membership, &self.proposal, &self.policy, votes)
        }
    }

    #[test]
    fn signed_v1_canonical_majority_is_verified_without_requiring_host_eligibility() {
        let f = Fixture::new();
        let proof = f.verify(&[f.vote(1), f.vote(2)]).unwrap();
        assert_eq!(proof.target_realm_id(), f.proposal.target_realm_id);
        assert_ne!(proof.target_realm_id(), f.pins.world_id);
        assert_eq!(proof.migration_proposal_hash(), f.proposal.proposal_hash().unwrap());
        assert_eq!(proof.source_membership_hash(), f.pins.membership_hash);
        assert_eq!(proof.activation_epoch(), 7);
        assert_eq!(proof.activation_fencing_token(), 10);
        assert_eq!(
            f.verify(&[f.vote(0)]),
            Err(V2MigrationQuorumError::NoQuorum { votes: 1, required: 2 })
        );
    }

    #[test]
    fn duplicate_voters_cannot_manufacture_quorum() {
        let f = Fixture::new();
        assert_eq!(
            f.verify(&[f.vote(0), f.vote(0)]),
            Err(V2MigrationQuorumError::InvalidVoterSet)
        );
    }

    #[test]
    fn tampering_and_replay_against_stale_canonical_pins_fail_closed() {
        let f = Fixture::new();
        let mut pins = f.pins;
        pins.epoch += 1;
        assert_eq!(
            verify_v2_migration_votes(pins, &f.membership, &f.proposal, &f.policy, &[f.vote(0), f.vote(1)]),
            Err(V2MigrationQuorumError::UntrustedSourceHead)
        );
        let mut pins = f.pins;
        pins.snapshot_hash = Hash32([88; 32]);
        assert_eq!(
            verify_v2_migration_votes(pins, &f.membership, &f.proposal, &f.policy, &[f.vote(0), f.vote(1)]),
            Err(V2MigrationQuorumError::UntrustedSourceHead)
        );
        let mut pins = f.pins;
        pins.pending_membership_transition = true;
        assert_eq!(
            verify_v2_migration_votes(pins, &f.membership, &f.proposal, &f.policy, &[f.vote(0), f.vote(1)]),
            Err(V2MigrationQuorumError::PendingMembershipTransition)
        );
    }

    #[test]
    fn forged_source_proposal_and_stale_fencing_fail() {
        let mut f = Fixture::new();
        f.proposal.signature[0] ^= 1;
        assert_eq!(f.verify(&[f.vote(0), f.vote(1)]), Err(V2MigrationQuorumError::InvalidSourceSignature));
        let mut f = Fixture::new();
        let votes = [f.vote(0), f.vote(1)];
        f.proposal.activation_fencing_token = f.pins.fencing_token;
        assert_eq!(f.verify(&votes), Err(V2MigrationQuorumError::InvalidProposal));
    }

    #[test]
    fn unsigned_replayed_or_mismatched_initial_policy_is_rejected() {
        let mut f = Fixture::new();
        let votes = [f.vote(0), f.vote(1)];
        f.policy.signature[0] ^= 1;
        assert_eq!(f.verify(&votes), Err(V2MigrationQuorumError::InvalidPolicy));

        let mut f = Fixture::new();
        let votes = [f.vote(0), f.vote(1)];
        f.proposal.initial_policy_hash = Hash32([89; 32]);
        f.proposal.signature = f.keys[0].sign(&f.proposal.signing_bytes().unwrap()).to_bytes().to_vec();
        assert_eq!(f.verify(&votes), Err(V2MigrationQuorumError::InvalidPolicy));

        let mut f = Fixture::new();
        let votes = [f.vote(0), f.vote(1)];
        f.policy.sequence = 2;
        f.policy.previous_policy_hash = Some(Hash32([9; 32]));
        assert_eq!(f.verify(&votes), Err(V2MigrationQuorumError::InvalidPolicy));
    }

    #[test]
    fn stale_v1_realm_or_unsupported_v2_feature_is_rejected() {
        let mut f = Fixture::new();
        let votes = [f.vote(0), f.vote(1)];
        f.proposal.target_realm_id = f.pins.world_id;
        assert_eq!(f.verify(&votes), Err(V2MigrationQuorumError::InvalidProposal));

        let mut f = Fixture::new();
        let votes = [f.vote(0), f.vote(1)];
        f.proposal.realm_genesis.required_authority_feature_version = 1;
        assert_eq!(f.verify(&votes), Err(V2MigrationQuorumError::InvalidProposal));
    }

    #[test]
    fn corrupted_or_foreign_voter_signature_is_rejected() {
        let f = Fixture::new();
        let mut forged = f.vote(0);
        forged.signature[0] ^= 1;
        assert_eq!(
            f.verify(&[forged, f.vote(1)]),
            Err(V2MigrationQuorumError::InvalidVoteSignature)
        );
        let mut foreign = f.vote(0);
        foreign.voter_peer_id = PeerId([99; 32]);
        assert_eq!(f.verify(&[foreign, f.vote(1)]), Err(V2MigrationQuorumError::InvalidVoter));
    }

    #[test]
    fn banned_voter_and_downgraded_or_invalid_membership_are_rejected() {
        let mut f = Fixture::new();
        // In a newly signed canonical membership, banned members cannot vote.
        let banned_id = f.vote(1).voter_peer_id;
        f.membership.members.iter_mut().find(|m| m.peer_id == banned_id).unwrap().banned = true;
        f.membership.signature = f.keys[0].sign(&f.membership.signing_bytes().unwrap()).to_bytes().to_vec();
        f.pins.membership_hash = f.membership.record_hash().unwrap();
        f.proposal.realm_genesis.source.source_membership_hash = f.pins.membership_hash;
        f.proposal.target_realm_id = f.proposal.realm_genesis.realm_id().unwrap();
        f.policy.realm_id = f.proposal.target_realm_id;
        f.policy.signature = f.keys[0].sign(&f.policy.signing_bytes().unwrap()).to_bytes().to_vec();
        f.proposal.initial_policy_hash = f.policy.record_hash().unwrap();
        f.proposal.signature = f.keys[0].sign(&f.proposal.signing_bytes().unwrap()).to_bytes().to_vec();
        assert_eq!(f.verify(&[f.vote(0), f.vote(1)]), Err(V2MigrationQuorumError::InvalidVoter));
        let mut f = Fixture::new();
        f.membership.signature[0] ^= 1;
        f.pins.membership_hash = f.membership.record_hash().unwrap();
        f.proposal.realm_genesis.source.source_membership_hash = f.pins.membership_hash;
        f.proposal.target_realm_id = f.proposal.realm_genesis.realm_id().unwrap();
        f.policy.realm_id = f.proposal.target_realm_id;
        f.policy.signature = f.keys[0].sign(&f.policy.signing_bytes().unwrap()).to_bytes().to_vec();
        f.proposal.initial_policy_hash = f.policy.record_hash().unwrap();
        f.proposal.signature = f.keys[0].sign(&f.proposal.signing_bytes().unwrap()).to_bytes().to_vec();
        assert_eq!(
            f.verify(&[f.vote(0), f.vote(1)]),
            Err(V2MigrationQuorumError::InvalidSourceSignature)
        );
    }
}
