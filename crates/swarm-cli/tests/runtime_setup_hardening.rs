#![cfg(unix)]

use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use swarm_cli::migration::{
    load_migration_status, load_runtime_config, run_authority_runtime, save_runtime_config, HostOptions,
    MigrationPhase, RuntimeLaunchConfig,
};
use swarm_cli::runtime_installer::RuntimeInstaller;
use swarm_core::{create_world_genesis_with_fingerprint, DataPaths, PeerIdentity};
use swarm_protocol::{
    AuthorityPolicyV1, EpochMode, EpochRecordV1, MembershipPolicyV1, RuntimeCompatibilityManifestV1, WorldConfigV1,
    WorldDescriptorV1, WorldId, WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
    STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, WorldMetadataV1};

struct RuntimeFixture {
    paths: DataPaths,
    storage: Storage,
    world: WorldId,
    baseline_hash: swarm_protocol::Hash32,
    baseline_snapshot_number: u64,
}

fn fixture(root: PathBuf) -> RuntimeFixture {
    let paths = DataPaths::from_root(root);
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
    let compatibility_fingerprint = compatibility.fingerprint().unwrap();
    let (world, genesis) =
        create_world_genesis_with_fingerprint(&identity, "26.1.2".into(), "0.19.3".into(), compatibility_fingerprint)
            .unwrap();

    storage
        .create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "runtime-hardening".into(),
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
    let mut world_config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 1 },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: "runtime-hardening".into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    world_config.signature = identity.sign(&world_config.signing_bytes().unwrap());
    storage.save_world_config(&world_config).unwrap();

    let source = paths.root.join("fixture-world");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-runtime-hardening\n").unwrap();
    let mut snapshot = storage
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
    identity.sign_snapshot(&mut snapshot).unwrap();
    storage.commit_snapshot(&snapshot).unwrap();

    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 0,
        previous_epoch_hash: None,
        base_state_hash: snapshot.state_root,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        mode: EpochMode::Solo,
        fencing_token: 1,
        reason: "runtime hardening fixture".into(),
        signature: Vec::new(),
    };
    epoch.signature = identity.sign(&epoch.signing_bytes().unwrap());
    storage.save_epoch_record(&epoch).unwrap();

    RuntimeFixture {
        paths,
        storage,
        world,
        baseline_hash: snapshot.manifest_hash().unwrap(),
        baseline_snapshot_number: snapshot.snapshot_number,
    }
}

fn runtime_dir(fixture: &RuntimeFixture) -> PathBuf {
    fixture.paths.root.join("runtime").join(fixture.world.to_hex())
}

fn canonical_hash(fixture: &RuntimeFixture) -> swarm_protocol::Hash32 {
    fixture.storage.latest_snapshot(fixture.world).unwrap().unwrap().manifest_hash().unwrap()
}

fn write_mock_java(path: &Path, minecraft_version: &str, loader_version: &str) {
    let script = format!(
        r#"#!/usr/bin/env python3
import os
import socket
import sys
import time

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
    writer.write(
        "WORLD_INFO\t"
        + encoded({minecraft:?})
        + "\t"
        + encoded({loader:?})
        + "\t"
        + encoded(world)
        + "\t"
        + fingerprint
        + "\n"
    )
    writer.flush()
    time.sleep(0.35)
"#,
        minecraft = minecraft_version,
        loader = loader_version,
    );
    fs::write(path, script).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn manual_runtime_files(temp: &Path, minecraft: &str, loader: &str) -> (PathBuf, PathBuf, PathBuf) {
    let java = temp.join("mock-java");
    let server = temp.join("fabric-server.jar");
    let bridge = temp.join("swarmcraft-fabric.jar");
    write_mock_java(&java, minecraft, loader);
    fs::write(&server, b"mock-server").unwrap();
    fs::write(&bridge, b"mock-bridge").unwrap();
    (java, server, bridge)
}

#[tokio::test]
async fn eula_rejection_never_launches_or_replaces_existing_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer"));
    let runtime = runtime_dir(&fixture);
    fs::create_dir_all(&runtime).unwrap();
    let marker = runtime.join("existing-runtime.marker");
    fs::write(&marker, b"keep-me").unwrap();

    let error = run_authority_runtime(
        &fixture.paths,
        &fixture.storage,
        HostOptions {
            world: fixture.world,
            java: temp.path().join("missing-java"),
            server_jar: temp.path().join("missing-server.jar"),
            mod_jar: temp.path().join("missing-bridge.jar"),
            accept_eula: false,
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("EULA"));
    assert_eq!(fs::read(&marker).unwrap(), b"keep-me");
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);
}

#[tokio::test]
async fn missing_java_never_publishes_runtime_ready_or_changes_canonical_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer"));
    let server = temp.path().join("fabric-server.jar");
    let bridge = temp.path().join("swarmcraft-fabric.jar");
    fs::write(&server, b"mock-server").unwrap();
    fs::write(&bridge, b"mock-bridge").unwrap();

    let missing_java = temp.path().join("java-disappeared");
    let error = run_authority_runtime(
        &fixture.paths,
        &fixture.storage,
        HostOptions {
            world: fixture.world,
            java: missing_java.clone(),
            server_jar: server,
            mod_jar: bridge,
            accept_eula: true,
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("cannot launch Java runtime"));
    assert!(error.to_string().contains(missing_java.to_string_lossy().as_ref()));
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);
    fixture.storage.verify_snapshot(&fixture.storage.latest_snapshot(fixture.world).unwrap().unwrap()).unwrap();

    let status = load_migration_status(&fixture.paths, fixture.world).unwrap();
    assert_eq!(status.phase, MigrationPhase::LaunchingRuntime);
    assert!(!status.runtime_ready);
}

#[tokio::test]
async fn failed_partial_setup_retries_cleanly_and_manual_advanced_runtime_still_works() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer"));
    let java = temp.path().join("mock-java");
    let server = temp.path().join("fabric-server.jar");
    let bridge = temp.path().join("swarmcraft-fabric.jar");
    write_mock_java(&java, "26.1.2", "0.19.3");
    fs::write(&server, b"mock-server").unwrap();

    let options = HostOptions {
        world: fixture.world,
        java: java.clone(),
        server_jar: server.clone(),
        mod_jar: bridge.clone(),
        accept_eula: true,
    };

    let first = run_authority_runtime(&fixture.paths, &fixture.storage, options.clone()).await.unwrap_err();
    assert!(first.to_string().contains("cannot install Fabric bridge"));
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);

    fs::write(&bridge, b"mock-bridge").unwrap();
    run_authority_runtime(&fixture.paths, &fixture.storage, options).await.unwrap();

    let latest = fixture.storage.latest_snapshot(fixture.world).unwrap().unwrap();
    fixture.storage.verify_snapshot(&latest).unwrap();
    assert_eq!(latest.snapshot_number, fixture.baseline_snapshot_number + 1);
    assert_eq!(
        fs::read_to_string(runtime_dir(&fixture).join("world/level.dat")).unwrap(),
        "canonical-runtime-hardening\n"
    );
    assert_eq!(fs::read_to_string(runtime_dir(&fixture).join("eula.txt")).unwrap(), "eula=true\n");
}

#[tokio::test]
async fn incompatible_fabric_handshake_is_rejected_before_ready_and_world_stays_canonical() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer"));
    let (java, server, bridge) = manual_runtime_files(temp.path(), "definitely-wrong", "0.19.3");

    let error = run_authority_runtime(
        &fixture.paths,
        &fixture.storage,
        HostOptions { world: fixture.world, java, server_jar: server, mod_jar: bridge, accept_eula: true },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("Fabric reported Minecraft"));
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);
    fixture.storage.verify_snapshot(&fixture.storage.latest_snapshot(fixture.world).unwrap().unwrap()).unwrap();

    let status = load_migration_status(&fixture.paths, fixture.world).unwrap();
    assert_eq!(status.phase, MigrationPhase::VerifyingFabric);
    assert!(!status.runtime_ready);
}

#[test]
fn corrupt_runtime_metadata_can_be_repaired_without_touching_world_state() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer"));
    let (java, server, bridge) = manual_runtime_files(temp.path(), "26.1.2", "0.19.3");
    let config = RuntimeLaunchConfig {
        java,
        server_jar: server,
        mod_jar: bridge,
        accept_eula: true,
        game_endpoint: Some("127.0.0.1:25565".into()),
    };

    save_runtime_config(&fixture.paths, fixture.world, &config).unwrap();
    assert_eq!(load_runtime_config(&fixture.paths, fixture.world).unwrap(), config);

    let config_path = fixture.paths.root.join("control").join(fixture.world.to_hex()).join("runtime.json");
    fs::write(&config_path, b"{ interrupted write").unwrap();
    assert!(load_runtime_config(&fixture.paths, fixture.world).is_err());
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);

    save_runtime_config(&fixture.paths, fixture.world, &config).unwrap();
    assert_eq!(load_runtime_config(&fixture.paths, fixture.world).unwrap(), config);
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);
}

#[test]
fn manual_advanced_config_is_launchable_without_being_reclassified_as_missing_managed_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer-manual-inspect"));
    let (java, server, bridge) = manual_runtime_files(temp.path(), "26.1.2", "0.19.3");
    let config = RuntimeLaunchConfig {
        java,
        server_jar: server,
        mod_jar: bridge,
        accept_eula: true,
        game_endpoint: Some("127.0.0.1:25565".into()),
    };
    save_runtime_config(&fixture.paths, fixture.world, &config).unwrap();

    let installer = RuntimeInstaller::new(&fixture.paths, &fixture.storage);
    let status = installer.inspect(fixture.world).unwrap();
    assert!(status.manual_configuration);
    assert!(
        status.ready,
        "valid manual runtime should be launchable without automatic managed re-resolution: {status:?}"
    );
    assert!(status
        .components
        .iter()
        .filter(|component| {
            matches!(
                component.kind,
                swarm_cli::runtime_installer::RuntimeComponentKind::MinecraftServer
                    | swarm_cli::runtime_installer::RuntimeComponentKind::FabricLoader
                    | swarm_cli::runtime_installer::RuntimeComponentKind::FabricApi
                    | swarm_cli::runtime_installer::RuntimeComponentKind::SwarmcraftFabric
            )
        })
        .all(|component| !component.managed));

    let readiness = swarm_cli::host_readiness::local_runtime_readiness(
        &fixture.paths,
        fixture.world,
        fixture.storage.load_world_descriptor(fixture.world).unwrap().compatibility_fingerprint,
    )
    .unwrap();
    assert_eq!(readiness, swarm_network::HostRuntimeReadinessV1::Unverified);
}

#[tokio::test]
async fn concurrent_authority_launch_fails_fast_across_processes_and_recovers_after_release() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer-concurrent"));
    let lock_path = fixture.paths.root.join("control").join(fixture.world.to_hex()).join("authority-runtime.lock");
    let ready_path = temp.path().join("lock-holder-ready");
    let helper_path = temp.path().join("hold-lock.py");
    fs::write(
        &helper_path,
        r#"import fcntl
import pathlib
import sys
import time

lock_path = pathlib.Path(sys.argv[1])
ready_path = pathlib.Path(sys.argv[2])
lock_path.parent.mkdir(parents=True, exist_ok=True)
with lock_path.open("a+") as handle:
    fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
    ready_path.write_text("locked", encoding="utf-8")
    time.sleep(30)
"#,
    )
    .unwrap();
    let mut holder =
        std::process::Command::new("python3").arg(&helper_path).arg(&lock_path).arg(&ready_path).spawn().unwrap();
    for _ in 0..100 {
        if ready_path.is_file() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(ready_path.is_file(), "cross-process lock holder did not become ready");

    let error = run_authority_runtime(
        &fixture.paths,
        &fixture.storage,
        HostOptions {
            world: fixture.world,
            java: temp.path().join("unused-java"),
            server_jar: temp.path().join("unused-server.jar"),
            mod_jar: temp.path().join("unused-bridge.jar"),
            accept_eula: true,
        },
    )
    .await
    .unwrap_err();
    holder.kill().unwrap();
    holder.wait().unwrap();

    assert!(
        error.to_string().contains("already has an active authority runtime"),
        "second process must fail fast with a clear message: {error}"
    );
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);
    assert!(
        load_migration_status(&fixture.paths, fixture.world).is_err(),
        "a raced-out launch must not publish migration status"
    );

    let (java, server, bridge) = manual_runtime_files(temp.path(), "26.1.2", "0.19.3");
    run_authority_runtime(
        &fixture.paths,
        &fixture.storage,
        HostOptions { world: fixture.world, java, server_jar: server, mod_jar: bridge, accept_eula: true },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn authority_lock_io_failure_is_not_misreported_as_contention() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer-lock-io"));
    let control = fixture.paths.root.join("control");
    fs::write(&control, b"not-a-directory").unwrap();

    let error = run_authority_runtime(
        &fixture.paths,
        &fixture.storage,
        HostOptions {
            world: fixture.world,
            java: temp.path().join("unused-java"),
            server_jar: temp.path().join("unused-server.jar"),
            mod_jar: temp.path().join("unused-bridge.jar"),
            accept_eula: true,
        },
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("authority runtime lock directory"),
        "lock-path I/O failure should be surfaced: {error}"
    );
    assert!(!error.to_string().contains("already has an active authority runtime"));
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);
}

fn write_seed_runtime_lock(fixture: &RuntimeFixture, minecraft: &str, sha256: &str) {
    let lock_path =
        fixture.paths.root.join("runtime-components").join(fixture.world.to_hex()).join("runtime-lock.json");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let lock = serde_json::json!({
        "schema_version": 1,
        "world_id": fixture.world.to_string(),
        "minecraft_version": minecraft,
        "artifacts": { "minecraft_server": { "sha256": sha256 } }
    });
    fs::write(lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
}

#[tokio::test]
async fn fresh_runtime_is_seeded_only_from_hash_verified_staged_game_jar() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer-seed"));
    let java = temp.path().join("mock-java");
    let server = temp.path().join("fabric-server.jar");
    let bridge = temp.path().join("swarmcraft-fabric.jar");
    write_mock_java(&java, "26.1.2", "0.19.3");
    fs::write(&server, b"mock-server").unwrap();
    fs::write(&bridge, b"mock-bridge").unwrap();

    let staged_bytes = b"staged-game-jar-bytes";
    let staged_sha256 = hex::encode(Sha256::digest(staged_bytes));
    let staged_dir = fixture.paths.root.join("runtime-components").join(fixture.world.to_hex()).join("server");
    fs::create_dir_all(&staged_dir).unwrap();
    fs::write(staged_dir.join("server.jar"), staged_bytes).unwrap();
    write_seed_runtime_lock(&fixture, "26.1.2", &staged_sha256);

    run_authority_runtime(
        &fixture.paths,
        &fixture.storage,
        HostOptions { world: fixture.world, java, server_jar: server, mod_jar: bridge, accept_eula: true },
    )
    .await
    .unwrap();

    let seeded = runtime_dir(&fixture).join(".fabric/server/26.1.2-server.jar");
    assert_eq!(
        fs::read(&seeded).unwrap(),
        staged_bytes,
        "the Fabric launcher must find the verified staged game jar without re-downloading Minecraft at boot"
    );
}

#[tokio::test]
async fn corrupt_staged_game_jar_is_rejected_before_java_launch() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer-seed-corrupt"));
    let server = temp.path().join("fabric-server.jar");
    let bridge = temp.path().join("swarmcraft-fabric.jar");
    fs::write(&server, b"mock-server").unwrap();
    fs::write(&bridge, b"mock-bridge").unwrap();

    let expected_sha256 = hex::encode(Sha256::digest(b"expected-game-jar-bytes"));
    let staged_dir = fixture.paths.root.join("runtime-components").join(fixture.world.to_hex()).join("server");
    fs::create_dir_all(&staged_dir).unwrap();
    fs::write(staged_dir.join("server.jar"), b"tampered-game-jar-bytes").unwrap();
    write_seed_runtime_lock(&fixture, "26.1.2", &expected_sha256);

    let error = run_authority_runtime(
        &fixture.paths,
        &fixture.storage,
        HostOptions {
            world: fixture.world,
            java: temp.path().join("java-must-not-launch"),
            server_jar: server,
            mod_jar: bridge,
            accept_eula: true,
        },
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("runtime-lock SHA-256 verification"),
        "corrupt staged game jar must fail closed before Java launch: {error}"
    );
    assert_eq!(canonical_hash(&fixture), fixture.baseline_hash);
}
