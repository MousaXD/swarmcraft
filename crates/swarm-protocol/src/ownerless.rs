//! Isolated, non-activatable ownerless creation contract (#64).
//! Never decode this as V1 genesis, membership, config or authority.
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

use super::{peer_id_from_public_key, Hash32, PeerId, ProtocolError};

pub const OWNERLESS_FORMAT_V1: u16 = 1;
pub const MIN_OWNERLESS_VALIDATORS: usize = 3;
pub const MAX_OWNERLESS_VALIDATORS: usize = 15;
pub const MAX_OWNERLESS_DOCUMENT_BYTES: usize = 64 * 1024;
const GENESIS_DOMAIN: &[u8] = b"swarmcraft/ownerless-genesis/v1\0";
const POLICY_DOMAIN: &[u8] = b"swarmcraft/ownerless-policy/v1\0";
const APPROVAL_DOMAIN: &[u8] = b"swarmcraft/ownerless-creation-approval/v1\0";

#[derive(Debug, Error)]
pub enum OwnerlessError {
    #[error("invalid ownerless creation: {0}")]
    Invalid(&'static str),
    #[error("ownerless creation signature invalid")]
    Signature,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OwnerlessWorldIdV1(Hash32);

impl fmt::Display for OwnerlessWorldIdV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scownerless:{}", self.0)
    }
}

impl FromStr for OwnerlessWorldIdV1 {
    type Err = OwnerlessError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hash =
            s.strip_prefix("scownerless:").ok_or(OwnerlessError::Invalid("ownerless identity prefix required"))?;
        let hash: Hash32 = hash.parse()?;
        if hash == Hash32::default() {
            return Err(OwnerlessError::Invalid("zero ownerless identity"));
        }
        Ok(Self(hash))
    }
}

/// These are immutable required invariants, not configurable defaults.
/// Changing any protected rule requires a new identity and new approvals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessPolicyV1 {
    pub founder_privileges: bool,
    pub player_op: bool,
    pub rcon: bool,
    pub privileged_console: bool,
    pub command_blocks: bool,
    pub solo_advancement: bool,
    pub compute_grants_vote: bool,
    pub authenticated_invite_only: bool,
    pub immutable_rules: bool,
}

pub const OWNERLESS_NO_OP_V1: OwnerlessPolicyV1 = OwnerlessPolicyV1 {
    founder_privileges: false,
    player_op: false,
    rcon: false,
    privileged_console: false,
    command_blocks: false,
    solo_advancement: false,
    compute_grants_vote: false,
    authenticated_invite_only: true,
    immutable_rules: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessValidatorV1 {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
}

impl OwnerlessValidatorV1 {
    pub fn from_key(public_key: [u8; 32]) -> Self {
        Self { peer_id: peer_id_from_public_key(&public_key), public_key }
    }
}

/// A separate identity namespace. No creator key, initial host, or compute
/// entitlement. The initial state commitment is a pin, not a verified save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessGenesisV1 {
    pub format_version: u16,
    pub name: String,
    pub creation_nonce: [u8; 32],
    pub compatibility_fingerprint: Hash32,
    pub initial_state_commitment: Hash32,
    pub policy: OwnerlessPolicyV1,
    pub validators: Vec<OwnerlessValidatorV1>,
}

impl OwnerlessGenesisV1 {
    pub fn validate(&self) -> Result<(), OwnerlessError> {
        if self.format_version != OWNERLESS_FORMAT_V1 {
            return Err(OwnerlessError::Invalid("unsupported format"));
        }
        if self.name.is_empty()
            || self.name.len() > 96
            || self.name.trim() != self.name
            || self.name.chars().any(char::is_control)
        {
            return Err(OwnerlessError::Invalid("invalid world name"));
        }
        if self.creation_nonce == [0; 32]
            || self.compatibility_fingerprint == Hash32::default()
            || self.initial_state_commitment == Hash32::default()
        {
            return Err(OwnerlessError::Invalid("missing genesis commitment"));
        }
        if self.policy != OWNERLESS_NO_OP_V1 {
            return Err(OwnerlessError::Invalid("protected no-OP policy differs"));
        }
        if !(MIN_OWNERLESS_VALIDATORS..=MAX_OWNERLESS_VALIDATORS).contains(&self.validators.len())
            || self.validators.windows(2).any(|w| w[0].peer_id >= w[1].peer_id)
            || self.validators.iter().any(|v| *v != OwnerlessValidatorV1::from_key(v.public_key))
        {
            return Err(OwnerlessError::Invalid("validator roster must be bounded, sorted, unique and key-bound"));
        }
        Ok(())
    }

    pub fn world_id(&self) -> Result<OwnerlessWorldIdV1, OwnerlessError> {
        self.validate()?;
        let encoded = postcard::to_allocvec(self).map_err(ProtocolError::from)?;
        Ok(OwnerlessWorldIdV1(Hash32::from_domain_bytes(GENESIS_DOMAIN, &encoded)))
    }

    pub fn policy_hash(&self) -> Result<Hash32, OwnerlessError> {
        self.validate()?;
        let encoded = postcard::to_allocvec(&self.policy).map_err(ProtocolError::from)?;
        Ok(Hash32::from_domain_bytes(POLICY_DOMAIN, &encoded))
    }

    /// Proposed stable-roster maintenance threshold only; NOT a certificate.
    /// #66 must implement canonical voter history and durable conflict fencing.
    pub fn maintenance_threshold(&self) -> Result<usize, OwnerlessError> {
        self.validate()?;
        Ok(self.validators.len() / 2 + 1)
    }

    pub fn validator(&self, peer: PeerId) -> Option<&OwnerlessValidatorV1> {
        self.validators.iter().find(|v| v.peer_id == peer)
    }

    pub fn approval_bytes(&self, voter: PeerId) -> Result<Vec<u8>, OwnerlessError> {
        let world = self.world_id()?;
        if self.validator(voter).is_none() {
            return Err(OwnerlessError::Invalid("signer is not in founding validator roster"));
        }
        let encoded = postcard::to_allocvec(&(self.format_version, world, self.policy_hash()?, voter))
            .map_err(ProtocolError::from)?;
        let mut bytes = APPROVAL_DOMAIN.to_vec();
        bytes.extend(encoded);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessCreationApprovalV1 {
    pub world_id: OwnerlessWorldIdV1,
    pub validator: PeerId,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessCreationV1 {
    pub genesis: OwnerlessGenesisV1,
    pub approvals: Vec<OwnerlessCreationApprovalV1>,
}

impl OwnerlessCreationV1 {
    /// Unanimous opt-in at creation. No implicit vote from a founder or host.
    /// Exact identity pin MUST come from independent review of the draft.
    pub fn verify(
        &self,
        expected: OwnerlessWorldIdV1,
        verify_ed25519: impl Fn(&[u8; 32], &[u8], &[u8]) -> bool,
    ) -> Result<(), OwnerlessError> {
        if self.genesis.world_id()? != expected {
            return Err(OwnerlessError::Invalid("creation does not match expected identity"));
        }
        if self.approvals.len() != self.genesis.validators.len() {
            return Err(OwnerlessError::Invalid("every founding validator must explicitly approve"));
        }
        for (approval, validator) in self.approvals.iter().zip(&self.genesis.validators) {
            if approval.world_id != expected
                || approval.validator != validator.peer_id
                || approval.signature.len() != 64
            {
                return Err(OwnerlessError::Invalid("approval set does not match exact roster and identity"));
            }
            let bytes = self.genesis.approval_bytes(validator.peer_id)?;
            if !verify_ed25519(&validator.public_key, &bytes, &approval.signature) {
                return Err(OwnerlessError::Signature);
            }
        }
        Ok(())
    }
}
