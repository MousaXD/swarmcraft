#![cfg(unix)]

use std::{
    fs,
    net::UdpSocket,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_cli::authority_permit::permit_path;
use swarm_core::{create_world_genesis, DataPaths, PeerIdentity};
use swarm_network::load_or_create_transport_key;
use swarm_protocol::{
    EpochMode, EpochRecordV1, MembershipRecordV1, PeerId, SnapshotManifestV1, WorldDescriptorV1, WorldMemberV1,
    WorldId, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
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

fn install_canonical_replica(
    peer: &PeerFixture,
    metadata: &WorldMetadataV1,
    descriptor: &WorldDescriptorV1,
    membership: &MembershipRecordV1,
    epoch: &EpochRecordV1,
    manifest: &SnapshotManifestV1,
    source: &std::path::Path,
    authority: &PeerIdentity,
) {
    peer.storage.create_world(metadata).unwrap();
    peer.storage.save_world_descriptor(descriptor).unwrap();
    peer.storage.save_membership_record(membership).unwrap();
    peer.storage.save_epoch_record(epoch).unwrap();
    let mut local = peer
        .storage
        .snapshot_directory(
            source,
            SnapshotContext {
                world: metadata.world_id,
                snapshot_number: manifest.snapshot_number,
                epoch: manifest.epoch,
                sequence: manifest.sequence,
                previous_snapshot_hash: manifest.previous_snapshot_hash,
                authority_peer_id: authority.peer_id(),
                authority_public_key: authority.public_key(),
            },
        )
        .unwrap();
    authority.sign_snapshot(&mut local).unwrap();
    assert_eq!(local.manifest_hash().unwrap(), manifest.manifest_hash().unwrap());
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

#[test]
fn hard_kill_recovers_one_authority_and_stale_peer_resyncs() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();

    let (world, genesis) = create_world_genesis(
        &a.identity,
        "26.1.2".into(),
        "0.19.3".into(),
        b"three-daemon-recovery",
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

    for peer in [&a, &b, &c] {
        install_canonical_replica(peer, &metadata, &descriptor, &membership, &epoch, &manifest, &source, &a.identity);
    }

    let a_addr = transport_address(&a);
    let b_addr = transport_address(&b);
    let c_addr = transport_address(&c);

    let mut daemon_a = spawn_daemon(&a, &[]);
    thread::sleep(Duration::from_secs(1));
    let _daemon_b = spawn_daemon(&b, std::slice::from_ref(&a_addr));
    thread::sleep(Duration::from_secs(1));
    let _daemon_c = spawn_daemon(&c, &[a_addr.clone(), b_addr.clone()]);

    wait_until("initial authority quorum permit", Duration::from_secs(20), || {
        permit_generation(&a, world).is_some_and(|(epoch, fencing, heartbeat)| epoch == 1 && fencing == 1 && heartbeat >= 2)
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

    let (winner, loser) = if b.identity.peer_id() == expected_successor { (&b, &c) } else { (&c, &b) };
    wait_until("successor live authority permit", Duration::from_secs(20), || {
        permit_generation(winner, world)
            .is_some_and(|(epoch, fencing, heartbeat)| epoch == 2 && fencing == 2 && heartbeat >= 2)
    });
    assert!(permit_generation(loser, world).is_none());

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
    restarted_a.stop();

    let hashes = [
        a.storage.latest_snapshot(world).unwrap().unwrap().manifest_hash().unwrap(),
        b.storage.latest_snapshot(world).unwrap().unwrap().manifest_hash().unwrap(),
        c.storage.latest_snapshot(world).unwrap().unwrap().manifest_hash().unwrap(),
    ];
    assert_eq!(hashes[0], hashes[1]);
    assert_eq!(hashes[1], hashes[2]);
}
