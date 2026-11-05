use crate::{transaction::sync_parent, Storage, StorageError};
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
    #[error(transparent)]
    Protocol(#[from] swarm_protocol::ProtocolError),
    #[error("blob offset mismatch: receiver has {expected} bytes but sender used offset {received}")]
    OffsetMismatch { expected: u64, received: u64 },
    #[error("blob encoded size mismatch: expected {expected} bytes, got {received}")]
    SizeMismatch { expected: u64, received: u64 },
    #[error("snapshot #{0} is not fully replicated")]
    Incomplete(u64),
    #[error("snapshot #{snapshot_number} does not directly extend the accepted canonical history")]
    HistoryConflict { snapshot_number: u64 },
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

    /// Protocol-facing history gate for replicated manifests. Agent 2 calls
    /// this before blob negotiation; finalization repeats the same check. Agent
    /// 3 owns the later atomic durable-head/fencing recheck at commit time.
    pub fn validate_replica_history(&self, manifest: &SnapshotManifestV1) -> Result<(), ReplicationError> {
        manifest.validate_semantics()?;
        let current = self.latest_snapshot(manifest.world_id)?;
        validate_replica_direct_extension(current.as_ref(), manifest)
    }

    pub fn has_complete_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool {
        let path = blob_path(self, world, descriptor);
        path.is_file() && verify_encoded_blob(&path, descriptor).is_ok()
    }

    pub fn partial_blob_offset(&self, world: WorldId, descriptor: &BlobDescriptor) -> Result<u64, ReplicationError> {
        let path = partial_blob_path(self, world, descriptor);
        if !path.exists() {
            return Ok(0);
        }
        fs::metadata(&path).map(|metadata| metadata.len()).map_err(|source| StorageError::Io { path, source }.into())
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
        let path = blob_path(self, world, descriptor);
        let metadata = fs::metadata(&path).map_err(|source| StorageError::Io { path: path.clone(), source })?;
        if metadata.len() != descriptor.encoded_size {
            return Err(StorageError::BlobCorrupt(descriptor.hash).into());
        }
        let mut file = File::open(&path).map_err(|source| StorageError::Io { path: path.clone(), source })?;
        file.seek(SeekFrom::Start(offset)).map_err(|source| StorageError::Io { path: path.clone(), source })?;
        let requested = (descriptor.encoded_size - offset).min(max_bytes as u64) as usize;
        let mut data = vec![0u8; requested];
        file.read_exact(&mut data).map_err(|source| StorageError::Io { path, source })?;
        Ok((data, offset + requested as u64 == descriptor.encoded_size))
    }

    pub fn receive_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        data: &[u8],
        finished: bool,
    ) -> Result<u64, ReplicationError> {
        let dir = self.world_dir(world).join("blobs");
        fs::create_dir_all(&dir).map_err(|source| StorageError::Io { path: dir.clone(), source })?;
        let partial = partial_blob_path(self, world, descriptor);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&partial)
            .map_err(|source| StorageError::Io { path: partial.clone(), source })?;
        let current = file.metadata().map_err(|source| StorageError::Io { path: partial.clone(), source })?.len();
        if current != offset {
            return Err(ReplicationError::OffsetMismatch { expected: current, received: offset });
        }
        let next = current
            .checked_add(data.len() as u64)
            .ok_or(ReplicationError::SizeMismatch { expected: descriptor.encoded_size, received: u64::MAX })?;
        if next > descriptor.encoded_size {
            return Err(ReplicationError::SizeMismatch { expected: descriptor.encoded_size, received: next });
        }
        file.write_all(data).map_err(|source| StorageError::Io { path: partial.clone(), source })?;
        file.sync_data().map_err(|source| StorageError::Io { path: partial.clone(), source })?;
        drop(file);
        if finished {
            if next != descriptor.encoded_size {
                return Err(ReplicationError::SizeMismatch { expected: descriptor.encoded_size, received: next });
            }
            if let Err(error) = verify_encoded_blob(&partial, descriptor) {
                match fs::remove_file(&partial) {
                    Ok(()) => sync_parent(&dir)?,
                    Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => return Err(StorageError::Io { path: partial, source }.into()),
                }
                return Err(error);
            }
            let final_path = blob_path(self, world, descriptor);
            if final_path.exists() {
                fs::remove_file(&final_path).map_err(|source| StorageError::Io { path: final_path.clone(), source })?;
                sync_parent(&dir)?;
            }
            fs::rename(&partial, &final_path)
                .map_err(|source| StorageError::Io { path: final_path.clone(), source })?;
            sync_parent(&dir)?;
        }
        Ok(next)
    }

    pub fn finalize_replica(&self, manifest: &SnapshotManifestV1) -> Result<(), ReplicationError> {
        self.validate_replica_history(manifest)?;
        if !self.missing_blobs(manifest).is_empty() {
            return Err(ReplicationError::Incomplete(manifest.snapshot_number));
        }
        if self.latest_snapshot(manifest.world_id)?.as_ref() == Some(manifest) {
            return Ok(());
        }
        self.commit_snapshot_streaming(manifest)?;
        Ok(())
    }
}

fn validate_replica_direct_extension(
    previous: Option<&SnapshotManifestV1>,
    manifest: &SnapshotManifestV1,
) -> Result<(), ReplicationError> {
    match previous {
        None => {
            if manifest.snapshot_number != 1 || manifest.previous_snapshot_hash.is_some() {
                return Err(ReplicationError::HistoryConflict { snapshot_number: manifest.snapshot_number });
            }
        }
        Some(previous) => {
            if manifest == previous {
                return Ok(());
            }
            let expected_number =
                previous.snapshot_number.checked_add(1).ok_or(StorageError::CounterExhausted("snapshot number"))?;
            let expected_sequence =
                previous.sequence.checked_add(1).ok_or(StorageError::CounterExhausted("snapshot sequence"))?;
            let expected_parent = previous.manifest_hash()?;
            if manifest.snapshot_number != expected_number
                || manifest.sequence != expected_sequence
                || manifest.previous_snapshot_hash != Some(expected_parent)
            {
                return Err(ReplicationError::HistoryConflict { snapshot_number: manifest.snapshot_number });
            }
        }
    }
    Ok(())
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

fn verify_encoded_blob(path: &Path, descriptor: &BlobDescriptor) -> Result<(), ReplicationError> {
    if fs::metadata(path).map_err(|source| StorageError::Io { path: path.to_path_buf(), source })?.len()
        != descriptor.encoded_size
    {
        return Err(StorageError::BlobCorrupt(descriptor.hash).into());
    }
    let file = File::open(path).map_err(|source| StorageError::Io { path: path.to_path_buf(), source })?;
    let mut reader: Box<dyn Read> = match descriptor.encoding {
        BlobEncoding::Raw => Box::new(file),
        BlobEncoding::Zstd => {
            Box::new(zstd::stream::read::Decoder::new(file).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?)
        }
    };
    verify_decoded_blob(&mut reader, path, descriptor)
}

fn verify_decoded_blob(
    reader: &mut dyn Read,
    path: &Path,
    descriptor: &BlobDescriptor,
) -> Result<(), ReplicationError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(BLOB_HASH_DOMAIN);
    let mut total = 0u64;
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let remaining = descriptor.uncompressed_size.saturating_sub(total);
        let read_limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let read = reader
            .read(&mut buffer[..read_limit])
            .map_err(|source| StorageError::Io { path: path.to_path_buf(), source })?;
        if read == 0 {
            break;
        }
        if read as u64 > remaining {
            return Err(StorageError::BlobCorrupt(descriptor.hash).into());
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
    use swarm_protocol::{snapshot_state_root, PeerId, SnapshotEntry, PROTOCOL_VERSION};

    fn manifest(number: u64, sequence: u64, previous: Option<Hash32>, marker: u8) -> SnapshotManifestV1 {
        let entries = vec![SnapshotEntry {
            path: "level.dat".into(),
            blob: BlobDescriptor {
                hash: Hash32([marker; 32]),
                uncompressed_size: 1,
                encoded_size: 1,
                encoding: BlobEncoding::Raw,
            },
        }];
        SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            snapshot_number: number,
            epoch: 1,
            sequence,
            previous_snapshot_hash: previous,
            state_root: snapshot_state_root(&entries).unwrap(),
            entries,
            authority_peer_id: PeerId([2; 32]),
            authority_public_key: [2; 32],
            signature: vec![marker; 64],
        }
    }

    struct ExpansionProbe {
        reads: usize,
        largest_request: usize,
    }

    impl Read for ExpansionProbe {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            self.largest_request = self.largest_request.max(buffer.len());
            buffer.fill(0);
            Ok(buffer.len())
        }
    }

    #[test]
    fn replicated_snapshot_history_requires_exact_direct_parent() {
        let first = manifest(1, 7, None, 1);
        assert!(validate_replica_direct_extension(None, &first).is_ok());
        assert!(validate_replica_direct_extension(Some(&first), &first).is_ok());

        let parent = first.manifest_hash().unwrap();
        let next = manifest(2, 8, Some(parent), 2);
        assert!(validate_replica_direct_extension(Some(&first), &next).is_ok());

        let skipped_number = manifest(3, 8, Some(parent), 3);
        assert!(matches!(
            validate_replica_direct_extension(Some(&first), &skipped_number),
            Err(ReplicationError::HistoryConflict { .. })
        ));

        let skipped_sequence = manifest(2, 9, Some(parent), 4);
        assert!(matches!(
            validate_replica_direct_extension(Some(&first), &skipped_sequence),
            Err(ReplicationError::HistoryConflict { .. })
        ));

        let wrong_parent = manifest(2, 8, Some(Hash32([9; 32])), 5);
        assert!(matches!(
            validate_replica_direct_extension(Some(&first), &wrong_parent),
            Err(ReplicationError::HistoryConflict { .. })
        ));

        let same_sequence_conflict = manifest(2, 7, Some(parent), 6);
        assert!(matches!(
            validate_replica_direct_extension(Some(&first), &same_sequence_conflict),
            Err(ReplicationError::HistoryConflict { .. })
        ));
    }

    #[test]
    fn decoded_verifier_reads_only_one_byte_beyond_declared_size_before_rejection() {
        let descriptor = BlobDescriptor {
            hash: Hash32([0; 32]),
            uncompressed_size: 1,
            encoded_size: 0,
            encoding: BlobEncoding::Zstd,
        };
        let mut probe = ExpansionProbe { reads: 0, largest_request: 0 };
        let error = verify_decoded_blob(&mut probe, Path::new("amplification.zst"), &descriptor).unwrap_err();

        assert!(matches!(
            error,
            ReplicationError::Storage(StorageError::BlobCorrupt(hash)) if hash == descriptor.hash
        ));
        assert_eq!(probe.reads, 1);
        assert_eq!(probe.largest_request, 2);
    }

    #[test]
    fn zstd_replica_verifier_rejects_expansion_past_declared_size() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("amplification.zst");
        let expanded = vec![0u8; 8 * 1024 * 1024];
        let encoded = zstd::stream::encode_all(expanded.as_slice(), 3).unwrap();
        fs::write(&path, &encoded).unwrap();
        let mut hasher = blake3::Hasher::new();
        hasher.update(BLOB_HASH_DOMAIN);
        hasher.update(&[0]);
        let descriptor = BlobDescriptor {
            hash: Hash32(*hasher.finalize().as_bytes()),
            uncompressed_size: 1,
            encoded_size: encoded.len() as u64,
            encoding: BlobEncoding::Zstd,
        };

        assert!(matches!(
            verify_encoded_blob(&path, &descriptor),
            Err(ReplicationError::Storage(StorageError::BlobCorrupt(hash))) if hash == descriptor.hash
        ));
    }
}
