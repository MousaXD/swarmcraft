//! One local Linux account-wide JVM reservation, independent of application
//! data roots, HOME/XDG variables, and replica-only profiles. A durable
//! process record complements the kernel lock when the controller crashes.
//! No claim of cross-account or cross-container coordination is made.
use anyhow::Result;
use std::path::PathBuf;

pub(crate) struct MachineHostingReservation {
    pub(crate) record_path: PathBuf,
    #[cfg(target_os = "linux")]
    _lock: std::fs::File,
}

impl MachineHostingReservation {
    pub(crate) fn acquire() -> Result<Option<Self>> {
        #[cfg(target_os = "linux")]
        {
            let home = linux::account_home()?;
            // A shared/network home cannot establish a machine-local
            // reservation. Fail closed rather than silently using XDG/HOME
            // or a different temporary lock that other launches may miss.
            linux::require_local_persistent_fs(&home)?;
            let directory = home.join(".swarmcraft-machine-hosting-v1");
            linux::validate_ancestors(&directory)?;
            swarm_core::create_private_dir_all(&directory)?;
            linux::validate_ancestors(&directory)?;
            linux::require_local_persistent_fs(&directory)?;
            Self::acquire_at(&directory).map(Some)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(None)
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn acquire_at(directory: &std::path::Path) -> Result<Self> {
        use anyhow::{ensure, Context};
        use fs2::FileExt;
        use std::{
            fs::{self, OpenOptions},
            os::unix::fs::{OpenOptionsExt, PermissionsExt},
        };
        linux::validate_ancestors(directory)?;
        swarm_core::create_private_dir_all(directory)?;
        linux::validate_ancestors(directory)?;
        ensure!(
            fs::metadata(directory)?.permissions().mode() & 0o077 == 0,
            "account-wide hosting reservation directory must be private (0700)"
        );
        let lock_path = directory.join("reservation.lock");
        linux::validate_file(&lock_path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&lock_path)
            .context("cannot open account-wide hosting reservation")?;
        linux::validate_file(&lock_path)?;
        FileExt::try_lock_exclusive(&lock).context(
            "another Minecraft hosting reservation is active for this Linux account (possibly another data root)",
        )?;
        let record_path = directory.join("runtime-process.json");
        // A crashed owner can leave its Java child alive even though the
        // file lock was released. Never use the lock alone as capacity proof.
        crate::runtime_process_guard::clear_quiescent_record(&record_path)
            .context("account-wide hosting reservation has unresolved previous runtime ownership")?;
        Ok(Self { record_path, _lock: lock })
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use anyhow::{bail, ensure, Context, Result};
    use std::{
        ffi::{CStr, CString, OsStr},
        fs,
        os::unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
        },
        path::{Component, Path, PathBuf},
    };

    pub(crate) fn uid() -> u32 {
        // SAFETY: geteuid has no memory or lifetime preconditions.
        unsafe { libc::geteuid() }
    }

    pub(super) fn account_home() -> Result<PathBuf> {
        // Do not let two supported launches bypass the common lock by
        // supplying different HOME or XDG_* environment variables.
        let mut size = 16 * 1024;
        loop {
            let mut buffer = vec![0u8; size];
            let mut entry = std::mem::MaybeUninit::<libc::passwd>::zeroed();
            let mut result = std::ptr::null_mut();
            // SAFETY: the buffer, passwd and result slots live until return.
            let code = unsafe {
                libc::getpwuid_r(uid(), entry.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut result)
            };
            if code == libc::ERANGE && size < 1024 * 1024 {
                size *= 2;
                continue;
            }
            if code != 0 {
                return Err(std::io::Error::from_raw_os_error(code)).context("cannot resolve Linux account home");
            }
            ensure!(!result.is_null(), "Linux account has no home record");
            // SAFETY: successful getpwuid_r populated this passwd and its
            // pointer refers to a NUL-terminated string in the live buffer.
            let entry = unsafe { entry.assume_init() };
            ensure!(!entry.pw_dir.is_null(), "Linux account home is unavailable");
            let value = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes();
            let path = PathBuf::from(OsStr::from_bytes(value));
            ensure!(
                path.is_absolute() && path != Path::new("/"),
                "Linux account home is not a dedicated absolute path"
            );
            validate_ancestors(&path)?;
            let meta = fs::metadata(&path).context("Linux account home unavailable")?;
            ensure!(meta.is_dir() && meta.uid() == uid(), "Linux account home ownership cannot be established");
            return Ok(path);
        }
    }

    pub(super) fn require_local_persistent_fs(path: &Path) -> Result<()> {
        let bytes = CString::new(path.as_os_str().as_bytes())?;
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
        // SAFETY: NUL-terminated path and valid output storage.
        let result = unsafe { libc::statfs(bytes.as_ptr(), stat.as_mut_ptr()) };
        if result != 0 {
            return Err(std::io::Error::last_os_error()).context("cannot identify account hosting filesystem");
        }
        let kind = unsafe { stat.assume_init() }.f_type as u64 & 0xffff_ffff;
        // Explicit known persistent local filesystems. A tmpfs, remote
        // filesystem, unknown mount, or overlay may not share the same
        // durable reservation with all application launches.
        ensure!(
            matches!(
                kind,
                0xEF53 | 0x58465342 | 0x9123683E | 0x2FC12FC1 | 0xF2F52010 | 0x3153464A | 0x52654973 | 0x24051905
            ),
            "account-wide hosting requires a known persistent local filesystem"
        );
        Ok(())
    }

    pub(crate) fn validate_ancestors(path: &Path) -> Result<()> {
        let absolute = if path.is_absolute() { path.to_owned() } else { std::env::current_dir()?.join(path) };
        ensure!(
            !absolute.components().any(|part| part == Component::ParentDir),
            "unsafe parent traversal in hosting state"
        );
        let mut current = PathBuf::new();
        for part in absolute.components() {
            current.push(part.as_os_str());
            let metadata = match fs::symlink_metadata(&current) {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(error).context("cannot inspect hosting state ancestry"),
            };
            let mode = metadata.permissions().mode();
            let owner = metadata.uid();
            let root_sticky = owner == 0 && mode & 0o1000 != 0;
            ensure!(
                metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && (owner == 0 || owner == uid())
                    && (mode & 0o022 == 0 || root_sticky),
                "unsafe hosting state ancestor {}",
                current.display()
            );
        }
        Ok(())
    }

    pub(crate) fn validate_file(path: &Path) -> Result<bool> {
        let parent = path.parent().context("hosting record has no parent directory")?;
        validate_ancestors(parent)?;
        let metadata = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("cannot inspect hosting process record"),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != uid()
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o022 != 0
        {
            bail!("untrusted hosting process record {}", path.display());
        }
        Ok(true)
    }
}
