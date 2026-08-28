#![cfg(unix)]

use std::{
    fs,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_core::{create_world_genesis, DataPaths, PeerIdentity};
use swarm_network::{
    load_or_create_transport_key, ConnectivityDiagnosticsV1, DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE,
};
use swarm_protocol::{
    EpochMode, EpochRecordV1, MembershipRecordV1, WorldDescriptorV1, WorldMemberV1, PROTOCOL_VERSION,
    STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, WorldMetadataV1};
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
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
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

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

#[test]
fn automatic_invite_bootstrap_joins_and_replicates_without_manual_multiaddr() {
    let a = peer_fixture();
    let b = peer_fixture();
    let local_ip = local_non_loopback_ipv4();
    let a_address = automatic_transport_address(&a, local_ip);

    let (world, genesis) =
        create_world_genesis(&a.identity, "26.1.2".into(), "0.19.3".into(), b"automatic-invite-join").unwrap();
    let metadata = WorldMetadataV1 {
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        display_name: "automatic-invite-join".into(),
        world_id: world,
        genesis: genesis.clone(),
    };
    a.storage.create_world(&metadata).unwrap();

    let descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        compatibility_fingerprint: genesis.compatibility_fingerprint,
        members: vec![member(&a.identity)],
        preferred_replication_factor: 2,
    };
    a.storage.save_world_descriptor(&descriptor).unwrap();

    let mut membership = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch: 1,
        sequence: 1,
        previous_membership_hash: None,
        members: descriptor.members.clone(),
        authority_peer_id: a.identity.peer_id(),
        authority_public_key: a.identity.public_key(),
        signature: Vec::new(),
    };
    a.identity.sign_membership(&mut membership).unwrap();
    a.storage.save_membership_record(&membership).unwrap();

    let source_temp = tempfile::tempdir().unwrap();
    let source = source_temp.path().join("world");
    fs::create_dir_all(source.join("region")).unwrap();
    fs::write(source.join("level.dat"), b"automatic-invite-level\n").unwrap();
    fs::write(source.join("region/r.0.0.mca"), b"automatic-invite-region\n").unwrap();
    let mut manifest = a
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 1,
                sequence: 7,
                previous_snapshot_hash: None,
                authority_peer_id: a.identity.peer_id(),
                authority_public_key: a.identity.public_key(),
            },
        )
        .unwrap();
    a.identity.sign_snapshot(&mut manifest).unwrap();
    a.storage.commit_snapshot(&manifest).unwrap();

    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 1,
        previous_epoch_hash: None,
        base_state_hash: manifest.state_root,
        authority_peer_id: a.identity.peer_id(),
        authority_public_key: a.identity.public_key(),
        mode: EpochMode::Solo,
        fencing_token: 1,
        reason: "automatic invite acceptance seed".into(),
        signature: Vec::new(),
    };
    epoch.signature = a.identity.sign(&epoch.signing_bytes().unwrap());
    a.storage.save_epoch_record(&epoch).unwrap();

    // Seed the exact backend diagnostics snapshot that ordinary invite creation
    // consumes. No --bootstrap argument is supplied below.
    let diagnostics = ConnectivityDiagnosticsV1 { local_addresses: vec![a_address.clone()], ..Default::default() };
    let diagnostics_path = a.paths.root.join(DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE);
    fs::create_dir_all(&a.paths.root).unwrap();
    fs::write(&diagnostics_path, serde_json::to_vec(&diagnostics).unwrap()).unwrap();

    let token = run_cli(&a, &["invite", "create", &world.to_string()]);
    assert!(token.starts_with("scinvite:"));
    assert!(!token.contains(&a_address), "multiaddresses must remain inside the signed token encoding");

    let join_output = run_cli(&b, &["world", "join", &token]);
    assert!(join_output.contains("Join request staged"));
    let pending = b.storage.load_pending_join(world).unwrap();
    assert_eq!(pending.invite.bootstrap_addrs, vec![a_address]);

    // The fixture advertises this concrete non-loopback address in the diagnostics
    // snapshot, so its QUIC listener must bind the same address. A wildcard QUIC
    // listener is not self-dialable through the advertised interface on macOS.
    let _daemon_a = spawn_daemon(&a, local_ip);
    thread::sleep(Duration::from_secs(1));
    let _daemon_b = spawn_daemon(&b, Ipv4Addr::LOCALHOST);

    wait_until("canonical membership on both peers", Duration::from_secs(20), || {
        let Ok(a_membership) = a.storage.load_membership_record(world) else { return false };
        let Ok(b_membership) = b.storage.load_membership_record(world) else { return false };
        a_membership.sequence == 2
            && b_membership.sequence == 2
            && a_membership.members.iter().any(|entry| entry.peer_id == b.identity.peer_id())
            && b_membership.members.iter().any(|entry| entry.peer_id == b.identity.peer_id())
    });

    wait_until("joined peer snapshot replication", Duration::from_secs(20), || {
        let Ok(Some(replica)) = b.storage.latest_snapshot(world) else { return false };
        replica.manifest_hash().ok() == manifest.manifest_hash().ok() && b.storage.verify_snapshot(&replica).is_ok()
    });

    assert!(b.storage.load_pending_join(world).is_err());
    let replica = b.storage.latest_snapshot(world).unwrap().unwrap();
    assert_eq!(replica.state_root, manifest.state_root);
    assert_eq!(replica.sequence, manifest.sequence);
}
