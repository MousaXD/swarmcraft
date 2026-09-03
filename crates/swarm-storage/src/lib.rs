//! Crash-safe content-addressed storage and directory snapshots.

use crate::transaction::durable_atomic_write;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use swarm_protocol::{
    BlobDescriptor, BlobEncoding, Hash32, PeerId, SnapshotManifestV1, WorldGenesisV1, WorldId, BLOB_HASH_DOMAIN,
    STORAGE_SCHEMA_VERSION,
};
use thiserror::Error;
use walkdir::WalkDir;

pub const LEGACY_READ_BLOB_MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("snapshot source is not a directory: {0}")]
    SourceNotDirectory(PathBuf),
    #[error("symlinks are not allowed in snapshots: {0}")]
    SymlinkUnsupported(PathBuf),
    #[error("snapshot path is not portable UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("unsafe snapshot relative path: {0}")]
    UnsafeRelativePath(String),
    #[error("snapshot paths collide under the portable filesystem identity policy: {0}")]
    PortablePathCollision(String),
    #[error("blob is corrupt or incomplete: {0}")]
    BlobCorrupt(Hash32),
    #[error("legacy blob read declares {declared} bytes, above the bounded maximum {maximum}")]
    BlobReadTooLarge { declared: u64, maximum: u64 },
    #[error("snapshot state root mismatch")]
    StateRootMismatch,
    #[error("snapshot publication belongs to world {publication_world} but manifest targets {manifest_world}")]
    SnapshotPublicationWorldMismatch { publication_world: WorldId, manifest_world: WorldId },
    #[error("snapshot publication does not own a pin for blob {0}")]
    SnapshotPublicationMissingPin(Hash32),
    #[error("snapshot manifest does not exist: {0}")]
    SnapshotNotFound(u64),
    #[error("canonical snapshot head record is missing for world {0}")]
    MissingCanonicalHead(WorldId),
    #[error("canonical snapshot head for world {world} targets missing snapshot #{snapshot_number} ({manifest_hash})")]
    MissingCanonicalHeadTarget { world: WorldId, snapshot_number: u64, manifest_hash: Hash32 },
    #[error("canonical snapshot head for world {world} does not match snapshot #{snapshot_number}")]
    CanonicalHeadMismatch { world: WorldId, snapshot_number: u64 },
    #[error("snapshot commit for world {world} snapshot #{snapshot_number} was interrupted and requires recovery")]
    SnapshotCommitIncomplete { world: WorldId, snapshot_number: u64 },
    #[error("uncommitted snapshot #{snapshot_number} exists above the canonical head for world {world}")]
    UncommittedSnapshotOrphan { world: WorldId, snapshot_number: u64 },
    #[error("snapshot #{snapshot_number} does not directly extend the canonical snapshot head")]
    SnapshotHistoryConflict { snapshot_number: u64 },
    #[error("snapshot numbering or sequence counter is exhausted")]
    SnapshotNumberExhausted,
    #[error("canonical counter exhausted: {0}")]
    CounterExhausted(&'static str),
    #[error(
        "snapshot authority fence mismatch for world {world}; expected epoch {expected_epoch} fencing token {expected_fencing_token}"
    )]
    SnapshotFenceMismatch { world: WorldId, expected_epoch: u64, expected_fencing_token: u64 },
    #[error(
        "snapshot #{snapshot_number} for world {world} is immutable; existing hash {existing}, attempted hash {attempted}"
    )]
    SnapshotManifestConflict { world: WorldId, snapshot_number: u64, existing: Hash32, attempted: Hash32 },
    #[error("restore destination is marked incomplete and must be discarded before retry: {0}")]
    RestoreIncomplete(PathBuf),
    #[error("world metadata does not exist: {0}")]
    WorldNotFound(WorldId),
    #[error("world metadata is inconsistent with its directory")]
    WorldMetadataMismatch,
    #[error("snapshot protocol version {0} is unsupported")]
    UnsupportedProtocol(u16),
    #[error("decode failed: {0}")]
    Decode(#[from] postcard::Error),
    #[error("metadata JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Protocol(#[from] swarm_protocol::ProtocolError),
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldMetadataV1 {
    pub storage_schema_version: u16,
    pub display_name: String,
    pub world_id: WorldId,
    pub genesis: WorldGenesisV1,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotContext {
    pub world: WorldId,
    pub snapshot_number: u64,
    pub epoch: u64,
    pub sequence: u64,
    pub previous_snapshot_hash: Option<Hash32>,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let this = Self { root: root.into() };
        fs::create_dir_all(this.worlds_dir()).map_err(|error| io_error(this.worlds_dir(), error))?;
        this.recover_abandoned_snapshot_publications()?;
        Ok(this)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn worlds_dir(&self) -> PathBuf {
        self.root.join("worlds")
    }

    pub fn world_dir(&self, world: WorldId) -> PathBuf {
        self.worlds_dir().join(world.to_hex())
    }

    fn metadata_dir(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("metadata")
    }

    fn snapshots_dir(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("snapshots")
    }

    fn blobs_dir(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("blobs")
    }

    pub fn create_world(&self, metadata: &WorldMetadataV1) -> Result<(), StorageError> {
        if metadata.storage_schema_version != STORAGE_SCHEMA_VERSION
            || metadata.genesis.world_id()? != metadata.world_id
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        for dir in [
            self.metadata_dir(metadata.world_id),
            self.snapshots_dir(metadata.world_id),
            self.blobs_dir(metadata.world_id),
            self.world_dir(metadata.world_id).join("logs"),
            self.world_dir(metadata.world_id).join("recovery"),
        ] {
            fs::create_dir_all(&dir).map_err(|error| io_error(dir, error))?;
        }
        let json = serde_json::to_vec_pretty(metadata)?;
        durable_atomic_write(&self.metadata_dir(metadata.world_id).join("world.json"), &json)?;
        self.canonical_snapshot_head(metadata.world_id)?;
        Ok(())
    }

    pub fn load_world(&self, world: WorldId) -> Result<WorldMetadataV1, StorageError> {
        let path = self.metadata_dir(world).join("world.json");
        if !path.exists() {
            return Err(StorageError::WorldNotFound(world));
        }
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let metadata: WorldMetadataV1 = serde_json::from_slice(&bytes)?;
        if metadata.world_id != world || metadata.genesis.world_id()? != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(metadata)
    }

    pub fn list_worlds(&self) -> Result<Vec<WorldMetadataV1>, StorageError> {
        let mut worlds = Vec::new();
        if !self.worlds_dir().exists() {
            return Ok(worlds);
        }
        for entry in fs::read_dir(self.worlds_dir()).map_err(|error| io_error(self.worlds_dir(), error))? {
            let entry = entry.map_err(|error| io_error(self.worlds_dir(), error))?;
            if !entry.file_type().map_err(|error| io_error(entry.path(), error))?.is_dir() {
                continue;
            }
            let Ok(world) = entry.file_name().to_string_lossy().parse::<WorldId>() else {
                continue;
            };
            if let Ok(metadata) = self.load_world(world) {
                worlds.push(metadata);
            }
        }
        worlds
            .sort_by(|left, right| left.display_name.cmp(&right.display_name).then(left.world_id.cmp(&right.world_id)));
        Ok(worlds)
    }

    pub fn put_blob(&self, world: WorldId, bytes: &[u8]) -> Result<BlobDescriptor, StorageError> {
        let hash = BlobDescriptor::hash_uncompressed(bytes);
        let dir = self.blobs_dir(world);
        fs::create_dir_all(&dir).map_err(|error| io_error(&dir, error))?;
        let encoded = zstd::stream::encode_all(bytes, 3).map_err(|error| io_error(&dir, error))?;
        let descriptor = BlobDescriptor {
            hash,
            uncompressed_size: bytes.len() as u64,
            encoded_size: encoded.len() as u64,
            encoding: BlobEncoding::Zstd,
        };
        let path = self.blob_path(world, hash, BlobEncoding::Zstd);
        if !path.exists() {
            durable_atomic_write(&path, &encoded)?;
        }
        Ok(descriptor)
    }

    /// Legacy convenience API retained for compatibility, now implemented with
    /// the same bounded streaming semantics as the primary restore path.
    pub fn read_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> Result<Vec<u8>, StorageError> {
        if descriptor.uncompressed_size > LEGACY_READ_BLOB_MAX_BYTES {
            return Err(StorageError::BlobReadTooLarge {
                declared: descriptor.uncompressed_size,
                maximum: LEGACY_READ_BLOB_MAX_BYTES,
            });
        }
        let path = self.blob_path(world, descriptor.hash, descriptor.encoding);
        let metadata = fs::metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.len() != descriptor.encoded_size {
            return Err(StorageError::BlobCorrupt(descriptor.hash));
        }
        let encoded = File::open(&path).map_err(|error| io_error(&path, error))?;
        let mut reader: Box<dyn Read> = match descriptor.encoding {
            BlobEncoding::Raw => Box::new(encoded),
            BlobEncoding::Zstd => Box::new(
                zstd::stream::read::Decoder::new(encoded).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?,
            ),
        };
        let capacity = usize::try_from(descriptor.uncompressed_size).map_err(|_| StorageError::BlobReadTooLarge {
            declared: descriptor.uncompressed_size,
            maximum: LEGACY_READ_BLOB_MAX_BYTES,
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        let mut hasher = blake3::Hasher::new();
        hasher.update(BLOB_HASH_DOMAIN);
        let mut total = 0u64;
        let mut buffer = vec![0u8; crate::streaming::STREAM_BUFFER_SIZE];
        loop {
            let remaining = descriptor.uncompressed_size.saturating_sub(total);
            let read_limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
            let read =
                reader.read(&mut buffer[..read_limit]).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?;
            if read == 0 {
                break;
            }
            if read as u64 > remaining {
                return Err(StorageError::BlobCorrupt(descriptor.hash));
            }
            hasher.update(&buffer[..read]);
            bytes.extend_from_slice(&buffer[..read]);
            total += read as u64;
        }
        if total != descriptor.uncompressed_size || Hash32(*hasher.finalize().as_bytes()) != descriptor.hash {
            return Err(StorageError::BlobCorrupt(descriptor.hash));
        }
        Ok(bytes)
    }

    pub fn snapshot_directory(
        &self,
        source: &Path,
        context: SnapshotContext,
    ) -> Result<crate::streaming::SnapshotPublication, StorageError> {
        self.snapshot_directory_streaming(source, context)
    }

    pub fn commit_snapshot<T: crate::streaming::SnapshotCommitInput>(&self, snapshot: &T) -> Result<(), StorageError> {
        self.commit_snapshot_streaming(snapshot)
    }

    pub fn load_snapshot(&self, world: WorldId, number: u64) -> Result<SnapshotManifestV1, StorageError> {
        let manifest = self.load_snapshot_file_unchecked(world, number)?;
        self.validate_canonical_snapshot_namespace(world)?;
        Ok(manifest)
    }

    pub fn list_snapshots(&self, world: WorldId) -> Result<Vec<SnapshotManifestV1>, StorageError> {
        self.validate_canonical_snapshot_namespace(world)?;
        self.raw_snapshot_manifests(world)
    }

    pub fn latest_snapshot(&self, world: WorldId) -> Result<Option<SnapshotManifestV1>, StorageError> {
        self.validate_canonical_snapshot_namespace(world)?;
        let head = self.canonical_snapshot_head(world)?;
        head.head.map(|reference| self.load_snapshot_file_unchecked(world, reference.snapshot_number)).transpose()
    }

    pub fn verify_snapshot(&self, manifest: &SnapshotManifestV1) -> Result<(), StorageError> {
        self.verify_snapshot_streaming(manifest)
    }

    pub fn restore_snapshot(&self, manifest: &SnapshotManifestV1, destination: &Path) -> Result<(), StorageError> {
        self.restore_snapshot_streaming(manifest, destination)
    }

    pub fn next_snapshot_number(&self, world: WorldId) -> Result<u64, StorageError> {
        let head = self.canonical_snapshot_head(world)?;
        match head.head {
            None => Ok(1),
            Some(reference) => reference.snapshot_number.checked_add(1).ok_or(StorageError::SnapshotNumberExhausted),
        }
    }

    /// Report ignored crash debris conservatively without deleting anything that
    /// could still belong to a live writer. Operators/tests can use this to make
    /// repeated-crash disk growth visible.
    pub fn storage_temp_debris(&self) -> Result<Vec<PathBuf>, StorageError> {
        let mut debris = Vec::new();
        let worlds_dir = self.worlds_dir();
        if !worlds_dir.exists() {
            return Ok(debris);
        }
        for entry in WalkDir::new(&worlds_dir).follow_links(false) {
            let entry = entry.map_err(|error| {
                let error_path = error.path().map(Path::to_path_buf).unwrap_or_else(|| worlds_dir.clone());
                io_error(error_path, std::io::Error::other(error.to_string()))
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if name.starts_with(".atomic-")
                || name.starts_with(".blob-")
                || name.starts_with(".restore-")
                || name.starts_with(".deleted-")
            {
                debris.push(entry.path().to_path_buf());
            }
        }
        debris.sort();
        Ok(debris)
    }

    fn blob_path(&self, world: WorldId, hash: Hash32, encoding: BlobEncoding) -> PathBuf {
        let suffix = match encoding {
            BlobEncoding::Raw => "raw",
            BlobEncoding::Zstd => "zst",
        };
        self.blobs_dir(world).join(format!("{}.{}", hash.to_hex(), suffix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> WorldId {
        WorldId([1; 32])
    }

    #[test]
    fn snapshot_round_trip_and_corruption_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        fs::create_dir_all(source.join("region")).unwrap();
        fs::write(source.join("level.dat"), b"level").unwrap();
        fs::write(source.join("region/r.0.0.mca"), b"region-data").unwrap();
        let store = Storage::open(tmp.path().join("data")).unwrap();
        let mut manifest = store
            .snapshot_directory(
                &source,
                SnapshotContext {
                    world: world(),
                    snapshot_number: 1,
                    epoch: 1,
                    sequence: 1,
                    previous_snapshot_hash: None,
                    authority_peer_id: PeerId([2; 32]),
                    authority_public_key: [3; 32],
                },
            )
            .unwrap();
        manifest.signature = vec![0; 64];
        store.commit_snapshot(&manifest).unwrap();

        let restored = tmp.path().join("restored");
        store.restore_snapshot(&manifest, &restored).unwrap();
        assert_eq!(fs::read(restored.join("level.dat")).unwrap(), b"level");
        assert_eq!(fs::read(restored.join("region/r.0.0.mca")).unwrap(), b"region-data");

        let descriptor = &manifest.entries[0].blob;
        let blob = store.blob_path(world(), descriptor.hash, descriptor.encoding);
        fs::write(blob, b"broken").unwrap();
        assert!(matches!(store.verify_snapshot(&manifest), Err(StorageError::BlobCorrupt(_))));
    }

    #[test]
    fn rejects_oversized_legacy_blob_reads_before_allocation() {
        let store = Storage::open(tempfile::tempdir().unwrap().path()).unwrap();
        let descriptor = BlobDescriptor {
            hash: Hash32([0; 32]),
            uncompressed_size: LEGACY_READ_BLOB_MAX_BYTES + 1,
            encoded_size: 0,
            encoding: BlobEncoding::Zstd,
        };
        assert!(matches!(store.read_blob(world(), &descriptor), Err(StorageError::BlobReadTooLarge { .. })));
    }
}
