//! Stable protocol/state types for SwarmCraft protocol version 1.
//!
//! Anything hashed or signed is encoded with postcard and a domain separator.
//! JSON is intentionally not used for canonical bytes.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;
pub const STORAGE_SCHEMA_VERSION: u16 = 1;

const WORLD_GENESIS_DOMAIN: &[u8] = b"swarmcraft/world-genesis/v1\0";
const BLOB_DOMAIN: &[u8] = b"swarmcraft/blob/v1\0";
const SNAPSHOT_STATE_DOMAIN: &[u8] = b"swarmcraft/snapshot-state/v1\0";
const SNAPSHOT_SIGN_DOMAIN: &[u8] = b"swarmcraft/snapshot-sign/v1\0";
const SNAPSHOT_HASH_DOMAIN: &[u8] = b"swarmcraft/snapshot/v1\0";
const PEER_HELLO_SIGN_DOMAIN: &[u8] = b"swarmcraft/peer-hello/v1\0";
const EPOCH_SIGN_DOMAIN: &[u8] = b"swarmcraft/epoch/v1\0";

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid {kind} identifier: {value}")]
    InvalidIdentifier { kind: &'static str, value: String },
    #[error("canonical encoding failed: {0}")]
    Encode(#[from] postcard::Error),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Hash32(pub [u8; 32]);

impl Hash32 {
    pub fn from_domain_bytes(domain: &[u8], bytes: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain);
        hasher.update(bytes);
        Self(*hasher.finalize().as_bytes())
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Debug for Hash32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Hash32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for Hash32 {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_32("hash", s).map(Self)
    }
}

macro_rules! id_type {
    ($name:ident, $prefix:literal, $kind:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
        pub struct $name(pub [u8; 32]);

        impl $name {
            pub fn to_hex(self) -> String {
                hex::encode(self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), hex::encode(self.0))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), hex::encode(self.0))
            }
        }

        impl FromStr for $name {
            type Err = ProtocolError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let raw = value.strip_prefix($prefix).unwrap_or(value);
                parse_32($kind, raw).map(Self)
            }
        }
    };
}

id_type!(PeerId, "scpeer:", "peer");
id_type!(WorldId, "scworld:", "world");

fn parse_32(kind: &'static str, value: &str) -> Result<[u8; 32], ProtocolError> {
    let bytes = hex::decode(value).map_err(|_| ProtocolError::InvalidIdentifier { kind, value: value.to_owned() })?;
    bytes.try_into().map_err(|_| ProtocolError::InvalidIdentifier { kind, value: value.to_owned() })
}

pub fn peer_id_from_public_key(public_key: &[u8; 32]) -> PeerId {
    // Peer identity intentionally hashes only the public key. The scpeer prefix is presentation only.
    PeerId(*blake3::hash(public_key).as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldGenesisV1 {
    pub protocol_version: u16,
    pub minecraft_version: String,
    pub fabric_loader_version: String,
    pub compatibility_fingerprint: Hash32,
    pub creation_nonce: [u8; 32],
    pub creator_public_key: [u8; 32],
    pub initial_membership: Vec<PeerId>,
}

impl WorldGenesisV1 {
    pub fn world_id(&self) -> Result<WorldId, ProtocolError> {
        let bytes = postcard::to_allocvec(self)?;
        Ok(WorldId(Hash32::from_domain_bytes(WORLD_GENESIS_DOMAIN, &bytes).0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlobEncoding {
    Raw,
    Zstd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobDescriptor {
    pub hash: Hash32,
    pub uncompressed_size: u64,
    pub encoded_size: u64,
    pub encoding: BlobEncoding,
}

impl BlobDescriptor {
    pub fn hash_uncompressed(bytes: &[u8]) -> Hash32 {
        Hash32::from_domain_bytes(BLOB_DOMAIN, bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    /// UTF-8 relative path using `/` separators. Absolute and parent paths are forbidden.
    pub path: String,
    pub blob: BlobDescriptor,
}

pub fn snapshot_state_root(entries: &[SnapshotEntry]) -> Result<Hash32, ProtocolError> {
    let bytes = postcard::to_allocvec(entries)?;
    Ok(Hash32::from_domain_bytes(SNAPSHOT_STATE_DOMAIN, &bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotManifestV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub snapshot_number: u64,
    pub epoch: u64,
    pub sequence: u64,
    pub previous_snapshot_hash: Option<Hash32>,
    pub entries: Vec<SnapshotEntry>,
    pub state_root: Hash32,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl SnapshotManifestV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.snapshot_number,
            self.epoch,
            self.sequence,
            self.previous_snapshot_hash,
            &self.entries,
            self.state_root,
            self.authority_peer_id,
            self.authority_public_key,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(SNAPSHOT_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(SNAPSHOT_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    pub fn manifest_hash(&self) -> Result<Hash32, ProtocolError> {
        let bytes = postcard::to_allocvec(self)?;
        Ok(Hash32::from_domain_bytes(SNAPSHOT_HASH_DOMAIN, &bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EpochMode {
    Quorum,
    Solo,
    Recovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRecordV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub epoch_number: u64,
    pub previous_epoch_hash: Option<Hash32>,
    pub base_state_hash: Hash32,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub mode: EpochMode,
    pub fencing_token: u64,
    pub reason: String,
    pub signature: Vec<u8>,
}

impl EpochRecordV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.epoch_number,
            self.previous_epoch_hash,
            self.base_state_hash,
            self.authority_peer_id,
            self.authority_public_key,
            self.mode,
            self.fencing_token,
            &self.reason,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(EPOCH_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(EPOCH_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityLeaseV1 {
    pub world_id: WorldId,
    pub epoch: u64,
    pub fencing_token: u64,
    /// Duration negotiated by the protocol. Local monotonic clocks decide expiry.
    pub lease_duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHelloV1 {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub protocol_versions: Vec<u16>,
    pub capabilities: Vec<String>,
    pub nonce: [u8; 32],
    pub signature: Vec<u8>,
}

impl PeerHelloV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (self.peer_id, self.public_key, &self.protocol_versions, &self.capabilities, self.nonce);
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(PEER_HELLO_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(PEER_HELLO_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldStatusV1 {
    pub world_id: WorldId,
    pub epoch: u64,
    pub sequence: u64,
    pub latest_snapshot: Option<Hash32>,
    pub state_hash: Option<Hash32>,
    pub compatibility_fingerprint: Hash32,
    pub authority_eligible: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_id_is_deterministic_and_prefixed() {
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([7; 32]),
            creation_nonce: [9; 32],
            creator_public_key: [3; 32],
            initial_membership: vec![PeerId([4; 32])],
        };
        let a = genesis.world_id().unwrap();
        let b = genesis.world_id().unwrap();
        assert_eq!(a, b);
        assert!(a.to_string().starts_with("scworld:"));
        assert_eq!(WorldId::from_str(&a.to_string()).unwrap(), a);
    }

    #[test]
    fn state_root_changes_with_path_or_content_hash() {
        let blob = BlobDescriptor {
            hash: Hash32([1; 32]),
            uncompressed_size: 5,
            encoded_size: 4,
            encoding: BlobEncoding::Zstd,
        };
        let a = vec![SnapshotEntry { path: "a".into(), blob: blob.clone() }];
        let b = vec![SnapshotEntry { path: "b".into(), blob }];
        assert_ne!(snapshot_state_root(&a).unwrap(), snapshot_state_root(&b).unwrap());
    }
}
