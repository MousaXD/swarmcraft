//! Reconciled #122/#125 signature and voter admission prerequisite.
//!
//! This adapter binds the one protocol quorum verifier to independent
//! accepted V1 history pins and real Ed25519 verification. It cannot create
//! an accepted V2 head, activate a policy, or authorize a runtime.
use std::collections::BTreeSet;

use ed25519_dalek::{Signature, VerifyingKey};
use swarm_protocol::{
    peer_id_from_public_key, Hash32, HostingMigrationSourceContextV2, HostingMigrationVoterSetV2,
    MembershipCertificateV1, PeerId, SignedHostingMigrationQuorumV2, SourceHistoryAnchorV2, WorldId,
    MAX_HOSTING_MIGRATION_VOTES_V2,
};
use thiserror::Error;

/// MUST be selected independently of the proposed V2 envelope. A completed
/// historical V1 membership change is stable, not a perpetually joint set.
/// A proposed change alone cannot be considered certified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedVoterUniverseV1 {
    Stable,
    CertifiedJoint { previous_membership_hash: Hash32, transition_proposal_hash: Hash32 },
}

/// Pins from independently verified canonical V1 history, not from the
/// remote proposal. For a joint transition the anchor's membership is the
/// exactly certified proposed membership at the cutover; the old membership
/// and V1 certificate are independently pinned as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedSourceV1Pins {
    pub anchor: SourceHistoryAnchorV2,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub voters: AcceptedVoterUniverseV1,
}

#[derive(Debug, Error)]
pub enum V2MigrationQuorumError {
    #[error("source history is not the independently pinned V1 head")]
    UntrustedSourceHead,
    #[error("source authority does not match the independently pinned accepted epoch")]
    InvalidSourceAuthority,
    #[error("the independently selected stable/joint voter universe is missing or different")]
    WrongVoterUniverse,
    #[error("the exact V1 membership transition lacks a verified joint certificate")]
    InvalidJointCertificate,
    #[error("V2 migration signatures or voter quorum are invalid: {0}")]
    InvalidQuorum(#[from] swarm_protocol::HostingRealmV2Error),
    #[error(transparent)]
    Protocol(#[from] swarm_protocol::ProtocolError),
}

/// A verified vote set, not a durable cutover record or runtime capability.
/// No conversion to CanonicalPolicyView::Active exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedV2MigrationVoteSet {
    source: AcceptedSourceV1Pins,
    target_realm_id: WorldId,
    migration_proposal_hash: Hash32,
    initial_policy_hash: Hash32,
    activation_epoch: u64,
    activation_fencing_token: u64,
}

impl VerifiedV2MigrationVoteSet {
    pub fn source(self) -> AcceptedSourceV1Pins {
        self.source
    }
    pub fn target_realm_id(self) -> WorldId {
        self.target_realm_id
    }
    pub fn migration_proposal_hash(self) -> Hash32 {
        self.migration_proposal_hash
    }
    pub fn initial_policy_hash(self) -> Hash32 {
        self.initial_policy_hash
    }
    pub fn activation_epoch(self) -> u64 {
        self.activation_epoch
    }
    pub fn activation_fencing_token(self) -> u64 {
        self.activation_fencing_token
    }
}

fn verify_signature(public_key: &[u8; 32], bytes: &[u8], signature: &[u8]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else { return false };
    let Ok(signature) = Signature::from_slice(signature) else { return false };
    key.verify_strict(bytes, &signature).is_ok()
}

/// Verify the exact stable or certified joint voter set, using the single
/// #125 protocol verifier rather than maintaining a second majority
/// algorithm. Source snapshot completeness, epoch ancestry, the accepted
/// V1 cutover and authority fencing must be independently established by
/// the caller. This function does not persist or consume a migration.
pub fn verify_v2_migration_votes(
    accepted: AcceptedSourceV1Pins,
    source: &HostingMigrationSourceContextV2<'_>,
    quorum: &SignedHostingMigrationQuorumV2,
    joint_certificate: Option<&MembershipCertificateV1>,
) -> Result<VerifiedV2MigrationVoteSet, V2MigrationQuorumError> {
    let proposal = source.proposal;
    let anchor = accepted.anchor;
    if proposal.realm_genesis.source != anchor
        || anchor.source_world_id == WorldId::default()
        || anchor.source_config_hash == Hash32::default()
        || anchor.source_membership_hash == Hash32::default()
        || anchor.source_snapshot_hash == Hash32::default()
        || anchor.source_epoch == 0
        || anchor.source_fencing_token == 0
        || source.genesis.world_id()? != anchor.source_world_id
        || source.config.compatibility_fingerprint()? != source.genesis.compatibility_fingerprint
        || source.current.world_id != anchor.source_world_id
        || source.current.epoch != anchor.source_epoch
        || source.current.sequence != anchor.source_membership_sequence
        || source.current.record_hash()? != anchor.source_membership_hash
        || source.config.sequence != anchor.source_config_sequence
        || source.config.config_hash()? != anchor.source_config_hash
    {
        return Err(V2MigrationQuorumError::UntrustedSourceHead);
    }
    if accepted.authority_peer_id != peer_id_from_public_key(&accepted.authority_public_key)
        || source.current.authority_peer_id != accepted.authority_peer_id
        || source.current.authority_public_key != accepted.authority_public_key
        || proposal.source_authority_peer_id != accepted.authority_peer_id
        || proposal.source_authority_public_key != accepted.authority_public_key
    {
        return Err(V2MigrationQuorumError::InvalidSourceAuthority);
    }

    match (accepted.voters, source.voter_set, joint_certificate) {
        (AcceptedVoterUniverseV1::Stable, HostingMigrationVoterSetV2::AcceptedStable, None) => {}
        (
            AcceptedVoterUniverseV1::CertifiedJoint { previous_membership_hash, transition_proposal_hash },
            HostingMigrationVoterSetV2::PendingJoint { previous },
            Some(certificate),
        ) => {
            if previous.record_hash()? != previous_membership_hash
                || certificate.proposal.previous != *previous
                || certificate.proposal.proposed != *source.current
                || certificate.proposal.proposal_hash()? != transition_proposal_hash
            {
                return Err(V2MigrationQuorumError::WrongVoterUniverse);
            }
            if certificate.votes.is_empty()
                || certificate.votes.len() > MAX_HOSTING_MIGRATION_VOTES_V2
                || previous.members.len() > MAX_HOSTING_MIGRATION_VOTES_V2
                || source.current.members.len() > MAX_HOSTING_MIGRATION_VOTES_V2
                || crate::validate_membership_certificate_shape(certificate).is_err()
            {
                return Err(V2MigrationQuorumError::InvalidJointCertificate);
            }
            let mut seen = BTreeSet::new();
            for vote in &certificate.votes {
                if !seen.insert(vote.voter_peer_id)
                    || peer_id_from_public_key(&vote.voter_public_key) != vote.voter_peer_id
                    || !vote.matches_proposal(&certificate.proposal)?
                    || !verify_signature(&vote.voter_public_key, &vote.signing_bytes()?, &vote.signature)
                {
                    return Err(V2MigrationQuorumError::InvalidJointCertificate);
                }
            }
        }
        _ => return Err(V2MigrationQuorumError::WrongVoterUniverse),
    }

    quorum.verify_for_accepted_source(source, verify_signature)?;
    Ok(VerifiedV2MigrationVoteSet {
        source: accepted,
        target_realm_id: proposal.target_realm_id,
        migration_proposal_hash: proposal.proposal_hash()?,
        initial_policy_hash: proposal.initial_policy_hash,
        activation_epoch: proposal.activation_epoch,
        activation_fencing_token: proposal.activation_fencing_token,
    })
}
