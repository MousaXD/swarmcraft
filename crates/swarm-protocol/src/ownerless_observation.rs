//! Isolated, nonce-bound current-head observations. Never an execution lease
//! or a governance vote. No wire compatibility with ordinary V1 realms.
use serde::{Deserialize, Serialize};

use super::{Hash32, OwnerlessError, OwnerlessWorldIdV1, PeerId, ProtocolError};

const REQUEST_DOMAIN: &[u8] = b"swarmcraft/ownerless-observation/request/v1\0";
const RESPONSE_DOMAIN: &[u8] = b"swarmcraft/ownerless-observation/response/v1\0";
const CONTEXT_DOMAIN: &[u8] = b"swarmcraft/ownerless-observation/context/v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessObservationV1 {
    pub world: OwnerlessWorldIdV1,
    pub head: Hash32,
    pub sequence: u64,
    pub fence: u64,
    pub checkpoint: Hash32,
    pub policy: Hash32,
    pub compatibility: Hash32,
    pub verifier: PeerId,
    pub nonce: [u8; 32],
}

impl OwnerlessObservationV1 {
    pub fn context_hash(&self) -> Result<Hash32, OwnerlessError> {
        let bytes = postcard::to_allocvec(self).map_err(ProtocolError::from)?;
        Ok(Hash32::from_domain_bytes(CONTEXT_DOMAIN, &bytes))
    }

    pub fn request_bytes(&self) -> Result<Vec<u8>, OwnerlessError> {
        let mut bytes = REQUEST_DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(&(self.context_hash()?, self.verifier)).map_err(ProtocolError::from)?);
        Ok(bytes)
    }

    pub fn response_bytes(&self, validator: PeerId) -> Result<Vec<u8>, OwnerlessError> {
        let mut bytes = RESPONSE_DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(&(self.context_hash()?, validator)).map_err(ProtocolError::from)?);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessObservationRequestV1 {
    pub context: OwnerlessObservationV1,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerlessObservationResponseV1 {
    pub context_hash: Hash32,
    pub validator: PeerId,
    pub signature: Vec<u8>,
}
