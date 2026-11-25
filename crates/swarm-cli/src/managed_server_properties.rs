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

/// A successful TCP connect proves only that something answers at a port.
/// On Linux, additionally require that the actual supervised process owns
/// the exact private listener. Reject a foreign listener or wildcard bind,
/// including an IPv6 wildcard accepting IPv4 connections. Other platforms
/// retain the configured-address and authenticated Fabric checks; they do
/// not yet claim kernel-level listener ownership verification.
pub(super) fn verify_process_listener(pid: u32, port: u16) -> Result<()> {
    if pid == 0 || !valid_managed_port(port) {
        bail!("invalid managed Minecraft listener identity");
    }
    #[cfg(target_os = "linux")]
    linux_listener::verify(pid, port)?;
    #[cfg(not(target_os = "linux"))]
    let _ = (pid, port);
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux_listener {
    use anyhow::{bail, Context, Result};
    use std::{
        collections::HashSet,
        fs,
        io::{BufRead, BufReader},
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
        path::Path,
    };

    pub(super) fn verify(pid: u32, port: u16) -> Result<()> {
        let process = std::path::PathBuf::from(format!("/proc/{pid}"));
        let before = socket_inodes(&process)?;
        let mut listeners = Vec::new();
        read_listeners(&process.join("net/tcp"), false, port, &mut listeners)?;
        read_listeners(&process.join("net/tcp6"), true, port, &mut listeners)?;
        // Read ownership on both sides of the table observation. A socket
        // that disappeared or was newly substituted is not a live proof.
        let after = socket_inodes(&process)?;
        if listeners.is_empty() {
            bail!("managed Minecraft process has no listening socket on its reserved port");
        }
        for (address, inode) in listeners {
            if !private_ipv4_loopback(address) {
                bail!("managed Minecraft listener is not bound exclusively to IPv4 loopback");
            }
            if !before.contains(&inode) || !after.contains(&inode) {
                bail!("reserved Minecraft port has a listener not owned by the supervised process");
            }
        }
        Ok(())
    }

    fn socket_inodes(process: &Path) -> Result<HashSet<u64>> {
        let mut result = HashSet::new();
        for entry in fs::read_dir(process.join("fd")).context("cannot inspect managed process socket ownership")? {
            let entry = entry?;
            let target = match fs::read_link(entry.path()) {
                Ok(target) => target,
                // Java can close unrelated descriptors while being inspected.
                // The required listener must still appear in both snapshots.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error).context("cannot read managed process socket descriptor"),
            };
            if let Some(inode) = target
                .to_str()
                .and_then(|target| target.strip_prefix("socket:["))
                .and_then(|target| target.strip_suffix(']'))
            {
                result.insert(inode.parse().context("malformed kernel socket descriptor")?);
            }
        }
        Ok(result)
    }

    fn read_listeners(path: &Path, ipv6: bool, port: u16, result: &mut Vec<(IpAddr, u64)>) -> Result<()> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            // A kernel without IPv6 support may not expose a tcp6 table.
            Err(error) if ipv6 && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("cannot inspect managed process TCP listeners"),
        };
        for line in BufReader::new(file).lines().skip(1) {
            let line = line?;
            let fields: Vec<_> = line.split_ascii_whitespace().collect();
            if fields.len() < 10 {
                bail!("malformed kernel TCP socket row");
            }
            if fields[3] != "0A" {
                continue;
            }
            let (address, number) = fields[1].split_once(':').context("malformed kernel TCP address")?;
            if u16::from_str_radix(number, 16)? != port {
                continue;
            }
            let inode: u64 = fields[9].parse()?;
            if inode == 0 {
                bail!("kernel listener has no socket inode");
            }
            result.push((parse_address(address, ipv6)?, inode));
        }
        Ok(())
    }

    fn parse_address(value: &str, ipv6: bool) -> Result<IpAddr> {
        if !value.is_ascii() || value.len() != if ipv6 { 32 } else { 8 } {
            bail!("malformed kernel TCP address length");
        }
        // Linux presents each 32-bit word in native byte order.
        if !ipv6 {
            return Ok(Ipv4Addr::from(u32::from_str_radix(value, 16)?.to_ne_bytes()).into());
        }
        let mut bytes = [0u8; 16];
        for (index, part) in bytes.chunks_exact_mut(4).enumerate() {
            part.copy_from_slice(&u32::from_str_radix(&value[index * 8..index * 8 + 8], 16)?.to_ne_bytes());
        }
        Ok(Ipv6Addr::from(bytes).into())
    }

    fn private_ipv4_loopback(address: IpAddr) -> bool {
        match address {
            IpAddr::V4(ip) => ip == Ipv4Addr::LOCALHOST,
            // Java/Netty may use an IPv4-mapped IPv6 socket for an explicit
            // IPv4 bind. This is not an IPv6 wildcard or a separate address.
            IpAddr::V6(ip) => ip.to_ipv4_mapped() == Some(Ipv4Addr::LOCALHOST),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::net::TcpListener;

        #[test]
        fn actual_kernel_listener_requires_exact_address_and_ownership() {
            let private = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = private.local_addr().unwrap().port();
            verify(std::process::id(), port).unwrap();

            // A live child process has no ownership of the parent's socket.
            let mut other = std::process::Command::new("sleep").arg("10").spawn().unwrap();
            let result = verify(other.id(), port);
            other.kill().unwrap();
            other.wait().unwrap();
            assert!(result.is_err(), "foreign loopback listener was accepted");

            let wildcard = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
            assert!(verify(std::process::id(), wildcard.local_addr().unwrap().port()).is_err());
        }

        #[test]
        fn supervised_child_must_own_the_loopback_socket_not_just_answer_tcp() {
            use std::{
                io::{BufRead, BufReader},
                process::{Child, Command, Stdio},
            };

            struct Fixture(Child);
            impl Drop for Fixture {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
            // This is a real OS process/socket, not an installed JVM.
            // Keep the child alive until its own private stdin closes.
            for (address, accepted) in [("127.0.0.1", true), ("0.0.0.0", false)] {
                let mut child = Fixture(Command::new("python3")
                    .args(["-c",
                        "import socket,sys; s=socket.socket(); s.bind((sys.argv[1],0)); s.listen(); print(s.getsockname()[1],flush=True); sys.stdin.read()",
                        address,
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn().unwrap());
                let mut stdout = BufReader::new(child.0.stdout.take().unwrap());
                let mut line = String::new();
                stdout.read_line(&mut line).unwrap();
                let port: u16 = line.trim().parse().unwrap();
                // A plain TCP probe succeeds in BOTH cases.
                let tcp = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
                assert_eq!(verify(child.0.id(), port).is_ok(), accepted);
                assert!(verify(std::process::id(), port).is_err());
                drop(tcp);
            }
        }

        #[test]
        fn exact_ipv4_and_mapped_ipv6_are_private_but_wildcards_are_not() {
            for ip in ["127.0.0.1", "::ffff:127.0.0.1"] {
                assert!(private_ipv4_loopback(ip.parse().unwrap()));
            }
            for ip in ["0.0.0.0", "::", "::ffff:0.0.0.0", "::1", "127.0.0.2", "192.168.0.1"] {
                assert!(!private_ipv4_loopback(ip.parse().unwrap()));
            }
            let encoded_v4 = format!("{:08X}", u32::from_ne_bytes(Ipv4Addr::LOCALHOST.octets()));
            assert_eq!(parse_address(&encoded_v4, false).unwrap(), IpAddr::V4(Ipv4Addr::LOCALHOST));
            let mapped = Ipv4Addr::LOCALHOST.to_ipv6_mapped();
            let encoded_v6 = mapped
                .octets()
                .chunks_exact(4)
                .map(|part| format!("{:08X}", u32::from_ne_bytes(part.try_into().unwrap())))
                .collect::<String>();
            assert_eq!(parse_address(&encoded_v6, true).unwrap(), IpAddr::V6(mapped));
            assert!(parse_address("bad", true).is_err());
        }
    }
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
        // Another process can claim this unreserved port immediately after
        // release; that is not a failure of the saved-port refusal contract.
        // The persistent assignment must remain unchanged in either case.
        assert_eq!(recorded_port(&paths, world).unwrap(), Some(port));
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
