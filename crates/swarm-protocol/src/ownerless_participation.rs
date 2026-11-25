//! #67 reference participation. These are identity and eligibility proofs,
//! not V1 invitations, Minecraft account authentication, or execution permits.
use serde::{Deserialize, Serialize};

use super::{peer_id_from_public_key, Hash32, OwnerlessError, OwnerlessWorldIdV1, PeerId, ProtocolError};

pub const MAX_OWNERLESS_PLAYERS: usize = 128;
pub const MAX_OWNERLESS_WORKERS: usize = 32;
const CHALLENGE_DOMAIN: &[u8] = b"swarmcraft/ownerless-participation/challenge/v1\0";
const REQUEST_DOMAIN: &[u8] = b"swarmcraft/ownerless-participation/request/v1\0";
const RESPONSE_DOMAIN: &[u8] = b"swarmcraft/ownerless-participation/response/v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerlessParticipationMode {
    InviteOnly,
    PublicAuthenticated,
}

/// This is deliberately not a validator enrollment record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessParticipantV1 {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
}
impl OwnerlessParticipantV1 {
    pub fn from_key(public_key: [u8; 32]) -> Self {
        Self { peer_id: peer_id_from_public_key(&public_key), public_key }
    }
}

/// Whole reference-worker budget. Neither a Java heap target nor observed
/// kernel enforcement. Positive limits alone confer no eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessWorkerBudgetV1 {
    pub memory_mib: u32,
    pub cpu_millicores: u32,
}
impl OwnerlessWorkerBudgetV1 {
    pub fn validate(&self) -> Result<(), OwnerlessError> {
        if !(512..=65536).contains(&self.memory_mib) || !(250..=64000).contains(&self.cpu_millicores) {
            return Err(OwnerlessError::Invalid("invalid positive ownerless worker budget"));
        }
        Ok(())
    }
    pub fn contains(&self, requested: Self) -> bool {
        requested.memory_mib <= self.memory_mib && requested.cpu_millicores <= self.cpu_millicores
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessWorkerEligibilityV1 {
    pub worker: OwnerlessParticipantV1,
    pub maximum: OwnerlessWorkerBudgetV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerlessParticipationIntentV1 {
    Play,
    /// Explicit one-shot volunteer offer. Not retained hosting consent.
    Volunteer {
        budget: OwnerlessWorkerBudgetV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessParticipationChallengeV1 {
    pub world: OwnerlessWorldIdV1,
    pub head: Hash32,
    pub fence: u64,
    pub policy: Hash32,
    pub compatibility: Hash32,
    pub verifier: OwnerlessParticipantV1,
    pub subject: OwnerlessParticipantV1,
    pub nonce: [u8; 32],
    pub intent: OwnerlessParticipationIntentV1,
}
impl OwnerlessParticipationChallengeV1 {
    pub fn challenge_hash(&self) -> Result<Hash32, OwnerlessError> {
        let encoded = postcard::to_allocvec(self).map_err(ProtocolError::from)?;
        Ok(Hash32::from_domain_bytes(CHALLENGE_DOMAIN, &encoded))
    }
    pub fn request_bytes(&self) -> Result<Vec<u8>, OwnerlessError> {
        let encoded =
            postcard::to_allocvec(&(self.challenge_hash()?, self.verifier.peer_id)).map_err(ProtocolError::from)?;
        let mut bytes = REQUEST_DOMAIN.to_vec();
        bytes.extend(encoded);
        Ok(bytes)
    }
    pub fn response_bytes(&self) -> Result<Vec<u8>, OwnerlessError> {
        let encoded =
            postcard::to_allocvec(&(self.challenge_hash()?, self.subject.peer_id)).map_err(ProtocolError::from)?;
        let mut bytes = RESPONSE_DOMAIN.to_vec();
        bytes.extend(encoded);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessParticipationResponseV1 {
    pub challenge: OwnerlessParticipationChallengeV1,
    pub signature: Vec<u8>,
}

/// Verifier-signed request. The subject must independently review its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessParticipationRequestV1 {
    pub challenge: OwnerlessParticipationChallengeV1,
    pub signature: Vec<u8>,
}
