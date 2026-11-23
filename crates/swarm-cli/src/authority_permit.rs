#[cfg(unix)]
use std::fs::File;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use swarm_consensus::AuthorityGeneration;
use swarm_core::DataPaths;
use swarm_protocol::WorldId;

pub const PERMIT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
pub const PERMIT_TIMEOUT: Duration = Duration::from_secs(6);
pub const PERMIT_START_TIMEOUT: Duration = Duration::from_secs(15);
static PERMIT_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn permit_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("control").join(world.to_hex()).join("authority.permit")
}

pub fn refresh_permit(
    paths: &DataPaths,
    world: WorldId,
    generation: AuthorityGeneration,
    heartbeat: u64,
) -> io::Result<()> {
    let path = permit_path(paths, world);
    let value = format!("{} {} {}\n", generation.epoch, generation.fencing_token, heartbeat);
    write_permit_atomically(&path, value.as_bytes())
}

/// Fabric polls this file while Rust refreshes it. Never truncate the published
/// permit: a partial read would self-fence a healthy, current authority.
/// A failed publication leaves the preceding complete generation on disk, and
/// the ordinary heartbeat timeout still fails closed if no refresh succeeds.
fn write_permit_atomically(path: &Path, value: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "permit has no parent"))?;
    // An attacker-controlled/symlinked control path must never redirect an
    // authority heartbeat into an unrelated location. Check both the world
    // control directory and its parent before allowing any publication.
    for directory in [parent.parent(), Some(parent)].into_iter().flatten() {
        reject_unsafe_entry(directory, true)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(parent)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(parent)?;
    reject_unsafe_entry(parent, true)?;
    reject_unsafe_entry(path, false)?;
    let sequence = PERMIT_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!("authority.permit.{}.{}.tmp", std::process::id(), sequence));
    let outcome = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(value)?;
        file.sync_all()?;
        drop(file);
        replace_permit_atomically(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    outcome
}

fn reject_unsafe_entry(path: &Path, expect_directory: bool) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink()
        || (expect_directory && !metadata.is_dir())
        || (!expect_directory && !metadata.is_file())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe authority permit path is not a regular file or directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.permissions().mode() & 0o022 != 0 || (!expect_directory && metadata.nlink() != 1) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe writable or hard-linked authority permit path",
            ));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_permit_atomically(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

/// `std::fs::rename` cannot replace an existing destination reliably on
/// Windows. Never delete the published permit before moving its replacement.
#[cfg(windows)]
fn replace_permit_atomically(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
    }
    let old = temporary.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    let new = destination.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    // SAFETY: both UTF-16 buffers are NUL-terminated and live for the call.
    let result = unsafe { MoveFileExW(old.as_ptr(), new.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn clear_permit(paths: &DataPaths, world: WorldId) -> io::Result<()> {
    let path = permit_path(paths, world);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[derive(Debug)]
pub struct PermitWatch {
    expected: AuthorityGeneration,
    last_heartbeat: Option<u64>,
    last_refresh: Option<Instant>,
}

impl PermitWatch {
    pub fn new(expected: AuthorityGeneration) -> Self {
        Self { expected, last_heartbeat: None, last_refresh: None }
    }

    pub fn observe(&mut self, paths: &DataPaths, world: WorldId, now: Instant) -> io::Result<bool> {
        let value = fs::read_to_string(permit_path(paths, world))?;
        let Some((generation, heartbeat)) = parse_permit(&value) else {
            return Ok(false);
        };
        if generation != self.expected {
            return Ok(false);
        }
        match self.last_heartbeat {
            None => self.last_heartbeat = Some(heartbeat),
            Some(previous) if previous != heartbeat => {
                self.last_heartbeat = Some(heartbeat);
                self.last_refresh = Some(now);
            }
            Some(_) => {}
        }
        Ok(self.is_fresh(now))
    }

    pub fn is_fresh(&self, now: Instant) -> bool {
        self.last_refresh.is_some_and(|refresh| now.duration_since(refresh) < PERMIT_TIMEOUT)
    }
}

fn parse_permit(value: &str) -> Option<(AuthorityGeneration, u64)> {
    let mut fields = value.split_whitespace();
    let epoch = fields.next()?.parse().ok()?;
    let fencing_token = fields.next()?.parse().ok()?;
    let heartbeat = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some((AuthorityGeneration { epoch, fencing_token }, heartbeat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_permit_never_becomes_live() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([7; 32]);
        let generation = AuthorityGeneration { epoch: 4, fencing_token: 9 };
        refresh_permit(&paths, world, generation, 1).unwrap();
        let start = Instant::now();
        let mut watch = PermitWatch::new(generation);
        assert!(!watch.observe(&paths, world, start).unwrap());
        assert!(!watch.observe(&paths, world, start + Duration::from_secs(1)).unwrap());
    }

    #[test]
    fn changing_exact_generation_becomes_live_then_expires() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([8; 32]);
        let generation = AuthorityGeneration { epoch: 11, fencing_token: 22 };
        let start = Instant::now();
        let mut watch = PermitWatch::new(generation);
        refresh_permit(&paths, world, generation, 1).unwrap();
        assert!(!watch.observe(&paths, world, start).unwrap());
        refresh_permit(&paths, world, generation, 2).unwrap();
        assert!(watch.observe(&paths, world, start + Duration::from_secs(1)).unwrap());
        assert!(!watch.is_fresh(start + Duration::from_secs(8)));
    }

    #[test]
    fn different_generation_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([9; 32]);
        let expected = AuthorityGeneration { epoch: 2, fencing_token: 2 };
        refresh_permit(&paths, world, AuthorityGeneration { epoch: 3, fencing_token: 3 }, 1).unwrap();
        let mut watch = PermitWatch::new(expected);
        assert!(!watch.observe(&paths, world, Instant::now()).unwrap());
    }

    #[test]
    fn concurrent_reads_never_observe_a_torn_published_permit() {
        use std::sync::{atomic::AtomicBool, Arc, Barrier};
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([13; 32]);
        let generation = AuthorityGeneration { epoch: 25, fencing_token: 38 };
        refresh_permit(&paths, world, generation, 0).unwrap();
        let path = permit_path(&paths, world);
        let running = Arc::new(AtomicBool::new(true));
        let barrier = Arc::new(Barrier::new(2));
        let reader_running = Arc::clone(&running);
        let reader_barrier = Arc::clone(&barrier);
        let reader = std::thread::spawn(move || {
            reader_barrier.wait();
            while reader_running.load(Ordering::Relaxed) {
                let value = fs::read_to_string(&path).unwrap();
                let (observed, _) = parse_permit(&value).expect("reader saw a truncated permit");
                assert_eq!(observed, generation);
                std::thread::yield_now();
            }
        });
        barrier.wait();
        for heartbeat in 1..=100 {
            refresh_permit(&paths, world, generation, heartbeat).unwrap();
        }
        running.store(false, Ordering::Relaxed);
        reader.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_existing_permit_symlink_fails_closed() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([14; 32]);
        let outside = temp.path().join("outside");
        fs::write(&outside, b"never edit me").unwrap();
        let path = permit_path(&paths, world);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(refresh_permit(&paths, world, AuthorityGeneration { epoch: 1, fencing_token: 2 }, 3).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"never edit me");
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_world_control_directory_fails_closed() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([15; 32]);
        let outside = temp.path().join("elsewhere");
        fs::create_dir(&outside).unwrap();
        let parent = permit_path(&paths, world).parent().unwrap().to_path_buf();
        fs::create_dir_all(parent.parent().unwrap()).unwrap();
        symlink(&outside, parent).unwrap();
        assert!(refresh_permit(&paths, world, AuthorityGeneration { epoch: 1, fencing_token: 2 }, 3).is_err());
        assert!(!outside.join("authority.permit").exists());
    }
}
