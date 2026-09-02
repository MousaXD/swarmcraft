#[path = "lib.rs"]
mod base;
pub use base::*;

mod canonical_modpack;
pub use canonical_modpack::*;

mod runtime_support;
pub use runtime_support::*;

mod v2;
pub use v2::*;

mod discovery;
pub use discovery::*;

use serde::{Deserialize, Serialize};

const JOIN_SIGN_DOMAIN: &[u8] = b"swarmcraft/join-request/v1\0";
const LEAVE_SIGN_DOMAIN: &[u8] = b"swarmcraft/leave-request/v1\0";
const SLEEP_SIGN_DOMAIN: &[u8] = b"swarmcraft/sleep-record/v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequestV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub invite: InviteV1,
    pub joining_member: WorldMemberV1,
    pub nonce: [u8; 32],
    pub signature: Vec<u8>,
}

impl JoinRequestV1 {
    pub fn validate_shape(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
            && self.invite.protocol_version == PROTOCOL_VERSION
            && self.invite.world_id == self.world_id
            && self.invite.genesis.world_id().is_ok_and(|world| world == self.world_id)
            && peer_id_from_public_key(&self.joining_member.public_key) == self.joining_member.peer_id
            && !self.joining_member.banned
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (self.protocol_version, self.world_id, &self.invite, &self.joining_member, self.nonce);
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(JOIN_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(JOIN_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaveRequestV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub membership_hash: Hash32,
    pub leaving_peer_id: PeerId,
    pub leaving_public_key: [u8; 32],
    pub nonce: [u8; 32],
    pub signature: Vec<u8>,
}

impl LeaveRequestV1 {
    pub fn validate_shape(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
            && peer_id_from_public_key(&self.leaving_public_key) == self.leaving_peer_id
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.membership_hash,
            self.leaving_peer_id,
            self.leaving_public_key,
            self.nonce,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(LEAVE_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(LEAVE_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SleepRecordV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub latest_snapshot_hash: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl SleepRecordV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.latest_snapshot_hash,
            self.epoch,
            self.fencing_token,
            self.authority_peer_id,
            self.authority_public_key,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(SLEEP_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(SLEEP_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }
}
