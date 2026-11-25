//! Real public-CLI tests: no fake persistent preferences or simulated host admission.
use std::{
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_cli::authority_permit::{permit_path, refresh_permit};
use swarm_consensus::AuthorityGeneration;
use swarm_core::DataPaths;
use swarm_protocol::WorldId;
use swarm_storage::Storage;

fn invoke(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .output()
        .expect("launch real SwarmCraft CLI")
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "real CLI exited unsuccessfully: {}", String::from_utf8_lossy(&output.stderr));
}

fn fresh_world(data: &Path) -> String {
    assert_success(&invoke(data, &["init"]));
    let created = invoke(
        data,
        &[
            "world",
            "create",
            "--name",
            "Hosting CLI explicit consent",
            "--minecraft",
            "26.1.2",
            "--fabric-loader",
            "0.19.3",
        ],
    );
    assert_success(&created);
    let output = String::from_utf8(created.stdout).expect("CLI output is UTF-8");
    output.lines().find_map(|line| line.strip_prefix("World ID: ")).expect("real world ID").to_owned()
}

fn status_json(data: &Path, world: &str) -> serde_json::Value {
    let status = invoke(data, &["world", "hosting", "status", world, "--json"]);
    assert_success(&status);
    serde_json::from_slice(&status.stdout).expect("typed JSON readiness from Rust backend")
}

#[test]
fn new_world_is_player_only_until_explicit_public_cli_action() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let world = fresh_world(&data);
    let status = status_json(&data, &world);
    assert_eq!(status["selected_heap_mib"], 0);
    assert_eq!(status["consented"], false);
    assert_eq!(status["may_host_now"], false);
    assert_eq!(status["hard_process_memory_cap_guaranteed"], false);
    assert!(status["reason"].as_str().unwrap().contains("Player-only"));
    assert!(status.get("peer_id").is_none());

    let set_zero = invoke(&data, &["world", "hosting", "set", &world, "--heap-mib", "0", "--json"]);
    assert_success(&set_zero);
    let saved: serde_json::Value = serde_json::from_slice(&set_zero.stdout).unwrap();
    assert_eq!(saved["consented"], false);
    assert_eq!(saved["selected_heap_mib"], 0);
    let revision = saved["consent_revision"].as_str().expect("persisted decimal-string revision");
    assert!(revision.parse::<u64>().unwrap() > 0);
    assert!(saved.get("peer_id").is_none());
    let reloaded = status_json(&data, &world);
    assert_eq!(reloaded["consent_revision"], revision);
    assert_eq!(reloaded["may_host_now"], false);
}

#[test]
fn invalid_heap_or_world_never_mutates_local_consent() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let world = fresh_world(&data);
    for heap in ["-1", "65537", "abc"] {
        let invalid = invoke(&data, &["world", "hosting", "set", &world, "--heap-mib", heap]);
        assert!(!invalid.status.success(), "invalid heap unexpectedly accepted");
    }
    for invalid_world in ["../escape", "scworld:", "not-a-world"] {
        let invalid = invoke(&data, &["world", "hosting", "status", invalid_world, "--json"]);
        assert!(!invalid.status.success(), "invalid world unexpectedly accepted");
    }
    let status = status_json(&data, &world);
    assert_eq!(status["consent_revision"], serde_json::Value::Null);
    assert_eq!(status["consented"], false);
    assert_eq!(status["may_host_now"], false);
}

#[test]
fn opt_in_requires_actual_local_world_record() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let world = fresh_world(&data);
    let other = tempfile::tempdir().unwrap();
    let other_data = other.path().join("other");
    assert_success(&invoke(&other_data, &["init"]));
    let denied = invoke(&other_data, &["world", "hosting", "set", &world, "--heap-mib", "0"]);
    assert!(!denied.status.success(), "nonexistent local world accepted hosting preference");
}

#[cfg(unix)]
#[test]
fn explicit_archive_preserves_retained_bytes_and_never_changes_signed_history() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let world_text = fresh_world(&data);
    let world: WorldId = world_text.parse().unwrap();
    let paths = DataPaths::from_root(data.clone());
    let storage = Storage::open(data.clone()).unwrap();
    let epoch = storage.load_epoch_record(world).unwrap();
    let config = storage.load_world_config(world).unwrap();
    let membership = storage.load_membership_record(world).unwrap();
    let checkpoint = storage.latest_snapshot(world).unwrap().map(|v| v.manifest_hash().unwrap());
    let runtime = data.join("runtime").join(world.to_hex());
    let save = runtime.join("world");
    let control = data.join("control").join(world.to_hex());
    swarm_core::create_private_dir_all(&save).unwrap();
    swarm_core::create_private_dir_all(&control).unwrap();
    fs::set_permissions(&control, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(save.join("level.dat"), b"unconfirmed-world-data").unwrap();
    fs::write(save.join("session.lock"), b"").unwrap();
    fs::write(runtime.join("latest.log"), b"local runtime log").unwrap();
    let marker = control.join("shutdown-checkpoint-pending.json");
    fs::write(
        &marker,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "world_id": world,
            "epoch": epoch.epoch_number, "fencing_token": epoch.fencing_token,
            "authority_peer_id": epoch.authority_peer_id,
        }))
        .unwrap(),
    )
    .unwrap();
    let run_archive = || {
        invoke(&data, &["world", "archive-unconfirmed-runtime", &world_text, "--confirm-unconfirmed-data", "--json"])
    };
    assert!(!invoke(&data, &["world", "archive-unconfirmed-runtime", &world_text]).status.success());
    assert!(marker.exists());

    // A still-live recorded process must deny even when no supervisor holds
    // the advisory runtime lock. This fixture only refers to our test PID.
    let process_record = control.join("runtime-process.json");
    fs::write(
        &process_record,
        serde_json::to_vec(&serde_json::json!({
            "version": 1, "controller_pid": std::process::id(), "java_pid": std::process::id()
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(!run_archive().status.success());
    assert!(save.join("level.dat").exists());
    assert!(marker.exists());
    fs::remove_file(&process_record).unwrap();

    // Minecraft-compatible exclusive POSIX record lock held by a separate
    // OS process, not a same-process flock or a mock liveness verdict.
    let ready = temp.path().join("lock-ready");
    let release = temp.path().join("lock-release");
    let mut owner = ReplicaDaemon(
        Command::new("python3")
            .args([
                "-c",
                r#"
import fcntl, pathlib, sys, time
with open(sys.argv[1], 'r+b') as f:
    fcntl.lockf(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    pathlib.Path(sys.argv[2]).write_text("ready")
    end = time.monotonic() + 30
    while not pathlib.Path(sys.argv[3]).exists() and time.monotonic() < end:
        time.sleep(0.02)
"#,
            ])
            .arg(save.join("session.lock"))
            .arg(&ready)
            .arg(&release)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready.exists() {
        assert!(owner.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline, "fixture did not acquire Minecraft session lock");
        thread::sleep(Duration::from_millis(20));
    }
    let refused = run_archive();
    assert!(!refused.status.success(), "archival bypassed the Minecraft session lock");
    assert!(marker.exists());
    assert!(save.join("level.dat").exists());
    fs::write(&release, "").unwrap();
    assert!(owner.0.wait().unwrap().success());

    let archived = run_archive();
    assert_success(&archived);
    let result: swarm_cli::migration::RuntimeArchiveResult = serde_json::from_slice(&archived.stdout).unwrap();
    assert_eq!(result.world_id, world_text);
    assert!(result.unconfirmed_save);
    assert_eq!(fs::read(result.world_save_path.join("level.dat")).unwrap(), b"unconfirmed-world-data");
    assert_eq!(fs::read(result.archive_path.join("latest.log")).unwrap(), b"local runtime log");
    assert!(!runtime.exists());
    assert!(!marker.exists());
    assert!(!run_archive().status.success());
    assert_eq!(storage.load_epoch_record(world).unwrap(), epoch);
    assert_eq!(storage.load_world_config(world).unwrap(), config);
    assert_eq!(storage.load_membership_record(world).unwrap(), membership);
    assert_eq!(storage.latest_snapshot(world).unwrap().map(|v| v.manifest_hash().unwrap()), checkpoint);
    assert!(storage.load_sleep_record(world).is_err());
    let status = status_json(&data, &world_text);
    assert_eq!(status["checkpoint_state"], "no_pending_checkpoint");
    assert_eq!(status["consented"], false);
    assert_eq!(status["may_host_now"], false);
    assert!(paths.root.join("runtime-archives").exists());
}

struct ReplicaDaemon(Child);

impl Drop for ReplicaDaemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_replica_daemon(data: &Path, world: &str, permit: &Path) -> ReplicaDaemon {
    let mut daemon = ReplicaDaemon(
        Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
            .arg("--data-dir")
            .arg(data)
            .args(["daemon", "--replica-only", "--listen", "/ip4/127.0.0.1/udp/0/quic-v1"])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(daemon.0.try_wait().unwrap().is_none(), "replica daemon stopped during startup");
        let status = status_json(data, world);
        if !permit.exists() && status["reason"].as_str().unwrap().contains("replica-only") {
            return daemon;
        }
        assert!(Instant::now() < deadline, "replica daemon did not establish its no-hosting profile");
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn replica_only_daemon_restarts_without_java_and_refuses_public_hosting_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let world = fresh_world(&data);
    let world_id: WorldId = world.parse().unwrap();
    let paths = DataPaths::from_root(data.clone());
    let storage = Storage::open(data.clone()).unwrap();
    let epoch = storage.load_epoch_record(world_id).unwrap();
    let generation = AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token };
    let permit = permit_path(&paths, world_id);
    assert_success(&invoke(&data, &["world", "hosting", "set", &world, "--heap-mib", "0"]));
    let initial = status_json(&data, &world);
    let revision = initial["consent_revision"].clone();

    // No Java installation, server JAR or Fabric JAR is necessary for a
    // storage voter. Even a stale launch config must not start a supervisor.
    swarm_cli::migration::save_runtime_config(
        &paths,
        world_id,
        &swarm_cli::migration::RuntimeLaunchConfig {
            java: data.join("nonexistent-java"),
            server_jar: data.join("nonexistent-server.jar"),
            mod_jar: data.join("nonexistent-fabric.jar"),
            accept_eula: true,
            game_endpoint: None,
        },
    )
    .unwrap();
    for heartbeat in [1, 2] {
        // A previous process may have left a stale permit. A replica-only
        // start must remove it and must never renew it.
        refresh_permit(&paths, world_id, generation, heartbeat).unwrap();
        assert!(permit.exists());
        let mut daemon = start_replica_daemon(&data, &world, &permit);
        let status = status_json(&data, &world);
        assert_eq!(status["consent_revision"], revision);
        assert_eq!(status["may_host_now"], false);
        let denied = invoke(&data, &["world", "hosting", "set", &world, "--heap-mib", "512"]);
        assert!(!denied.status.success());
        assert!(String::from_utf8_lossy(&denied.stderr).contains("replica-only"));
        assert_eq!(status_json(&data, &world)["consent_revision"], revision);
        assert_eq!(storage.load_epoch_record(world_id).unwrap(), epoch);
        thread::sleep(Duration::from_millis(1200));
        assert!(daemon.0.try_wait().unwrap().is_none());
        assert!(!permit.exists());
        assert!(!paths.root.join("runtime").join(world_id.to_hex()).exists());
        drop(daemon);
    }
    assert!(!status_json(&data, &world)["reason"].as_str().unwrap().contains("replica-only"));
    assert_eq!(storage.load_epoch_record(world_id).unwrap(), epoch);
}
