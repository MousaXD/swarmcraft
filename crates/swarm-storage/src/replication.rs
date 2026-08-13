//! Resumable, bounded snapshot blob replication.

use crate::{io_error, sync_parent, Storage, StorageError};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use swarm_protocol::{BlobDescriptor, BlobEncoding, Hash32, SnapshotManifestV1, WorldId, BLOB_HASH_DOMAIN};
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
        let mut missing = Vec::new();
        for entry in &manifest.entries {
            if seen.insert(entry.blob.hash) && !self.has_complete_blob(manifest.world_id, &entry.blob) {
                missing.push(entry.blob.clone());
            }
        }
        missing
    }

    pub fn has_complete_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool {
        let path = self.blob_path(world, descriptor.hash, descriptor.encoding);
        path.is_file() && verify_encoded_blob_path(&path, descriptor).is_ok()
    }

    pub fn partial_blob_offset(&self, world: WorldId, descriptor: &BlobDescriptor) -> Result<u64, ReplicationError> {
        let path = self.partial_blob_path(world, descriptor);
        if !path.exists() {
            return Ok(0);
        }
        fs::metadata(&path).map(|metadata| metadata.len()).map_err(|error| io_error(path, error).into())
    }

    pub fn read_encoded_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, bool), ReplicationError> {
        if offset > descriptor.encoded_size {
            return Err(ReplicationError::OffsetMismatch { expected: descriptor.encoded_size, received: offset });
        }
        if max_bytes == 0 {
            return Ok((Vec::new(), offset == descriptor.encoded_size));
        }
        let path = self.blob_path(world, descriptor.hash, descriptor.encoding);
        let metadata = fs::metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.len() != descriptor.encoded_size {
            return Err(StorageError::BlobCorrupt(descriptor.hash).into());
        }
        let requested = (descriptor.encoded_size - offset).min(max_bytes as u64) as usize;
        let mut file = File::open(&path).map_err(|error| io_error(&path, error))?;
        file.seek(SeekFrom::Start(offset)).map_err(|error| io_error(&path, error))?;
        let mut data = vec![0u8; requested];
        file.read_exact(&mut data).map_err(|error| io_error(&path, error))?;
        let next = offset + data.len() as u64;
        Ok((data, next == descriptor.encoded_size))
    }

    pub fn receive_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        data: &[u8],
        finished: bool,
    ) -> Result<u64, ReplicationError> {
        let dir = self.blobs_dir(world);
        fs::create_dir_all(&dir).map_err(|error| io_error(&dir, error))?;
        let partial = self.partial_blob_path(world, descriptor);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&partial)
            .map_err(|error| io_error(&partial, error))?;
        let current = file.metadata().map_err(|error| io_error(&partial, error))?.len();
        if current != offset {
            return Err(ReplicationError::OffsetMismatch { expected: current, received: offset });
        }
        let next = current.saturating_add(data.len() as u64);
        if next > descriptor.encoded_size {
            return Err(ReplicationError::SizeMismatch { expected: descriptor.encoded_size, received: next });
        }
        file.seek(SeekFrom::End(0)).map_err(|error| io_error(&partial, error))?;
        file.write_all(data).map_err(|error| io_error(&partial, error))?;
        file.sync_data().map_err(|error| io_error(&partial, error))?;
        drop(file);

        if finished {
            if next != descriptor.encoded_size {
                return Err(ReplicationError::SizeMismatch { expected: descriptor.encoded_size, received: next });
            }
            verify_encoded_blob_path(&partial, descriptor)?;
            let final_path = self.blob_path(world, descriptor.hash, descriptor.encoding);
            if final_path.exists() {
                fs::remove_file(&final_path).map_err(|error| io_error(&final_path, error))?;
            }
            fs::rename(&partial, &final_path).map_err(|error| io_error(&final_path, error))?;
            sync_parent(&dir)?;
        }
        Ok(next)
    }

    pub fn finalize_replica(&self, manifest: &SnapshotManifestV1) -> Result<(), ReplicationError> {
        if !self.missing_blobs(manifest).is_empty() {
            return Err(ReplicationError::Incomplete(manifest.snapshot_number));
        }
        self.commit_snapshot(manifest)?;
        Ok(())
    }

    fn partial_blob_path(&self, world: WorldId, descriptor: &BlobDescriptor) -> PathBuf {
        let suffix = match descriptor.encoding {
            BlobEncoding::Raw => "raw",
            BlobEncoding::Zstd => "zst",
        };
        self.blobs_dir(world).join(format!("{}.{}.part", descriptor.hash.to_hex(), suffix))
    }
}

fn verify_encoded_blob_path(path: &Path, descriptor: &BlobDescriptor) -> Result<(), ReplicationError> {
    let metadata = fs::metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.len() != descriptor.encoded_size {
        return Err(StorageError::BlobCorrupt(descriptor.hash).into());
    }
    let file = File::open(path).map_err(|error| io_error(path, error))?;
    let mut reader: Box<dyn Read> = match descriptor.encoding {
        BlobEncoding::Raw => Box::new(file),
        BlobEncoding::Zstd => {
            Box::new(zstd::stream::read::Decoder::new(file).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?)
        }
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(BLOB_HASH_DOMAIN);
    let mut total = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|error| io_error(path, error))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        hasher.update(&buffer[..read]);
    }
    if total != descriptor.uncompressed_size || Hash32(*hasher.finalize().as_bytes()) != descriptor.hash {
        return Err(StorageError::BlobCorrupt(descriptor.hash).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SnapshotContext;
    use swarm_protocol::PeerId;

    fn world() -> WorldId {
        WorldId([9; 32])
    }

    #[test]
    fn interrupted_replication_resumes_and_restores_exact_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = temp.path().join("source-world");
        fs::create_dir_all(source_dir.join("region")).unwrap();
        fs::write(source_dir.join("level.dat"), b"swarmcraft-replication-level").unwrap();
        fs::write(source_dir.join("region/r.0.0.mca"), vec![42u8; 96 * 1024]).unwrap();

        let authority = Storage::open(temp.path().join("authority-store")).unwrap();
        let mut manifest = authority
            .snapshot_directory(
                &source_dir,
                SnapshotContext {
                    world: world(),
                    snapshot_number: 1,
                    epoch: 1,
                    sequence: 7,
                    previous_snapshot_hash: None,
                    authority_peer_id: PeerId([1; 32]),
                    authority_public_key: [2; 32],
                },
            )
            .unwrap();
        manifest.signature = vec![0; 64];
        authority.commit_snapshot(&manifest).unwrap();

        let replica_root = temp.path().join("replica-store");
        let replica = Storage::open(&replica_root).unwrap();
        for descriptor in replica.missing_blobs(&manifest) {
            let (first, finished) = authority.read_encoded_blob_chunk(world(), &descriptor, 0, 777).unwrap();
            let mut offset = replica.receive_blob_chunk(world(), &descriptor, 0, &first, finished).unwrap();
            if !finished {
                drop(replica);
                let replica = Storage::open(&replica_root).unwrap();
                assert_eq!(replica.partial_blob_offset(world(), &descriptor).unwrap(), offset);
                loop {
                    let (chunk, finished) =
                        authority.read_encoded_blob_chunk(world(), &descriptor, offset, 1024).unwrap();
                    offset = replica.receive_blob_chunk(world(), &descriptor, offset, &chunk, finished).unwrap();
                    if finished {
                        break;
                    }
                }
            }
        }

        let replica = Storage::open(&replica_root).unwrap();
        replica.finalize_replica(&manifest).unwrap();
        assert!(replica.missing_blobs(&manifest).is_empty());
        let restored = temp.path().join("restored-world");
        replica.restore_snapshot(&manifest, &restored).unwrap();
        assert_eq!(fs::read(restored.join("level.dat")).unwrap(), b"swarmcraft-replication-level");
        assert_eq!(fs::read(restored.join("region/r.0.0.mca")).unwrap(), vec![42u8; 96 * 1024]);
    }

    #[test]
    fn wrong_resume_offset_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let descriptor = BlobDescriptor {
            hash: Hash32([4; 32]),
            uncompressed_size: 1,
            encoded_size: 3,
            encoding: BlobEncoding::Raw,
        };
        let error = store.receive_blob_chunk(world(), &descriptor, 2, b"x", false).unwrap_err();
        assert!(matches!(error, ReplicationError::OffsetMismatch { expected: 0, received: 2 }));
    }
}
