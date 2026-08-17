//! Crash-safe content-addressed storage and directory snapshots.

use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::fs::File;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use swarm_protocol::{
    BlobDescriptor, BlobEncoding, Hash32, PeerId, SnapshotManifestV1, WorldGenesisV1, WorldId, STORAGE_SCHEMA_VERSION,
};
use thiserror::Error;
use tracing::debug;

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
    #[error("blob is corrupt or incomplete: {0}")]
    BlobCorrupt(Hash32),
    #[error("snapshot state root mismatch")]
    StateRootMismatch,
    #[error("snapshot manifest does not exist: {0}")]
    SnapshotNotFound(u64),
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
        fs::create_dir_all(this.worlds_dir()).map_err(|e| io_error(this.worlds_dir(), e))?;
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
            fs::create_dir_all(&dir).map_err(|e| io_error(dir, e))?;
        }
        let json = serde_json::to_vec_pretty(metadata)?;
        atomic_write(&self.metadata_dir(metadata.world_id).join("world.json"), &json)
    }

    pub fn load_world(&self, world: WorldId) -> Result<WorldMetadataV1, StorageError> {
        let path = self.metadata_dir(world).join("world.json");
        if !path.exists() {
            return Err(StorageError::WorldNotFound(world));
        }
        let bytes = fs::read(&path).map_err(|e| io_error(&path, e))?;
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
        for entry in fs::read_dir(self.worlds_dir()).map_err(|e| io_error(self.worlds_dir(), e))? {
            let entry = entry.map_err(|e| io_error(self.worlds_dir(), e))?;
            if !entry.file_type().map_err(|e| io_error(entry.path(), e))?.is_dir() {
                continue;
            }
            let Ok(world) = entry.file_name().to_string_lossy().parse::<WorldId>() else {
                continue;
            };
            if let Ok(metadata) = self.load_world(world) {
                worlds.push(metadata);
            }
        }
        worlds.sort_by(|a, b| a.display_name.cmp(&b.display_name).then(a.world_id.cmp(&b.world_id)));
        Ok(worlds)
    }

    pub fn put_blob(&self, world: WorldId, bytes: &[u8]) -> Result<BlobDescriptor, StorageError> {
        let hash = BlobDescriptor::hash_uncompressed(bytes);
        let dir = self.blobs_dir(world);
        fs::create_dir_all(&dir).map_err(|e| io_error(&dir, e))?;
        let encoded = zstd::stream::encode_all(bytes, 3).map_err(|e| io_error(&dir, e))?;
        let descriptor = BlobDescriptor {
            hash,
            uncompressed_size: bytes.len() as u64,
            encoded_size: encoded.len() as u64,
            encoding: BlobEncoding::Zstd,
        };
        let path = self.blob_path(world, hash, BlobEncoding::Zstd);
        if !path.exists() {
            atomic_write(&path, &encoded)?;
        }
        Ok(descriptor)
    }

    pub fn read_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> Result<Vec<u8>, StorageError> {
        let path = self.blob_path(world, descriptor.hash, descriptor.encoding);
        let encoded = fs::read(&path).map_err(|e| io_error(&path, e))?;
        if encoded.len() as u64 != descriptor.encoded_size {
            return Err(StorageError::BlobCorrupt(descriptor.hash));
        }
        let bytes = match descriptor.encoding {
            BlobEncoding::Raw => encoded,
            BlobEncoding::Zstd => {
                let decoder = zstd::stream::read::Decoder::new(encoded.as_slice())
                    .map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?;
                let mut reader = decoder.take(descriptor.uncompressed_size.saturating_add(1));
                let mut decoded = Vec::new();
                reader.read_to_end(&mut decoded).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?;
                decoded
            }
        };
        if bytes.len() as u64 != descriptor.uncompressed_size
            || BlobDescriptor::hash_uncompressed(&bytes) != descriptor.hash
        {
            return Err(StorageError::BlobCorrupt(descriptor.hash));
        }
        Ok(bytes)
    }

    pub fn snapshot_directory(
        &self,
        source: &Path,
        context: SnapshotContext,
    ) -> Result<SnapshotManifestV1, StorageError> {
        self.snapshot_directory_streaming(source, context)
    }

    pub fn commit_snapshot(&self, manifest: &SnapshotManifestV1) -> Result<(), StorageError> {
        self.commit_snapshot_streaming(manifest)
    }

    pub fn load_snapshot(&self, world: WorldId, number: u64) -> Result<SnapshotManifestV1, StorageError> {
        let path = self.snapshot_path(world, number);
        if !path.exists() {
            return Err(StorageError::SnapshotNotFound(number));
        }
        let bytes = fs::read(&path).map_err(|e| io_error(&path, e))?;
        let manifest: SnapshotManifestV1 = postcard::from_bytes(&bytes)?;
        Ok(manifest)
    }

    pub fn list_snapshots(&self, world: WorldId) -> Result<Vec<SnapshotManifestV1>, StorageError> {
        let dir = self.snapshots_dir(world);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut snapshots = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| io_error(&dir, e))? {
            let entry = entry.map_err(|e| io_error(&dir, e))?;
            if entry.path().extension().and_then(|x| x.to_str()) != Some("postcard") {
                continue;
            }
            let bytes = fs::read(entry.path()).map_err(|e| io_error(entry.path(), e))?;
            let manifest: SnapshotManifestV1 = postcard::from_bytes(&bytes)?;
            if manifest.world_id == world {
                snapshots.push(manifest);
            }
        }
        snapshots.sort_by_key(|m| m.snapshot_number);
        Ok(snapshots)
    }

    pub fn latest_snapshot(&self, world: WorldId) -> Result<Option<SnapshotManifestV1>, StorageError> {
        Ok(self.list_snapshots(world)?.pop())
    }

    pub fn verify_snapshot(&self, manifest: &SnapshotManifestV1) -> Result<(), StorageError> {
        self.verify_snapshot_streaming(manifest)
    }

    pub fn restore_snapshot(&self, manifest: &SnapshotManifestV1, destination: &Path) -> Result<(), StorageError> {
        self.restore_snapshot_streaming(manifest, destination)
    }

    pub fn next_snapshot_number(&self, world: WorldId) -> Result<u64, StorageError> {
        Ok(self.latest_snapshot(world)?.map_or(1, |m| m.snapshot_number + 1))
    }

    fn snapshot_path(&self, world: WorldId, number: u64) -> PathBuf {
        self.snapshots_dir(world).join(format!("{number:020}.postcard"))
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
fn validate_portable_path(path: &str) -> Result<(), StorageError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || (path.len() >= 2 && path.as_bytes()[1] == b':')
    {
        return Err(StorageError::UnsafeRelativePath(path.to_owned()));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|x| x.to_str()).unwrap_or("data")));
    debug!(path = %path.display(), bytes = bytes.len(), "atomic write");
    let mut file =
        OpenOptions::new().create(true).truncate(true).write(true).open(&tmp).map_err(|e| io_error(&tmp, e))?;
    file.write_all(bytes).map_err(|e| io_error(&tmp, e))?;
    file.sync_all().map_err(|e| io_error(&tmp, e))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| io_error(path, e))?;
    sync_parent(parent)?;
    Ok(())
}

fn sync_parent(_parent: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        let file = File::open(_parent).map_err(|e| io_error(_parent, e))?;
        file.sync_all().map_err(|e| io_error(_parent, e))?;
    }
    Ok(())
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
                    sequence: 0,
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
    fn rejects_traversal_paths() {
        assert!(validate_portable_path("../secret").is_err());
        assert!(validate_portable_path("C:/secret").is_err());
        assert!(validate_portable_path("safe/region.mca").is_ok());
    }
}
