from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
path = ROOT / 'crates/swarm-cli/tests/migration_core.rs'
text = path.read_text()

if 'fn configure_safe_stop_runtime(' not in text:
    anchor = 'fn spawn_heartbeat(paths: DataPaths, world: WorldId, generation: AuthorityGeneration) -> JoinHandle<()> {'
    block = r'''fn configure_safe_stop_runtime(temp: &Path, peer: &PeerFixture, world: WorldId, endpoint: &str) {
    let mock_java = temp.join(format!("safe-stop-java-{}", world.to_hex()));
    write_safe_stop_java(&mock_java);
    let server = temp.join(format!("safe-stop-server-{}.jar", world.to_hex()));
    let fabric = temp.join(format!("safe-stop-fabric-{}.jar", world.to_hex()));
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

fn write_safe_stop_java(path: &Path) {
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

def encoded(value):
    return value.encode("utf-8").hex()

with socket.create_connection((host, port), timeout=5) as connection:
    reader = connection.makefile("r", encoding="utf-8", newline="\n")
    writer = connection.makefile("w", encoding="utf-8", newline="\n")
    writer.write("AUTH\t" + token + "\n")
    writer.write("WORLD_INFO\t" + encoded("26.1.2") + "\t" + encoded("0.19.3") + "\t" + encoded(world) + "\t" + fingerprint + "\n")
    writer.flush()
    for line in reader:
        fields = line.strip().split("\t")
        if len(fields) == 2 and fields[0] == "PREPARE_SHUTDOWN":
            pathlib.Path(world, "safe-stop-latest.txt").write_text("saved-at-shutdown\n", encoding="utf-8")
            writer.write("READY_FOR_SHUTDOWN\t" + fields[1] + "\n")
            writer.flush()
            break
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

'''
    if anchor not in text:
        raise RuntimeError('safe stop fixture anchor moved')
    text = text.replace(anchor, block + anchor, 1)

if 'safe_stop_waits_for_fabric_barrier_then_commits_latest_state_and_sleep_record' not in text:
    anchor = '#[tokio::test]\nasync fn manual_transfer_uses_shared_runner_and_fences_alice() {'
    test = r'''#[tokio::test]
async fn safe_stop_waits_for_fabric_barrier_then_commits_latest_state_and_sleep_record() {
    let temp = tempfile::tempdir().unwrap();
    let alice = peer(temp.path().join("alice-safe-stop"));
    let bob = peer(temp.path().join("bob-safe-stop"));
    let source = temp.path().join("source-safe-stop");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-before-safe-stop\n").unwrap();
    let shared = initialize_two_peer_world(&alice, &bob, &source);
    let before = alice.storage.latest_snapshot(shared.world).unwrap().unwrap();
    let before_hash = before.manifest_hash().unwrap();

    let endpoint = "127.0.0.1:25568";
    configure_safe_stop_runtime(temp.path(), &alice, shared.world, endpoint);
    let generation = AuthorityGeneration {
        epoch: shared.epoch.epoch_number,
        fencing_token: shared.epoch.fencing_token,
    };
    let heartbeat = spawn_heartbeat(alice.paths.clone(), shared.world, generation);
    let supervisor_paths = alice.paths.clone();
    let supervisor = tokio::spawn(async move { migration::supervise(supervisor_paths).await });

    wait_until_ready(&alice, shared.world, endpoint).await;
    migration::request_world_stop(&alice.paths, &alice.storage, shared.world).unwrap();
    wait_until_sleeping(&alice, shared.world).await;

    let status = migration::load_migration_status(&alice.paths, shared.world).unwrap();
    assert_eq!(status.phase, MigrationPhase::Sleeping);
    assert!(!status.runtime_ready);

    let final_snapshot = alice.storage.latest_snapshot(shared.world).unwrap().unwrap();
    alice.storage.verify_snapshot(&final_snapshot).unwrap();
    assert_eq!(final_snapshot.snapshot_number, before.snapshot_number + 1);
    assert_eq!(final_snapshot.previous_snapshot_hash, Some(before_hash));

    let sleep_record = alice.storage.load_sleep_record(shared.world).unwrap();
    assert_eq!(sleep_record.latest_snapshot_hash, final_snapshot.manifest_hash().unwrap());
    assert_eq!(sleep_record.epoch, shared.epoch.epoch_number);
    assert_eq!(sleep_record.fencing_token, shared.epoch.fencing_token);

    let restored = temp.path().join("safe-stop-restored");
    alice.storage.restore_snapshot(&final_snapshot, &restored).unwrap();
    assert_eq!(
        fs::read_to_string(restored.join("safe-stop-latest.txt")).unwrap(),
        "saved-at-shutdown\n"
    );
    assert_eq!(
        fs::read_to_string(restored.join("level.dat")).unwrap(),
        "canonical-before-safe-stop\n"
    );

    supervisor.abort();
    heartbeat.abort();
}

'''
    if anchor not in text:
        raise RuntimeError('safe stop test anchor moved')
    text = text.replace(anchor, test + anchor, 1)

path.write_text(text)
