use crate::{
    portable::{validate_manifest_paths, validate_portable_path},
    retention::SnapshotPublicationLease,
    transaction::{create_unique_temp, durable_atomic_write, durable_remove, sync_parent},
    SnapshotCommitFence, SnapshotContext, Storage, StorageError,
};
use std::{
    any::Any,
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    ops::{Deref, DerefMut},
    path::{Component, Path, PathBuf},
};
use swarm_protocol::{
    snapshot_state_root, BlobDescriptor, BlobEncoding, Hash32, SnapshotEntry, SnapshotManifestV1, WorldId,
    BLOB_HASH_DOMAIN, PROTOCOL_VERSION,
};
use walkdir::WalkDir;

pub const STREAM_BUFFER_SIZE: usize = 1024 * 1024;
const RESTORE_INCOMPLETE_MARKER: &str = ".swarmcraft-restore-incomplete";

/// A local snapshot manifest together with the durable publication ownership
/// that protects its complete blobs until the manifest and canonical head are durable.
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

        let mut lease = self.begin_snapshot_publication(context.world)?;
        let mut entries = Vec::with_capacity(files.len());
        for path in files {
            let relative = path.strip_prefix(source).expect("walkdir entries stay beneath root");
            let relative = portable_relative_path(relative)?;
            let blob = self.put_file_blob_streaming(&mut lease, &path)?;
            entries.push(SnapshotEntry { path: relative, blob });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        validate_manifest_paths(&entries)?;
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
            uncompressed_size =
                uncompressed_size.checked_add(read as u64).ok_or(StorageError::SnapshotNumberExhausted)?;
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

    pub fn commit_snapshot_streaming<T: SnapshotCommitInput>(&self, target: &T) -> Result<(), StorageError> {
        self.commit_snapshot_streaming_inner(target, None)
    }

    pub fn commit_snapshot_fenced<T: SnapshotCommitInput>(
        &self,
        target: &T,
        fence: SnapshotCommitFence,
    ) -> Result<(), StorageError> {
        self.commit_snapshot_streaming_inner(target, Some(fence))
    }

    fn commit_snapshot_streaming_inner<T: SnapshotCommitInput>(
        &self,
        target: &T,
        fence: Option<SnapshotCommitFence>,
    ) -> Result<(), StorageError> {
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
        let attempted_hash = manifest.manifest_hash()?;

        // Lock order is GC then world transaction. GC currently reads canonical
        // snapshot state while holding its lock, so taking the world lock first
        // would create a lock-order inversion. The existing-slot check is also
        // serialized under this lock: two publishers may both observe the slot
        // absent before commit, but after the first publishes the exact manifest
        // the second must become an idempotent success rather than a history conflict.
        let _gc_guard = self.lock_blob_gc_for_snapshot_commit(manifest.world_id)?;

        // Exact historical duplicates are idempotent, but a numbered slot is
        // immutable forever once present. Validate the canonical namespace first
        // so an interrupted orphan above the head can never be blessed as a duplicate.
        if path.exists() {
            self.validate_canonical_snapshot_namespace(manifest.world_id)?;
            let existing = self.load_snapshot_file_unchecked(manifest.world_id, manifest.snapshot_number)?;
            let existing_hash = existing.manifest_hash()?;
            if existing_hash != attempted_hash {
                return Err(StorageError::SnapshotManifestConflict {
                    world: manifest.world_id,
                    snapshot_number: manifest.snapshot_number,
                    existing: existing_hash,
                    attempted: attempted_hash,
                });
            }
            if let Some(publication) = publication {
                self.release_snapshot_publication_pins(manifest.world_id, publication.publication_id())?;
            }
            return Ok(());
        }

        let bytes = postcard::to_allocvec(manifest)?;
        let transaction = self.begin_snapshot_commit_transaction(manifest, fence)?;
        let publish = publish_immutable_manifest(&path, &bytes);
        if let Err(error) = publish {
            // Keep the durable intent for ambiguous I/O failures. Only a known
            // pre-publication slot conflict is safe to cancel immediately.
            if matches!(error, StorageError::SnapshotHistoryConflict { .. }) {
                self.cancel_snapshot_commit_before_manifest(transaction)?;
            }
            return Err(error);
        }
        self.finish_snapshot_commit_transaction(transaction)?;
        if let Some(publication) = publication {
            self.release_snapshot_publication_pins(manifest.world_id, publication.publication_id())?;
        }
        Ok(())
    }

    /// Restore with an explicit crash marker. Source integrity is fully checked
    /// before the destination is touched. Once mutation starts, any interruption
    /// leaves a durable marker and subsequent restore attempts fail closed until
    /// `discard_incomplete_restore` explicitly clears the partial tree.
    pub fn restore_snapshot_streaming(
        &self,
        manifest: &SnapshotManifestV1,
        destination: &Path,
    ) -> Result<(), StorageError> {
        self.verify_snapshot_streaming(manifest)?;
        if restore_marker(destination).exists() {
            return Err(StorageError::RestoreIncomplete(destination.to_path_buf()));
        }
        ensure_restore_root(destination)?;
        validate_existing_restore_tree(destination)?;
        let marker = restore_marker(destination);
        durable_atomic_write(&marker, b"swarmcraft-restore-incomplete-v1\n")?;
        clear_restore_destination(destination, &marker)?;

        for entry in &manifest.entries {
            let output = destination.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            restore_blob_streaming(self, manifest.world_id, &entry.blob, destination, &output)?;
        }
        verify_restored_tree(manifest, destination)?;
        durable_remove(&marker)?;
        Ok(())
    }

    /// Explicit recovery action for a destination left partial by a crashed
    /// restore. This never follows symlinks and only acts when the durable marker
    /// exists, avoiding accidental deletion of an unrelated directory.
    pub fn discard_incomplete_restore(&self, destination: &Path) -> Result<bool, StorageError> {
        let marker = restore_marker(destination);
        if !marker.exists() {
            return Ok(false);
        }
        if !restore_directory_exists(destination)? {
            return Err(StorageError::RestoreIncomplete(destination.to_path_buf()));
        }
        clear_restore_destination(destination, &marker)?;
        durable_remove(&marker)?;
        Ok(true)
    }
}

fn publish_immutable_manifest(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    let (temporary_path, mut temporary_file) = create_unique_temp(parent, "manifest", "tmp")?;
    temporary_file.write_all(bytes).map_err(|error| io_error(&temporary_path, error))?;
    temporary_file.sync_all().map_err(|error| io_error(&temporary_path, error))?;
    drop(temporary_file);

    if path.exists() {
        remove_if_present(&temporary_path)?;
        let snapshot_number =
            path.file_stem().and_then(|value| value.to_str()).and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
        return Err(StorageError::SnapshotHistoryConflict { snapshot_number });
    }
    fs::rename(&temporary_path, path).map_err(|error| io_error(path, error))?;
    sync_parent(parent)
}

fn validate_manifest_shape(manifest: &SnapshotManifestV1) -> Result<(), StorageError> {
    if manifest.protocol_version != PROTOCOL_VERSION {
        return Err(StorageError::UnsupportedProtocol(manifest.protocol_version));
    }
    validate_manifest_paths(&manifest.entries)?;
    if snapshot_state_root(&manifest.entries)? != manifest.state_root {
        return Err(StorageError::StateRootMismatch);
    }
    Ok(())
}

fn restore_marker(destination: &Path) -> PathBuf {
    destination.join(RESTORE_INCOMPLETE_MARKER)
}

fn validate_existing_restore_tree(destination: &Path) -> Result<(), StorageError> {
    for entry in WalkDir::new(destination).min_depth(1).follow_links(false) {
        let entry = entry
            .map_err(|error| io_error(error.path().unwrap_or(destination), std::io::Error::other(error.to_string())))?;
        if entry.file_type().is_symlink() {
            return Err(StorageError::SymlinkUnsupported(entry.path().to_path_buf()));
        }
    }
    Ok(())
}

fn clear_restore_destination(destination: &Path, marker: &Path) -> Result<(), StorageError> {
    for entry in fs::read_dir(destination).map_err(|error| io_error(destination, error))? {
        let entry = entry.map_err(|error| io_error(destination, error))?;
        let path = entry.path();
        if path == marker {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| io_error(&path, error))?;
        if file_type.is_symlink() || file_type.is_file() {
            durable_remove(&path)?;
        } else if file_type.is_dir() {
            fs::remove_dir_all(&path).map_err(|error| io_error(&path, error))?;
            sync_parent(destination)?;
        } else {
            return Err(StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()));
        }
    }
    Ok(())
}

fn verify_restored_tree(manifest: &SnapshotManifestV1, destination: &Path) -> Result<(), StorageError> {
    let expected: BTreeSet<&str> = manifest.entries.iter().map(|entry| entry.path.as_str()).collect();
    let mut actual = BTreeSet::new();
    for entry in WalkDir::new(destination).follow_links(false) {
        let entry = entry
            .map_err(|error| io_error(error.path().unwrap_or(destination), std::io::Error::other(error.to_string())))?;
        if entry.file_type().is_symlink() {
            return Err(StorageError::SymlinkUnsupported(entry.path().to_path_buf()));
        }
        if !entry.file_type().is_file() || entry.path() == restore_marker(destination) {
            continue;
        }
        let relative = entry.path().strip_prefix(destination).expect("walk stays beneath restore destination");
        actual.insert(portable_relative_path(relative)?);
    }
    let actual_refs: BTreeSet<&str> = actual.iter().map(String::as_str).collect();
    if actual_refs != expected {
        return Err(StorageError::StateRootMismatch);
    }
    Ok(())
}

fn restore_directory_exists(path: &Path) -> Result<bool, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(StorageError::SymlinkUnsupported(path.to_path_buf())),
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(io_error(
            path,
            std::io::Error::new(std::io::ErrorKind::NotADirectory, "restore path component is not a directory"),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path, error)),
    }
}

fn ensure_restore_root(destination: &Path) -> Result<(), StorageError> {
    if !restore_directory_exists(destination)? {
        fs::create_dir_all(destination).map_err(|error| io_error(destination, error))?;
        if !restore_directory_exists(destination)? {
            return Err(io_error(
                destination,
                std::io::Error::new(std::io::ErrorKind::NotADirectory, "restore destination is not a directory"),
            ));
        }
        if let Some(parent) = destination.parent() {
            sync_parent(parent)?;
        }
    }
    Ok(())
}

fn ensure_restore_parent(destination: &Path, parent: &Path) -> Result<(), StorageError> {
    let relative = parent
        .strip_prefix(destination)
        .map_err(|_| StorageError::UnsafeRelativePath(parent.to_string_lossy().into_owned()))?;
    if !restore_directory_exists(destination)? {
        return Err(io_error(
            destination,
            std::io::Error::new(std::io::ErrorKind::NotFound, "restore destination disappeared"),
        ));
    }

    let mut current = destination.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(StorageError::UnsafeRelativePath(parent.to_string_lossy().into_owned()));
        };
        current.push(part);
        if !restore_directory_exists(&current)? {
            fs::create_dir(&current).map_err(|error| io_error(&current, error))?;
            if !restore_directory_exists(&current)? {
                return Err(io_error(
                    &current,
                    std::io::Error::new(std::io::ErrorKind::NotADirectory, "restore directory was replaced"),
                ));
            }
            if let Some(parent) = current.parent() {
                sync_parent(parent)?;
            }
        }
    }
    Ok(())
}

fn restore_blob_streaming(
    storage: &Storage,
    world: WorldId,
    descriptor: &BlobDescriptor,
    destination: &Path,
    output: &Path,
) -> Result<(), StorageError> {
    let encoded_path = blob_path(storage, world, descriptor);
    ensure_encoded_size(&encoded_path, descriptor)?;
    let parent =
        output.parent().ok_or_else(|| StorageError::UnsafeRelativePath(output.to_string_lossy().into_owned()))?;
    ensure_restore_parent(destination, parent)?;
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
    match fs::symlink_metadata(output) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            remove_if_present(&temporary_path)?;
            return Err(StorageError::SymlinkUnsupported(output.to_path_buf()));
        }
        Ok(_) => remove_if_present(output)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            remove_if_present(&temporary_path)?;
            return Err(io_error(output, error));
        }
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

fn remove_if_present(path: &Path) -> Result<(), StorageError> {
    durable_remove(path).map(|_| ())
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
    use std::fs::OpenOptions;
    use std::{sync::mpsc, thread};
    use swarm_protocol::PeerId;

    fn context(world: WorldId) -> SnapshotContext {
        context_for(world, 1, 1, None)
    }

    fn context_for(
        world: WorldId,
        snapshot_number: u64,
        sequence: u64,
        previous_snapshot_hash: Option<Hash32>,
    ) -> SnapshotContext {
        SnapshotContext {
            world,
            snapshot_number,
            epoch: 1,
            sequence,
            previous_snapshot_hash,
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
        write_pattern(&source.join("region/r.0.0.mca"), 8 * 1024 * 1024);

        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([9; 32]);
        let mut manifest = storage.snapshot_directory_streaming(&source, context(world)).unwrap();
        manifest.signature = vec![0; 64];
        storage.commit_snapshot_streaming(&manifest).unwrap();
        storage.verify_snapshot_streaming(&manifest).unwrap();
        storage.restore_snapshot_streaming(&manifest, &restore).unwrap();
        assert_eq!(fs::metadata(restore.join("region/r.0.0.mca")).unwrap().len(), 8 * 1024 * 1024);

        let descriptor = &manifest.entries[0].blob;
        let blob = blob_path(&storage, world, descriptor);
        let file = OpenOptions::new().write(true).open(&blob).unwrap();
        file.set_len(descriptor.encoded_size / 2).unwrap();
        assert!(matches!(
            storage.verify_blob_streaming(world, descriptor),
            Err(StorageError::BlobCorrupt(hash)) if hash == descriptor.hash
        ));
    }

    #[test]
    fn exact_repeat_is_idempotent_but_numbered_slot_is_immutable() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("level.dat"), b"one").unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0x41; 32]);
        let mut first = storage.snapshot_directory_streaming(&source, context(world)).unwrap();
        first.signature = vec![1; 64];
        storage.commit_snapshot_streaming(&first).unwrap();
        storage.commit_snapshot_streaming(first.manifest()).unwrap();

        let mut conflict = first.manifest().clone();
        conflict.signature = vec![2; 64];
        assert!(matches!(
            storage.commit_snapshot_streaming(&conflict),
            Err(StorageError::SnapshotManifestConflict { snapshot_number: 1, .. })
        ));
        assert_eq!(storage.load_snapshot(world, 1).unwrap().manifest_hash().unwrap(), first.manifest_hash().unwrap());
    }

    #[test]
    fn direct_parent_and_sequence_are_required() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("level.dat"), b"one").unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0x42; 32]);
        let mut first = storage.snapshot_directory_streaming(&source, context(world)).unwrap();
        first.signature = vec![1; 64];
        storage.commit_snapshot_streaming(&first).unwrap();

        fs::write(source.join("level.dat"), b"two").unwrap();
        let mut skipped = storage
            .snapshot_directory_streaming(&source, context_for(world, 3, 3, Some(first.manifest_hash().unwrap())))
            .unwrap();
        skipped.signature = vec![2; 64];
        assert!(matches!(
            storage.commit_snapshot_streaming(&skipped),
            Err(StorageError::SnapshotHistoryConflict { snapshot_number: 3 })
        ));
    }

    #[test]
    fn restore_marker_forces_explicit_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let restore = temp.path().join("restore");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("level.dat"), b"safe").unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0x43; 32]);
        let mut manifest = storage.snapshot_directory_streaming(&source, context(world)).unwrap();
        manifest.signature = vec![1; 64];
        storage.commit_snapshot_streaming(&manifest).unwrap();
        fs::create_dir_all(&restore).unwrap();
        fs::write(restore.join(RESTORE_INCOMPLETE_MARKER), b"crashed").unwrap();
        fs::write(restore.join("partial.dat"), b"partial").unwrap();

        assert!(matches!(
            storage.restore_snapshot_streaming(&manifest, &restore),
            Err(StorageError::RestoreIncomplete(path)) if path == restore
        ));
        assert!(storage.discard_incomplete_restore(&restore).unwrap());
        storage.restore_snapshot_streaming(&manifest, &restore).unwrap();
        assert_eq!(fs::read(restore.join("level.dat")).unwrap(), b"safe");
        assert!(!restore.join(RESTORE_INCOMPLETE_MARKER).exists());
    }

    #[cfg(unix)]
    #[test]
    fn restore_rejects_symlinked_parent_component() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let restore = temp.path().join("restore");
        let outside = temp.path().join("outside");
        fs::create_dir_all(source.join("region")).unwrap();
        fs::write(source.join("region/r.0.0.mca"), b"safe-data").unwrap();
        fs::create_dir_all(&restore).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, restore.join("region")).unwrap();

        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0x71; 32]);
        let mut manifest = storage.snapshot_directory_streaming(&source, context(world)).unwrap();
        manifest.signature = vec![0; 64];
        storage.commit_snapshot_streaming(&manifest).unwrap();
        assert!(matches!(
            storage.restore_snapshot_streaming(&manifest, &restore),
            Err(StorageError::SymlinkUnsupported(path)) if path == restore.join("region")
        ));
        assert!(!outside.join("r.0.0.mca").exists());
    }

    #[test]
    fn two_live_publications_keep_distinct_pin_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("level.dat"), b"same-content-for-both-publishers").unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0xa1; 32]);

        let mut first = storage.snapshot_directory_streaming(&source, context_for(world, 1, 1, None)).unwrap();
        first.signature = vec![0; 64];
        let hash = first.entries[0].blob.hash;
        let first_id = first.publication_id().to_owned();
        let mut second = storage.snapshot_directory_streaming(&source, context_for(world, 2, 2, None)).unwrap();
        let second_id = second.publication_id().to_owned();
        assert_ne!(first_id, second_id);
        assert!(storage.snapshot_publication_has_pin(world, &first_id, hash));
        assert!(storage.snapshot_publication_has_pin(world, &second_id, hash));

        storage.commit_snapshot_streaming(&first).unwrap();
        second.previous_snapshot_hash = Some(first.manifest_hash().unwrap());
        second.signature = vec![1; 64];
        storage.commit_snapshot_streaming(&second).unwrap();
        assert!(!storage.snapshot_publication_has_pin(world, &first_id, hash));
        assert!(!storage.snapshot_publication_has_pin(world, &second_id, hash));
        assert_eq!(storage.list_snapshots(world).unwrap().len(), 2);
    }

    #[test]
    fn gc_cannot_reclaim_local_blob_before_manifest_commit() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("level.dat"), vec![7u8; 64 * 1024]).unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0x90; 32]);
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
    }

    #[test]
    fn encoded_verifier_rejects_expansion_past_declared_size() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("amplification.zst");
        let expanded = vec![0u8; 2 * 1024 * 1024];
        let encoded = zstd::stream::encode_all(expanded.as_slice(), 3).unwrap();
        fs::write(&path, &encoded).unwrap();
        let descriptor = BlobDescriptor {
            hash: BlobDescriptor::hash_uncompressed(&[0]),
            uncompressed_size: 1,
            encoded_size: encoded.len() as u64,
            encoding: BlobEncoding::Zstd,
        };
        assert!(matches!(verify_encoded_blob_streaming(&path, &descriptor), Err(StorageError::BlobCorrupt(_))));
    }

    #[test]
    #[ignore = "release soak: streams a 1 GiB synthetic file"]
    fn release_large_world_streaming_profile() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        write_pattern(&source.join("level.dat.large"), 1024 * 1024 * 1024);
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0xee; 32]);
        let manifest = storage.snapshot_directory_streaming(&source, context(world)).unwrap();
        storage.commit_snapshot_streaming(&manifest).unwrap();
        storage.verify_snapshot_streaming(&manifest).unwrap();
    }
}
