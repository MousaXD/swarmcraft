use crate::{Storage, StorageError};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
};
use swarm_protocol::{BlobDescriptor, BlobEncoding, SnapshotManifestV1, WorldId};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReplicationError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("blob offset mismatch: receiver has {expected} bytes but sender used offset {received}")]
    OffsetMismatch { expected: u64, received: u64 },
    #[error("blob encoded size mismatch: expected {expected} bytes, got {received}")]
    SizeMismatch { expected: u64, received: u64 },
    #[error("snapshot #{0} is not fully replicated")]
    Incomplete(u64),
}

impl Storage {
    pub fn missing_blobs(&self, manifest: &SnapshotManifestV1) -> Vec<BlobDescriptor> {
        let mut seen = BTreeSet::new();
        manifest
            .entries
            .iter()
            .filter(|entry| seen.insert(entry.blob.hash) && !self.has_complete_blob(manifest.world_id, &entry.blob))
            .map(|entry| entry.blob.clone())
            .collect()
    }

    pub fn has_complete_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool {
        let path = blob_path(self, world, descriptor);
        path.is_file() && fs::metadata(path).is_ok_and(|metadata| metadata.len() == descriptor.encoded_size)
    }

    pub fn partial_blob_offset(&self, world: WorldId, descriptor: &BlobDescriptor) -> Result<u64, ReplicationError> {
        let path = partial_blob_path(self, world, descriptor);
        if !path.exists() {
            return Ok(0);
        }
        fs::metadata(&path)
            .map(|metadata| metadata.len())
            .map_err(|source| StorageError::Io { path, source }.into())
    }
}

fn blob_path(storage: &Storage, world: WorldId, descriptor: &BlobDescriptor) -> PathBuf {
    let suffix = match descriptor.encoding {
        BlobEncoding::Raw => "raw",
        BlobEncoding::Zstd => "zst",
    };
    storage.world_dir(world).join("blobs").join(format!("{}.{}", descriptor.hash.to_hex(), suffix))
}

fn partial_blob_path(storage: &Storage, world: WorldId, descriptor: &BlobDescriptor) -> PathBuf {
    let mut path = blob_path(storage, world, descriptor);
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("blob");
    path.set_extension(format!("{extension}.part"));
    path
}
