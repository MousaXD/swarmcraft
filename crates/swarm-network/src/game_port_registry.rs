//! Durable per-world loopback Minecraft connection ports.
//!
//! Ports are immutable after the first successful reservation, so a gateway
//! restart, host migration or recovered world does not silently redirect a
//! player's saved Minecraft server entry. A collision fails closed rather than
//! changing another world's established endpoint. The *port* is not an
//! authorization credential; game_route and game_transport still independently
//! verify each join.

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use swarm_protocol::WorldId;
use thiserror::Error;

pub const MAX_REGISTERED_GAME_WORLDS: usize = 256;
pub const MIN_UNPRIVILEGED_GAME_PORT: u16 = 1024;
const MAX_PORT_RECORD_SIZE: usize = 128;
const PORT_REGISTRY_VERSION: u16 = 1;

#[derive(Debug, Error)]
pub enum GamePortRegistryError {
    #[error("world ID or requested local port is invalid")]
    InvalidAssignment,
    #[error("requested port is already assigned to a different world")]
    PortReservedForAnotherWorld,
    #[error("too many registered worlds")]
    CapacityExceeded,
    #[error("world-port registry is malformed or tampered with; manual recovery required")]
    CorruptRegistry,
    #[error("unable to read or persist the game gateway port: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct Assignment {
    version: u16,
    world_id: WorldId,
    port: u16,
}

/// Store under the app's durable PRIVATE per-user data path, never a tempdir,
/// migration status path, or an untrusted world save.
#[derive(Debug, Clone)]
pub struct GameWorldPortRegistry {
    directory: PathBuf,
}

impl GameWorldPortRegistry {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self { directory: directory.into() }
    }

    /// Assign once or return the world’s previously reserved stable port.
    /// The caller MUST bind TCP to 127.0.0.1 at the returned port and refuse
    /// startup if binding fails; a different free port is not an acceptable
    /// fallback because it would silently break the saved Minecraft entry.
    pub fn reserve(&self, world_id: WorldId, requested_port: u16) -> Result<u16, GamePortRegistryError> {
        check_assignment(world_id, requested_port)?;
        self.create_directory()?;
        let lock = self.open_lock()?;
        lock.lock_exclusive()?;

        let current = self.read_assignments()?;
        if let Some(assignment) = current.iter().find(|assignment| assignment.world_id == world_id) {
            return Ok(assignment.port);
        }
        if current.len() >= MAX_REGISTERED_GAME_WORLDS {
            return Err(GamePortRegistryError::CapacityExceeded);
        }
        if current.iter().any(|assignment| assignment.port == requested_port) {
            return Err(GamePortRegistryError::PortReservedForAnotherWorld);
        }

        let record = Assignment { version: PORT_REGISTRY_VERSION, world_id, port: requested_port };
        let bytes = postcard::to_allocvec(&record).map_err(|_| GamePortRegistryError::CorruptRegistry)?;
        let path = self.assignment_path(world_id);
        let mut file = private_options().write(true).create_new(true).open(&path)?;
        // Files are never overwritten. A crash leaving an incomplete file
        // becomes a fail-closed error instead of allocating a new port.
        file.write_all(&bytes)?;
        file.sync_all()?;
        sync_directory(&self.directory)?;
        Ok(requested_port)
    }

    /// Read a previously assigned port without allocating a new one.
    /// No authority or game-routing information can be inferred from this.
    pub fn lookup(&self, world_id: WorldId) -> Result<Option<u16>, GamePortRegistryError> {
        if world_id == WorldId::default() {
            return Err(GamePortRegistryError::InvalidAssignment);
        }
        self.create_directory()?;
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock)?;
        let assignments = self.read_assignments()?;
        Ok(assignments.iter().find(|assignment| assignment.world_id == world_id).map(|assignment| assignment.port))
    }

    fn assignment_path(&self, world_id: WorldId) -> PathBuf {
        self.directory.join(format!("{}.port", world_id.to_hex()))
    }

    fn create_directory(&self) -> Result<(), GamePortRegistryError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            builder.mode(0o700);
            builder.create(&self.directory)?;
            if fs::symlink_metadata(&self.directory)?.permissions().mode() & 0o022 != 0 {
                return Err(GamePortRegistryError::CorruptRegistry);
            }
        }
        #[cfg(not(unix))]
        fs::create_dir_all(&self.directory)?;
        if fs::symlink_metadata(&self.directory)?.file_type().is_symlink() {
            return Err(GamePortRegistryError::CorruptRegistry);
        }
        Ok(())
    }

    fn open_lock(&self) -> Result<File, GamePortRegistryError> {
        let path = self.directory.join("game-ports.lock");
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(GamePortRegistryError::CorruptRegistry);
        }
        private_options().read(true).write(true).create(true).truncate(false).open(path).map_err(Into::into)
    }

    /// Read *all* registered worlds to exclude duplicate port assignment,
    /// including corrupt/tampered records for unrelated worlds.
    fn read_assignments(&self) -> Result<Vec<Assignment>, GamePortRegistryError> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(GamePortRegistryError::CorruptRegistry);
            };
            if name == "game-ports.lock" {
                continue;
            }
            if !name.ends_with(".port")
                || name.len() != 64 + ".port".len()
                || !name.as_bytes()[..64].iter().all(u8::is_ascii_hexdigit)
                || entry.file_type()?.is_symlink()
            {
                return Err(GamePortRegistryError::CorruptRegistry);
            }
            if records.len() >= MAX_REGISTERED_GAME_WORLDS {
                return Err(GamePortRegistryError::CapacityExceeded);
            }
            let record = read_record(&entry.path())?;
            if record.world_id.to_hex().as_bytes() != &name.as_bytes()[..64]
                || records.iter().any(|prev: &Assignment| prev.world_id == record.world_id || prev.port == record.port)
            {
                return Err(GamePortRegistryError::CorruptRegistry);
            }
            records.push(record);
        }
        Ok(records)
    }
}

fn check_assignment(world_id: WorldId, port: u16) -> Result<(), GamePortRegistryError> {
    if world_id == WorldId::default() || port < MIN_UNPRIVILEGED_GAME_PORT {
        Err(GamePortRegistryError::InvalidAssignment)
    } else {
        Ok(())
    }
}

fn read_record(path: &Path) -> Result<Assignment, GamePortRegistryError> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(GamePortRegistryError::CorruptRegistry);
    }
    // Never read an unbounded local file before checking its size: a corrupt
    // or malicious record could otherwise exhaust memory before rejection.
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take((MAX_PORT_RECORD_SIZE + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_PORT_RECORD_SIZE {
        return Err(GamePortRegistryError::CorruptRegistry);
    }
    let (value, remainder): (Assignment, &[u8]) =
        postcard::take_from_bytes(&bytes).map_err(|_| GamePortRegistryError::CorruptRegistry)?;
    if !remainder.is_empty() {
        return Err(GamePortRegistryError::CorruptRegistry);
    }
    if value.version != PORT_REGISTRY_VERSION || check_assignment(value.world_id, value.port).is_err() {
        return Err(GamePortRegistryError::CorruptRegistry);
    }
    Ok(value)
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn sync_directory(path: &Path) -> Result<(), GamePortRegistryError> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn secure_test_dir() -> tempfile::TempDir {
        // tempfile uses default directory permissions under the process
        // umask; with umask=0o002 that can be group-writable (0o775).
        // Durable gateway configuration must instead be in a private path.
        let root = tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        root
    }

    #[test]
    fn port_remains_stable_across_restart_and_cannot_be_assigned_to_another_world() {
        let root = secure_test_dir();
        let dir = root.path().join("permanent");
        let first = GameWorldPortRegistry::new(&dir);
        let a = WorldId([1; 32]);
        let b = WorldId([2; 32]);
        assert_eq!(first.lookup(a).unwrap(), None);
        assert_eq!(first.reserve(a, 26565).unwrap(), 26565);
        assert_eq!(first.reserve(a, 26888).unwrap(), 26565);
        assert!(matches!(first.reserve(b, 26565), Err(GamePortRegistryError::PortReservedForAnotherWorld)));
        assert_eq!(first.reserve(b, 26566).unwrap(), 26566);

        // Simulate process death; no in-memory state is retained.
        let second = GameWorldPortRegistry::new(dir);
        assert_eq!(second.lookup(a).unwrap(), Some(26565));
        assert_eq!(second.reserve(a, 30000).unwrap(), 26565);
        assert_eq!(second.lookup(b).unwrap(), Some(26566));
    }

    #[test]
    fn malformed_records_block_reassignment_instead_of_resetting() {
        let root = secure_test_dir();
        let registry = GameWorldPortRegistry::new(root.path());
        let a = WorldId([3; 32]);
        let b = WorldId([4; 32]);
        registry.reserve(a, 35555).unwrap();
        let path = root.path().join(format!("{}.port", a.to_hex()));
        fs::write(path, b"truncated data").unwrap();
        assert!(matches!(registry.lookup(a), Err(GamePortRegistryError::CorruptRegistry)));
        assert!(matches!(registry.reserve(b, 35556), Err(GamePortRegistryError::CorruptRegistry)));
    }

    #[test]
    fn trailing_bytes_cannot_be_hidden_after_a_valid_world_port_record() {
        let root = secure_test_dir();
        let registry = GameWorldPortRegistry::new(root.path());
        let world = WorldId([7; 32]);
        registry.reserve(world, 35558).unwrap();
        let path = root.path().join(format!("{}.port", world.to_hex()));
        OpenOptions::new().append(true).open(path).unwrap().write_all(b"junk").unwrap();
        assert!(matches!(registry.lookup(world), Err(GamePortRegistryError::CorruptRegistry)));
    }

    #[test]
    fn oversized_record_is_read_with_a_hard_limit_and_refused() {
        let root = secure_test_dir();
        let registry = GameWorldPortRegistry::new(root.path());
        let world = WorldId([13; 32]);
        registry.reserve(world, 35560).unwrap();
        let path = root.path().join(format!("{}.port", world.to_hex()));
        fs::write(&path, vec![42_u8; 2 * 1024 * 1024]).unwrap();
        assert!(matches!(registry.lookup(world), Err(GamePortRegistryError::CorruptRegistry)));
        assert!(matches!(registry.reserve(WorldId([14; 32]), 35561), Err(GamePortRegistryError::CorruptRegistry)));
    }

    #[test]
    fn no_privileged_ports_or_default_world() {
        let root = secure_test_dir();
        let registry = GameWorldPortRegistry::new(root.path());
        assert!(matches!(registry.reserve(WorldId::default(), 25565), Err(GamePortRegistryError::InvalidAssignment)));
        assert!(matches!(registry.reserve(WorldId([1; 32]), 0), Err(GamePortRegistryError::InvalidAssignment)));
        assert!(matches!(registry.reserve(WorldId([1; 32]), 1023), Err(GamePortRegistryError::InvalidAssignment)));
        assert!(registry.reserve(WorldId([1; 32]), 1024).is_ok());
    }

    #[test]
    fn concurrent_process_style_reservation_is_single_assignment() {
        let root = secure_test_dir();
        let dir = root.path().to_path_buf();
        let a = WorldId([5; 32]);
        let mut handles = Vec::new();
        for port in [36666, 36667, 36668, 36669] {
            let dir = dir.clone();
            handles.push(std::thread::spawn(move || GameWorldPortRegistry::new(dir).reserve(a, port).unwrap()));
        }
        let results = handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>();
        assert!(results.iter().all(|port| *port == results[0]));
        assert_eq!(GameWorldPortRegistry::new(dir).lookup(a).unwrap(), Some(results[0]));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_record_and_directory_fail_closed() {
        use std::os::unix::fs::symlink;
        let root = tempdir().unwrap();
        let registry = GameWorldPortRegistry::new(root.path().join("ports"));
        let a = WorldId([9; 32]);
        registry.reserve(a, 39000).unwrap();
        let source = root.path().join("elsewhere");
        fs::write(&source, b"not a port").unwrap();
        let record_path = root.path().join("ports").join(format!("{}.port", a.to_hex()));
        fs::remove_file(&record_path).unwrap();
        symlink(source, record_path).unwrap();
        assert!(matches!(registry.lookup(a), Err(GamePortRegistryError::CorruptRegistry)));
        assert!(matches!(registry.reserve(WorldId([10; 32]), 39001), Err(GamePortRegistryError::CorruptRegistry)));
        let linked = GameWorldPortRegistry::new(root.path().join("linked"));
        symlink(root.path().join("ports"), root.path().join("linked")).unwrap();
        assert!(matches!(linked.reserve(WorldId([11; 32]), 39002), Err(GamePortRegistryError::CorruptRegistry)));
    }

    #[cfg(unix)]
    #[test]
    fn world_writable_port_registry_is_not_an_authoritative_local_endpoint() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempdir().unwrap();
        let public = root.path().join("public");
        fs::create_dir(&public).unwrap();
        fs::set_permissions(&public, fs::Permissions::from_mode(0o777)).unwrap();
        let registry = GameWorldPortRegistry::new(public);
        assert!(matches!(registry.reserve(WorldId([1; 32]), 40001), Err(GamePortRegistryError::CorruptRegistry)));
    }
}
