//! Device-local storage admission. No policy changes canonical history or
//! creates authority. A quota limits new payload bytes, not emergency metadata.
use crate::{
    transaction::{canonical_file_exists, durable_atomic_write, io_error, read_canonical_file},
    Storage, StorageError,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use swarm_protocol::WorldId;
use walkdir::WalkDir;

pub const MIN_FREE_RESERVE_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_FREE_RESERVE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldStoragePolicy {
    /// Includes all files beneath this local world directory, including
    /// resumable partials and isolated replay staging. None = no local cap.
    pub max_world_bytes: Option<u64>,
    pub reserve_free_bytes: u64,
}

impl Default for WorldStoragePolicy {
    fn default() -> Self {
        Self { max_world_bytes: None, reserve_free_bytes: DEFAULT_FREE_RESERVE_BYTES }
    }
}

impl WorldStoragePolicy {
    fn validate(self) -> Result<Self, StorageError> {
        if self.reserve_free_bytes < MIN_FREE_RESERVE_BYTES {
            return Err(StorageError::StoragePressure {
                detail: format!("free-space reserve must be at least {MIN_FREE_RESERVE_BYTES} bytes"),
            });
        }
        Ok(self)
    }
}

#[derive(Debug, Serialize)]
pub struct WorldStorageStatus {
    pub world_id: String,
    pub policy: WorldStoragePolicy,
    pub world_bytes: u64,
    pub filesystem_available_bytes: u64,
    pub additional_payload_bytes: u64,
    pub payload_admission_paused: bool,
    pub retention: &'static str,
    pub remediation: &'static str,
}

pub(crate) struct BlobWriteBudget {
    _lock: File,
    directory: PathBuf,
    reserve_free_bytes: u64,
    quota_remaining: Option<u64>,
}

impl BlobWriteBudget {
    pub(crate) fn require(&self, additional: u64) -> Result<(), StorageError> {
        self.require_after(0, additional)
    }

    fn require_after(&self, written: u64, additional: u64) -> Result<(), StorageError> {
        if additional == 0 {
            return Ok(());
        }
        if let Some(remaining) = self.quota_remaining {
            if additional > remaining.saturating_sub(written) {
                return Err(StorageError::StoragePressure {
                    detail: format!(
                        "world storage quota: {additional} additional bytes requested, {} remain; use world storage-status, reclaim unreferenced blobs or raise the local quota",
                        remaining.saturating_sub(written)
                    ),
                });
            }
        }
        require_free_space(&self.directory, self.reserve_free_bytes, additional)
    }

    pub(crate) fn writer(&self, file: File) -> BudgetedWriter<'_> {
        BudgetedWriter { file, budget: self, written: 0 }
    }
}

pub(crate) struct BudgetedWriter<'a> {
    file: File,
    budget: &'a BlobWriteBudget,
    written: u64,
}

impl BudgetedWriter<'_> {
    pub(crate) fn into_file(self) -> File {
        self.file
    }
}

impl Write for BudgetedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.budget.require_after(self.written, bytes.len() as u64).map_err(io::Error::other)?;
        let written = self.file.write(bytes)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Storage {
    fn storage_policy_path(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("metadata/local-storage-policy.json")
    }

    pub fn world_storage_policy(&self, world: WorldId) -> Result<WorldStoragePolicy, StorageError> {
        let path = self.storage_policy_path(world);
        if !canonical_file_exists(&path)? {
            return Ok(WorldStoragePolicy::default());
        }
        if fs::metadata(&path).map_err(|error| io_error(&path, error))?.len() > 4096 {
            return Err(StorageError::WorldMetadataMismatch);
        }
        serde_json::from_slice::<WorldStoragePolicy>(&read_canonical_file(&path)?)?.validate()
    }

    fn lock_payload_writes(&self, world: WorldId, wait: bool) -> Result<File, StorageError> {
        let directory = self.world_dir(world);
        fs::create_dir_all(&directory).map_err(|error| io_error(&directory, error))?;
        let path = directory.join(".payload-write.lock");
        // The stable lock is never deleted. A symlink/non-file fails closed.
        canonical_file_exists(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| io_error(&path, error))?;
        if wait {
            FileExt::lock_exclusive(&file).map_err(|error| io_error(&path, error))?;
        } else if let Err(error) = FileExt::try_lock_exclusive(&file) {
            if error.kind() == io::ErrorKind::WouldBlock
                || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
            {
                return Err(StorageError::StoragePressure {
                    detail: "another local payload writer owns the storage admission lock; retry".into(),
                });
            }
            return Err(io_error(&path, error));
        }
        Ok(file)
    }

    pub fn set_world_storage_policy(&self, world: WorldId, policy: WorldStoragePolicy) -> Result<(), StorageError> {
        let policy = policy.validate()?;
        self.load_world(world)?;
        let _lock = self.lock_payload_writes(world, true)?;
        durable_atomic_write(&self.storage_policy_path(world), &serde_json::to_vec(&policy)?)
    }

    pub fn world_storage_status(&self, world: WorldId) -> Result<WorldStorageStatus, StorageError> {
        self.load_world(world)?;
        let _lock = self.lock_payload_writes(world, true)?;
        let directory = self.world_dir(world);
        let policy = self.world_storage_policy(world)?;
        let world_bytes = directory_bytes(&directory)?;
        let filesystem_available_bytes =
            fs2::available_space(&directory).map_err(|error| io_error(&directory, error))?;
        let additional_payload_bytes = filesystem_available_bytes
            .saturating_sub(policy.reserve_free_bytes)
            .min(policy.max_world_bytes.map_or(u64::MAX, |limit| limit.saturating_sub(world_bytes)));
        Ok(WorldStorageStatus {
            world_id: world.to_string(),
            policy,
            world_bytes,
            filesystem_available_bytes,
            additional_payload_bytes,
            payload_admission_paused: additional_payload_bytes == 0,
            retention: "preserve_all_committed_checkpoints",
            remediation: "Free space outside the store, run world storage-cleanup --confirm for unreferenced blobs, or raise the local quota. Never delete signed history or runtime recovery data.",
        })
    }

    /// All canonical payload writers share this per-world OS lock. Stale
    /// replay additionally holds the canonical world's lock while writing into
    /// its isolated nested store. No caller may acquire GC or canonical-world
    /// transaction locks while holding this budget.
    pub(crate) fn begin_blob_write(&self, world: WorldId) -> Result<BlobWriteBudget, StorageError> {
        self.blob_write_budget(world, true)
    }

    /// Network handlers must not wait behind a long local compression job.
    pub(crate) fn try_begin_blob_write(&self, world: WorldId) -> Result<BlobWriteBudget, StorageError> {
        self.blob_write_budget(world, false)
    }

    fn blob_write_budget(&self, world: WorldId, wait: bool) -> Result<BlobWriteBudget, StorageError> {
        let lock = self.lock_payload_writes(world, wait)?;
        let directory = self.world_dir(world);
        let policy = self.world_storage_policy(world)?;
        let quota_remaining = policy
            .max_world_bytes
            .map(|limit| directory_bytes(&directory).map(|used| limit.saturating_sub(used)))
            .transpose()?;
        Ok(BlobWriteBudget { _lock: lock, directory, reserve_free_bytes: policy.reserve_free_bytes, quota_remaining })
    }

    pub fn require_payload_space(&self, world: WorldId, additional: u64) -> Result<(), StorageError> {
        self.begin_blob_write(world)?.require(additional)
    }

    /// Nonblocking preflight for network schedulers. Actual writes must still
    /// acquire admission again; this is not a disk reservation.
    pub fn try_require_payload_space(&self, world: WorldId, additional: u64) -> Result<(), StorageError> {
        self.try_begin_blob_write(world)?.require(additional)
    }

    /// Restore may target a different filesystem. Check the actual existing
    /// ancestor, before touching the destination or retiring a runtime.
    pub fn require_restore_space(
        &self,
        world: WorldId,
        destination: &Path,
        additional: u64,
    ) -> Result<(), StorageError> {
        let policy = self.world_storage_policy(world)?;
        let mut existing = destination;
        while !existing.exists() {
            existing = existing.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        }
        require_free_space(existing, policy.reserve_free_bytes, additional.max(1))
    }
}

fn require_free_space(directory: &Path, reserve: u64, additional: u64) -> Result<(), StorageError> {
    let available = fs2::available_space(directory).map_err(|error| io_error(directory, error))?;
    if additional > available.saturating_sub(reserve) {
        return Err(StorageError::StoragePressure {
            detail: format!(
                "filesystem space: {available} bytes available, {reserve} reserved, {additional} requested; free disk space before retrying; use world storage-status and storage-cleanup --confirm"
            ),
        });
    }
    Ok(())
}

fn directory_bytes(directory: &Path) -> Result<u64, StorageError> {
    let mut total = 0u64;
    for entry in WalkDir::new(directory).follow_links(false) {
        let entry =
            entry.map_err(|error| io_error(error.path().unwrap_or(directory), io::Error::other(error.to_string())))?;
        if entry.file_type().is_symlink() {
            return Err(StorageError::SymlinkUnsupported(entry.path().to_path_buf()));
        }
        if entry.file_type().is_file() {
            let len =
                entry.metadata().map_err(|error| io_error(entry.path(), io::Error::other(error.to_string())))?.len();
            total = total.checked_add(len).ok_or(StorageError::CounterExhausted("world storage bytes"))?;
        }
    }
    Ok(total)
}
