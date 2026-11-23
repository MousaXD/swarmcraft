use fs2::FileExt;
#[cfg(unix)]
use std::fs::File;
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
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
const MAX_PERMIT_BYTES: u64 = 128;
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
    let _lock = lock_permit_publication(&path)?;
    let floor_path = permit_floor_path(&path);
    let floor = read_saved_permit(&floor_path)?;
    // Upgrade from an earlier daemon that wrote only authority.permit. A
    // signed/canonical lease must still be checked by the caller before this
    // function; local counters alone NEVER authorize an authority.
    let visible = match read_saved_permit(&path) {
        Ok(visible) => visible,
        // A valid durable floor lets a healthy publisher repair corrupted
        // visible permit bytes. Readers remain fenced until the replacement.
        Err(error) if floor.is_some() && error.kind() == io::ErrorKind::InvalidData => None,
        Err(error) => return Err(error),
    };
    let requested_generation = (generation.epoch, generation.fencing_token);
    let mut next_heartbeat = heartbeat;
    for (previous_generation, previous_heartbeat) in [floor, visible].into_iter().flatten() {
        let previous_generation = (previous_generation.epoch, previous_generation.fencing_token);
        if previous_generation > requested_generation {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "stale authority generation cannot replace a newer permit floor",
            ));
        }
        if previous_generation == requested_generation {
            next_heartbeat =
                next_heartbeat.max(previous_heartbeat.checked_add(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "authority heartbeat floor exhausted")
                })?);
        }
    }
    let value = format!("{} {} {}\n", generation.epoch, generation.fencing_token, next_heartbeat);
    // Publish the durable floor BEFORE the visible permit. If either write
    // fails, a subsequent publisher must not reuse the previous counter.
    write_permit_atomically(&floor_path, value.as_bytes())?;
    write_permit_atomically(&path, value.as_bytes())
}

fn permit_floor_path(permit: &Path) -> PathBuf {
    permit.with_extension("permit.floor")
}

fn lock_permit_publication(permit: &Path) -> io::Result<fs::File> {
    let parent = permit.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "permit has no parent"))?;
    ensure_private_permit_directory(parent)?;
    let lock_path = parent.join("authority.permit.lock");
    reject_unsafe_entry(&lock_path, false)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(&lock_path)?;
    FileExt::lock_exclusive(&lock)?;
    reject_unsafe_entry(&lock_path, false)?;
    Ok(lock)
}

fn read_saved_permit(path: &Path) -> io::Result<Option<(AuthorityGeneration, u64)>> {
    let text = match read_private_permit(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    parse_permit(&text)
        .map(Some)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed authority heartbeat floor or permit"))
}

/// Fabric polls this file while Rust refreshes it. Never truncate the published
/// permit: a partial read would self-fence a healthy, current authority.
/// A failed publication leaves the preceding complete generation on disk, and
/// the ordinary heartbeat timeout still fails closed if no refresh succeeds.
fn write_permit_atomically(path: &Path, value: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "permit has no parent"))?;
    ensure_private_permit_directory(parent)?;
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

fn ensure_private_permit_directory(parent: &Path) -> io::Result<()> {
    // An attacker-controlled/symlinked control path must never redirect an
    // authority heartbeat into an unrelated location. Check both the world
    // control directory and its parent before allowing any publication.
    for directory in [parent.parent().and_then(Path::parent), parent.parent(), Some(parent)].into_iter().flatten() {
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
    Ok(())
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
                format!(
                    "unsafe writable or hard-linked authority permit path {} (mode {:o}, links {})",
                    path.display(),
                    metadata.permissions().mode() & 0o7777,
                    metadata.nlink()
                ),
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
    let _lock = lock_permit_publication(&path)?;
    // A fencing/consent revocation must not follow a substituted control
    // directory into an unrelated location or remove a link to another file.
    validate_permit_path(&path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_permit_path(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "permit has no parent"))?;
    for directory in [parent.parent().and_then(Path::parent), parent.parent(), Some(parent)].into_iter().flatten() {
        reject_unsafe_entry(directory, true)?;
    }
    reject_unsafe_entry(path, false)
}

fn read_private_permit(path: &Path) -> io::Result<String> {
    validate_permit_path(path)?;
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_PERMIT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "authority permit is not a bounded regular file"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PERMIT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_PERMIT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "authority permit exceeded its bounded size"));
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "authority permit is not valid UTF-8"))
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
        let value = match read_private_permit(&permit_path(paths, world)) {
            Ok(value) => value,
            Err(error) => {
                self.reset();
                return Err(error);
            }
        };
        let Some((generation, heartbeat)) = parse_permit(&value) else {
            self.reset();
            return Ok(false);
        };
        if generation != self.expected {
            self.reset();
            return Ok(false);
        }
        match self.last_heartbeat {
            None => self.last_heartbeat = Some(heartbeat),
            Some(previous) if heartbeat > previous => {
                self.last_heartbeat = Some(heartbeat);
                self.last_refresh = Some(now);
            }
            Some(previous) if heartbeat < previous => {
                // A rolled-back copy of an old heartbeat is not evidence of
                // ongoing majority lease renewal. Do not refresh or lower
                // the monotonic floor: only a strictly newer tick can.
                self.last_refresh = None;
            }
            Some(_) => {}
        }
        Ok(self.is_fresh(now))
    }

    fn reset(&mut self) {
        self.last_heartbeat = None;
        self.last_refresh = None;
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

    fn private_tempdir() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        // The agent runner uses umask 0002: Rust tempfile directories can
        // inherit mode 0775. An authority permit must reside under a private
        // application root, so give test fixtures the same security boundary.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        temp
    }

    #[test]
    fn frozen_permit_never_becomes_live() {
        let temp = private_tempdir();
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
        let temp = private_tempdir();
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
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([9; 32]);
        let expected = AuthorityGeneration { epoch: 2, fencing_token: 2 };
        refresh_permit(&paths, world, AuthorityGeneration { epoch: 3, fencing_token: 3 }, 1).unwrap();
        let mut watch = PermitWatch::new(expected);
        assert!(!watch.observe(&paths, world, Instant::now()).unwrap());
    }

    #[test]
    fn rolled_back_heartbeat_never_renews_a_fresh_lease() {
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([10; 32]);
        let generation = AuthorityGeneration { epoch: 8, fencing_token: 18 };
        let now = Instant::now();
        let mut watch = PermitWatch::new(generation);
        refresh_permit(&paths, world, generation, 100).unwrap();
        assert!(!watch.observe(&paths, world, now).unwrap());
        refresh_permit(&paths, world, generation, 101).unwrap();
        assert!(watch.observe(&paths, world, now + Duration::from_secs(1)).unwrap());

        // Simulate a stale copy written outside the canonical publisher. A
        // legitimate refresh_permit call never rolls its durable counter back.
        fs::write(permit_path(&paths, world), b"8 18 100\n").unwrap();
        assert!(!watch.observe(&paths, world, now + Duration::from_secs(2)).unwrap());
        assert!(!watch.is_fresh(now + Duration::from_secs(2)));
        // Repeated replay of a formerly fresh value must stay fenced.
        fs::write(permit_path(&paths, world), b"8 18 101\n").unwrap();
        assert!(!watch.observe(&paths, world, now + Duration::from_secs(3)).unwrap());
        refresh_permit(&paths, world, generation, 1).unwrap();
        assert!(watch.observe(&paths, world, now + Duration::from_secs(4)).unwrap());
    }

    #[test]
    fn daemon_restart_resumes_heartbeat_above_persistent_floor_after_clearing_visible_permit() {
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([18; 32]);
        let generation = AuthorityGeneration { epoch: 9, fencing_token: 32 };
        let now = Instant::now();
        let mut watch = PermitWatch::new(generation);
        refresh_permit(&paths, world, generation, 80).unwrap();
        assert!(!watch.observe(&paths, world, now).unwrap());
        refresh_permit(&paths, world, generation, 81).unwrap();
        assert!(watch.observe(&paths, world, now + Duration::from_secs(1)).unwrap());

        clear_permit(&paths, world).unwrap();
        assert!(!permit_path(&paths, world).exists());
        assert_eq!(read_saved_permit(&permit_floor_path(&permit_path(&paths, world))).unwrap(), Some((generation, 81)));

        // The daemon's in-memory HashMap resets its heartbeat to one. A Java
        // watchdog need not observe the brief deletion to accept the new
        // monotonic heartbeat after a same-generation daemon restart.
        let restarted = DataPaths::from_root(temp.path());
        refresh_permit(&restarted, world, generation, 1).unwrap();
        assert_eq!(read_saved_permit(&permit_path(&paths, world)).unwrap(), Some((generation, 82)));
        assert!(watch.observe(&paths, world, now + Duration::from_secs(2)).unwrap());
    }

    #[test]
    fn stale_generation_cannot_publish_after_newer_floor_even_when_permit_is_removed() {
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([19; 32]);
        let older = AuthorityGeneration { epoch: 4, fencing_token: 18 };
        let newer = AuthorityGeneration { epoch: 5, fencing_token: 19 };
        refresh_permit(&paths, world, older, 70).unwrap();
        refresh_permit(&paths, world, newer, 1).unwrap();
        clear_permit(&paths, world).unwrap();

        assert!(refresh_permit(&paths, world, older, 71).is_err());
        assert!(!permit_path(&paths, world).exists());
        refresh_permit(&paths, world, newer, 1).unwrap();
        assert_eq!(read_saved_permit(&permit_path(&paths, world)).unwrap(), Some((newer, 2)));
    }

    #[test]
    fn corrupt_or_exhausted_heartbeat_floor_never_resets_to_a_fresh_permit() {
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([20; 32]);
        let generation = AuthorityGeneration { epoch: 6, fencing_token: 22 };
        refresh_permit(&paths, world, generation, 42).unwrap();
        let visible_path = permit_path(&paths, world);
        let floor_path = permit_floor_path(&visible_path);

        fs::write(&floor_path, b"invalid floor").unwrap();
        assert!(refresh_permit(&paths, world, generation, 1).is_err());
        assert_eq!(read_saved_permit(&visible_path).unwrap(), Some((generation, 42)));

        fs::write(&floor_path, format!("6 22 {}\n", u64::MAX)).unwrap();
        assert!(refresh_permit(&paths, world, generation, 1).is_err());
        assert_eq!(read_saved_permit(&visible_path).unwrap(), Some((generation, 42)));
    }

    #[test]
    fn concurrent_publishers_serialize_heartbeat_updates() {
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([21; 32]);
        let generation = AuthorityGeneration { epoch: 14, fencing_token: 20 };
        let writers: Vec<_> = (0..4)
            .map(|_| {
                let paths = paths.clone();
                std::thread::spawn(move || {
                    for _ in 0..12 {
                        refresh_permit(&paths, world, generation, 1).unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(read_saved_permit(&permit_path(&paths, world)).unwrap(), Some((generation, 48)));
        assert_eq!(read_saved_permit(&permit_floor_path(&permit_path(&paths, world))).unwrap(), Some((generation, 48)));
    }

    #[test]
    fn unreadable_or_malformed_permit_retires_cached_freshness() {
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([11; 32]);
        let generation = AuthorityGeneration { epoch: 6, fencing_token: 16 };
        let now = Instant::now();
        let mut watch = PermitWatch::new(generation);
        refresh_permit(&paths, world, generation, 10).unwrap();
        assert!(!watch.observe(&paths, world, now).unwrap());
        refresh_permit(&paths, world, generation, 11).unwrap();
        assert!(watch.observe(&paths, world, now + Duration::from_secs(1)).unwrap());

        fs::write(permit_path(&paths, world), b"malformed permit").unwrap();
        assert!(!watch.observe(&paths, world, now + Duration::from_secs(2)).unwrap());
        assert!(!watch.is_fresh(now + Duration::from_secs(2)));
        refresh_permit(&paths, world, generation, 12).unwrap();
        assert!(!watch.observe(&paths, world, now + Duration::from_secs(3)).unwrap());
        refresh_permit(&paths, world, generation, 13).unwrap();
        assert!(watch.observe(&paths, world, now + Duration::from_secs(4)).unwrap());

        fs::write(permit_path(&paths, world), vec![b'1'; MAX_PERMIT_BYTES as usize + 1]).unwrap();
        assert!(watch.observe(&paths, world, now + Duration::from_secs(5)).is_err());
        assert!(!watch.is_fresh(now + Duration::from_secs(5)));
        clear_permit(&paths, world).unwrap();
        assert!(watch.observe(&paths, world, now + Duration::from_secs(6)).is_err());
        assert!(!watch.is_fresh(now + Duration::from_secs(6)));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_permit_cannot_be_observed_or_cleared_through_the_link() {
        use std::os::unix::fs::symlink;
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([12; 32]);
        let generation = AuthorityGeneration { epoch: 8, fencing_token: 16 };
        refresh_permit(&paths, world, generation, 1).unwrap();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"8 16 2\n").unwrap();
        let path = permit_path(&paths, world);
        fs::remove_file(&path).unwrap();
        symlink(&outside, &path).unwrap();

        let mut watch = PermitWatch::new(generation);
        assert!(watch.observe(&paths, world, Instant::now()).is_err());
        assert!(clear_permit(&paths, world).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"8 16 2\n");
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn writable_or_linked_control_paths_never_prove_fresh_authority() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = private_tempdir();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([17; 32]);
        let generation = AuthorityGeneration { epoch: 2, fencing_token: 5 };
        refresh_permit(&paths, world, generation, 1).unwrap();
        let path = permit_path(&paths, world);
        let world_control = path.parent().unwrap();
        let mut watch = PermitWatch::new(generation);

        fs::set_permissions(world_control, fs::Permissions::from_mode(0o770)).unwrap();
        assert!(watch.observe(&paths, world, Instant::now()).is_err());
        assert!(clear_permit(&paths, world).is_err());
        assert!(refresh_permit(&paths, world, generation, 2).is_err());
        fs::set_permissions(world_control, fs::Permissions::from_mode(0o700)).unwrap();

        let other = temp.path().join("hardlinked");
        fs::hard_link(&path, &other).unwrap();
        assert!(watch.observe(&paths, world, Instant::now()).is_err());
        assert!(clear_permit(&paths, world).is_err());
        fs::remove_file(other).unwrap();

        // A replaced world directory must not redirect a stop/refresh into
        // another folder, even if it contains a plausible changing permit.
        let elsewhere = temp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::rename(world_control, temp.path().join("moved-control")).unwrap();
        symlink(&elsewhere, world_control).unwrap();
        assert!(watch.observe(&paths, world, Instant::now()).is_err());
        assert!(clear_permit(&paths, world).is_err());
        assert!(!elsewhere.join("authority.permit").exists());
    }

    #[test]
    fn concurrent_reads_never_observe_a_torn_published_permit() {
        use std::sync::{atomic::AtomicBool, Arc, Barrier};
        let temp = private_tempdir();
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
        let temp = private_tempdir();
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
        let temp = private_tempdir();
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
