//! Local Minecraft listener policy for authority runtime launches.
//!
//! This is a loopback bind configuration, NOT a route or authority proof. A
//! remote player must still pass the live signed route, membership, quorum,
//! fencing, consent and Fabric IPC checks before its stream reaches Java.

use anyhow::{bail, Context, Result};
use std::{
    fs::OpenOptions,
    io::Write,
    net::{Ipv4Addr, SocketAddrV4, TcpListener},
    path::Path,
};
use swarm_core::DataPaths;
use swarm_network::game_port_registry::{GamePortRegistryError, GameWorldPortRegistry};
use swarm_protocol::WorldId;

// Intentionally separate from the user-facing saved gateway endpoint. One
// Minecraft server per machine can occupy a port, but different worlds must
// never share a durable server assignment or silently fall back to 25565.
const FIRST_MANAGED_SERVER_PORT: u16 = 40_000;
const MANAGED_SERVER_PORT_COUNT: u16 = 20_000;

fn registry(paths: &DataPaths) -> GameWorldPortRegistry {
    GameWorldPortRegistry::new(paths.root.join("managed-minecraft-ports"))
}

/// A saved assignment is a local configuration hint only. It is not proof that
/// a JVM has started or that the Fabric IPC session is live.
pub(super) fn recorded_port(paths: &DataPaths, world: WorldId) -> Result<Option<u16>> {
    let saved = registry(paths).lookup(world)?;
    match saved {
        Some(port) if !valid_managed_port(port) => {
            bail!("saved Minecraft server port is outside the managed loopback range")
        }
        other => Ok(other),
    }
}

/// Allocate a stable, per-world private port. Corrupt durable records and
/// already-established assignments that are busy fail closed: never silently
/// redirect a running or saved world to another port.
fn reserve_port(paths: &DataPaths, world: WorldId) -> Result<u16> {
    let ports = registry(paths);
    if let Some(port) = recorded_port(paths, world)? {
        ensure_port_free(port)?;
        return Ok(port);
    }
    let base = u16::from_be_bytes([world.0[0], world.0[1]]) % MANAGED_SERVER_PORT_COUNT;
    for offset in 0..MANAGED_SERVER_PORT_COUNT {
        let slot = (u32::from(base) + u32::from(offset)) % u32::from(MANAGED_SERVER_PORT_COUNT);
        let port = FIRST_MANAGED_SERVER_PORT + slot as u16;
        // Do not record an already occupied external service as a valid
        // Minecraft listener. Recheck immediately before spawning Java too.
        if TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).is_err() {
            continue;
        }
        match ports.reserve(world, port) {
            Ok(assigned) => {
                if !valid_managed_port(assigned) {
                    bail!("saved Minecraft server port is outside the managed loopback range");
                }
                ensure_port_free(assigned)?;
                return Ok(assigned);
            }
            Err(GamePortRegistryError::PortReservedForAnotherWorld) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!("no private managed Minecraft loopback port is available; refusing wildcard or shared bind")
}

fn valid_managed_port(port: u16) -> bool {
    (FIRST_MANAGED_SERVER_PORT..FIRST_MANAGED_SERVER_PORT + MANAGED_SERVER_PORT_COUNT).contains(&port)
}

/// Preflight is not a permanent port reservation. If another process races
/// Java for this port, Minecraft must fail startup rather than advertise a
/// different address. A fresh authenticated Fabric readiness check still has
/// to verify the actual bound listener.
pub(super) fn ensure_port_free(port: u16) -> Result<()> {
    if !valid_managed_port(port) {
        bail!("refusing invalid managed Minecraft server port {port}");
    }
    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).with_context(|| {
        format!("managed Minecraft loopback port {port} is occupied; refusing to reassign this world")
    })?;
    Ok(())
}

/// Write the only `server.properties` that a managed authority runtime may
/// use. The runtime directory was freshly reset by the guarded launch path.
/// Explicit online mode retains Mojang UUIDs/inventories across migrations.
/// The Minecraft allowlist is intentionally not configured from unverified
/// SwarmCraft peer identities; authentication is enforced by the game route,
/// and a separate Microsoft-UUID mapping is required before enforcing a
/// Minecraft whitelist in P1.
pub(super) fn prepare(paths: &DataPaths, world: WorldId, runtime: &Path) -> Result<u16> {
    let port = reserve_port(paths, world)?;
    write_properties(runtime, port)?;
    Ok(port)
}

fn write_properties(runtime: &Path, port: u16) -> Result<()> {
    if !valid_managed_port(port) {
        bail!("refusing unmanaged Minecraft server port");
    }
    let contents = format!(
        "# Managed by SwarmCraft. Keep Minecraft reachable only through the authenticated game gateway.\n\
         server-ip=127.0.0.1\n\
         server-port={port}\n\
         online-mode=true\n\
         enforce-secure-profile=true\n\
         enable-rcon=false\n\
         enable-query=false\n\
         white-list=false\n\
         enforce-whitelist=false\n"
    );
    let path = runtime.join("server.properties");
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).with_context(|| format!("cannot exclusively create {}", path.display()))?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let paths = DataPaths::from_root(dir.path());
        (dir, paths)
    }

    #[test]
    fn two_worlds_with_same_initial_slot_get_unique_durable_loopback_ports() {
        let (dir, paths) = setup();
        let first = WorldId([6; 32]);
        let mut second = WorldId([6; 32]);
        second.0[20] = 8;
        let one = prepare(&paths, first, dir.path()).unwrap();
        let text = fs::read_to_string(dir.path().join("server.properties")).unwrap();
        assert!(text.contains("server-ip=127.0.0.1\n"));
        assert!(text.contains(&format!("server-port={one}\n")));
        assert!(text.contains("online-mode=true\n"));
        assert!(text.contains("enable-rcon=false\n"));
        assert!(text.contains("enable-query=false\n"));
        assert!(text.contains("white-list=false\n"));
        fs::remove_file(dir.path().join("server.properties")).unwrap();
        let two = prepare(&paths, second, dir.path()).unwrap();
        assert_ne!(one, two);
        assert_eq!(recorded_port(&paths, first).unwrap(), Some(one));
        assert_eq!(recorded_port(&paths, second).unwrap(), Some(two));
    }

    #[test]
    fn saved_busy_port_is_refused_not_silently_reassigned() {
        let (dir, paths) = setup();
        let world = WorldId([13; 32]);
        let port = prepare(&paths, world, dir.path()).unwrap();
        let occupied = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).unwrap();
        fs::remove_file(dir.path().join("server.properties")).unwrap();
        assert!(prepare(&paths, world, dir.path()).is_err());
        assert_eq!(recorded_port(&paths, world).unwrap(), Some(port));
        drop(occupied);
        assert_eq!(prepare(&paths, world, dir.path()).unwrap(), port);
    }

    #[test]
    fn no_existing_properties_file_is_ever_truncated() {
        let (dir, _) = setup();
        let path = dir.path().join("server.properties");
        fs::write(&path, b"server-ip=0.0.0.0\n").unwrap();
        assert!(write_properties(dir.path(), 44_444).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"server-ip=0.0.0.0\n");
    }

    #[test]
    fn malformed_registry_does_not_get_reset_to_allocate_a_new_port() {
        let (dir, paths) = setup();
        let world = WorldId([15; 32]);
        prepare(&paths, world, dir.path()).unwrap();
        let path = paths.root.join("managed-minecraft-ports").join(format!("{}.port", world.to_hex()));
        fs::write(path, b"tampered").unwrap();
        assert!(recorded_port(&paths, world).is_err());
        assert!(reserve_port(&paths, WorldId([16; 32])).is_err());
    }
}
