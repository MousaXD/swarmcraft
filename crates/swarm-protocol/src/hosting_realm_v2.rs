//! Proposed, INERT V2 hosting-authority protocol records.
//!
//! These are distinct from every V1 signed/hash tuple and the current wire
//! protocol. No caller may infer canonical acceptance or authority from them.
//! In particular, an old V1 binary cannot be forced to obey a sidecar policy.
//! The coordinator must approve the V2 realm migration and Agent 11 must
//! implement quorum/transport enforcement before these may be activated.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    peer_id_from_public_key, Hash32, HostingPolicyError, MembershipRecordV1, PeerId, ProtocolError, WorldConfigV1,
    WorldGenesisV1, WorldHostingPolicyV1, WorldId,
};

pub const HOSTING_REALM_FORMAT_VERSION_V2: u16 = 2;
pub const HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2: u16 = 2;

const REALM_ID_DOMAIN: &[u8] = b"swarmcraft/hosting-realm-id/v2\0";
const REALM_POLICY_SIGN_DOMAIN: &[u8] = b"swarmcraft/hosting-realm-policy-sign/v2\0";
const REALM_POLICY_RECORD_DOMAIN: &[u8] = b"swarmcraft/hosting-realm-policy-record/v2\0";
const MIGRATION_SIGN_DOMAIN: &[u8] = b"swarmcraft/hosting-migration-proposal-sign/v2\0";
const MIGRATION_RECORD_DOMAIN: &[u8] = b"swarmcraft/hosting-migration-proposal-record/v2\0";
const MIGRATION_VOTE_DOMAIN: &[u8] = b"swarmcraft/hosting-migration-vote-sign/v2\0";
const AUTHORITY_ACTION_DOMAIN: &[u8] = b"swarmcraft/hosting-authority-action-sign/v2\0";

#[derive(Debug, Error)]
pub enum HostingRealmV2Error {
    #[error("unsupported hosting realm record version {0}")]
    UnsupportedVersion(u16),
    #[error("invalid proposed hosting realm record: {0}")]
    Invalid(&'static str),
    #[error("hosting realm signature is missing or invalid")]
    InvalidSignature,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Policy(#[from] HostingPolicyError),
}

fn require_signature(signature: &[u8]) -> Result<(), HostingRealmV2Error> {
    if signature.len() == 64 {
        Ok(())
    } else {
        Err(HostingRealmV2Error::InvalidSignature)
    }
}

fn require_identity(peer: PeerId, public_key: &[u8; 32]) -> Result<(), HostingRealmV2Error> {
    if peer_id_from_public_key(public_key) == peer {
        Ok(())
    } else {
        Err(HostingRealmV2Error::Invalid("signer peer ID does not match its public key"))
    }
}

fn signed_bytes<T: Serialize>(domain: &[u8], value: &T) -> Result<Vec<u8>, HostingRealmV2Error> {
    let encoded = postcard::to_allocvec(value).map_err(ProtocolError::from)?;
    let mut out = Vec::with_capacity(domain.len() + encoded.len());
    out.extend_from_slice(domain);
    out.extend_from_slice(&encoded);
    Ok(out)
}

/// Frozen V1 history anchor. All referenced records must already be accepted,
/// signed and quorum-verified by the independent V1 consensus/storage system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceHistoryAnchorV2 {
    pub source_world_id: WorldId,
    pub source_config_sequence: u64,
    pub source_config_hash: Hash32,
    pub source_membership_sequence: u64,
    pub source_membership_hash: Hash32,
    pub source_snapshot_hash: Hash32,
    pub source_epoch: u64,
    pub source_fencing_token: u64,
}

impl SourceHistoryAnchorV2 {
    pub fn validate_against_accepted_v1(
        &self,
        genesis: &WorldGenesisV1,
        config: &WorldConfigV1,
        membership: &MembershipRecordV1,
    ) -> Result<(), HostingRealmV2Error> {
        genesis.validate_semantics()?;
        config.validate_semantics()?;
        membership.validate_semantics()?;
        if self.source_world_id != genesis.world_id()?
            || self.source_world_id != config.world_id
            || self.source_world_id != membership.world_id
            || self.source_config_sequence != config.sequence
            || self.source_config_hash != config.config_hash()?
            || self.source_membership_sequence != membership.sequence
            || self.source_membership_hash != membership.record_hash()?
            || self.source_snapshot_hash == Hash32::default()
            || self.source_epoch == 0
            || self.source_fencing_token == 0
            || membership.epoch > self.source_epoch
        {
            return Err(HostingRealmV2Error::Invalid("source V1 history anchor does not match accepted records"));
        }
        Ok(())
    }
}

/// Derives a *different active world ID*, with the original world retained as
/// signed historical ancestry. This is NOT an in-place V1 world-ID upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedHostingRealmGenesisV2 {
    pub format_version: u16,
    pub source: SourceHistoryAnchorV2,
    pub migration_nonce: [u8; 32],
    pub required_authority_feature_version: u16,
}

impl ProposedHostingRealmGenesisV2 {
    pub fn realm_id(&self) -> Result<WorldId, HostingRealmV2Error> {
        if self.format_version != HOSTING_REALM_FORMAT_VERSION_V2 {
            return Err(HostingRealmV2Error::UnsupportedVersion(self.format_version));
        }
        if self.required_authority_feature_version != HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2 {
            return Err(HostingRealmV2Error::Invalid("mandatory V2 authority feature is absent"));
        }
        if self.migration_nonce == [0; 32] {
            return Err(HostingRealmV2Error::Invalid("migration nonce must be nonzero"));
        }
        let bytes = postcard::to_allocvec(self).map_err(ProtocolError::from)?;
        let id = WorldId(Hash32::from_domain_bytes(REALM_ID_DOMAIN, &bytes).0);
        if id == self.source.source_world_id {
            return Err(HostingRealmV2Error::Invalid("target realm must differ from the V1 source world"));
        }
        Ok(id)
    }
}

/// Separate signed *target-realm* policy. A V1 signed-policy sidecar binds to
/// the source V1 world/config and cannot be reused as this record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRealmHostingPolicyV2 {
    pub format_version: u16,
    pub realm_id: WorldId,
    pub sequence: u64,
    pub previous_policy_hash: Option<Hash32>,
    pub policy: WorldHostingPolicyV1,
    pub signer_peer_id: PeerId,
    pub signer_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl SignedRealmHostingPolicyV2 {
    fn validate_unsigned(&self) -> Result<(), HostingRealmV2Error> {
        if self.format_version != HOSTING_REALM_FORMAT_VERSION_V2 {
            return Err(HostingRealmV2Error::UnsupportedVersion(self.format_version));
        }
        self.policy.validate()?;
        if self.sequence == 0 || (self.sequence == 1) != self.previous_policy_hash.is_none() {
            return Err(HostingRealmV2Error::Invalid("invalid policy sequence or predecessor"));
        }
        require_identity(self.signer_peer_id, &self.signer_public_key)
    }

    pub fn validate_shape(&self) -> Result<(), HostingRealmV2Error> {
        self.validate_unsigned()?;
        require_signature(&self.signature)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, HostingRealmV2Error> {
        self.validate_unsigned()?;
        signed_bytes(
            REALM_POLICY_SIGN_DOMAIN,
            &(
                self.format_version,
                self.realm_id,
                self.sequence,
                self.previous_policy_hash,
                self.policy,
                self.signer_peer_id,
                self.signer_public_key,
            ),
        )
    }

    pub fn record_hash(&self) -> Result<Hash32, HostingRealmV2Error> {
        self.validate_shape()?;
        Ok(Hash32::from_domain_bytes(
            REALM_POLICY_RECORD_DOMAIN,
            &postcard::to_allocvec(self).map_err(ProtocolError::from)?,
        ))
    }

    pub fn verify_for_realm(
        &self,
        realm: &ProposedHostingRealmGenesisV2,
        expected_signer: PeerId,
        verify_ed25519: impl FnOnce(&[u8; 32], &[u8], &[u8]) -> bool,
    ) -> Result<(), HostingRealmV2Error> {
        self.validate_shape()?;
        if self.realm_id != realm.realm_id()? || self.signer_peer_id != expected_signer {
            return Err(HostingRealmV2Error::Invalid("policy realm or signer differs from expected identity"));
        }
        if !verify_ed25519(&self.signer_public_key, &self.signing_bytes()?, &self.signature) {
            return Err(HostingRealmV2Error::InvalidSignature);
        }
        Ok(())
    }

    /// Hash-chain *shape* only; canonical acceptance and uniqueness are
    /// delegated to Agent 11 quorum and durable head implementation.
    pub fn validate_successor(&self, previous: &Self) -> Result<(), HostingRealmV2Error> {
        self.validate_shape()?;
        previous.validate_shape()?;
        if self.realm_id != previous.realm_id
            || previous.sequence.checked_add(1) != Some(self.sequence)
            || self.previous_policy_hash != Some(previous.record_hash()?)
        {
            return Err(HostingRealmV2Error::Invalid("policy does not extend accepted target-realm predecessor"));
        }
        Ok(())
    }
}

/// V1 source authority's *proposal*, NOT a canonical migration certificate.
/// Existing signed history is left untouched and the V2 realm ID is distinct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedHostingMigrationProposalV2 {
    pub format_version: u16,
    pub realm_genesis: ProposedHostingRealmGenesisV2,
    pub target_realm_id: WorldId,
    pub initial_policy_hash: Hash32,
    pub activation_epoch: u64,
    pub activation_fencing_token: u64,
    pub source_authority_peer_id: PeerId,
    pub source_authority_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl SignedHostingMigrationProposalV2 {
    fn validate_unsigned(&self) -> Result<(), HostingRealmV2Error> {
        if self.format_version != HOSTING_REALM_FORMAT_VERSION_V2 {
            return Err(HostingRealmV2Error::UnsupportedVersion(self.format_version));
        }
        if self.target_realm_id != self.realm_genesis.realm_id()?
            || self.initial_policy_hash == Hash32::default()
            || self.activation_epoch <= self.realm_genesis.source.source_epoch
            || self.activation_fencing_token <= self.realm_genesis.source.source_fencing_token
        {
            return Err(HostingRealmV2Error::Invalid("migration target, policy or fencing is invalid"));
        }
        require_identity(self.source_authority_peer_id, &self.source_authority_public_key)
    }

    pub fn validate_shape(&self) -> Result<(), HostingRealmV2Error> {
        self.validate_unsigned()?;
        require_signature(&self.signature)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, HostingRealmV2Error> {
        self.validate_unsigned()?;
        signed_bytes(
            MIGRATION_SIGN_DOMAIN,
            &(
                self.format_version,
                self.realm_genesis,
                self.target_realm_id,
                self.initial_policy_hash,
                self.activation_epoch,
                self.activation_fencing_token,
                self.source_authority_peer_id,
                self.source_authority_public_key,
            ),
        )
    }

    pub fn proposal_hash(&self) -> Result<Hash32, HostingRealmV2Error> {
        self.validate_shape()?;
        Ok(Hash32::from_domain_bytes(
            MIGRATION_RECORD_DOMAIN,
            &postcard::to_allocvec(self).map_err(ProtocolError::from)?,
        ))
    }

    /// The caller MUST independently verify signed/canonical V1 history,
    /// current V1 authority fencing, complete snapshot and all quorum votes.
    pub fn verify_against_accepted_source(
        &self,
        genesis: &WorldGenesisV1,
        config: &WorldConfigV1,
        membership: &MembershipRecordV1,
        initial_policy: &SignedRealmHostingPolicyV2,
        verify_ed25519: impl Fn(&[u8; 32], &[u8], &[u8]) -> bool,
    ) -> Result<(), HostingRealmV2Error> {
        self.validate_shape()?;
        self.realm_genesis.source.validate_against_accepted_v1(genesis, config, membership)?;
        if self.source_authority_peer_id != config.authority_peer_id
            || self.source_authority_public_key != config.authority_public_key
            || initial_policy.sequence != 1
            || initial_policy.record_hash()? != self.initial_policy_hash
        {
            return Err(HostingRealmV2Error::Invalid("migration signer or initial policy is not source-bound"));
        }
        initial_policy.verify_for_realm(&self.realm_genesis, self.source_authority_peer_id, &verify_ed25519)?;
        if !verify_ed25519(&self.source_authority_public_key, &self.signing_bytes()?, &self.signature) {
            return Err(HostingRealmV2Error::InvalidSignature);
        }
        Ok(())
    }
}

/// Signed V2 migration vote. Only Agent 11 may assemble and accept a canonical
/// signed majority/joint certificate from an authenticated voter universe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedHostingMigrationVoteV2 {
    pub format_version: u16,
    pub target_realm_id: WorldId,
    pub migration_proposal_hash: Hash32,
    pub source_membership_hash: Hash32,
    pub voter_peer_id: PeerId,
    pub voter_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl SignedHostingMigrationVoteV2 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, HostingRealmV2Error> {
        if self.format_version != HOSTING_REALM_FORMAT_VERSION_V2 {
            return Err(HostingRealmV2Error::UnsupportedVersion(self.format_version));
        }
        require_identity(self.voter_peer_id, &self.voter_public_key)?;
        signed_bytes(
            MIGRATION_VOTE_DOMAIN,
            &(
                self.format_version,
                self.target_realm_id,
                self.migration_proposal_hash,
                self.source_membership_hash,
                self.voter_peer_id,
                self.voter_public_key,
            ),
        )
    }

    pub fn verify_for_proposal(
        &self,
        proposal: &SignedHostingMigrationProposalV2,
        membership: &MembershipRecordV1,
        verify_ed25519: impl FnOnce(&[u8; 32], &[u8], &[u8]) -> bool,
    ) -> Result<(), HostingRealmV2Error> {
        require_signature(&self.signature)?;
        membership.validate_semantics()?;
        if self.target_realm_id != proposal.target_realm_id
            || self.migration_proposal_hash != proposal.proposal_hash()?
            || self.source_membership_hash != membership.record_hash()?
            || membership.world_id != proposal.realm_genesis.source.source_world_id
        {
            return Err(HostingRealmV2Error::Invalid("migration vote does not bind the accepted source/proposal"));
        }
        if !membership
            .members
            .iter()
            .any(|m| m.peer_id == self.voter_peer_id && m.public_key == self.voter_public_key && !m.banned)
        {
            return Err(HostingRealmV2Error::Invalid("migration voter is absent or banned"));
        }
        if !verify_ed25519(&self.voter_public_key, &self.signing_bytes()?, &self.signature) {
            return Err(HostingRealmV2Error::InvalidSignature);
        }
        Ok(())
    }
}

/// Operations use a distinct V2 signature domain and cannot be interpreted as
/// a V1 RecoveryBallot, transfer or lease. Still no live wire/consensus code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostingAuthorityOperationV2 {
    RecoveryBallot,
    RecoveryVote,
    LeaseGrant,
    LeaseRenew,
    TransferPrepare,
    TransferAccept,
    TransferCommit,
    Wake,
    DirectStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedHostingAuthorityActionV2 {
    pub format_version: u16,
    pub operation: HostingAuthorityOperationV2,
    pub realm_id: WorldId,
    pub committed_policy_hash: Hash32,
    pub committed_migration_hash: Hash32,
    pub accepted_membership_hash: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub snapshot_hash: Hash32,
    pub candidate_peer_id: PeerId,
    pub consent_revision: u64,
    pub selected_heap_mib: u32,
    pub signer_peer_id: PeerId,
    pub signer_public_key: [u8; 32],
    pub nonce: [u8; 32],
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExpectedHostingAuthorityHeadV2 {
    pub operation: HostingAuthorityOperationV2,
    pub realm_id: WorldId,
    pub committed_policy_hash: Hash32,
    pub committed_migration_hash: Hash32,
    pub accepted_membership_hash: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub snapshot_hash: Hash32,
    pub candidate_peer_id: PeerId,
    pub expected_consent_revision: u64,
    pub expected_selected_heap_mib: u32,
    pub expected_signer_peer_id: PeerId,
    pub expected_nonce: [u8; 32],
}

impl SignedHostingAuthorityActionV2 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, HostingRealmV2Error> {
        if self.format_version != HOSTING_REALM_FORMAT_VERSION_V2 {
            return Err(HostingRealmV2Error::UnsupportedVersion(self.format_version));
        }
        if self.epoch == 0
            || self.fencing_token == 0
            || self.nonce == [0; 32]
            || self.consent_revision == 0
            || !(super::MIN_HOSTING_HEAP_MIB..=super::MAX_HOSTING_HEAP_MIB).contains(&self.selected_heap_mib)
        {
            return Err(HostingRealmV2Error::Invalid("authority generation, nonce or consent is invalid"));
        }
        require_identity(self.signer_peer_id, &self.signer_public_key)?;
        signed_bytes(
            AUTHORITY_ACTION_DOMAIN,
            &(
                self.format_version,
                self.operation,
                self.realm_id,
                self.committed_policy_hash,
                self.committed_migration_hash,
                self.accepted_membership_hash,
                self.epoch,
                self.fencing_token,
                self.snapshot_hash,
                self.candidate_peer_id,
                self.consent_revision,
                self.selected_heap_mib,
                self.signer_peer_id,
                self.signer_public_key,
                self.nonce,
            ),
        )
    }

    /// Verifies only the signed record and exact expected context. A success
    /// is NOT a lease, quorum, nonce consumption or local hosting permit.
    pub fn verify_for_head(
        &self,
        expected: &ExpectedHostingAuthorityHeadV2,
        verify_ed25519: impl FnOnce(&[u8; 32], &[u8], &[u8]) -> bool,
    ) -> Result<(), HostingRealmV2Error> {
        if self.operation != expected.operation
            || self.realm_id != expected.realm_id
            || self.committed_policy_hash != expected.committed_policy_hash
            || self.committed_migration_hash != expected.committed_migration_hash
            || self.accepted_membership_hash != expected.accepted_membership_hash
            || self.epoch != expected.epoch
            || self.fencing_token != expected.fencing_token
            || self.snapshot_hash != expected.snapshot_hash
            || self.candidate_peer_id != expected.candidate_peer_id
            || self.consent_revision != expected.expected_consent_revision
            || self.selected_heap_mib != expected.expected_selected_heap_mib
            || self.signer_peer_id != expected.expected_signer_peer_id
            || self.nonce != expected.expected_nonce
        {
            return Err(HostingRealmV2Error::Invalid("V2 authority action does not match expected canonical head"));
        }
        require_signature(&self.signature)?;
        if !verify_ed25519(&self.signer_public_key, &self.signing_bytes()?, &self.signature) {
            return Err(HostingRealmV2Error::InvalidSignature);
        }
        Ok(())
    }
}
