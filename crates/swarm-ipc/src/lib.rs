//! Local daemon <-> Fabric message schema.
//!
//! The transport is deliberately separate from these message types so Unix sockets, named pipes,
//! or authenticated localhost TCP can be selected without changing Minecraft semantics.

use serde::{Deserialize, Serialize};
use swarm_protocol::Hash32;

pub const IPC_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpcRequestV1 {
    Ping,
    GetWorldInfo,
    SaveBarrier { request_id: u64 },
    PrepareShutdown { request_id: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpcResponseV1 {
    Pong,
    WorldInfo {
        minecraft_version: String,
        fabric_loader_version: String,
        world_directory: String,
        compatibility_fingerprint: Hash32,
    },
    SaveComplete { request_id: u64 },
    ReadyForShutdown { request_id: u64 },
    Error { request_id: Option<u64>, code: String, message: String },
}

pub fn encode_request(request: &IpcRequestV1) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(request)
}

pub fn decode_request(bytes: &[u8]) -> Result<IpcRequestV1, postcard::Error> {
    postcard::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let request = IpcRequestV1::SaveBarrier { request_id: 42 };
        assert_eq!(decode_request(&encode_request(&request).unwrap()).unwrap(), request);
    }
}
