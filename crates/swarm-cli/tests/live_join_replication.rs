#![cfg(unix)]

use std::{
    fs,
    net::UdpSocket,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_core::{create_world_genesis, random_nonce, DataPaths, PeerIdentity};
use swarm_network::load_or_create_transport_key;
use swarm_protocol::{
    EpochMode, EpochRecordV1, InviteV1, JoinRequestV1, MembershipRecordV1, WorldDescriptorV1, WorldMemberV1,
    PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
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

fn spawn_daemon(peer: &PeerFixture) -> ManagedChild {
    let child = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/127.0.0.1/udp/{}/quic-v1", peer.port))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    ManagedChild(child)
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
fn authority_accepts_live_join_and_replicates_without_reconnect() {
    let a = peer_fixture();
    let b = peer_fixture();
    let a_address = transport_address(&a);

    let (world, genesis) =
        create_world_genesis(&a.identity, "26.1.2".into(), "0.19.3".into(), b"live-join-replication").unwrap();
    let metadata = WorldMetadataV1 {
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        display_name: "live-join-replication".into(),
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
        epoch: 0,
        sequence: 0,
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
    fs::write(source.join("level.dat"), b"join-source-level\n").unwrap();
    fs::write(source.join("region/r.0.0.mca"), b"join-source-region\n").unwrap();
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
        mode: EpochMode::Quorum,
        fencing_token: 1,
        reason: "live join quorum-of-one seed".into(),
        signature: Vec::new(),
    };
    epoch.signature = a.identity.sign(&epoch.signing_bytes().unwrap());
    a.storage.save_epoch_record(&epoch).unwrap();

    let mut promoted_membership = membership.clone();
    promoted_membership.epoch = 1;
    promoted_membership.sequence = 1;
    promoted_membership.previous_membership_hash = Some(membership.record_hash().unwrap());
    promoted_membership.signature.clear();
    a.identity.sign_membership(&mut promoted_membership).unwrap();
    a.storage.save_membership_record(&promoted_membership).unwrap();

    b.storage.create_world(&metadata).unwrap();
    b.storage.save_world_descriptor(&descriptor).unwrap();
    let mut invite = InviteV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        display_name: metadata.display_name.clone(),
        genesis,
        inviter_peer_id: a.identity.peer_id(),
        inviter_public_key: a.identity.public_key(),
        bootstrap_addrs: vec![a_address],
        expires_unix_ms: u64::MAX,
        nonce: random_nonce(),
        signature: Vec::new(),
    };
    a.identity.sign_invite(&mut invite).unwrap();
    let mut join = JoinRequestV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        invite,
        joining_member: member(&b.identity),
        nonce: random_nonce(),
        signature: Vec::new(),
    };
    b.identity.sign_join_request(&mut join).unwrap();
    b.storage.save_pending_join(&join).unwrap();

    let _daemon_a = spawn_daemon(&a);
    thread::sleep(Duration::from_secs(1));
    let _daemon_b = spawn_daemon(&b);

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
