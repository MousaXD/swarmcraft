//! The matrix is read-only; neither unsupported nor merely adapter-compatible
//! tuples can publish an imported world through the real import sidecar.
use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn import_command(data: &Path, source: &Path, minecraft: &str, loader: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft-import"));
    command.arg("--data-dir").arg(data).arg("--source").arg(source).args([
        "--name",
        "Matrix fixture",
        "--minecraft",
        minecraft,
        "--fabric-loader",
        loader,
        "--no-server-mods",
    ]);
    command
}

#[test]
fn matrix_does_not_initialize_storage_or_claim_live_acceptance() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("absent");
    let output = Command::new(env!("CARGO_BIN_EXE_swarmcraft-runtime"))
        .arg("--data-dir")
        .arg(&data)
        .arg("compatibility-matrix")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let matrix: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(matrix["adapter_id"], "swarmcraft-fabric-v1");
    assert_eq!(matrix["declared_minecraft_range"], "~26.1.2");
    let entries = matrix["new_world_baselines"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["minecraft_version"], "26.1.2");
    assert_eq!(entries[0]["fabric_loader_version"], "0.19.3");
    assert_eq!(entries[0]["reference_java_major"], 25);
    assert_eq!(entries[0]["qualification"], "pinned_baseline");
    assert!(entries[0]["field_acceptance"].as_str().unwrap().starts_with("pending_"));
    assert!(!data.exists());
}

#[test]
fn real_import_rejects_unqualified_and_incompatible_tuples_without_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("absent");
    let save = tmp.path().join("save");
    fs::create_dir(&save).unwrap();
    fs::write(save.join("level.dat"), b"matrix fixture").unwrap();
    fs::write(save.join("session.lock"), b"fixture lock").unwrap();
    let baseline = import_command(&data, &save, "26.1.2", "0.19.3").arg("--preview").output().unwrap();
    assert!(baseline.status.success());
    let baseline: Value = serde_json::from_slice(&baseline.stdout).unwrap();
    assert_eq!(baseline["state"], "ready");
    let fingerprint = baseline["compatibility_fingerprint"].as_str().unwrap();
    for (minecraft, loader, blocker) in [
        ("26.1.5", "0.19.3", "unqualified_runtime"),
        ("26.1.2", "0.20.1", "unqualified_runtime"),
        ("26.2", "0.19.3", "unsupported_runtime"),
        ("26.1.2", "0.19.2", "unsupported_runtime"),
    ] {
        let preview = import_command(&data, &save, minecraft, loader).arg("--preview").output().unwrap();
        assert!(preview.status.success());
        let preview: Value = serde_json::from_slice(&preview.stdout).unwrap();
        assert_eq!(preview, serde_json::json!({"state":"blocked","blocker":blocker}));
        for reviewed in [false, true] {
            let mut command = import_command(&data, &save, minecraft, loader);
            if reviewed {
                command.args(["--expected-compatibility", fingerprint]);
            }
            let rejected = command.output().unwrap();
            assert!(!rejected.status.success());
            assert!(rejected.stdout.is_empty());
            assert!(!data.exists());
        }
    }
    assert_eq!(fs::read(save.join("level.dat")).unwrap(), b"matrix fixture");
    assert_eq!(fs::read(save.join("session.lock")).unwrap(), b"fixture lock");
}
