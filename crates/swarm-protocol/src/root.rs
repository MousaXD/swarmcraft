#[path = "lib.rs"]
mod base;
pub use base::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequestV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub invite: InviteV1,
    pub joining_member: WorldMemberV1,
    pub nonce: [u8; 32],
    pub signature: Vec<u8>,
}
