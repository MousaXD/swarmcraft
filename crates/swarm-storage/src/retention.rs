//! Conservative snapshot retention and blob garbage collection.
//!
//! Retention is mark-before-sweep. Snapshot manifests are the primary roots,
//! canonical/recovery control records add mandatory roots, and active replication
//! or local snapshot-publication pins keep not-yet-committed blobs alive. Unknown
//! files and partial transfers are never reclaimed by this module.

use crate::{Storage, StorageError};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
};
use swarm_protocol::{Hash32, SnapshotManifestV1, WorldId};
use thiserror::Error;
use tracing::{debug, warn};

static PIN_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Number of newest snapshots to retain in addition to mandatory recovery roots.
    /// The latest snapshot is always retained even when this is zero.
    pub keep_latest: usize,
    /// Operator/application-selected snapshots that must remain available.
    pub protected_snapshots: BTreeSet<u64>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self { keep_latest: 3, protected_snapshots: BTreeSet::new() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionReport {
    pub retained_snapshots: Vec<u64>,
    pub removed_snapshots: Vec<u64>,
    pub removed_blobs: usize,
    pub reclaimed_bytes: u64,
}

#[derive(Debug, Error)]
pub enum RetentionError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("blob garbage collection is already active for world {0}")]
    GarbageCollectionActive(WorldId),
    #[error("required recovery snapshot {0} is not present locally; refusing to prune")]
    MissingRequiredSnapshot(Hash32),
}

#[derive(Debug)]
pub struct ActiveReplicationLease {
    pin_paths: Vec<PathBuf>,
}

impl ActiveReplicationLease {
    pub fn pinned_blobs(&self) -> usize {
        self.pin_paths.len()
    }
}

impl Drop for ActiveReplicationLease {
    fn drop(&mut self) {
        remove_pin_paths(&self.pin_paths, "replication GC pin");
    }
}

#[derive(Debug)]
pub(crate) struct SnapshotPublicationPin {
    path: Option<PathBuf>,
}

impl SnapshotPublicationPin {
    pub(crate) fn persist(mut self) {
        self.path = None;
    }
}

impl Drop for SnapshotPublicationPin {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            remove_pin_paths(&[path], "snapshot publication GC pin");
        }
    }
}

#[derive(Debug)]
pub(crate) struct BlobGcCoordinationGuard {
    _file: File,
}

impl Storage {
    /// Pins blobs that an active reconstruction may publish into the complete
    /// blob namespace before its snapshot manifest is committed.
    ///
    /// Pin creation and GC use the same kernel-owned lock. If GC already owns
    /// the lock, replication fails before transfer starts. Once the durable pin
    /// exists, GC may proceed and will treat the pinned hash as live.
    pub fn pin_replication_hashes(
        &self,
        world: WorldId,
        hashes: &[Hash32],
    ) -> Result<ActiveReplicationLease, RetentionError> {
        let world_dir = self.world_dir(world);
        let _lock = acquire_gc_lock(&world_dir, world)?;
        let pins_dir = replication_pins_dir(&world_dir);
        let pin_paths = write_hash_pins(&pins_dir, hashes.iter().copied())?;
        Ok(ActiveReplicationLease { pin_paths })
    }

    /// Creates a durable ownership pin before a local snapshot publisher makes
    /// a complete blob visible. The returned guard removes the pin on an error;
    /// callers persist it only after complete-blob publication succeeds.
    pub(crate) fn pin_snapshot_publication_hash(
        &self,
        world: WorldId,
        hash: Hash32,
    ) -> Result<SnapshotPublicationPin, StorageError> {
        let world_dir = self.world_dir(world);
        let _lock = acquire_gc_lock_blocking(&world_dir)?;
        let pins_dir = snapshot_publication_pins_dir(&world_dir);
        let mut paths = write_hash_pins(&pins_dir, std::iter::once(hash))?;
        let path = paths.pop().expect("one snapshot publication pin was requested");
        Ok(SnapshotPublicationPin { path: Some(path) })
    }

    /// Serializes snapshot manifest publication and publication-pin cleanup
    /// against blob GC. This lock blocks rather than requiring a retry because
    /// local snapshot creation is an ordinary foreground storage operation.
    pub(crate) fn lock_blob_gc_for_snapshot_commit(
        &self,
        world: WorldId,
    ) -> Result<BlobGcCoordinationGuard, StorageError> {
        acquire_gc_lock_blocking(&self.world_dir(world))
    }

    /// Releases at most one local publication pin for each manifest entry.
    /// Call only while holding the blob-GC coordination lock and only when this
    /// call created the manifest path for the first time. This preserves pins
    /// belonging to concurrent publishers of the same content hash.
    pub(crate) fn release_snapshot_publication_pins(
        &self,
        world: WorldId,
        hashes: impl IntoIterator<Item = Hash32>,
    ) -> Result<(), StorageError> {
        let pins_dir = snapshot_publication_pins_dir(&self.world_dir(world));
        if !pins_dir.exists() {
            return Ok(());
        }

        let mut available = BTreeMap::<Hash32, Vec<PathBuf>>::new();
        for entry in fs::read_dir(&pins_dir).map_err(|source| io_error(&pins_dir, source))? {
            let entry = entry.map_err(|source| io_error(&pins_dir, source))?;
            if !entry.file_type().map_err(|source| io_error(entry.path(), source))?.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("pin") {
                continue;
            }
            let hash = read_pin_hash(&path)?;
            available.entry(hash).or_default().push(path);
        }

        let mut removed_any = false;
        for hash in hashes {
            let Some(paths) = available.get_mut(&hash) else {
                continue;
            };
            let Some(path) = paths.pop() else {
                continue;
            };
            match fs::remove_file(&path) {
                Ok(()) => removed_any = true,
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(io_error(&path, source)),
            }
        }
        if removed_any {
            sync_dir(&pins_dir)?;
        }
        Ok(())
    }

    /// Removes snapshot manifests outside the retention set. No blobs are
    /// deleted here, so interruption can only retain extra data.
    pub fn prune_snapshots(&self, world: WorldId, policy: &RetentionPolicy) -> Result<RetentionReport, RetentionError> {
        let snapshots = self.list_snapshots(world)?;
        if snapshots.is_empty() {
            return Ok(RetentionReport::default());
        }

        let retained = retention_roots(self, world, &snapshots, policy)?;
        let snapshots_dir = self.world_dir(world).join("snapshots");
        let mut report =
            RetentionReport { retained_snapshots: retained.iter().copied().collect(), ..Default::default() };

        for manifest in snapshots {
            if retained.contains(&manifest.snapshot_number) {
                continue;
            }
            let path = snapshots_dir.join(format!("{:020}.postcard", manifest.snapshot_number));
            match fs::remove_file(&path) {
                Ok(()) => report.removed_snapshots.push(manifest.snapshot_number),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(io_error(&path, source).into()),
            }
        }
        if snapshots_dir.exists() {
            sync_dir(&snapshots_dir)?;
        }
        report.removed_snapshots.sort_unstable();
        Ok(report)
    }

    /// Reclaims only complete blob files that are unreferenced by every
    /// currently committed snapshot and are not pinned by active replication
    /// or by local snapshot publication.
    ///
    /// `.part`, temporary, unknown, and malformed files are ignored.
    pub fn garbage_collect_blobs(&self, world: WorldId) -> Result<RetentionReport, RetentionError> {
        let world_dir = self.world_dir(world);
        let _lock = acquire_gc_lock(&world_dir, world)?;

        // Read roots only after the exclusive OS lock exists. Replication and
        // local publication create their pins while holding the same lock, so
        // GC either sees a durable pin or wins before complete publication.
        let snapshots = self.list_snapshots(world)?;
        let mut live = referenced_blob_hashes(&snapshots);
        live.extend(read_hash_pins(&replication_pins_dir(&world_dir))?);
        live.extend(read_hash_pins(&snapshot_publication_pins_dir(&world_dir))?);

        let blobs_dir = world_dir.join("blobs");
        if !blobs_dir.exists() {
            return Ok(RetentionReport::default());
        }

        let mut report = RetentionReport::default();
        for entry in fs::read_dir(&blobs_dir).map_err(|source| io_error(&blobs_dir, source))? {
            let entry = entry.map_err(|source| io_error(&blobs_dir, source))?;
            let file_type = entry.file_type().map_err(|source| io_error(entry.path(), source))?;
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            let Some(hash) = complete_blob_hash(&path) else {
                continue;
            };
            if live.contains(&hash) {
                continue;
            }
            let bytes = entry.metadata().map_err(|source| io_error(&path, source))?.len();
            match fs::remove_file(&path) {
                Ok(()) => {
                    report.removed_blobs += 1;
                    report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(bytes);
                }
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(io_error(&path, source).into()),
            }
        }
        sync_dir(&blobs_dir)?;
        debug!(
            world = %world,
            removed_blobs = report.removed_blobs,
            reclaimed_bytes = report.reclaimed_bytes,
            "blob garbage collection complete"
        );
        Ok(report)
    }

    /// Conservative two-phase retention: prune manifests first, then re-read
    /// remaining manifests under the GC lock before sweeping blobs.
    pub fn apply_retention(&self, world: WorldId, policy: &RetentionPolicy) -> Result<RetentionReport, RetentionError> {
        let mut report = self.prune_snapshots(world, policy)?;
        let gc = self.garbage_collect_blobs(world)?;
        report.removed_blobs = gc.removed_blobs;
        report.reclaimed_bytes = gc.reclaimed_bytes;

        // Re-read for an authoritative final retained list. If pruning was
        // interrupted by an I/O error, apply_retention returned before GC.
        report.retained_snapshots =
            self.list_snapshots(world)?.into_iter().map(|manifest| manifest.snapshot_number).collect();
        Ok(report)
    }
}

fn retention_roots(
    storage: &Storage,
    world: WorldId,
    snapshots: &[SnapshotManifestV1],
    policy: &RetentionPolicy,
) -> Result<BTreeSet<u64>, RetentionError> {
    let mut retained = policy.protected_snapshots.clone();
    let latest = snapshots.last().expect("non-empty snapshot list");
    retained.insert(latest.snapshot_number);

    for manifest in snapshots.iter().rev().take(policy.keep_latest) {
        retained.insert(manifest.snapshot_number);
    }

    let metadata_dir = storage.world_dir(world).join("metadata");
    let mut required_hashes = BTreeSet::new();
    if metadata_dir.join("transfer.postcard").exists() {
        required_hashes.insert(storage.load_transfer_record(world)?.base_snapshot_hash);
    }
    if metadata_dir.join("sleep.postcard").exists() {
        required_hashes.insert(storage.load_sleep_record(world)?.latest_snapshot_hash);
    }
    if metadata_dir.join("recovery-promise.postcard").exists() {
        required_hashes.insert(storage.load_recovery_promise(world)?.ballot.base_snapshot_hash);
    }
    if metadata_dir.join("recovery-certificate.postcard").exists() {
        required_hashes.insert(storage.load_recovery_certificate(world)?.ballot.base_snapshot_hash);
    }
    if metadata_dir.join("solo-branch.postcard").exists() {
        let branch = storage.load_solo_branch(world)?;
        required_hashes.insert(branch.base_snapshot_hash);
        required_hashes.insert(branch.head_snapshot_hash);
    }
    for branch in storage.list_solo_conflicts(world)? {
        required_hashes.insert(branch.base_snapshot_hash);
        required_hashes.insert(branch.head_snapshot_hash);
    }

    if !required_hashes.is_empty() {
        let mut by_hash = BTreeMap::new();
        for manifest in snapshots {
            let hash = manifest.manifest_hash().map_err(StorageError::from)?;
            by_hash.insert(hash, manifest.snapshot_number);
        }
        for hash in required_hashes {
            let Some(number) = by_hash.get(&hash) else {
                return Err(RetentionError::MissingRequiredSnapshot(hash));
            };
            retained.insert(*number);
        }
    }

    Ok(retained)
}

fn referenced_blob_hashes(snapshots: &[SnapshotManifestV1]) -> BTreeSet<Hash32> {
    snapshots.iter().flat_map(|manifest| manifest.entries.iter().map(|entry| entry.blob.hash)).collect()
}

fn acquire_gc_lock(world_dir: &Path, world: WorldId) -> Result<BlobGcCoordinationGuard, RetentionError> {
    fs::create_dir_all(world_dir).map_err(|source| io_error(world_dir, source))?;
    let path = gc_lock_path(world_dir);
    let file = open_gc_lock_file(&path)?;
    match platform_lock::try_lock_exclusive(&file) {
        Ok(true) => Ok(BlobGcCoordinationGuard { _file: file }),
        Ok(false) => Err(RetentionError::GarbageCollectionActive(world)),
        Err(source) => Err(io_error(&path, source).into()),
    }
}

fn acquire_gc_lock_blocking(world_dir: &Path) -> Result<BlobGcCoordinationGuard, StorageError> {
    fs::create_dir_all(world_dir).map_err(|source| io_error(world_dir, source))?;
    let path = gc_lock_path(world_dir);
    let file = open_gc_lock_file(&path)?;
    platform_lock::lock_exclusive(&file).map_err(|source| io_error(&path, source))?;
    Ok(BlobGcCoordinationGuard { _file: file })
}

fn open_gc_lock_file(path: &Path) -> Result<File, StorageError> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| io_error(path, source))
}

fn write_hash_pins(
    pins_dir: &Path,
    hashes: impl IntoIterator<Item = Hash32>,
) -> Result<Vec<PathBuf>, StorageError> {
    fs::create_dir_all(pins_dir).map_err(|source| io_error(pins_dir, source))?;
    let token = PIN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut paths = Vec::new();
    let mut unique = BTreeSet::new();
    for hash in hashes.into_iter().filter(|hash| unique.insert(*hash)) {
        let path = pins_dir.join(format!("{}-{token}-{}.pin", std::process::id(), hash.to_hex()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|source| io_error(&path, source))?;
        paths.push(path.clone());
        if let Err(source) = file.write_all(hash.to_hex().as_bytes()).and_then(|()| file.sync_all()) {
            remove_pin_paths(&paths, "incomplete GC pin");
            return Err(io_error(&path, source));
        }
    }
    if let Err(error) = sync_dir(pins_dir) {
        remove_pin_paths(&paths, "unsynced GC pin");
        return Err(error);
    }
    Ok(paths)
}

fn read_hash_pins(pins_dir: &Path) -> Result<BTreeSet<Hash32>, RetentionError> {
    if !pins_dir.exists() {
        return Ok(BTreeSet::new());
    }
    let mut hashes = BTreeSet::new();
    for entry in fs::read_dir(pins_dir).map_err(|source| io_error(pins_dir, source))? {
        let entry = entry.map_err(|source| io_error(pins_dir, source))?;
        if !entry.file_type().map_err(|source| io_error(entry.path(), source))?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("pin") {
            continue;
        }
        hashes.insert(read_pin_hash(&path)?);
    }
    Ok(hashes)
}

fn read_pin_hash(path: &Path) -> Result<Hash32, StorageError> {
    let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
    let value = std::str::from_utf8(&bytes).map_err(|_| {
        io_error(
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, "GC pin is not UTF-8"),
        )
    })?;
    Hash32::from_str(value.trim()).map_err(StorageError::from)
}

fn remove_pin_paths(paths: &[PathBuf], kind: &str) {
    for path in paths {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %path.display(), %error, kind, "failed to remove GC pin");
            }
        }
    }
    if let Some(parent) = paths.first().and_then(|path| path.parent()) {
        let _ = sync_dir(parent);
    }
}

fn complete_blob_hash(path: &Path) -> Option<Hash32> {
    let extension = path.extension()?.to_str()?;
    if extension != "raw" && extension != "zst" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    if stem.len() != 64 {
        return None;
    }
    Hash32::from_str(stem).ok()
}

fn gc_lock_path(world_dir: &Path) -> PathBuf {
    world_dir.join(".blob-gc.lock")
}

fn replication_pins_dir(world_dir: &Path) -> PathBuf {
    world_dir.join(".replication-pins")
}

fn snapshot_publication_pins_dir(world_dir: &Path) -> PathBuf {
    world_dir.join(".snapshot-publication-pins")
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}

fn sync_dir(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        fs::File::open(path).and_then(|file| file.sync_all()).map_err(|source| io_error(path, source))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(unix)]
mod platform_lock {
    use std::{fs::File, io, os::fd::AsRawFd, os::raw::c_int};

    const LOCK_EX: c_int = 2;
    const LOCK_NB: c_int = 4;

    extern "C" {
        fn flock(fd: c_int, operation: c_int) -> c_int;
    }

    pub(super) fn try_lock_exclusive(file: &File) -> io::Result<bool> {
        loop {
            if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
                return Ok(true);
            }
            let error = io::Error::last_os_error();
            match error.kind() {
                io::ErrorKind::WouldBlock => return Ok(false),
                io::ErrorKind::Interrupted => continue,
                _ => return Err(error),
            }
        }
    }

    pub(super) fn lock_exclusive(file: &File) -> io::Result<()> {
        loop {
            if unsafe { flock(file.as_raw_fd(), LOCK_EX) } == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

#[cfg(windows)]
mod platform_lock {
    use std::{
        ffi::c_void,
        fs::File,
        io,
        os::windows::io::AsRawHandle,
        ptr,
    };

    const LOCKFILE_FAIL_IMMEDIATELY: u32 = 0x0000_0001;
    const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x0000_0002;
    const ERROR_LOCK_VIOLATION: i32 = 33;

    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        h_event: *mut c_void,
    }

    #[link(name = "Kernel32")]
    extern "system" {
        fn LockFileEx(
            file: *mut c_void,
            flags: u32,
            reserved: u32,
            bytes_low: u32,
            bytes_high: u32,
            overlapped: *mut Overlapped,
        ) -> i32;
    }

    pub(super) fn try_lock_exclusive(file: &File) -> io::Result<bool> {
        match lock(file, true) {
            Ok(()) => Ok(true),
            Err(error) if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(super) fn lock_exclusive(file: &File) -> io::Result<()> {
        lock(file, false)
    }

    fn lock(file: &File, fail_immediately: bool) -> io::Result<()> {
        let mut overlapped = Overlapped {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            h_event: ptr::null_mut(),
        };
        let mut flags = LOCKFILE_EXCLUSIVE_LOCK;
        if fail_immediately {
            flags |= LOCKFILE_FAIL_IMMEDIATELY;
        }
        let result = unsafe {
            LockFileEx(
                file.as_raw_handle().cast(),
                flags,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> WorldId {
        WorldId([0x6c; 32])
    }

    #[test]
    fn stale_lock_file_is_recoverable_after_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let store_root = temp.path().join("store");
        let storage = Storage::open(&store_root).unwrap();
        let world_dir = storage.world_dir(world());
        fs::create_dir_all(&world_dir).unwrap();
        fs::write(gc_lock_path(&world_dir), b"stale pre-advisory lock\n").unwrap();
        drop(storage);

        let reopened = Storage::open(&store_root).unwrap();
        reopened.garbage_collect_blobs(world()).unwrap();
        let lease = reopened.pin_replication_hashes(world(), &[Hash32([7; 32])]).unwrap();
        drop(lease);
        reopened.garbage_collect_blobs(world()).unwrap();
        assert!(gc_lock_path(&world_dir).exists());
    }

    #[test]
    fn replication_and_snapshot_publication_pins_coexist_during_gc() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let replication = storage.put_blob(world(), b"replication-in-flight").unwrap();
        let local = storage.put_blob(world(), b"local-snapshot-in-flight").unwrap();
        let replication_lease = storage.pin_replication_hashes(world(), &[replication.hash]).unwrap();
        let local_pin = storage.pin_snapshot_publication_hash(world(), local.hash).unwrap();
        local_pin.persist();

        let report = storage.garbage_collect_blobs(world()).unwrap();
        assert_eq!(report.removed_blobs, 0);

        drop(replication_lease);
        let commit_guard = storage.lock_blob_gc_for_snapshot_commit(world()).unwrap();
        storage.release_snapshot_publication_pins(world(), [local.hash]).unwrap();
        drop(commit_guard);
        let report = storage.garbage_collect_blobs(world()).unwrap();
        assert_eq!(report.removed_blobs, 2);
    }

    #[test]
    fn live_lock_cannot_be_stolen_and_releases_on_handle_close() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let world_dir = storage.world_dir(world());
        let guard = acquire_gc_lock(&world_dir, world()).unwrap();

        assert!(matches!(
            storage.garbage_collect_blobs(world()),
            Err(RetentionError::GarbageCollectionActive(id)) if id == world()
        ));
        assert!(matches!(
            storage.pin_replication_hashes(world(), &[Hash32([8; 32])]),
            Err(RetentionError::GarbageCollectionActive(id)) if id == world()
        ));

        drop(guard);
        let lease = storage.pin_replication_hashes(world(), &[Hash32([8; 32])]).unwrap();
        drop(lease);
        storage.garbage_collect_blobs(world()).unwrap();
    }
}
