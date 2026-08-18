#![cfg(unix)]

#[path = "../src/host.rs"]
mod host;

use std::{fs, os::unix::fs::PermissionsExt};
use swarm_core::{create_world_genesis_with_fingerprint, sign_world_config, DataPaths, PeerIdentity};
use swarm_protocol::{
    AuthorityPolicyV1, MembershipPolicyV1, RuntimeCompatibilityManifestV1, WorldConfigV1, WorldDescriptorV1,
    WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, WorldMetadataV1};

#[tokio::test]
async fn real_host_process_restores_launches_and_commits_mutated_world() {
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path().join("data"));
    let storage = Storage::open(paths.root.clone()).unwrap();
    let identity = PeerIdentity::load_or_create(&paths).unwrap();
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
        &identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        fingerprint,
    )
    .unwrap();

    storage
        .create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "runtime-process-smoke".into(),
            world_id: world,
            genesis: genesis.clone(),
        })
        .unwrap();
    storage
        .save_world_descriptor(&WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: genesis.compatibility_fingerprint,
            members: vec![WorldMemberV1 {
                peer_id: identity.peer_id(),
                public_key: identity.public_key(),
                authority_eligible: true,
                banned: false,
            }],
            preferred_replication_factor: 1,
        })
        .unwrap();
    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 1 },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: "runtime-process-smoke".into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    sign_world_config(&identity, &mut config).unwrap();
    storage.save_world_config(&config).unwrap();

    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"initial-state\n").unwrap();
    let mut initial = storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
            },
        )
        .unwrap();
    identity.sign_snapshot(&mut initial).unwrap();
    storage.commit_snapshot(&initial).unwrap();

    let mock = temp.path().join("mock-java");
    fs::write(
        &mock,
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
pathlib.Path(world, "swarmcraft-runtime-smoke.txt").write_text("mutated-by-real-host-process\n", encoding="utf-8")

def encoded(value):
    return value.encode("utf-8").hex()

with socket.create_connection((host, port), timeout=5) as connection:
    writer = connection.makefile("w", encoding="utf-8", newline="\n")
    writer.write("AUTH\t" + token + "\n")
    writer.write("WORLD_INFO\t" + encoded("26.1.2") + "\t" + encoded("0.19.3") + "\t" + encoded(world) + "\t" + fingerprint + "\n")
    writer.flush()
    time.sleep(0.5)
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&mock).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&mock, permissions).unwrap();

    let dummy_server = temp.path().join("mock-server.jar");
    let dummy_mod = temp.path().join("swarmcraft-fabric.jar");
    fs::write(&dummy_server, b"mock").unwrap();
    fs::write(&dummy_mod, b"mock").unwrap();

    host::run(
        &paths,
        &storage,
        host::HostOptions { world, java: mock, server_jar: dummy_server, mod_jar: dummy_mod, accept_eula: true },
    )
    .await
    .unwrap();

    let latest = storage.latest_snapshot(world).unwrap().unwrap();
    assert_eq!(latest.snapshot_number, 2);
    assert_eq!(latest.sequence, 2);
    assert_eq!(latest.previous_snapshot_hash, Some(initial.manifest_hash().unwrap()));
    storage.verify_snapshot(&latest).unwrap();

    let sleep = storage.load_sleep_record(world).unwrap();
    assert_eq!(sleep.latest_snapshot_hash, latest.manifest_hash().unwrap());

    let restored = temp.path().join("restored");
    storage.restore_snapshot(&latest, &restored).unwrap();
    assert_eq!(
        fs::read_to_string(restored.join("swarmcraft-runtime-smoke.txt")).unwrap(),
        "mutated-by-real-host-process\n"
    );
}
