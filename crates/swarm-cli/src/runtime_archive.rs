//! Local recovery of an unconfirmed runtime, never canonical history.
//! A durable journal replaces the pending checkpoint marker before rename;
//! that same marker continues to fence all supported runtime launch paths.
use super::*;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeArchiveResult {
    pub world_id: String,
    pub archive_id: String,
    pub archive_path: PathBuf,
    pub world_save_path: PathBuf,
    pub unconfirmed_save: bool,
}

pub fn archive_unconfirmed_runtime(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    confirm_unconfirmed_data: bool,
) -> Result<RuntimeArchiveResult> {
    if !confirm_unconfirmed_data {
        bail!("archive requires explicit confirmation that retained runtime bytes are NOT a verified canonical checkpoint");
    }
    #[cfg(not(unix))]
    {
        let _ = (paths, storage, world);
        bail!("durable unconfirmed-runtime archival is not supported on this platform; keep the retained files and pending marker");
    }
    #[cfg(unix)]
    {
        storage.load_world(world)?;
        // Hold the same runtime lock used by launch and historical replay.
        // A durable child identity also denies orphaned or ambiguous runtimes.
        #[cfg(target_os = "linux")]
        crate::machine_hosting::linux::validate_ancestors(&control_dir(paths, world))?;
        let _slot = AuthorityRuntimeGuard::acquire(paths, world)?;
        // The foreground stop and transfer publishers use this same lock.
        // Do not retire a local request concurrently with its publication.
        let _requests = lock_control_file(&control_dir(paths, world).join("transfer-request.lock"))?;
        crate::runtime_process_guard::refuse_unaccounted_world_runtime(paths, world)?;
        hosting_budget::with_player_only_recovery(paths, world, || {
            crate::runtime_process_guard::refuse_unaccounted_world_runtime(paths, world)?;
            archive_locked(paths, world, ArchiveFaults::default())
        })
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ArchiveJournal {
    archive_schema_version: u16,
    archive_id: String,
    checkpoint: StopIntentV1,
    runtime_device: u64,
    runtime_inode: u64,
}

#[cfg(unix)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ArchiveReceipt {
    journal: ArchiveJournal,
    unconfirmed_save: bool,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, Default)]
struct ArchiveFaults {
    #[cfg(test)]
    after_rename: bool,
    #[cfg(test)]
    after_receipt: bool,
}

#[cfg(unix)]
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.nlink() != 1
        || metadata.len() == 0
        || metadata.len() > 8192
    {
        bail!("untrusted or oversized recovery record {}", path.display());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(8193).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 8192 {
        bail!("invalid recovery record size");
    }
    Ok(bytes)
}

#[cfg(unix)]
fn require_directory(path: &Path) -> Result<fs::Metadata> {
    let meta =
        fs::symlink_metadata(path).with_context(|| format!("cannot inspect recovery directory {}", path.display()))?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        bail!("recovery directory is not a real directory: {}", path.display());
    }
    Ok(meta)
}

#[cfg(unix)]
fn existing_directory(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(Some(meta)),
        Ok(_) => bail!("unsafe recovery directory {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all().with_context(|| format!("cannot sync recovery directory {}", path.display()))
}

#[cfg(unix)]
fn prepare_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let parent = path.parent().context("recovery directory has no parent")?;
    require_directory(parent)?;
    if existing_directory(path)?.is_none() {
        // Deliberately create one level at a time. Never follow an existing
        // symlink or replace an existing archive directory.
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let meta = require_directory(path)?;
    #[cfg(target_os = "linux")]
    crate::machine_hosting::linux::validate_ancestors(path)?;
    if meta.permissions().mode() & 0o077 != 0 {
        bail!("archive directory must be private (0700): {}", path.display());
    }
    sync_directory(path)?;
    sync_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn validate_journal(journal: &ArchiveJournal, world: WorldId) -> Result<()> {
    if journal.archive_schema_version != 1
        || journal.checkpoint.schema_version != STOP_INTENT_SCHEMA_VERSION
        || journal.checkpoint.world_id != world
        || journal.archive_id.len() != 64
        || !journal.archive_id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || journal.runtime_inode == 0
    {
        bail!("invalid unconfirmed-runtime archive journal");
    }
    Ok(())
}

/// Atomic, same-filesystem directory rename with a kernel no-replace
/// guarantee on supported Unix desktop platforms. Never fall back to copy
/// and delete or a replacement-capable rename on an unsupported filesystem.
#[cfg(target_os = "linux")]
fn rename_runtime_no_replace(source: &Path, target: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // SAFETY: both NUL-terminated paths are live for the syscall.
    let result = unsafe {
        libc::renameat2(libc::AT_FDCWD, source.as_ptr(), libc::AT_FDCWD, target.as_ptr(), libc::RENAME_NOREPLACE)
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("atomic no-replace archive rename failed");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn rename_runtime_no_replace(source: &Path, target: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    // Darwin <stdio.h>: RENAME_EXCL prevents replacement of the target.
    const RENAME_EXCL: u32 = 0x0000_0004;
    extern "C" {
        fn renamex_np(source: *const std::ffi::c_char, target: *const std::ffi::c_char, flags: u32) -> i32;
    }
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // SAFETY: both NUL-terminated paths are live for the syscall.
    let result = unsafe { renamex_np(source.as_ptr(), target.as_ptr(), RENAME_EXCL) };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("atomic no-replace archive rename failed");
    }
    Ok(())
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn rename_runtime_no_replace(_source: &Path, _target: &Path) -> Result<()> {
    bail!("atomic no-replace runtime archive rename is not implemented on this platform")
}

#[cfg(unix)]
fn require_exact_runtime(meta: &fs::Metadata, journal: &ArchiveJournal) -> Result<()> {
    if meta.dev() != journal.runtime_device || meta.ino() != journal.runtime_inode {
        bail!("retained runtime directory identity changed; recovery denied");
    }
    Ok(())
}

/// Sync all bytes before publishing the directory move. Refuse special
/// files, symlinks, hardlinks and other mounted filesystems, rather than
/// following links outside the retained runtime. The session lock is already
/// open and held; opening and closing it again would release POSIX record locks.
#[cfg(unix)]
fn sync_runtime_tree(path: &Path, device: u64, lock_path: &Path) -> Result<()> {
    let meta = require_directory(path)?;
    if meta.dev() != device {
        bail!("retained runtime contains a separate filesystem");
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.dev() != device || meta.file_type().is_symlink() {
            bail!("retained runtime contains an unsafe link or mount: {}", path.display());
        }
        if meta.is_dir() {
            sync_runtime_tree(&path, device, lock_path)?;
        } else if meta.is_file() && meta.nlink() == 1 {
            if path != lock_path {
                fs::File::open(&path)?
                    .sync_all()
                    .with_context(|| format!("cannot sync retained file {}", path.display()))?;
            }
        } else {
            bail!("retained runtime contains a special or multiply-linked file: {}", path.display());
        }
    }
    sync_directory(path)
}

#[cfg(unix)]
fn archive_locked(paths: &DataPaths, world: WorldId, faults: ArchiveFaults) -> Result<RuntimeArchiveResult> {
    // Canonicalize the application root once, then reject links in the
    // managed control/runtime/archive subdirectories. No user-provided
    // destination or cross-filesystem copy/delete path exists.
    #[cfg(not(test))]
    let _ = faults;
    let root = fs::canonicalize(&paths.root)?;
    let paths = DataPaths::from_root(root.clone());
    #[cfg(target_os = "linux")]
    crate::machine_hosting::linux::validate_ancestors(&root)?;
    require_directory(&root.join("control"))?;
    require_directory(&control_dir(&paths, world))?;
    let marker = shutdown_checkpoint_path(&paths, world);
    let runtime_parent = root.join("runtime");
    require_directory(&runtime_parent)?;
    let runtime = runtime_parent.join(world.to_hex());
    let bytes = read_bounded(&marker).context("no trustworthy pending shutdown checkpoint to archive")?;
    let journal = if let Ok(journal) = serde_json::from_slice::<ArchiveJournal>(&bytes) {
        validate_journal(&journal, world)?;
        journal
    } else {
        let checkpoint: StopIntentV1 =
            serde_json::from_slice(&bytes).context("pending shutdown checkpoint record is malformed")?;
        let meta = require_directory(&runtime)?;
        let journal = ArchiveJournal {
            archive_schema_version: 1,
            archive_id: hex::encode(swarm_core::random_nonce()),
            checkpoint,
            runtime_device: meta.dev(),
            runtime_inode: meta.ino(),
        };
        validate_journal(&journal, world)?;
        // The original marker never disappears; its atomically replaced
        // contents now record the single resumable archive transaction.
        atomic_json(&marker, &journal)?;
        journal
    };

    let archives = root.join("runtime-archives");
    prepare_directory(&archives)?;
    let world_archives = archives.join(world.to_hex());
    prepare_directory(&world_archives)?;
    let destination = world_archives.join(&journal.archive_id);
    prepare_directory(&destination)?;
    let retained = destination.join("runtime");
    let receipt_path = destination.join("receipt.json");

    let source_exists = existing_directory(&runtime)?;
    let retained_exists = existing_directory(&retained)?;
    let active_path = match (&source_exists, &retained_exists) {
        (Some(meta), None) => {
            require_exact_runtime(meta, &journal)?;
            // No previous receipt may exist before the directory moved.
            match fs::symlink_metadata(&receipt_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => bail!("unexpected archive receipt before runtime move"),
            }
            &runtime
        }
        (None, Some(meta)) => {
            require_exact_runtime(meta, &journal)?;
            &retained
        }
        _ => bail!("ambiguous archive transaction: exactly one runtime directory must exist"),
    };

    if require_directory(&destination)?.dev() != journal.runtime_device {
        bail!("archive destination is not on the retained runtime filesystem; no copy/delete fallback");
    }
    let save = active_path.join("world");
    require_directory(&save)?;
    let session_lock = save.join("session.lock");
    let lock_meta = fs::symlink_metadata(&session_lock)?;
    if !lock_meta.is_file() || lock_meta.file_type().is_symlink() || lock_meta.nlink() != 1 {
        bail!("unsafe Minecraft session.lock in retained runtime");
    }
    let _session = crate::world_import::acquire_archive_quiescence(&save)?;
    _session.sync_lock_file()?;
    sync_runtime_tree(active_path, journal.runtime_device, &session_lock)?;

    // Retire cached readiness before removing anything from the active
    // path. Signed metadata, authority records and snapshot bytes are not
    // touched by this operator operation.
    host_readiness::invalidate_runtime_verification(&paths, world)?;
    host_readiness::invalidate_cached_readiness(&paths, world)?;
    if source_exists.is_some() {
        // Private destination and exclusive authority slot serialize all
        // cooperating writers. Never rename over an existing retained save.
        if fs::symlink_metadata(&retained).is_ok() {
            bail!("retained archive destination already exists");
        }
        rename_runtime_no_replace(&runtime, &retained)?;
    }
    require_exact_runtime(&require_directory(&retained)?, &journal)?;
    sync_directory(&retained)?;
    sync_directory(&runtime_parent)?;
    sync_directory(&destination)?;
    #[cfg(test)]
    if faults.after_rename {
        bail!("injected interruption after durable runtime rename");
    }

    let receipt = ArchiveReceipt { journal: journal.clone(), unconfirmed_save: true };
    match fs::symlink_metadata(&receipt_path) {
        Ok(_) => {
            let existing: ArchiveReceipt = serde_json::from_slice(&read_bounded(&receipt_path)?)?;
            if existing != receipt {
                bail!("existing recovery receipt does not describe this archive");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => atomic_json(&receipt_path, &receipt)?,
        Err(error) => return Err(error.into()),
    }
    sync_directory(&destination)?;
    #[cfg(test)]
    if faults.after_receipt {
        bail!("injected interruption after durable archive receipt");
    }
    // The archived runtime is now separately durable and is not reachable
    // from the launch reset path. Only local pending commands are retired.
    clear_stop_intent(&paths, world)?;
    clear_transfer_intent(&paths, world)?;
    clear_wake_intent(&paths, world)?;
    // Keep the old marker until all preceding steps were confirmed.
    clear_shutdown_checkpoint(&paths, world)?;

    Ok(RuntimeArchiveResult {
        world_id: world.to_string(),
        archive_id: journal.archive_id,
        archive_path: retained.clone(),
        world_save_path: retained.join("world"),
        unconfirmed_save: true,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, DataPaths, WorldId, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(fs::canonicalize(temp.path()).unwrap());
        let world = WorldId([44; 32]);
        let runtime = paths.root.join("runtime").join(world.to_hex());
        swarm_core::create_private_dir_all(&runtime.join("world/region")).unwrap();
        fs::write(runtime.join("world/session.lock"), b"").unwrap();
        fs::write(runtime.join("world/level.dat"), b"unconfirmed-save").unwrap();
        fs::write(runtime.join("world/region/r.0.0.mca"), b"region").unwrap();
        fs::write(runtime.join("server.properties"), b"server-ip=127.0.0.1").unwrap();
        atomic_json(
            &shutdown_checkpoint_path(&paths, world),
            &StopIntentV1 {
                schema_version: STOP_INTENT_SCHEMA_VERSION,
                world_id: world,
                epoch: 2,
                fencing_token: 5,
                authority_peer_id: PeerId([77; 32]),
            },
        )
        .unwrap();
        (temp, paths, world, runtime)
    }

    #[test]
    fn archive_resumes_after_rename_and_receipt_interruption_without_second_copy() {
        let (_temp, paths, world, runtime) = fixture();
        let original_inode = fs::metadata(&runtime).unwrap().ino();
        let error =
            archive_locked(&paths, world, ArchiveFaults { after_rename: true, ..Default::default() }).unwrap_err();
        assert!(error.to_string().contains("injected interruption"));
        assert!(!runtime.exists());
        assert!(shutdown_checkpoint_pending(&paths, world).unwrap());
        let journal: ArchiveJournal =
            serde_json::from_slice(&fs::read(shutdown_checkpoint_path(&paths, world)).unwrap()).unwrap();
        let dest = paths.root.join("runtime-archives").join(world.to_hex()).join(&journal.archive_id);
        assert_eq!(fs::metadata(dest.join("runtime")).unwrap().ino(), original_inode);
        let error =
            archive_locked(&paths, world, ArchiveFaults { after_receipt: true, ..Default::default() }).unwrap_err();
        assert!(error.to_string().contains("injected interruption"));
        assert!(shutdown_checkpoint_pending(&paths, world).unwrap());
        let result = archive_locked(&paths, world, ArchiveFaults::default()).unwrap();
        assert_eq!(result.archive_id, journal.archive_id);
        assert!(result.unconfirmed_save);
        assert_eq!(fs::read(result.world_save_path.join("level.dat")).unwrap(), b"unconfirmed-save");
        assert_eq!(fs::read(result.archive_path.join("server.properties")).unwrap(), b"server-ip=127.0.0.1");
        assert!(!shutdown_checkpoint_pending(&paths, world).unwrap());
        assert!(!runtime.exists());
        assert!(archive_locked(&paths, world, ArchiveFaults::default()).is_err());
        assert!(dest.join("receipt.json").exists());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn kernel_rename_never_replaces_an_existing_archive_directory() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let destination = temp.path().join("destination");
        fs::create_dir(&old).unwrap();
        fs::create_dir(&destination).unwrap();
        let destination_inode = fs::metadata(&destination).unwrap().ino();
        fs::write(old.join("save"), b"original").unwrap();
        assert!(rename_runtime_no_replace(&old, &destination).is_err());
        assert_eq!(fs::read(old.join("save")).unwrap(), b"original");
        assert_eq!(fs::metadata(&destination).unwrap().ino(), destination_inode);
    }

    #[test]
    fn unsafe_links_and_conflicting_runtime_paths_keep_pending_marker() {
        use std::os::unix::fs::symlink;
        let (_temp, paths, world, runtime) = fixture();
        symlink("/etc/passwd", runtime.join("world/unsafe")).unwrap();
        assert!(archive_locked(&paths, world, ArchiveFaults::default()).is_err());
        assert!(runtime.join("world/level.dat").exists());
        assert!(shutdown_checkpoint_pending(&paths, world).unwrap());
        fs::remove_file(runtime.join("world/unsafe")).unwrap();
        archive_locked(&paths, world, ArchiveFaults { after_rename: true, ..Default::default() }).unwrap_err();
        fs::create_dir(&runtime).unwrap();
        assert!(archive_locked(&paths, world, ArchiveFaults::default()).unwrap_err().to_string().contains("ambiguous"));
        assert!(shutdown_checkpoint_pending(&paths, world).unwrap());
    }
}
