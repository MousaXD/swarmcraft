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
