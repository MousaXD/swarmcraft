#![cfg(unix)]

use std::{
    fs,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_network::{
    load_or_create_transport_key, ConnectivityDiagnosticsV1, DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE,
};
use swarm_protocol::{EpochMode, WorldId};
use swarm_storage::Storage;
use tempfile::TempDir;

const WAIT_STEP: Duration = Duration::from_millis(200);

struct PeerFixture {
    _temp: TempDir,
    paths: DataPaths,
    storage: Storage,
    identity: PeerIdentity,
    port: u16,
    transport_peer: String,
}

struct ManagedChild(Child);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("0.0.0.0:0").unwrap().local_addr().unwrap().port()
}

fn peer_fixture() -> PeerFixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path().join("data"));
    let storage = Storage::open(paths.root.clone()).unwrap();
    let identity = PeerIdentity::load_or_create(&paths).unwrap();
    let transport_key = load_or_create_transport_key(&paths.transport_key()).unwrap();
    let transport_peer = transport_key.public().to_peer_id().to_string();
    PeerFixture { _temp: temp, paths, storage, identity, port: free_udp_port(), transport_peer }
}

fn local_non_loopback_ipv4() -> Ipv4Addr {
    let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    socket.connect("1.1.1.1:53").unwrap();
    match socket.local_addr().unwrap().ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() && !ip.is_link_local() => ip,
        other => panic!("test runner has no usable non-loopback IPv4 route: {other}"),
    }
}

fn automatic_transport_address(peer: &PeerFixture, ip: Ipv4Addr) -> String {
    format!("/ip4/{ip}/udp/{}/quic-v1/p2p/{}", peer.port, peer.transport_peer)
}

fn spawn_daemon(peer: &PeerFixture, listen_ip: Ipv4Addr) -> ManagedChild {
    let child = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/{listen_ip}/udp/{}/quic-v1", peer.port))
        .env("RUST_LOG", "info")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    ManagedChild(child)
}

fn run_cli(peer: &PeerFixture, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .args(args)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "CLI failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn wait_until(label: &str, timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(WAIT_STEP);
    }
    panic!("timed out waiting for {label}");
}

#[test]
fn automatic_invite_bootstrap_joins_and_replicates_without_manual_multiaddr() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();
    let local_ip = local_non_loopback_ipv4();
    assert_ne!(a.port, b.port);
    assert_ne!(a.port, c.port);
    assert_ne!(b.port, c.port);
    let a_address = automatic_transport_address(&a, local_ip);

    // Exercise the real create path rather than seeding authority metadata.
    let output = run_cli(
        &a,
        &["world", "create", "--name", "automatic-invite-join", "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"],
    );
    let world: WorldId = output
        .lines()
        .find_map(|line| line.strip_prefix("World ID: "))
        .expect("world create must return its signed world ID")
        .parse()
        .unwrap();
    let epoch = a.storage.load_epoch_record(world).unwrap();
    assert_eq!(epoch.mode, EpochMode::Quorum);
    let genesis_membership = a.storage.load_membership_record(world).unwrap();
    assert_eq!(genesis_membership.sequence, 0);
    assert_eq!(genesis_membership.epoch, epoch.epoch_number);
    let manifest = a.storage.latest_snapshot(world).unwrap().unwrap();
    let config = a.storage.load_world_config(world).unwrap();

    // Ordinary invite creation consumes diagnostics, not a manually supplied
    // --bootstrap argument. Seed the exact real LAN listener for this fixture.
    let diagnostics = ConnectivityDiagnosticsV1 { local_addresses: vec![a_address.clone()], ..Default::default() };
    let diagnostics_path = a.paths.root.join(DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE);
    fs::write(&diagnostics_path, serde_json::to_vec(&diagnostics).unwrap()).unwrap();
    let token = run_cli(&a, &["invite", "create", &world.to_string()]);
    assert!(token.starts_with("scinvite:"));
    assert!(!token.contains(&a_address));

    // Desktop daemons are normally already running before a user joins.
    let _daemon_a = spawn_daemon(&a, local_ip);
    let _daemon_b = spawn_daemon(&b, local_ip);
    let _daemon_c = spawn_daemon(&c, local_ip);
    thread::sleep(Duration::from_secs(1));

    assert!(run_cli(&b, &["world", "join", &token, "--host-candidate"]).contains("Join request staged"));
    let pending = b.storage.load_pending_join(world).unwrap();
    assert_eq!(pending.invite.bootstrap_addrs, vec![a_address]);
    assert!(pending.joining_member.authority_eligible);

    wait_until("first join, accepted epoch and verified replica", Duration::from_secs(30), || {
        let Ok(current) = a.storage.load_membership_record(world) else { return false };
        current.sequence == 1
            && b.storage.load_membership_record(world).ok().as_ref() == Some(&current)
            && b.storage.load_epoch_record(world).ok().as_ref() == Some(&epoch)
            && b.storage.load_world_config(world).ok().as_ref() == Some(&config)
            && b.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&manifest)
            && b.storage.verify_snapshot(&manifest).is_ok()
    });

    assert!(run_cli(&c, &["world", "join", &token]).contains("Join request staged"));
    assert!(!c.storage.load_pending_join(world).unwrap().joining_member.authority_eligible);
    wait_until("third peer receives full genesis history, epoch and replica", Duration::from_secs(30), || {
        let Ok(current) = a.storage.load_membership_record(world) else { return false };
        if current.sequence != 2 {
            return false;
        }
        [&b, &c].iter().all(|peer| {
            peer.storage.load_membership_record(world).ok().as_ref() == Some(&current)
                && peer.storage.load_epoch_record(world).ok().as_ref() == Some(&epoch)
                && peer.storage.load_world_config(world).ok().as_ref() == Some(&config)
                && peer.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&manifest)
                && peer.storage.verify_snapshot(&manifest).is_ok()
                && peer.storage.load_membership_certificate_chain(world).is_ok_and(|chain| chain.len() == 2)
        })
    });

    let committed = a.storage.load_membership_record(world).unwrap();
    assert_eq!(committed.members.len(), 3);
    assert!(committed.members.iter().any(|member| member.peer_id == b.identity.peer_id() && member.authority_eligible));
    assert!(committed
        .members
        .iter()
        .any(|member| member.peer_id == c.identity.peer_id() && !member.authority_eligible));
    for peer in [&a, &b, &c] {
        assert!(peer.storage.load_pending_join(world).is_err());
        assert!(swarm_cli::hosting_budget::load_consent(&peer.paths, world).unwrap().is_none());
        let proof = swarm_protocol::DiscoveryMembershipProofV1 {
            protocol_version: swarm_protocol::PROTOCOL_VERSION,
            world_id: world,
            genesis: peer.storage.load_world(world).unwrap().genesis,
            initial_membership: peer.storage.load_genesis_membership(world).unwrap(),
            current_membership: peer.storage.load_membership_record(world).unwrap(),
            membership_certificates: peer.storage.load_membership_certificate_chain(world).unwrap(),
            pending_membership: None,
        };
        swarm_cli::live_authority::verify_genesis_anchored_membership(&proof).unwrap();
    }
    assert!(run_cli(&c, &["world", "join", &token, "--host-candidate"]).contains("Already a signed member"));
    assert_eq!(c.storage.load_membership_record(world).unwrap(), committed);
    assert_eq!(c.storage.load_world_descriptor(world).unwrap().members, committed.members);
}
