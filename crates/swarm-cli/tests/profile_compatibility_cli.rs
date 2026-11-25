//! Real sidecar preflight: incompatible profiles are not initialized or repaired.
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn run(binary: &str, root: &Path, args: &[&str]) -> Output {
    Command::new(binary).arg("--data-dir").arg(root).args(args).output().unwrap()
}
fn cli(root: &Path, args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_swarmcraft"), root, args)
}

#[test]
fn preflight_and_all_world_sidecars_deny_future_profile_before_any_mutation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("profile");
    let fresh = cli(&root, &["profile", "check"]);
    assert!(fresh.status.success());
    let report: Value = serde_json::from_slice(&fresh.stdout).unwrap();
    assert_eq!(report["initialized"], false);
    assert!(!root.exists());
    fs::create_dir(&root).unwrap();
    let future =
        br#"{"format_schema":1,"minimum_reader_generation":2,"storage_schema_version":1,"protocol_version":1}"#;
    fs::write(root.join("profile-compatibility.json"), future).unwrap();

    for args in [
        vec!["profile", "check"],
        vec!["profile", "check", "--target", "published-0.5.0"],
        vec!["init"],
        vec!["identity"],
        vec!["world", "list"],
        vec!["world", "create", "--name", "blocked", "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"],
    ] {
        let result = cli(&root, &args);
        assert!(!result.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("minimum reader generation 2"));
    }
    let world = format!("scworld:{}", "00".repeat(32));
    let runtime = run(env!("CARGO_BIN_EXE_swarmcraft-runtime"), &root, &["status", &world]);
    assert!(!runtime.status.success());
    assert!(String::from_utf8_lossy(&runtime.stderr).contains("minimum reader generation 2"));

    let source = tmp.path().join("save");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("level.dat"), b"fixture").unwrap();
    fs::write(source.join("session.lock"), b"fixture").unwrap();
    let source_string = source.to_str().unwrap();
    let args = [
        "--source",
        source_string,
        "--name",
        "blocked",
        "--minecraft",
        "26.1.2",
        "--fabric-loader",
        "0.19.3",
        "--no-server-mods",
    ];
    let mut preview = args.to_vec();
    preview.push("--preview");
    assert!(run(env!("CARGO_BIN_EXE_swarmcraft-import"), &root, &preview).status.success());
    let result = run(env!("CARGO_BIN_EXE_swarmcraft-import"), &root, &args);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("minimum reader generation 2"));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(fs::read(root.join("profile-compatibility.json")).unwrap(), future);
}

#[test]
fn actual_profile_check_refuses_both_historical_targets_after_current_open() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("profile");
    assert!(cli(&root, &["init"]).status.success());
    let marker = root.join("profile-compatibility.json");
    let before = fs::read(&marker).unwrap();
    for target in ["published-0.5.0", "published-0.5.1"] {
        let result = cli(&root, &["profile", "check", "--target", target]);
        assert!(!result.status.success());
        let reason = String::from_utf8_lossy(&result.stderr);
        assert!(reason.contains("unsupported downgrade into an initialized profile"));
        assert!(reason.contains("verified checkpoint export"));
    }
    assert_eq!(fs::read(marker).unwrap(), before);
    assert!(cli(&root, &["profile", "check"]).status.success());
}
