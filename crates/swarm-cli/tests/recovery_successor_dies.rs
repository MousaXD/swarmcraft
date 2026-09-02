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
    EpochMode, EpochRecordV1, MembershipRecordV1, SnapshotManifestV1, WorldDescriptorV1, WorldId, WorldMemberV1,
    PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
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

    let (world, genesis) =
        create_world_genesis(&a.identity, "26.1.2".into(), "0.19.3".into(), b"five-daemon-recovery-successor-dies")
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
