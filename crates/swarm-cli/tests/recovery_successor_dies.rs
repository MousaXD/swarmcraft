#![cfg(unix)]

use std::{
    fs,
    net::UdpSocket,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_cli::{
    authority_permit::permit_path,
    host_readiness,
    migration::{save_runtime_config, RuntimeLaunchConfig},
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
const RECOVERY_PAUSE_MS: u64 = 30_000;

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

struct ManagedChild {
    child: Child,
    log_path: std::path::PathBuf,
}

impl ManagedChild {
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn status(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().unwrap()
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_else(|error| format!("<failed to read daemon log: {error}>"))
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

fn spawn_daemon(peer: &PeerFixture, bootstraps: &[String], pause_after_certificate: bool) -> ManagedChild {
    let log_path = peer.paths.root.join("recovery-acceptance-daemon.log");
    let log = fs::File::create(&log_path).unwrap();
    let log_err = log.try_clone().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    command
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/127.0.0.1/udp/{}/quic-v1", peer.port))
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    if !bootstraps.is_empty() {
        command.env("SWARMCRAFT_BOOTSTRAP", bootstraps.join(","));
    }
    if pause_after_certificate {
        command.env("SWARMCRAFT_TEST_PAUSE_AFTER_RECOVERY_CERTIFICATE_MS", RECOVERY_PAUSE_MS.to_string());
    }
    ManagedChild { child: command.spawn().unwrap(), log_path }
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

fn wait_until_with_daemon(
    label: &str,
    timeout: Duration,
    daemon: &mut ManagedChild,
    mut predicate: impl FnMut() -> bool,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        if let Some(status) = daemon.status() {
            panic!("daemon exited while waiting for {label}: {status}\n{}", daemon.log());
        }
        thread::sleep(WAIT_STEP);
    }
    panic!("timed out waiting for {label}\n{}", daemon.log());
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
    let mut promoted_membership = seed.membership.clone();
    promoted_membership.epoch = seed.epoch.epoch_number;
    promoted_membership.sequence = seed.membership.sequence.checked_add(1).unwrap();
    promoted_membership.previous_membership_hash = Some(seed.membership.record_hash().unwrap());
    promoted_membership.signature.clear();
    seed.authority.sign_membership(&mut promoted_membership).unwrap();
    peer.storage.save_membership_record(&promoted_membership).unwrap();
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
    configure_host_capability(peer, seed.metadata.world_id);
}

fn configure_host_capability(peer: &PeerFixture, world: WorldId) {
    let directory = peer.paths.root.join("recovery-runtime-fixture");
    fs::create_dir_all(&directory).unwrap();
    let java = directory.join("mock-java");
    let server = directory.join("server.jar");
    let fabric = directory.join("swarmcraft-fabric.jar");
    fs::write(
        &java,
        r#"#!/usr/bin/env python3
import os
import socket
import sys

if "-version" in sys.argv:
    print('openjdk version "25.0.1"', file=sys.stderr)
    raise SystemExit(0)

host = os.environ["SWARMCRAFT_IPC_HOST"]
port = int(os.environ["SWARMCRAFT_IPC_PORT"])
token = os.environ["SWARMCRAFT_IPC_TOKEN"]
world = os.environ["SWARMCRAFT_WORLD_DIR"]
fingerprint = os.environ["SWARMCRAFT_COMPAT_FINGERPRINT"]

def encoded(value):
    return value.encode("utf-8").hex()

with socket.create_connection((host, port), timeout=5) as connection:
    writer = connection.makefile("w", encoding="utf-8", newline="\n")
    writer.write("AUTH\t" + token + "\n")
    writer.write("WORLD_INFO\t" + encoded("26.1.2") + "\t" + encoded("0.19.3") + "\t" + encoded(world) + "\t" + fingerprint + "\t25\n")
    writer.flush()
    reader = connection.makefile("r", encoding="utf-8")
    while reader.readline():
        pass
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&java).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&java, permissions).unwrap();
    fs::write(&server, b"mock").unwrap();
    fs::write(&fabric, b"mock").unwrap();
    let config =
        RuntimeLaunchConfig { java, server_jar: server, mod_jar: fabric, accept_eula: true, game_endpoint: None };
    save_runtime_config(&peer.paths, world, &config).unwrap();
    let fingerprint = peer.storage.load_world_descriptor(world).unwrap().compatibility_fingerprint;
    host_readiness::record_runtime_verified(&peer.paths, world, &config, fingerprint).unwrap();
}

fn permit_generation(peer: &PeerFixture, world: WorldId) -> Option<(u64, u64, u64)> {
    let value = fs::read_to_string(permit_path(&peer.paths, world)).ok()?;
    let mut fields = value.split_whitespace();
    Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}

#[test]
fn formed_recovery_certificate_locks_value_until_certified_candidate_resumes() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();
    let d = peer_fixture();
    let e = peer_fixture();
    let peers = [&a, &b, &c, &d, &e];

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
        display_name: "five-daemon-recovery-successor-dies".into(),
        world_id: world,
        genesis,
    };
    let mut descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        compatibility_fingerprint: metadata.genesis.compatibility_fingerprint,
        members: peers.iter().map(|peer| member(&peer.identity)).collect(),
        preferred_replication_factor: 5,
    };
    descriptor.normalize();
    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 { allow_solo_advancement: false, preferred_replication_factor: 5 },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: "five-daemon-recovery-successor-dies".into(),
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
        epoch: 0,
        sequence: 0,
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
    fs::write(source.join("level.dat"), b"canonical-before-two-crashes\n").unwrap();
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
        reason: "five-daemon successor-dies acceptance seed".into(),
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
    for peer in peers {
        install_canonical_replica(peer, &seed);
    }

    let a_addr = transport_address(&a);
    let survivors = [&b, &c, &d, &e];
    let survivor_addrs = survivors.iter().map(|peer| transport_address(peer)).collect::<Vec<_>>();
    let first_successor_id = survivors.iter().map(|peer| peer.identity.peer_id()).min().unwrap();
    let first_index = survivors.iter().position(|peer| peer.identity.peer_id() == first_successor_id).unwrap();

    let mut daemon_a = spawn_daemon(&a, &[], false);
    thread::sleep(Duration::from_secs(1));
    let mut survivor_daemons = Vec::new();
    for (index, peer) in survivors.iter().enumerate() {
        let mut bootstraps = Vec::with_capacity(index + 1);
        bootstraps.push(a_addr.clone());
        bootstraps.extend(survivor_addrs.iter().take(index).cloned());
        survivor_daemons.push(spawn_daemon(peer, &bootstraps, index == first_index));
        thread::sleep(Duration::from_millis(350));
    }

    wait_until("initial five-member authority quorum permit", Duration::from_secs(30), || {
        permit_generation(&a, world)
            .is_some_and(|(epoch, fencing, heartbeat)| epoch == 1 && fencing == 1 && heartbeat >= 2)
    });
    daemon_a.stop();

    let first_successor = survivors[first_index];
    wait_until("first successor persisting round-one quorum certificate", Duration::from_secs(40), || {
        first_successor.storage.load_recovery_certificate(world).is_ok_and(|certificate| {
            certificate.ballot.round == 1 && certificate.ballot.candidate_peer_id == first_successor_id
        })
    });
    assert_eq!(first_successor.storage.load_epoch_record(world).unwrap().epoch_number, 1);
    survivor_daemons[first_index].stop();

    let remaining = survivors
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != first_index)
        .map(|(_, peer)| *peer)
        .collect::<Vec<_>>();

    // The round-one certificate is already a chosen value for target generation 2.
    // While that certified candidate is down, a later proposer may raise the round,
    // but it must not switch the candidate and commit a conflicting same-generation
    // Recovery epoch. Safety deliberately wins over same-generation failover here.
    thread::sleep(Duration::from_secs(20));
    for peer in &remaining {
        let record = peer.storage.load_epoch_record(world).unwrap();
        assert_eq!(record.epoch_number, 1);
        assert_eq!(record.fencing_token, 1);
        assert_eq!(record.authority_peer_id, a.identity.peer_id());
        assert!(permit_generation(peer, world).is_none());
        if let Ok(certificate) = peer.storage.load_recovery_certificate(world) {
            assert_eq!(certificate.ballot.candidate_peer_id, first_successor_id);
        }
    }

    // Resume the candidate that actually owns the chosen certificate. Its durable
    // certificate must be sufficient to finish the exact value it previously won,
    // and every live voter must converge on that one Recovery epoch.
    let remaining_addrs = remaining.iter().map(|peer| transport_address(peer)).collect::<Vec<_>>();
    let mut restarted_first = spawn_daemon(first_successor, &remaining_addrs, false);
    wait_until_with_daemon(
        "certified first successor resuming chosen recovery value",
        Duration::from_secs(40),
        &mut restarted_first,
        || {
            first_successor.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.mode == EpochMode::Recovery
                    && record.authority_peer_id == first_successor_id
            })
        },
    );
    wait_until("remaining voters adopting the chosen recovery value", Duration::from_secs(40), || {
        remaining.iter().all(|peer| {
            peer.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.mode == EpochMode::Recovery
                    && record.authority_peer_id == first_successor_id
            })
        })
    });
    wait_until("resumed certified successor live permit", Duration::from_secs(30), || {
        permit_generation(first_successor, world)
            .is_some_and(|(epoch, fencing, heartbeat)| epoch == 2 && fencing == 2 && heartbeat >= 2)
    });
    for peer in &remaining {
        assert!(permit_generation(peer, world).is_none());
    }

    let authority_addr = transport_address(first_successor);
    let authority_bootstrap = vec![authority_addr];
    let mut restarted_a = spawn_daemon(&a, &authority_bootstrap, false);
    wait_until_with_daemon(
        "original stale authority adopting the chosen recovery value",
        Duration::from_secs(40),
        &mut restarted_a,
        || {
            a.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2 && record.fencing_token == 2 && record.authority_peer_id == first_successor_id
            })
        },
    );
    assert!(permit_generation(&a, world).is_none());
    restarted_a.stop();
    restarted_first.stop();
}
