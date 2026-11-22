//! Real public-CLI tests: no fake persistent preferences or simulated host admission.
use std::{
    path::Path,
    process::{Command, Output},
};

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
