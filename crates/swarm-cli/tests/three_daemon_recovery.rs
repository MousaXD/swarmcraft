#![cfg(unix)]

use std::{
    fs,
    net::UdpSocket,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_cli::{
    authority_permit::permit_path,
    migration::{
        load_migration_status, prepare_manual_transfer, save_runtime_config, MigrationPhase, MigrationTrigger,
        RuntimeLaunchConfig,
    },
};
use swarm_core::{create_world_genesis_with_fingerprint, sign_world_config, DataPaths, PeerIdentity};
use swarm_network::load_or_create_transport_key;
use swarm_protocol::{
    AuthorityPolicyV1, EpochMode, EpochRecordV1, MembershipPolicyV1, MembershipRecordV1,
    RuntimeCompatibilityManifestV1, SnapshotManifestV1, WorldConfigV1, WorldDescriptorV1, WorldId, WorldMemberV1,
    WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
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

struct CanonicalReplicaSeed<'a> {
    metadata: &'a WorldMetadataV1,
    config: &'a WorldConfigV1,
    descriptor: &'a WorldDescriptorV1,
    membership: &'a MembershipRecordV1,
    epoch: &'a EpochRecordV1,
    manifest: &'a SnapshotManifestV1,
    source: &'a std::path::Path,
    authority: &'a PeerIdentity,
}

struct ManagedChild(Child);

impl ManagedChild {
    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.stop();
    }
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
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

fn transport_address(peer: &PeerFixture) -> String {
    format!("/ip4/127.0.0.1/udp/{}/quic-v1/p2p/{}", peer.port, peer.transport_peer)
}

fn spawn_daemon(peer: &PeerFixture, bootstraps: &[String]) -> ManagedChild {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    command
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/127.0.0.1/udp/{}/quic-v1", peer.port))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if !bootstraps.is_empty() {
        command.env("SWARMCRAFT_BOOTSTRAP", bootstraps.join(","));
    }
    ManagedChild(command.spawn().unwrap())
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

fn install_canonical_replica(peer: &PeerFixture, seed: &CanonicalReplicaSeed<'_>) {
    peer.storage.create_world(seed.metadata).unwrap();
    peer.storage.save_world_config(seed.config).unwrap();
    peer.storage.save_world_descriptor(seed.descriptor).unwrap();
    peer.storage.save_membership_record(seed.membership).unwrap();
    peer.storage.save_epoch_record(seed.epoch).unwrap();
    let mut local = peer
        .storage
        .snapshot_directory(
            seed.source,
            SnapshotContext {
                world: seed.metadata.world_id,
                snapshot_number: seed.manifest.snapshot_number,
                epoch: seed.manifest.epoch,
                sequence: seed.manifest.sequence,
                previous_snapshot_hash: seed.manifest.previous_snapshot_hash,
                authority_peer_id: seed.authority.peer_id(),
                authority_public_key: seed.authority.public_key(),
            },
        )
        .unwrap();
    seed.authority.sign_snapshot(&mut local).unwrap();
    assert_eq!(local.manifest_hash().unwrap(), seed.manifest.manifest_hash().unwrap());
    peer.storage.commit_snapshot(&local).unwrap();
}

fn permit_generation(peer: &PeerFixture, world: WorldId) -> Option<(u64, u64, u64)> {
    let value = fs::read_to_string(permit_path(&peer.paths, world)).ok()?;
    let mut fields = value.split_whitespace();
    let epoch = fields.next()?.parse().ok()?;
    let fencing = fields.next()?.parse().ok()?;
    let heartbeat = fields.next()?.parse().ok()?;
    Some((epoch, fencing, heartbeat))
}

fn configure_mock_runtime(peer: &PeerFixture, world: WorldId, endpoint: &str) {
    let fixture_dir = peer.paths.root.join("migration-runtime-fixture");
    fs::create_dir_all(&fixture_dir).unwrap();
    let java = fixture_dir.join("mock-java");
    let server = fixture_dir.join("server.jar");
    let fabric = fixture_dir.join("swarmcraft-fabric.jar");
    write_mock_java(&java);
    fs::write(&server, b"mock").unwrap();
    fs::write(&fabric, b"mock").unwrap();
    save_runtime_config(
        &peer.paths,
        world,
        &RuntimeLaunchConfig {
            java,
            server_jar: server,
            mod_jar: fabric,
            accept_eula: true,
            game_endpoint: Some(endpoint.into()),
        },
    )
    .unwrap();
}

fn runtime_ready(peer: &PeerFixture, world: WorldId, endpoint: &str, trigger: MigrationTrigger) -> bool {
    let peer_id = peer.identity.peer_id().to_string();
    load_migration_status(&peer.paths, world).is_ok_and(|status| {
        status.phase == MigrationPhase::Ready
            && status.runtime_ready
            && status.trigger == Some(trigger)
            && status.game_endpoint.as_deref() == Some(endpoint)
            && status.authority_peer_id.as_deref() == Some(peer_id.as_str())
    })
}

fn write_mock_java(path: &Path) {
    fs::write(
        path,
        r#"#!/usr/bin/env python3
import os
import pathlib
import socket

host = os.environ["SWARMCRAFT_IPC_HOST"]
port = int(os.environ["SWARMCRAFT_IPC_PORT"])
token = os.environ["SWARMCRAFT_IPC_TOKEN"]
world = os.environ["SWARMCRAFT_WORLD_DIR"]
fingerprint = os.environ["SWARMCRAFT_COMPAT_FINGERPRINT"]
pathlib.Path(world, "swarmcraft-migration-smoke.txt").write_text("started-after-authority-permit\n", encoding="utf-8")

def encoded(value):
    return value.encode("utf-8").hex()

with socket.create_connection((host, port), timeout=5) as connection:
    writer = connection.makefile("w", encoding="utf-8", newline="\n")
    writer.write("AUTH\t" + token + "\n")
    writer.write("WORLD_INFO\t" + encoded("26.1.2") + "\t" + encoded("0.19.3") + "\t" + encoded(world) + "\t" + fingerprint + "\n")
    writer.flush()
    reader = connection.makefile("r", encoding="utf-8")
    while reader.readline():
        pass
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn hard_kill_recovers_one_authority_and_stale_peer_resyncs() {
    let a = peer_fixture();
    let b_candidate = peer_fixture();
    let c_candidate = peer_fixture();
    let (b, c) = if b_candidate.identity.peer_id() < c_candidate.identity.peer_id() {
        (b_candidate, c_candidate)
    } else {
        (c_candidate, b_candidate)
    };

    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: "26.1.2".into(),
        loader_id: "fabric".into(),
        loader_version: "0.19.3".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: env!("CARGO_PKG_VERSION").into(),
        required_server_mods: Vec::new(),
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let fingerprint = compatibility.fingerprint().unwrap();
    let (world, genesis) = create_world_genesis_with_fingerprint(
        &a.identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        fingerprint,
    )
    .unwrap();
    let metadata = WorldMetadataV1 {
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        display_name: "three-daemon-recovery".into(),
        world_id: world,
        genesis,
    };

    let mut descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        compatibility_fingerprint: metadata.genesis.compatibility_fingerprint,
        members: vec![member(&a.identity), member(&b.identity), member(&c.identity)],
        preferred_replication_factor: 3,
    };
    descriptor.normalize();

    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 3 },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: "three-daemon-recovery".into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: a.identity.peer_id(),
        authority_public_key: a.identity.public_key(),
        signature: Vec::new(),
    };
    sign_world_config(&a.identity, &mut config).unwrap();

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

    let source_temp = tempfile::tempdir().unwrap();
    let source = source_temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-before-crash\n").unwrap();
    let mut manifest = a
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 1,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: a.identity.peer_id(),
                authority_public_key: a.identity.public_key(),
            },
        )
        .unwrap();
    a.identity.sign_snapshot(&mut manifest).unwrap();

    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 1,
        previous_epoch_hash: None,
        base_state_hash: manifest.state_root,
        authority_peer_id: a.identity.peer_id(),
        authority_public_key: a.identity.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: 1,
        reason: "three-daemon recovery acceptance seed".into(),
        signature: Vec::new(),
    };
    epoch.signature = a.identity.sign(&epoch.signing_bytes().unwrap());

    let seed = CanonicalReplicaSeed {
        metadata: &metadata,
        config: &config,
        descriptor: &descriptor,
        membership: &membership,
        epoch: &epoch,
        manifest: &manifest,
        source: &source,
        authority: &a.identity,
    };
    for peer in [&a, &b, &c] {
        install_canonical_replica(peer, &seed);
    }

    configure_mock_runtime(&a, world, "alice.test:25565");
    configure_mock_runtime(&b, world, "bob.test:25565");
    configure_mock_runtime(&c, world, "carol.test:25565");

    let a_addr = transport_address(&a);
    let b_addr = transport_address(&b);
    let c_addr = transport_address(&c);

    let mut daemon_a = spawn_daemon(&a, &[]);
    thread::sleep(Duration::from_secs(1));
    let _daemon_b = spawn_daemon(&b, std::slice::from_ref(&a_addr));
    thread::sleep(Duration::from_secs(1));
    let _daemon_c = spawn_daemon(&c, &[a_addr.clone(), b_addr.clone()]);

    wait_until("initial authority quorum permit", Duration::from_secs(20), || {
        permit_generation(&a, world)
            .is_some_and(|(epoch, fencing, heartbeat)| epoch == 1 && fencing == 1 && heartbeat >= 2)
    });
    wait_until("Alice authority runtime ready", Duration::from_secs(20), || {
        runtime_ready(&a, world, "alice.test:25565", MigrationTrigger::DirectHost)
    });

    daemon_a.stop();

    let expected_successor = [b.identity.peer_id(), c.identity.peer_id()].into_iter().min().unwrap();
    wait_until("B and C accepting one Recovery epoch", Duration::from_secs(30), || {
        let b_epoch = b.storage.load_epoch_record(world).ok();
        let c_epoch = c.storage.load_epoch_record(world).ok();
        [b_epoch, c_epoch].into_iter().all(|record| {
            record.is_some_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.mode == EpochMode::Recovery
                    && record.authority_peer_id == expected_successor
            })
        })
    });

    let (winner, loser, winner_endpoint) = if b.identity.peer_id() == expected_successor {
        (&b, &c, "bob.test:25565")
    } else {
        (&c, &b, "carol.test:25565")
    };
    assert_eq!(
        winner.identity.peer_id(),
        b.identity.peer_id(),
        "Bob should be the deterministic successor in this fixture"
    );
    wait_until("successor live authority permit", Duration::from_secs(20), || {
        permit_generation(winner, world)
            .is_some_and(|(epoch, fencing, heartbeat)| epoch == 2 && fencing == 2 && heartbeat >= 2)
    });
    assert!(permit_generation(loser, world).is_none());
    wait_until("recovered authority runtime ready", Duration::from_secs(30), || {
        runtime_ready(winner, world, winner_endpoint, MigrationTrigger::AutomaticRecovery)
    });
    let restored_marker =
        winner.paths.root.join("runtime").join(world.to_hex()).join("world").join("swarmcraft-migration-smoke.txt");
    assert_eq!(fs::read_to_string(restored_marker).unwrap(), "started-after-authority-permit\n");

    let winner_latest = winner.storage.latest_snapshot(world).unwrap().unwrap();
    assert_eq!(winner_latest.snapshot_number, 2);
    assert_eq!(winner_latest.epoch, 2);
    assert_eq!(winner_latest.previous_snapshot_hash, Some(manifest.manifest_hash().unwrap()));

    let mut restarted_a = spawn_daemon(&a, &[b_addr, c_addr]);
    wait_until("stale A accepting recovery epoch and promoted snapshot", Duration::from_secs(30), || {
        let Ok(a_epoch) = a.storage.load_epoch_record(world) else { return false };
        let Ok(Some(a_latest)) = a.storage.latest_snapshot(world) else { return false };
        a_epoch.epoch_number == 2
            && a_epoch.authority_peer_id == expected_successor
            && a_latest.manifest_hash().ok() == winner_latest.manifest_hash().ok()
    });
    assert!(permit_generation(&a, world).is_none());
    let expected_successor_text = expected_successor.to_string();
    wait_until("stale Alice runtime fenced", Duration::from_secs(10), || {
        load_migration_status(&a.paths, world).is_ok_and(|status| {
            status.phase == MigrationPhase::WaitingForAuthority
                && !status.runtime_ready
                && status.authority_peer_id.as_deref() == Some(expected_successor_text.as_str())
        })
    });
    assert!(prepare_manual_transfer(&a.paths, &a.storage, world, winner.identity.peer_id()).is_err());
    restarted_a.stop();

    let hashes = [
        a.storage.latest_snapshot(world).unwrap().unwrap().manifest_hash().unwrap(),
        b.storage.latest_snapshot(world).unwrap().unwrap().manifest_hash().unwrap(),
        c.storage.latest_snapshot(world).unwrap().unwrap().manifest_hash().unwrap(),
    ];
    assert_eq!(hashes[0], hashes[1]);
    assert_eq!(hashes[1], hashes[2]);
}
