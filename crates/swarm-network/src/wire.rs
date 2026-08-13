use serde::{Deserialize, Serialize};
use swarm_protocol::{BlobEncoding, Hash32, PeerHelloV1, SnapshotManifestV1, WorldId, WorldStatusV1};
use thiserror::Error;

pub const MAX_BLOB_CHUNK: usize = 256 * 1024;
pub const MAX_MISSING_BLOBS: usize = 16_384;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaAckV1 {
    pub world_id: WorldId,
    pub snapshot_number: u64,
    pub manifest_hash: Hash32,
    pub state_root: Hash32,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireRequest {
    Hello(PeerHelloV1),
    Ping { nonce: u64 },
    WorldStatus { world_id: WorldId },
    SnapshotManifest(SnapshotManifestV1),
    MissingBlobs { world_id: WorldId, snapshot_number: u64, hashes: Vec<Hash32> },
    BlobChunk { world_id: WorldId, hash: Hash32, encoding: BlobEncoding, offset: u64, data: Vec<u8>, finished: bool },
    ReplicaAck(ReplicaAckV1),
}

impl WireRequest {
    pub fn validate_limits(&self) -> Result<(), WireLimitError> {
        match self {
            Self::BlobChunk { data, .. } if data.len() > MAX_BLOB_CHUNK => {
                Err(WireLimitError::BlobChunkTooLarge(data.len()))
            }
            Self::MissingBlobs { hashes, .. } if hashes.len() > MAX_MISSING_BLOBS => {
                Err(WireLimitError::TooManyBlobHashes(hashes.len()))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireResponse {
    HelloAccepted { protocol_version: u16 },
    Pong { nonce: u64 },
    WorldStatus(Option<WorldStatusV1>),
    ManifestAccepted { snapshot_number: u64, missing: Vec<Hash32> },
    MissingBlobs(Vec<Hash32>),
    BlobChunkAccepted { hash: Hash32, next_offset: u64 },
    ReplicaAckAccepted,
    Error { code: String, message: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WireLimitError {
    #[error("blob chunk is {0} bytes; maximum is {MAX_BLOB_CHUNK}")]
    BlobChunkTooLarge(usize),
    #[error("missing-blob request contains {0} hashes; maximum is {MAX_MISSING_BLOBS}")]
    TooManyBlobHashes(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_blob_chunks() {
        let request = WireRequest::BlobChunk {
            world_id: WorldId([1; 32]),
            hash: Hash32([2; 32]),
            encoding: BlobEncoding::Zstd,
            offset: 0,
            data: vec![0; MAX_BLOB_CHUNK + 1],
            finished: false,
        };
        assert_eq!(request.validate_limits(), Err(WireLimitError::BlobChunkTooLarge(MAX_BLOB_CHUNK + 1)));
    }

    #[test]
    fn rejects_unbounded_missing_blob_lists() {
        let request = WireRequest::MissingBlobs {
            world_id: WorldId([1; 32]),
            snapshot_number: 4,
            hashes: vec![Hash32([2; 32]); MAX_MISSING_BLOBS + 1],
        };
        assert_eq!(request.validate_limits(), Err(WireLimitError::TooManyBlobHashes(MAX_MISSING_BLOBS + 1)));
    }
}
