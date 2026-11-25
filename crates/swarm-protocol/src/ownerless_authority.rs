//! Separate, bounded ownerless reference-authority history (#66).
//! No type here is a V1 epoch, live permit, or worker enrollment.
use serde::{Deserialize, Serialize};

use super::{
    Hash32, OwnerlessCreationV1, OwnerlessError, OwnerlessValidatorV1, OwnerlessWorldIdV1, PeerId, ProtocolError,
};

pub const MAX_OWNERLESS_HISTORY_RECORDS: usize = 128;
pub const MAX_OWNERLESS_HISTORY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_OWNERLESS_CHECKPOINT_BYTES: usize = 1024 * 1024;
const GENESIS_HEAD_DOMAIN: &[u8] = b"swarmcraft/ownerless-authority/genesis-head/v1\0";
const PROPOSAL_DOMAIN: &[u8] = b"swarmcraft/ownerless-authority/proposal/v1\0";
const VOTE_DOMAIN: &[u8] = b"swarmcraft/ownerless-authority/vote/v1\0";
const CHECKPOINT_DOMAIN: &[u8] = b"swarmcraft/ownerless-authority/reference-checkpoint/v1\0";

/// The signed genesis anchors the first head; certificate witness subsets do
/// not affect the identity of a later accepted proposal.
pub fn ownerless_genesis_head(world: OwnerlessWorldIdV1) -> Result<Hash32, OwnerlessError> {
    let bytes = postcard::to_allocvec(&world).map_err(ProtocolError::from)?;
    Ok(Hash32::from_domain_bytes(GENESIS_HEAD_DOMAIN, &bytes))
}

/// A bounded opaque reference checkpoint, NOT a Minecraft snapshot proof.
pub fn ownerless_checkpoint_commitment(bytes: &[u8]) -> Result<Hash32, OwnerlessError> {
    if bytes.is_empty() || bytes.len() > MAX_OWNERLESS_CHECKPOINT_BYTES {
        return Err(OwnerlessError::Invalid("invalid reference checkpoint size"));
    }
    Ok(Hash32::from_domain_bytes(CHECKPOINT_DOMAIN, bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerlessAuthorityActionV1 {
    /// A quorum certifies the next opaque state commitment in the current fence.
    Checkpoint { commitment: Hash32 },
    /// Recovery retains EXACT accepted state and advances the reference fence.
    Recover,
    /// Joint old/new roster approval; retains EXACT state and advances fence.
    Reconfigure { validators: Vec<OwnerlessValidatorV1> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessAuthorityProposalV1 {
    pub world_id: OwnerlessWorldIdV1,
    pub sequence: u64,
    pub parent: Hash32,
    pub policy_hash: Hash32,
    pub compatibility: Hash32,
    pub base_fence: u64,
    pub base_checkpoint: Hash32,
    pub action: OwnerlessAuthorityActionV1,
}

impl OwnerlessAuthorityProposalV1 {
    pub fn proposal_hash(&self) -> Result<Hash32, OwnerlessError> {
        let bytes = postcard::to_allocvec(self).map_err(ProtocolError::from)?;
        Ok(Hash32::from_domain_bytes(PROPOSAL_DOMAIN, &bytes))
    }

    pub fn vote_bytes(&self, validator: PeerId) -> Result<Vec<u8>, OwnerlessError> {
        let encoded = postcard::to_allocvec(&(self.proposal_hash()?, validator)).map_err(ProtocolError::from)?;
        let mut bytes = VOTE_DOMAIN.to_vec();
        bytes.extend(encoded);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessAuthorityVoteV1 {
    pub proposal_hash: Hash32,
    pub validator: PeerId,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessAuthorityCertificateV1 {
    pub proposal: OwnerlessAuthorityProposalV1,
    /// Canonically sorted distinct validators; keys come ONLY from verified rosters.
    pub votes: Vec<OwnerlessAuthorityVoteV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessAuthorityHistoryV1 {
    pub creation: OwnerlessCreationV1,
    pub records: Vec<OwnerlessAuthorityCertificateV1>,
}
