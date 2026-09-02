use crate::{Storage, StorageError};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use swarm_protocol::WorldId;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Kernel-backed serialization guard for one world's durable control/head state.
///
/// The lock file is stable and intentionally never deleted. File locks are
/// released by the operating system when this handle is dropped or a process
/// exits, so a crashed writer cannot leave a permanently wedged logical lock.
#[derive(Debug)]
pub(crate) struct WorldTransactionGuard {
    _file: File,
}

impl Storage {
    pub(crate) fn lock_world_transaction(&self, world: WorldId) -> Result<WorldTransactionGuard, StorageError> {
        let metadata = self.world_dir(world).join("metadata");
        fs::create_dir_all(&metadata).map_err(|source| io_error(&metadata, source))?;
        let path = metadata.join(".storage-control.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| io_error(&path, source))?;
        FileExt::lock_exclusive(&file).map_err(|source| io_error(&path, source))?;
        Ok(WorldTransactionGuard { _file: file })
    }
}

pub(crate) fn durable_atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    let (temporary_path, mut temporary_file) = create_unique_temp(parent, "atomic", "tmp")?;
    temporary_file.write_all(bytes).map_err(|source| io_error(&temporary_path, source))?;
    temporary_file.sync_all().map_err(|source| io_error(&temporary_path, source))?;
    drop(temporary_file);

    #[cfg(unix)]
    {
        fs::rename(&temporary_path, path).map_err(|source| io_error(path, source))?;
    }
    #[cfg(not(unix))]
    {
        // Rust does not expose a single cross-platform replace-file primitive.
        // The higher-level canonical snapshot transaction persists an intent
        // record and validates head/manifest agreement on reopen so a Windows
        // namespace interruption is detected fail-closed rather than treated as
        // a successful canonical rollback. Control records are still serialized
        // by the per-world kernel lock and their file contents are synced first.
        match fs::remove_file(path) {
            Ok(()) => sync_parent(parent)?,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(path, source)),
        }
        fs::rename(&temporary_path, path).map_err(|source| io_error(path, source))?;
    }
    sync_parent(parent)
}

pub(crate) fn durable_create_once(path: &Path, bytes: &[u8]) -> Result<bool, StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    let mut file = match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(source) => return Err(io_error(path, source)),
    };
    if let Err(source) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(io_error(path, source));
    }
    drop(file);
    sync_parent(parent)?;
    Ok(true)
}

pub(crate) fn durable_remove(path: &Path) -> Result<bool, StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    match fs::remove_file(path) {
        Ok(()) => {
            sync_parent(parent)?;
            Ok(true)
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error(path, source)),
    }
}

pub(crate) fn create_unique_temp(
    parent: &Path,
    prefix: &str,
    extension: &str,
) -> Result<(PathBuf, File), StorageError> {
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    for _ in 0..256 {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{prefix}-{}-{counter}.{extension}", std::process::id()));
        match OpenOptions::new().create_new(true).read(true).write(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error(&path, source)),
        }
    }
    Err(io_error(
        parent,
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "unable to allocate unique transaction temporary file"),
    ))
}

pub(crate) fn sync_parent(parent: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        File::open(parent).and_then(|directory| directory.sync_all()).map_err(|source| io_error(parent, source))?;
    }
    #[cfg(not(unix))]
    {
        // Windows namespace durability is guarded at the canonical snapshot
        // transaction layer with an explicit durable intent/head consistency
        // protocol. A successful rename alone is never used as proof of a
        // canonical commit after reopen.
        let _ = parent;
    }
    Ok(())
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}
