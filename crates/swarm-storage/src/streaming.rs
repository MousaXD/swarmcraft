use crate::{SnapshotContext, Storage, StorageError};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use swarm_protocol::{
    snapshot_state_root, BlobDescriptor, BlobEncoding, Hash32, SnapshotEntry, SnapshotManifestV1, WorldId,
    BLOB_HASH_DOMAIN, PROTOCOL_VERSION,
};
use walkdir::WalkDir;

pub const STREAM_BUFFER_SIZE: usize = 1024 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Storage {
    pub fn snapshot_directory_streaming(
        &self,
        source: &Path,
        context: SnapshotContext,
    ) -> Result<SnapshotManifestV1, StorageError> {
        if !source.is_dir() {
            return Err(StorageError::SourceNotDirectory(source.to_path_buf()));
        }

        let mut files = Vec::new();
        for entry in WalkDir::new(source).follow_links(false) {
            let entry = entry
                .map_err(|error| io_error(error.path().unwrap_or(source), std::io::Error::other(error.to_string())))?;
            let path = entry.path();
            if entry.file_type().is_symlink() {
                return Err(StorageError::SymlinkUnsupported(path.to_path_buf()));
            }
            if entry.file_type().is_file() {
                files.push(path.to_path_buf());
            }
        }
        files.sort();

        let mut entries = Vec::with_capacity(files.len());
        for path in files {
            let relative = path.strip_prefix(source).expect("walkdir entries stay beneath root");
            let relative = portable_relative_path(relative)?;
            let blob = self.put_file_blob_streaming(context.world, &path)?;
            entries.push(SnapshotEntry { path: relative, blob });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let state_root = snapshot_state_root(&entries)?;
        Ok(SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: context.world,
            snapshot_number: context.snapshot_number,
            epoch: context.epoch,
            sequence: context.sequence,
            previous_snapshot_hash: context.previous_snapshot_hash,
            entries,
            state_root,
            authority_peer_id: context.authority_peer_id,
            authority_public_key: context.authority_public_key,
            signature: Vec::new(),
        })
    }

    pub fn put_file_blob_streaming(&self, world: WorldId, source: &Path) -> Result<BlobDescriptor, StorageError> {
        let blob_dir = self.world_dir(world).join("blobs");
        fs::create_dir_all(&blob_dir).map_err(|error| io_error(&blob_dir, error))?;
        let (temporary_path, temporary_file) = create_unique_temp(&blob_dir, "blob", "zst")?;
        let mut encoder =
            zstd::stream::write::Encoder::new(temporary_file, 3).map_err(|error| io_error(&temporary_path, error))?;
        let mut input = File::open(source).map_err(|error| io_error(source, error))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(BLOB_HASH_DOMAIN);
        let mut uncompressed_size = 0u64;
        let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];

        loop {
            let read = input.read(&mut buffer).map_err(|error| io_error(source, error))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            encoder.write_all(&buffer[..read]).map_err(|error| io_error(&temporary_path, error))?;
            uncompressed_size = uncompressed_size.saturating_add(read as u64);
        }

        let encoded_file = encoder.finish().map_err(|error| io_error(&temporary_path, error))?;
        encoded_file.sync_all().map_err(|error| io_error(&temporary_path, error))?;
        let encoded_size = encoded_file.metadata().map_err(|error| io_error(&temporary_path, error))?.len();
        drop(encoded_file);

        let hash = Hash32(*hasher.finalize().as_bytes());
        let mut descriptor = BlobDescriptor { hash, uncompressed_size, encoded_size, encoding: BlobEncoding::Zstd };
        let final_path = blob_path(self, world, &descriptor);

        if final_path.exists() {
            let existing_size = fs::metadata(&final_path).map_err(|error| io_error(&final_path, error))?.len();
            let existing = BlobDescriptor { encoded_size: existing_size, ..descriptor.clone() };
            if verify_encoded_blob_streaming(&final_path, &existing).is_ok() {
                remove_if_present(&temporary_path)?;
                return Ok(existing);
            }
            remove_if_present(&final_path)?;
        }

        fs::rename(&temporary_path, &final_path).map_err(|error| io_error(&final_path, error))?;
        sync_parent(&blob_dir)?;
        descriptor.encoded_size = fs::metadata(&final_path).map_err(|error| io_error(&final_path, error))?.len();
        Ok(descriptor)
    }

    pub fn verify_blob_streaming(&self, world: WorldId, descriptor: &BlobDescriptor) -> Result<(), StorageError> {
        verify_encoded_blob_streaming(&blob_path(self, world, descriptor), descriptor)
    }

    pub fn verify_snapshot_streaming(&self, manifest: &SnapshotManifestV1) -> Result<(), StorageError> {
        validate_manifest_shape(manifest)?;
        for entry in &manifest.entries {
            self.verify_blob_streaming(manifest.world_id, &entry.blob)?;
        }
        Ok(())
    }

    pub fn commit_snapshot_streaming(&self, manifest: &SnapshotManifestV1) -> Result<(), StorageError> {
        self.verify_snapshot_streaming(manifest)?;
        let snapshots = self.world_dir(manifest.world_id).join("snapshots");
        fs::create_dir_all(&snapshots).map_err(|error| io_error(&snapshots, error))?;
        let path = snapshots.join(format!("{:020}.postcard", manifest.snapshot_number));
        let bytes = postcard::to_allocvec(manifest)?;
        atomic_write(&path, &bytes)
    }

    pub fn restore_snapshot_streaming(
        &self,
        manifest: &SnapshotManifestV1,
        destination: &Path,
    ) -> Result<(), StorageError> {
        validate_manifest_shape(manifest)?;
        fs::create_dir_all(destination).map_err(|error| io_error(destination, error))?;
        for entry in &manifest.entries {
            let output = destination.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            restore_blob_streaming(self, manifest.world_id, &entry.blob, &output)?;
        }
        Ok(())
    }
}

fn validate_manifest_shape(manifest: &SnapshotManifestV1) -> Result<(), StorageError> {
    if manifest.protocol_version != PROTOCOL_VERSION {
        return Err(StorageError::UnsupportedProtocol(manifest.protocol_version));
    }
    let mut seen = BTreeSet::new();
    for entry in &manifest.entries {
        validate_portable_path(&entry.path)?;
        if !seen.insert(entry.path.as_str()) {
            return Err(StorageError::UnsafeRelativePath(entry.path.clone()));
        }
    }
    if snapshot_state_root(&manifest.entries)? != manifest.state_root {
        return Err(StorageError::StateRootMismatch);
    }
    Ok(())
}

fn restore_blob_streaming(
    storage: &Storage,
    world: WorldId,
    descriptor: &BlobDescriptor,
    output: &Path,
) -> Result<(), StorageError> {
    let encoded_path = blob_path(storage, world, descriptor);
    ensure_encoded_size(&encoded_path, descriptor)?;
    let parent =
        output.parent().ok_or_else(|| StorageError::UnsafeRelativePath(output.to_string_lossy().into_owned()))?;
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    let (temporary_path, mut temporary_file) = create_unique_temp(parent, "restore", "tmp")?;
    let encoded = File::open(&encoded_path).map_err(|error| io_error(&encoded_path, error))?;
    let mut reader: Box<dyn Read> = match descriptor.encoding {
        BlobEncoding::Raw => Box::new(encoded),
        BlobEncoding::Zstd => {
            Box::new(zstd::stream::read::Decoder::new(encoded).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?)
        }
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(BLOB_HASH_DOMAIN);
    let mut total = 0u64;
    let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
    loop {
        let read = reader.read(&mut buffer).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        temporary_file.write_all(&buffer[..read]).map_err(|error| io_error(&temporary_path, error))?;
        total = total.saturating_add(read as u64);
    }
    if total != descriptor.uncompressed_size || Hash32(*hasher.finalize().as_bytes()) != descriptor.hash {
        remove_if_present(&temporary_path)?;
        return Err(StorageError::BlobCorrupt(descriptor.hash));
    }
    temporary_file.sync_all().map_err(|error| io_error(&temporary_path, error))?;
    drop(temporary_file);
    if output.exists() {
        remove_if_present(output)?;
    }
    fs::rename(&temporary_path, output).map_err(|error| io_error(output, error))?;
    sync_parent(parent)
}

fn verify_encoded_blob_streaming(path: &Path, descriptor: &BlobDescriptor) -> Result<(), StorageError> {
    ensure_encoded_size(path, descriptor)?;
    let encoded = File::open(path).map_err(|error| io_error(path, error))?;
    let mut reader: Box<dyn Read> = match descriptor.encoding {
        BlobEncoding::Raw => Box::new(encoded),
        BlobEncoding::Zstd => {
            Box::new(zstd::stream::read::Decoder::new(encoded).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?)
        }
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(BLOB_HASH_DOMAIN);
    let mut total = 0u64;
    let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
    loop {
        let read = reader.read(&mut buffer).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    if total != descriptor.uncompressed_size || Hash32(*hasher.finalize().as_bytes()) != descriptor.hash {
        return Err(StorageError::BlobCorrupt(descriptor.hash));
    }
    Ok(())
}

fn ensure_encoded_size(path: &Path, descriptor: &BlobDescriptor) -> Result<(), StorageError> {
    let size = fs::metadata(path).map_err(|error| io_error(path, error))?.len();
    if size != descriptor.encoded_size {
        return Err(StorageError::BlobCorrupt(descriptor.hash));
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

fn portable_relative_path(path: &Path) -> Result<String, StorageError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                parts.push(part.to_str().ok_or_else(|| StorageError::NonUtf8Path(path.to_path_buf()))?.to_owned())
            }
            _ => return Err(StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned())),
        }
    }
    let value = parts.join("/");
    validate_portable_path(&value)?;
    Ok(value)
}

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

fn create_unique_temp(parent: &Path, prefix: &str, extension: &str) -> Result<(PathBuf, File), StorageError> {
    for _ in 0..128 {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{prefix}-{}-{counter}.{extension}", std::process::id()));
        match OpenOptions::new().create_new(true).write(true).read(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(&path, error)),
        }
    }
    Err(io_error(
        parent,
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "unable to allocate unique temporary file"),
    ))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    let (temporary_path, mut temporary_file) = create_unique_temp(parent, "atomic", "tmp")?;
    temporary_file.write_all(bytes).map_err(|error| io_error(&temporary_path, error))?;
    temporary_file.sync_all().map_err(|error| io_error(&temporary_path, error))?;
    drop(temporary_file);
    if path.exists() {
        remove_if_present(path)?;
    }
    fs::rename(&temporary_path, path).map_err(|error| io_error(path, error))?;
    sync_parent(parent)
}

fn remove_if_present(path: &Path) -> Result<(), StorageError> {
    if path.exists() {
        fs::remove_file(path).map_err(|error| io_error(path, error))?;
    }
    Ok(())
}

fn sync_parent(parent: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        fs::File::open(parent).and_then(|directory| directory.sync_all()).map_err(|error| io_error(parent, error))?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom};
    use swarm_protocol::PeerId;

    fn context(world: WorldId) -> SnapshotContext {
        SnapshotContext {
            world,
            snapshot_number: 1,
            epoch: 1,
            sequence: 1,
            previous_snapshot_hash: None,
            authority_peer_id: PeerId([3; 32]),
            authority_public_key: [4; 32],
        }
    }

    fn write_pattern(path: &Path, bytes: u64) {
        let mut file = File::create(path).unwrap();
        let mut block = vec![0u8; STREAM_BUFFER_SIZE];
        for (index, byte) in block.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        let mut remaining = bytes;
        while remaining != 0 {
            let write = remaining.min(block.len() as u64) as usize;
            file.write_all(&block[..write]).unwrap();
            remaining -= write as u64;
        }
        file.sync_all().unwrap();
    }

    #[test]
    fn streaming_snapshot_round_trip_and_truncation_detection() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let restore = temp.path().join("restore");
        fs::create_dir_all(source.join("region")).unwrap();
        write_pattern(&source.join("region/r.0.0.mca"), 32 * 1024 * 1024);

        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([9; 32]);
        let manifest = storage.snapshot_directory_streaming(&source, context(world)).unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].blob.uncompressed_size, 32 * 1024 * 1024);
        storage.commit_snapshot_streaming(&manifest).unwrap();
        storage.verify_snapshot_streaming(&manifest).unwrap();
        storage.restore_snapshot_streaming(&manifest, &restore).unwrap();
        assert_eq!(fs::metadata(restore.join("region/r.0.0.mca")).unwrap().len(), 32 * 1024 * 1024);

        let descriptor = &manifest.entries[0].blob;
        let blob = blob_path(&storage, world, descriptor);
        let mut file = OpenOptions::new().write(true).open(&blob).unwrap();
        file.seek(SeekFrom::Start(descriptor.encoded_size / 2)).unwrap();
        file.set_len(descriptor.encoded_size / 2).unwrap();
        assert!(matches!(
            storage.verify_blob_streaming(world, descriptor),
            Err(StorageError::BlobCorrupt(hash)) if hash == descriptor.hash
        ));
    }

    #[test]
    #[ignore = "release soak: streams 1, 5, and 10 GiB synthetic files"]
    fn release_large_world_streaming_profiles() {
        let sizes = [1u64, 5, 10];
        for gib in sizes {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            fs::create_dir_all(&source).unwrap();
            write_pattern(&source.join("level.dat.large"), gib * 1024 * 1024 * 1024);
            let storage = Storage::open(temp.path().join("store")).unwrap();
            let world = WorldId([gib as u8; 32]);
            let manifest = storage.snapshot_directory_streaming(&source, context(world)).unwrap();
            storage.commit_snapshot_streaming(&manifest).unwrap();
            storage.verify_snapshot_streaming(&manifest).unwrap();
        }
    }
}
