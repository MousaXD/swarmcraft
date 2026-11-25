//! Real import sidecar contract: preflight must not initialize storage.
use std::{fs, path::Path, process::Command};

fn sidecar(source: &Path, data: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft-import"));
    command.arg("--data-dir").arg(data).arg("--source").arg(source).args([
        "--name",
        "Reviewed world",
        "--minecraft",
        "26.1.2",
        "--fabric-loader",
        "0.19.3",
        "--visibility",
        "private",
        "--no-server-mods",
    ]);
    command
}

#[test]
fn preview_and_reviewed_commit_use_real_sidecar_without_eula_or_runtime_setup() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("save with spaces");
    let data = temp.path().join("data");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("level.dat"), b"fixture save bytes").unwrap();
    fs::write(source.join("session.lock"), b"fixture lock").unwrap();
    let result = sidecar(&source, &data).arg("--preview").output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let preview: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(preview["state"], "ready");
    assert_eq!(preview["source_files"], 2);
    assert_eq!(preview["minimum_java_major"], 25);
    assert_eq!(preview["required_server_mods"], serde_json::json!([]));
    assert!(!data.exists());
    assert!(!String::from_utf8_lossy(&result.stdout).contains(source.to_str().unwrap()));

    let rejected = sidecar(&source, &data).arg("--expected-compatibility").arg("0".repeat(64)).output().unwrap();
    assert!(!rejected.status.success());
    assert!(!data.exists());
    let fingerprint = preview["compatibility_fingerprint"].as_str().unwrap();
    let accepted = sidecar(&source, &data).arg("--expected-compatibility").arg(fingerprint).output().unwrap();
    assert!(accepted.status.success(), "{}", String::from_utf8_lossy(&accepted.stderr));
    let imported: serde_json::Value = serde_json::from_slice(&accepted.stdout).unwrap();
    assert_eq!(imported["compatibility_fingerprint"], fingerprint);
    let world: swarm_protocol::WorldId = imported["world_id"].as_str().unwrap().parse().unwrap();
    let storage = swarm_storage::Storage::open(data.clone()).unwrap();
    let snapshot = storage.latest_snapshot(world).unwrap().unwrap();
    storage.verify_snapshot(&snapshot).unwrap();
    swarm_core::verify_snapshot_signature(&snapshot).unwrap();
    assert!(!data.join("runtime").exists());
    assert!(!data.join("control").join(world.to_hex()).join("hosting-consent.json").exists());
    assert_eq!(fs::read(source.join("level.dat")).unwrap(), b"fixture save bytes");
    assert_eq!(fs::read(source.join("session.lock")).unwrap(), b"fixture lock");
    assert!(!source.join("eula.txt").exists());
}
