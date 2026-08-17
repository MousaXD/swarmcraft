use crate::{retention::SnapshotPublicationLease, SnapshotContext, Storage, StorageError};
use std::{
    any::Any,
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    ops::{Deref, DerefMut},
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

/// A local snapshot manifest together with the durable publication ownership
/// that protects its complete blobs until the manifest becomes durable.
#[derive(Debug)]
pub struct SnapshotPublication {
    manifest: SnapshotManifestV1,
    lease: SnapshotPublicationLease,
}

impl SnapshotPublication {
    pub fn manifest(&self) -> &SnapshotManifestV1 {
        &self.manifest
    }

    pub fn manifest_mut(&mut self) -> &mut SnapshotManifestV1 {
        &mut self.manifest
    }

    pub fn publication_id(&self) -> &str {
        self.lease.publication_id()
    }

    pub fn pinned_blobs(&self) -> usize {
        self.lease.pinned_blobs()
    }
}

impl AsRef<SnapshotManifestV1> for SnapshotPublication {
    fn as_ref(&self) -> &SnapshotManifestV1 {
        &self.manifest
    }
}

impl Deref for SnapshotPublication {
    type Target = SnapshotManifestV1;

    fn deref(&self) -> &Self::Target {
        &self.manifest
    }
}

impl DerefMut for SnapshotPublication {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.manifest
    }
}

/// A snapshot object accepted by the commit path. Only the concrete
/// `SnapshotPublication` type carries local publication ownership; any other
/// implementation is committed without releasing publication pins.
pub trait SnapshotCommitInput: Any {
    fn snapshot_manifest(&self) -> &SnapshotManifestV1;
}

impl SnapshotCommitInput for SnapshotManifestV1 {
    fn snapshot_manifest(&self) -> &SnapshotManifestV1 {
        self
    }
}

impl SnapshotCommitInput for SnapshotPublication {
    fn snapshot_manifest(&self) -> &SnapshotManifestV1 {
        &self.manifest
    }
}

#[cfg(test)]
struct PublicationHook {
    world: WorldId,
    published: std::sync::mpsc::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
static TEST_PUBLICATION_HOOK: std::sync::Mutex<Option<PublicationHook>> = std::sync::Mutex::new(None);

impl Storage {
    pub fn snapshot_directory_streaming(
        &self,
        source: &Path,
        context: SnapshotContext,
    ) -> Result<SnapshotPublication, StorageError> {
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

        // Ownership exists before any complete blob can become visible. Every
        // blob published by this snapshot is pinned inside this transaction's
        // directory until this exact publication commits.
        let mut lease = self.begin_snapshot_publication(context.world)?;
        let mut entries = Vec::with_capacity(files.len());
        for path in files {
            let relative = path.strip_prefix(source).expect("walkdir entries stay beneath root");
            let relative = portable_relative_path(relative)?;
            let blob = self.put_file_blob_streaming(&mut lease, &path)?;
            entries.push(SnapshotEntry { path: relative, blob });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let state_root = snapshot_state_root(&entries)?;
        let manifest = SnapshotManifestV1 {
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
        };
        Ok(SnapshotPublication { manifest, lease })
    }

    pub fn put_file_blob_streaming(
        &self,
        lease: &mut SnapshotPublicationLease,
        source: &Path,
    ) -> Result<BlobDescriptor, StorageError> {
        let world = lease.world();
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
        lease.pin_hash(hash)?;
        let mut descriptor = BlobDescriptor { hash, uncompressed_size, encoded_size, encoding: BlobEncoding::Zstd };
        let final_path = blob_path(self, world, &descriptor);

        if final_path.exists() {
            let existing_size = fs::metadata(&final_path).map_err(|error| io_error(&final_path, error))?.len();
            let existing = BlobDescriptor { encoded_size: existing_size, ..descriptor.clone() };
            if verify_encoded_blob_streaming(&final_path, &existing).is_ok() {
                remove_if_present(&temporary_path)?;
                test_after_complete_blob_published(world);
                return Ok(existing);
            }
            remove_if_present(&final_path)?;
        }

        match fs::rename(&temporary_path, &final_path) {
            Ok(()) => {}
            Err(rename_error) if final_path.is_file() => {
                // Another publisher may have won the identical-hash publish
                // race after our existence check. A valid complete target is
                // equivalent to our own bytes and is safe to reuse.
                let existing_size = fs::metadata(&final_path).map_err(|error| io_error(&final_path, error))?.len();
                let existing = BlobDescriptor { encoded_size: existing_size, ..descriptor.clone() };
                if verify_encoded_blob_streaming(&final_path, &existing).is_ok() {
                    remove_if_present(&temporary_path)?;
                    test_after_complete_blob_published(world);
                    return Ok(existing);
                }
                return Err(io_error(&final_path, rename_error));
            }
            Err(error) => return Err(io_error(&final_path, error)),
        }
        sync_parent(&blob_dir)?;
        descriptor.encoded_size = fs::metadata(&final_path).map_err(|error| io_error(&final_path, error))?.len();
        test_after_complete_blob_published(world);
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

    /// Commits either a local transaction-owned publication or a plain manifest.
    /// Plain manifests are used by replica finalization and never release local
    /// publication pins. Only the exact `SnapshotPublication` passed here may
    /// release the transaction directory it owns.
    pub fn commit_snapshot_streaming<T: SnapshotCommitInput>(&self, target: &T) -> Result<(), StorageError> {
        let manifest = target.snapshot_manifest();
        let publication = (target as &dyn Any).downcast_ref::<SnapshotPublication>();
        if let Some(publication) = publication {
            if publication.lease.world() != manifest.world_id {
                return Err(StorageError::SnapshotPublicationWorldMismatch {
                    publication_world: publication.lease.world(),
                    manifest_world: manifest.world_id,
                });
            }
            for hash in manifest.entries.iter().map(|entry| entry.blob.hash) {
                if !publication.lease.owns_durable_hash(hash) {
                    return Err(StorageError::SnapshotPublicationMissingPin(hash));
                }
            }
        }

        self.verify_snapshot_streaming(manifest)?;
        let snapshots = self.world_dir(manifest.world_id).join("snapshots");
        fs::create_dir_all(&snapshots).map_err(|error| io_error(&snapshots, error))?;
        let path = snapshots.join(format!("{:020}.postcard", manifest.snapshot_number));
        let bytes = postcard::to_allocvec(manifest)?;
        let _gc_guard = self.lock_blob_gc_for_snapshot_commit(manifest.world_id)?;
        atomic_write(&path, &bytes)?;
        if let Some(publication) = publication {
            self.release_snapshot_publication_pins(manifest.world_id, publication.publication_id())?;
        }
        Ok(())
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
        let remaining = descriptor.uncompressed_size.saturating_sub(total);
        let read_limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let read = reader.read(&mut buffer[..read_limit]).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?;
        if read == 0 {
            break;
        }
        if read as u64 > remaining {
            remove_if_present(&temporary_path)?;
            return Err(StorageError::BlobCorrupt(descriptor.hash));
        }
        hasher.update(&buffer[..read]);
        temporary_file.write_all(&buffer[..read]).map_err(|error| io_error(&temporary_path, error))?;
        total += read as u64;
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
        let remaining = descriptor.uncompressed_size.saturating_sub(total);
        let read_limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let read = reader.read(&mut buffer[..read_limit]).map_err(|_| StorageError::BlobCorrupt(descriptor.hash))?;
        if read == 0 {
            break;
        }
        if read as u64 > remaining {
            return Err(StorageError::BlobCorrupt(descriptor.hash));
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
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
fn test_after_complete_blob_published(world: WorldId) {
    let hook = {
        let mut slot = TEST_PUBLICATION_HOOK.lock().expect("publication test hook lock poisoned");
        if slot.as_ref().is_some_and(|hook| hook.world == world) {
            slot.take()
        } else {
            None
        }
    };
    if let Some(hook) = hook {
        hook.published.send(()).expect("publication test receiver dropped");
        hook.resume.recv().expect("publication test resume sender dropped");
    }
}

#[cfg(not(test))]
fn test_after_complete_blob_published(_world: WorldId) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Seek, SeekFrom},
        sync::mpsc,
        thread,
    };
    use swarm_protocol::PeerId;

    fn context(world: WorldId) -> SnapshotContext {
        context_for(world, 1, 1)
    }

    fn context_for(world: WorldId, snapshot_number: u64, sequence: u64) -> SnapshotContext {
        SnapshotContext {
            world,
            snapshot_number,
            epoch: 1,
            sequence,
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

    fn remove_snapshot_manifest(storage: &Storage, world: WorldId, snapshot_number: u64) {
        let path = storage.world_dir(world).join("snapshots").join(format!("{snapshot_number:020}.postcard"));
        fs::remove_file(path).unwrap();
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
    fn two_concurrent_publishers_same_hash_keep_distinct_publication_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("level.dat"), b"same-content-for-both-publishers").unwrap();
        fs::write(source.join("session.lock"), b"same-content-for-both-publishers").unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0xa1; 32]);

        let mut first = storage.snapshot_directory_streaming(&source, context_for(world, 1, 1)).unwrap();
        first.signature = vec![0; 64];
        let hash = first.entries[0].blob.hash;
        assert_eq!(first.entries.len(), 2);
        assert!(first.entries.iter().all(|entry| entry.blob.hash == hash));
        assert_eq!(first.pinned_blobs(), 1);
        let first_id = first.publication_id().to_owned();

        let mut second = storage.snapshot_directory_streaming(&source, context_for(world, 2, 2)).unwrap();
        second.signature = vec![0; 64];
        let second_id = second.publication_id().to_owned();
        assert!(second.entries.iter().all(|entry| entry.blob.hash == hash));
        assert_eq!(second.pinned_blobs(), 1);
        assert_ne!(first_id, second_id);
        assert!(storage.snapshot_publication_has_pin(world, &first_id, hash));
        assert!(storage.snapshot_publication_has_pin(world, &second_id, hash));

        let second_manifest = second.manifest().clone();
        storage.commit_snapshot_streaming(&second).unwrap();
        assert!(storage.snapshot_publication_has_pin(world, &first_id, hash));
        assert!(!storage.snapshot_publication_has_pin(world, &second_id, hash));

        // Remove the second snapshot root to recreate the audit's dangerous
        // window. The first publisher's independently owned pin must still keep
        // the shared complete blob alive.
        remove_snapshot_manifest(&storage, world, 2);
        let gc = storage.garbage_collect_blobs(world).unwrap();
        assert_eq!(gc.removed_blobs, 0);
        assert!(blob_path(&storage, world, &first.entries[0].blob).exists());
        assert!(storage.snapshot_publication_has_pin(world, &first_id, hash));

        storage.commit_snapshot_streaming(&first).unwrap();
        storage.finalize_replica(&second_manifest).unwrap();
        storage.verify_snapshot_streaming(&first).unwrap();
        storage.verify_snapshot_streaming(&second_manifest).unwrap();
        assert_eq!(storage.list_snapshots(world).unwrap().len(), 2);
    }

    #[test]
    fn replica_commit_never_releases_local_publication_pins() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("level.dat"), b"shared-local-and-replica-content").unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0xa2; 32]);

        let mut local = storage.snapshot_directory_streaming(&source, context_for(world, 1, 1)).unwrap();
        local.signature = vec![0; 64];
        let hash = local.entries[0].blob.hash;
        let publication_id = local.publication_id().to_owned();

        let mut replica = local.manifest().clone();
        replica.snapshot_number = 2;
        replica.sequence = 2;
        storage.finalize_replica(&replica).unwrap();
        assert!(storage.snapshot_publication_has_pin(world, &publication_id, hash));

        remove_snapshot_manifest(&storage, world, 2);
        let gc = storage.garbage_collect_blobs(world).unwrap();
        assert_eq!(gc.removed_blobs, 0);
        assert!(storage.snapshot_publication_has_pin(world, &publication_id, hash));
        storage.commit_snapshot_streaming(&local).unwrap();
        storage.verify_snapshot_streaming(&local).unwrap();
    }

    #[test]
    fn different_hash_publishers_release_only_their_own_data() {
        let temp = tempfile::tempdir().unwrap();
        let source_a = temp.path().join("source-a");
        let source_b = temp.path().join("source-b");
        fs::create_dir_all(&source_a).unwrap();
        fs::create_dir_all(&source_b).unwrap();
        fs::write(source_a.join("level.dat"), b"publisher-a").unwrap();
        fs::write(source_b.join("level.dat"), b"publisher-b").unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0xa3; 32]);

        let first = storage.snapshot_directory_streaming(&source_a, context_for(world, 1, 1)).unwrap();
        let first_blob = first.entries[0].blob.clone();
        let mut second = storage.snapshot_directory_streaming(&source_b, context_for(world, 2, 2)).unwrap();
        second.signature = vec![0; 64];
        let second_blob = second.entries[0].blob.clone();
        assert_ne!(first_blob.hash, second_blob.hash);

        storage.commit_snapshot_streaming(&second).unwrap();
        let while_first_live = storage.garbage_collect_blobs(world).unwrap();
        assert_eq!(while_first_live.removed_blobs, 0);
        assert!(blob_path(&storage, world, &first_blob).exists());
        assert!(blob_path(&storage, world, &second_blob).exists());

        drop(first);
        let after_first_abandoned = storage.garbage_collect_blobs(world).unwrap();
        assert_eq!(after_first_abandoned.removed_blobs, 1);
        assert!(!blob_path(&storage, world, &first_blob).exists());
        assert!(blob_path(&storage, world, &second_blob).exists());
        storage.verify_snapshot_streaming(&second).unwrap();
    }

    #[test]
    fn crashed_publication_recovery_never_steals_a_live_owner() {
        let temp = tempfile::tempdir().unwrap();
        let store_root = temp.path().join("store");
        let source_live = temp.path().join("source-live");
        let source_abandoned = temp.path().join("source-abandoned");
        let source_committed = temp.path().join("source-committed");
        for source in [&source_live, &source_abandoned, &source_committed] {
            fs::create_dir_all(source).unwrap();
        }
        fs::write(source_live.join("level.dat"), b"live-publication").unwrap();
        fs::write(source_abandoned.join("level.dat"), b"abandoned-publication").unwrap();
        fs::write(source_committed.join("level.dat"), b"committed-publication").unwrap();

        let storage = Storage::open(&store_root).unwrap();
        let world = WorldId([0xa4; 32]);
        let live = storage.snapshot_directory_streaming(&source_live, context_for(world, 1, 1)).unwrap();
        let live_blob = live.entries[0].blob.clone();
        let live_id = live.publication_id().to_owned();

        let abandoned = storage.snapshot_directory_streaming(&source_abandoned, context_for(world, 2, 2)).unwrap();
        let abandoned_blob = abandoned.entries[0].blob.clone();
        let abandoned_id = abandoned.publication_id().to_owned();
        drop(abandoned);

        let mut committed = storage.snapshot_directory_streaming(&source_committed, context_for(world, 3, 3)).unwrap();
        committed.signature = vec![0; 64];
        let committed_blob = committed.entries[0].blob.clone();
        storage.commit_snapshot_streaming(&committed).unwrap();

        // A reopen performs stale-publication recovery. The live publication's
        // owner lock must be unstealable, while the dropped transaction is
        // provably stale and may lose its pins.
        let reopened = Storage::open(&store_root).unwrap();
        assert!(reopened.snapshot_publication_has_pin(world, &live_id, live_blob.hash));
        assert!(!reopened.snapshot_publication_has_pin(world, &abandoned_id, abandoned_blob.hash));
        let first_gc = reopened.garbage_collect_blobs(world).unwrap();
        assert_eq!(first_gc.removed_blobs, 1);
        assert!(blob_path(&reopened, world, &live_blob).exists());
        assert!(!blob_path(&reopened, world, &abandoned_blob).exists());
        assert!(blob_path(&reopened, world, &committed_blob).exists());
        reopened.verify_snapshot_streaming(&committed).unwrap();

        drop(live);
        let reopened_again = Storage::open(&store_root).unwrap();
        let second_gc = reopened_again.garbage_collect_blobs(world).unwrap();
        assert_eq!(second_gc.removed_blobs, 1);
        assert!(!blob_path(&reopened_again, world, &live_blob).exists());
        assert!(blob_path(&reopened_again, world, &committed_blob).exists());
        reopened_again.verify_snapshot_streaming(&committed).unwrap();
    }

    #[test]
    fn gc_cannot_reclaim_local_blob_before_manifest_commit() {
        for iteration in 0..8u8 {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            let restore = temp.path().join("restore");
            fs::create_dir_all(&source).unwrap();
            fs::write(source.join("level.dat"), vec![iteration; 64 * 1024]).unwrap();

            let storage = Storage::open(temp.path().join("store")).unwrap();
            let world = WorldId([0x90 + iteration; 32]);
            let (published_tx, published_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            *TEST_PUBLICATION_HOOK.lock().unwrap() =
                Some(PublicationHook { world, published: published_tx, resume: resume_rx });

            let worker_storage = storage.clone();
            let worker_source = source.clone();
            let worker = thread::spawn(move || {
                let mut manifest = worker_storage.snapshot_directory_streaming(&worker_source, context(world)).unwrap();
                manifest.signature = vec![0; 64];
                worker_storage.commit_snapshot_streaming(&manifest).unwrap();
                manifest
            });

            published_rx.recv().unwrap();
            let report = storage.garbage_collect_blobs(world).unwrap();
            assert_eq!(report.removed_blobs, 0);
            resume_tx.send(()).unwrap();

            let manifest = worker.join().unwrap();
            storage.verify_snapshot_streaming(&manifest).unwrap();
            for entry in &manifest.entries {
                assert!(blob_path(&storage, world, &entry.blob).exists());
            }
            storage.restore_snapshot_streaming(&manifest, &restore).unwrap();
            assert_eq!(fs::read(restore.join("level.dat")).unwrap(), vec![iteration; 64 * 1024]);
            let after_commit = storage.garbage_collect_blobs(world).unwrap();
            assert_eq!(after_commit.removed_blobs, 0);
            storage.verify_snapshot_streaming(&manifest).unwrap();
        }
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
