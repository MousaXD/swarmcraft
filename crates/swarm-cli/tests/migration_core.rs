#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use swarm_cli::{
    authority_permit::refresh_permit,
    migration::{
        self, accept_manual_transfer, activate_manual_transfer, commit_manual_transfer, observe_manual_transfer_epoch,
        prepare_manual_transfer, MigrationPhase, RuntimeLaunchConfig, TransferPrepareResult,
    },
};
use swarm_consensus::AuthorityGeneration;
use swarm_core::{create_world_genesis, DataPaths, PeerIdentity};
use swarm_protocol::{
    EpochMode, EpochRecordV1, MembershipRecordV1, SleepRecordV1, WorldDescriptorV1, WorldId, WorldMemberV1,
    PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, WorldMetadataV1};
use tokio::{
    task::JoinHandle,
    time::{sleep, timeout},
};

struct PeerFixture {
    paths: DataPaths,
    storage: Storage,
    identity: PeerIdentity,
}

struct SharedWorld {
    world: WorldId,
    epoch: EpochRecordV1,
}

fn peer(root: PathBuf) -> PeerFixture {
    let paths = DataPaths::from_root(root);
    let storage = Storage::open(paths.root.clone()).unwrap();
    let identity = PeerIdentity::load_or_create(&paths).unwrap();
    PeerFixture { paths, storage, identity }
}

fn initialize_two_peer_world(alice: &PeerFixture, bob: &PeerFixture, source: &Path) -> SharedWorld {
    let (world, genesis) =
        create_world_genesis(&alice.identity, "26.1.2".into(), "0.19.3".into(), b"migration-core").unwrap();
    let metadata = WorldMetadataV1 {
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        display_name: "migration-core".into(),
        world_id: world,
        genesis: genesis.clone(),
    };
    alice.storage.create_world(&metadata).unwrap();
    bob.storage.create_world(&metadata).unwrap();

    let descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        compatibility_fingerprint: genesis.compatibility_fingerprint,
        members: vec![member(&alice.identity), member(&bob.identity)],
        preferred_replication_factor: 2,
    };
    alice.storage.save_world_descriptor(&descriptor).unwrap();
    bob.storage.save_world_descriptor(&descriptor).unwrap();

    let mut membership = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch: 0,
        sequence: 0,
        previous_membership_hash: None,
        members: descriptor.members.clone(),
        authority_peer_id: alice.identity.peer_id(),
        authority_public_key: alice.identity.public_key(),
        signature: Vec::new(),
    };
    alice.identity.sign_membership(&mut membership).unwrap();
    alice.storage.save_membership_record(&membership).unwrap();
    bob.storage.save_membership_record(&membership).unwrap();

    let alice_snapshot = snapshot(&alice.storage, &alice.identity, world, source);
    let bob_snapshot = snapshot(&bob.storage, &alice.identity, world, source);
    assert_eq!(alice_snapshot.manifest_hash().unwrap(), bob_snapshot.manifest_hash().unwrap());

    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 0,
        previous_epoch_hash: None,
        base_state_hash: alice_snapshot.state_root,
        authority_peer_id: alice.identity.peer_id(),
        authority_public_key: alice.identity.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: 1,
        reason: "initial authority".into(),
        signature: Vec::new(),
    };
    epoch.signature = alice.identity.sign(&epoch.signing_bytes().unwrap());
    alice.storage.save_epoch_record(&epoch).unwrap();
    bob.storage.save_epoch_record(&epoch).unwrap();
    SharedWorld { world, epoch }
}

fn snapshot(
    storage: &Storage,
    authority: &PeerIdentity,
    world: WorldId,
    source: &Path,
) -> swarm_protocol::SnapshotManifestV1 {
    let mut manifest = storage
        .snapshot_directory(
            source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: authority.peer_id(),
                authority_public_key: authority.public_key(),
            },
        )
        .unwrap();
    authority.sign_snapshot(&mut manifest).unwrap();
    storage.commit_snapshot(&manifest).unwrap();
    manifest
}

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn save_sleep(peer: &PeerFixture, world: WorldId) {
    let latest = peer.storage.latest_snapshot(world).unwrap().unwrap();
    let epoch = peer.storage.load_epoch_record(world).unwrap();
    let mut record = SleepRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        latest_snapshot_hash: latest.manifest_hash().unwrap(),
        epoch: epoch.epoch_number,
        fencing_token: epoch.fencing_token,
        authority_peer_id: peer.identity.peer_id(),
        authority_public_key: peer.identity.public_key(),
        signature: Vec::new(),
    };
    peer.identity.sign_sleep_record(&mut record).unwrap();
    peer.storage.save_sleep_record(&record).unwrap();
}

fn configure_mock_runtime(temp: &Path, peer: &PeerFixture, world: WorldId, endpoint: &str) {
    let mock_java = temp.join(format!("mock-java-{}", world.to_hex()));
    write_mock_java(&mock_java);
    let server = temp.join(format!("server-{}.jar", world.to_hex()));
    let fabric = temp.join(format!("swarmcraft-fabric-{}.jar", world.to_hex()));
    fs::write(&server, b"mock").unwrap();
    fs::write(&fabric, b"mock").unwrap();
    migration::save_runtime_config(
        &peer.paths,
        world,
        &RuntimeLaunchConfig {
            java: mock_java,
            server_jar: server,
            mod_jar: fabric,
            accept_eula: true,
            game_endpoint: Some(endpoint.into()),
        },
    )
    .unwrap();
}

fn spawn_heartbeat(paths: DataPaths, world: WorldId, generation: AuthorityGeneration) -> JoinHandle<()> {
    tokio::spawn(async move {
        for sequence in 1..100u64 {
            refresh_permit(&paths, world, generation, sequence).unwrap();
            sleep(Duration::from_millis(100)).await;
        }
    })
}

async fn wait_until_ready(peer: &PeerFixture, world: WorldId, endpoint: &str) {
    let peer_id = peer.identity.peer_id().to_string();
    timeout(Duration::from_secs(12), async {
        loop {
            if let Ok(status) = migration::load_migration_status(&peer.paths, world) {
                if status.phase == MigrationPhase::Ready && status.runtime_ready {
                    assert_eq!(status.authority_peer_id.as_deref(), Some(peer_id.as_str()));
                    assert_eq!(status.game_endpoint.as_deref(), Some(endpoint));
                    break;
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("authority never reached ready state");
}

async fn wait_until_sleeping(peer: &PeerFixture, world: WorldId) {
    timeout(Duration::from_secs(12), async {
        loop {
            if peer.storage.load_sleep_record(world).is_ok() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("authority never checkpointed after runtime exit");
}

#[tokio::test]
async fn manual_transfer_uses_shared_runner_and_fences_alice() {
    let temp = tempfile::tempdir().unwrap();
    let alice = peer(temp.path().join("alice"));
    let bob = peer(temp.path().join("bob"));
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-before-transfer\n").unwrap();
    let shared = initialize_two_peer_world(&alice, &bob, &source);
    save_sleep(&alice, shared.world);
    let alice_sleep = alice.storage.load_sleep_record(shared.world).unwrap();
    bob.storage.save_sleep_record(&alice_sleep).unwrap();

    let prepared =
        match prepare_manual_transfer(&alice.paths, &alice.storage, shared.world, bob.identity.peer_id()).unwrap() {
            TransferPrepareResult::Prepared(token) => token,
            TransferPrepareResult::CheckpointRequested => {
                panic!("sleeping world should already be checkpointed")
            }
        };
    let accepted = accept_manual_transfer(&bob.paths, &bob.storage, shared.world, &prepared).unwrap();
    let committed = commit_manual_transfer(&alice.paths, &alice.storage, shared.world, &accepted).unwrap();
    let epoch_token = activate_manual_transfer(&bob.paths, &bob.storage, shared.world, &committed).unwrap();
    observe_manual_transfer_epoch(&alice.paths, &alice.storage, shared.world, &epoch_token).unwrap();

    let alice_epoch = alice.storage.load_epoch_record(shared.world).unwrap();
    let bob_epoch = bob.storage.load_epoch_record(shared.world).unwrap();
    assert_eq!(alice_epoch.epoch_number, shared.epoch.epoch_number + 1);
    assert_eq!(alice_epoch.fencing_token, shared.epoch.fencing_token + 1);
    assert_eq!(alice_epoch.authority_peer_id, bob.identity.peer_id());
    assert_eq!(alice_epoch, bob_epoch);
    assert!(alice.storage.load_sleep_record(shared.world).is_err());
    assert!(bob.storage.load_sleep_record(shared.world).is_err());

    let stale = prepare_manual_transfer(&alice.paths, &alice.storage, shared.world, bob.identity.peer_id());
    assert!(stale.is_err(), "former authority must not initiate canonical work after observing Bob's generation");

    let endpoint = "127.0.0.1:25566";
    configure_mock_runtime(temp.path(), &bob, shared.world, endpoint);
    let generation = AuthorityGeneration { epoch: bob_epoch.epoch_number, fencing_token: bob_epoch.fencing_token };
    let heartbeat = spawn_heartbeat(bob.paths.clone(), shared.world, generation);
    let supervisor_paths = bob.paths.clone();
    let supervisor = tokio::spawn(async move { migration::supervise(supervisor_paths).await });

    wait_until_ready(&bob, shared.world, endpoint).await;
    wait_until_sleeping(&bob, shared.world).await;

    let final_snapshot = bob.storage.latest_snapshot(shared.world).unwrap().unwrap();
    assert_eq!(final_snapshot.epoch, bob_epoch.epoch_number);
    assert_eq!(final_snapshot.authority_peer_id, bob.identity.peer_id());
    bob.storage.verify_snapshot(&final_snapshot).unwrap();

    supervisor.abort();
    heartbeat.abort();
}

#[tokio::test]
async fn recovered_bob_waits_for_exact_permit_then_restores_and_reaches_fabric_ready() {
    let temp = tempfile::tempdir().unwrap();
    let alice = peer(temp.path().join("alice"));
    let bob = peer(temp.path().join("bob"));
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-before-crash\n").unwrap();
    let shared = initialize_two_peer_world(&alice, &bob, &source);
    let latest = bob.storage.latest_snapshot(shared.world).unwrap().unwrap();

    let mut recovery = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: shared.world,
        epoch_number: shared.epoch.epoch_number + 1,
        previous_epoch_hash: Some(epoch_hash(&shared.epoch)),
        base_state_hash: latest.state_root,
        authority_peer_id: bob.identity.peer_id(),
        authority_public_key: bob.identity.public_key(),
        mode: EpochMode::Recovery,
        fencing_token: shared.epoch.fencing_token + 1,
        reason: "test fixture: certified recovery already accepted".into(),
        signature: Vec::new(),
    };
    recovery.signature = bob.identity.sign(&recovery.signing_bytes().unwrap());
    bob.storage.save_epoch_record(&recovery).unwrap();

    let endpoint = "127.0.0.1:25565";
    configure_mock_runtime(temp.path(), &bob, shared.world, endpoint);
    let generation = AuthorityGeneration { epoch: recovery.epoch_number, fencing_token: recovery.fencing_token };
    let heartbeat = spawn_heartbeat(bob.paths.clone(), shared.world, generation);
    let supervisor_paths = bob.paths.clone();
    let supervisor = tokio::spawn(async move { migration::supervise(supervisor_paths).await });

    wait_until_ready(&bob, shared.world, endpoint).await;
    wait_until_sleeping(&bob, shared.world).await;

    let final_snapshot = bob.storage.latest_snapshot(shared.world).unwrap().unwrap();
    assert_eq!(final_snapshot.epoch, recovery.epoch_number);
    assert_eq!(final_snapshot.authority_peer_id, bob.identity.peer_id());
    bob.storage.verify_snapshot(&final_snapshot).unwrap();
    let restored = temp.path().join("restored");
    bob.storage.restore_snapshot(&final_snapshot, &restored).unwrap();
    assert_eq!(
        fs::read_to_string(restored.join("swarmcraft-migration-smoke.txt")).unwrap(),
        "started-after-authority-permit\n"
    );

    supervisor.abort();
    heartbeat.abort();
}

fn epoch_hash(record: &EpochRecordV1) -> swarm_protocol::Hash32 {
    let encoded = postcard::to_allocvec(record).unwrap();
    swarm_protocol::Hash32::from_domain_bytes(b"swarmcraft/epoch-record/v1\0", &encoded)
}

fn write_mock_java(path: &Path) {
    fs::write(
        path,
        r#"#!/usr/bin/env python3
import os
import pathlib
import socket
import time

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
    time.sleep(2.0)
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}
