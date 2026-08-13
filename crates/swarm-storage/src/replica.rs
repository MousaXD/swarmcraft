use crate::{Storage, StorageError};
use swarm_protocol::{BlobDescriptor, SnapshotManifestV1, WorldId};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReplicationError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("snapshot #{0} is not fully replicated")]
    Incomplete(u64),
}

impl Storage {
    pub fn missing_blobs(&self, _manifest: &SnapshotManifestV1) -> Vec<BlobDescriptor> {
        Vec::new()
    }

    pub fn partial_blob_offset(&self, _world: WorldId, _descriptor: &BlobDescriptor) -> Result<u64, ReplicationError> {
        Ok(0)
    }
}
