//! Conservative snapshot retention and blob garbage collection.
//!
//! Retention is mark-before-sweep. Snapshot manifests are the primary roots,
//! canonical/recovery control records add mandatory roots, and active replication
//! pins keep not-yet-committed blobs alive. Unknown files and partial transfers are
//! never reclaimed by this module.

use crate::{Storage, StorageError};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
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
        for path in &self.pin_paths {
            if let Err(error) = fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!(path = %path.display(), %error, "failed to remove replication GC pin");
                }
            }
        }
        if let Some(parent) = self.pin_paths.first().and_then(|path| path.parent()) {
            let _ = sync_dir(parent);
        }
    }
}

#[derive(Debug)]
struct GcLock {
    path: PathBuf,
}

impl Drop for GcLock {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %self.path.display(), %error, "failed to remove blob GC lock");
            }
        }
        if let Some(parent) = self.path.parent() {
            let _ = sync_dir(parent);
        }
    }
}

impl Storage {
    /// Pins blobs that an active reconstruction may publish into the complete
    /// blob namespace before its snapshot manifest is committed.
    ///
    /// A GC lock is checked before and after pin creation. If GC wins the race,
    /// the caller receives an error and must retry later; transfer must not start.
    pub fn pin_replication_hashes(
        &self,
        world: WorldId,
        hashes: &[Hash32],
    ) -> Result<ActiveReplicationLease, RetentionError> {
        let world_dir = self.world_dir(world);
        let lock_path = gc_lock_path(&world_dir);
        if lock_path.exists() {
            return Err(RetentionError::GarbageCollectionActive(world));
        }

        let pins_dir = replication_pins_dir(&world_dir);
        fs::create_dir_all(&pins_dir).map_err(|source| io_error(&pins_dir, source))?;
        let token = PIN_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut unique = BTreeSet::new();
        let mut lease = ActiveReplicationLease { pin_paths: Vec::new() };

        for hash in hashes.iter().copied().filter(|hash| unique.insert(*hash)) {
            let path = pins_dir.join(format!(
                "{}-{token}-{}.pin",
                std::process::id(),
                hash.to_hex()
            ));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .map_err(|source| io_error(&path, source))?;
            // Track immediately so any later write/sync error is cleaned up by Drop.
            lease.pin_paths.push(path.clone());
            file.write_all(hash.to_hex().as_bytes()).map_err(|source| io_error(&path, source))?;
            file.sync_all().map_err(|source| io_error(&path, source))?;
        }
        sync_dir(&pins_dir)?;

        // Close the check/create race with GC. GC creates its lock with
        // create_new before reading pins. If it appeared after our first check,
        // returning the error drops the lease and removes every pin.
        if lock_path.exists() {
            return Err(RetentionError::GarbageCollectionActive(world));
        }

        Ok(lease)
    }

    /// Removes snapshot manifests outside the retention set. No blobs are
    /// deleted here, so interruption can only retain extra data.
    pub fn prune_snapshots(
        &self,
        world: WorldId,
        policy: &RetentionPolicy,
    ) -> Result<RetentionReport, RetentionError> {
        let snapshots = self.list_snapshots(world)?;
        if snapshots.is_empty() {
            return Ok(RetentionReport::default());
        }

        let retained = retention_roots(self, world, &snapshots, policy)?;
        let snapshots_dir = self.world_dir(world).join("snapshots");
        let mut report = RetentionReport {
            retained_snapshots: retained.iter().copied().collect(),
            ..Default::default()
        };

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
    /// currently committed snapshot and are not pinned by active replication.
    ///
    /// `.part`, temporary, unknown, and malformed files are ignored.
    pub fn garbage_collect_blobs(&self, world: WorldId) -> Result<RetentionReport, RetentionError> {
        let world_dir = self.world_dir(world);
        let _lock = acquire_gc_lock(&world_dir, world)?;

        // Read roots only after the exclusive GC lock exists. A replication
        // attempt that races us must observe the lock on its second check and
        // abort before writing complete blobs.
        let snapshots = self.list_snapshots(world)?;
        let mut live = referenced_blob_hashes(&snapshots);
        live.extend(read_replication_pins(&world_dir)?);

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
    pub fn apply_retention(
        &self,
        world: WorldId,
        policy: &RetentionPolicy,
    ) -> Result<RetentionReport, RetentionError> {
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
    snapshots
        .iter()
        .flat_map(|manifest| manifest.entries.iter().map(|entry| entry.blob.hash))
        .collect()
}

fn acquire_gc_lock(world_dir: &Path, world: WorldId) -> Result<GcLock, RetentionError> {
    fs::create_dir_all(world_dir).map_err(|source| io_error(world_dir, source))?;
    let path = gc_lock_path(world_dir);
    match OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(mut file) => {
            file.write_all(b"swarmcraft blob gc\n").map_err(|source| io_error(&path, source))?;
            file.sync_all().map_err(|source| io_error(&path, source))?;
            sync_dir(world_dir)?;
            Ok(GcLock { path })
        }
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(RetentionError::GarbageCollectionActive(world))
        }
        Err(source) => Err(io_error(&path, source).into()),
    }
}

fn read_replication_pins(world_dir: &Path) -> Result<BTreeSet<Hash32>, RetentionError> {
    let pins_dir = replication_pins_dir(world_dir);
    if !pins_dir.exists() {
        return Ok(BTreeSet::new());
    }
    let mut hashes = BTreeSet::new();
    for entry in fs::read_dir(&pins_dir).map_err(|source| io_error(&pins_dir, source))? {
        let entry = entry.map_err(|source| io_error(&pins_dir, source))?;
        if !entry.file_type().map_err(|source| io_error(entry.path(), source))?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("pin") {
            continue;
        }
        let bytes = fs::read(&path).map_err(|source| io_error(&path, source))?;
        let Ok(value) = std::str::from_utf8(&bytes) else {
            // A malformed pin is conservative: abort GC rather than guess.
            return Err(io_error(
                &path,
                std::io::Error::new(std::io::ErrorKind::InvalidData, "replication pin is not UTF-8"),
            )
            .into());
        };
        let hash = Hash32::from_str(value.trim()).map_err(StorageError::from)?;
        hashes.insert(hash);
    }
    Ok(hashes)
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
