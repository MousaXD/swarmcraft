//! Public CLI contract: independently initialized signers, no playable world.
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use swarm_core::{DataPaths, PeerIdentity};

fn invoke(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swarmcraft")).arg("--data-dir").arg(data).args(args).output().unwrap()
}
fn success(out: Output) -> Vec<u8> {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}
fn rejected(out: Output, reason: &str) {
    assert!(!out.status.success(), "unexpected success");
    assert!(String::from_utf8_lossy(&out.stderr).contains(reason), "{}", String::from_utf8_lossy(&out.stderr));
}
fn json(out: Output) -> Value {
    serde_json::from_slice(&success(out)).unwrap()
}

#[test]
fn independent_signers_create_verified_offline_candidate_without_world_or_permit() {
    let tmp = tempfile::tempdir().unwrap();
    let founder = tmp.path().join("founder");
    let keys: Vec<_> = (0..3)
        .map(|i| {
            let data = tmp.path().join(format!("validator-{i}"));
            let id = PeerIdentity::load_or_create(&DataPaths::from_root(&data)).unwrap();
            (data, hex::encode(id.public_key()))
        })
        .collect();
    let outsider = tmp.path().join("outsider");
    PeerIdentity::load_or_create(&DataPaths::from_root(&outsider)).unwrap();
    let draft = tmp.path().join("draft.json");
    let certificate = tmp.path().join("certificate.json");
    let hash = "a".repeat(64);
    let state = "b".repeat(64);
    let output = success(invoke(
        &founder,
        &[
            "ownerless",
            "draft",
            "--name",
            "No OP",
            "--compatibility-fingerprint",
            &hash,
            "--initial-state-commitment",
            &state,
            "--validator-key",
            &keys[2].1,
            "--validator-key",
            &keys[0].1,
            "--validator-key",
            &keys[1].1,
        ],
    ));
    fs::write(&draft, output).unwrap();
    let draft_path = draft.to_str().unwrap();
    let report = json(invoke(&founder, &["ownerless", "inspect", "--draft", draft_path]));
    let world = report["world_id"].as_str().unwrap();
    assert!(world.starts_with("scownerless:"));
    assert_eq!(report["required_creation_approvals"], 3);
    assert_eq!(report["proposed_maintenance_threshold"], 2);
    assert_eq!(report["runtime_activation"], false);
    assert!(!founder.exists());

    let args = ["ownerless", "approve", "--draft", draft_path, "--expected-world", world, "--confirm"];
    rejected(invoke(&founder, &args), "peer.key");
    assert!(!founder.exists());
    rejected(invoke(&outsider, &args), "not an enrolled validator");
    rejected(invoke(&keys[0].0, &args[..args.len() - 1]), "--confirm");
    let wrong_pin = format!("scownerless:{}", "d".repeat(64));
    rejected(
        invoke(
            &keys[0].0,
            &["ownerless", "approve", "--draft", draft_path, "--expected-world", &wrong_pin, "--confirm"],
        ),
        "differs from expected",
    );

    let mut approvals = Vec::new();
    for (i, (data, _)) in keys.iter().enumerate() {
        let before = fs::read(DataPaths::from_root(data).identity_key()).unwrap();
        let path = tmp.path().join(format!("approval-{i}.json"));
        fs::write(&path, success(invoke(data, &args))).unwrap();
        assert_eq!(fs::read(DataPaths::from_root(data).identity_key()).unwrap(), before);
        assert_eq!(fs::read_dir(data.join("worlds")).unwrap().count(), 0);
        approvals.push(path);
    }
    let a: Vec<_> = approvals.iter().map(|p| p.to_str().unwrap()).collect();
    let assemble = ["ownerless", "assemble", "--draft", draft_path, "--expected-world", world];
    let command = |files: &[&str]| {
        let mut args = assemble.to_vec();
        for file in files {
            args.extend(["--approval", file]);
        }
        invoke(&founder, &args)
    };
    rejected(command(&a[..2]), "every validator");
    rejected(command(&[a[0], a[0], a[1]]), "approval set");
    fs::write(&certificate, success(command(&[a[2], a[0], a[1]]))).unwrap();
    let certificate_path = certificate.to_str().unwrap();
    let verify_args = ["ownerless", "verify", "--certificate", certificate_path, "--expected-world", world];
    let verified = json(invoke(&founder, &verify_args));
    assert_eq!(verified["classification"], "verified_ownerless_creation_candidate");
    assert_eq!(verified["verified_creation_approvals"], 3);
    assert_eq!(verified["canonical_world_created"], false);
    assert_eq!(verified["runtime_activation"], false);
    assert_eq!(verified["recovery_authorized"], false);
    assert_eq!(verified["policy"]["player_op"], false);
    assert_eq!(verified["policy"]["compute_grants_vote"], false);
    assert!(!founder.exists());
    rejected(
        invoke(&founder, &["world", "create", "--ownerless", "--name", "No OP", "--minecraft", "26.1.2"]),
        "No world created",
    );
    assert!(!founder.exists());
    rejected(
        invoke(&founder, &["ownerless", "verify", "--certificate", certificate_path, "--expected-world", &wrong_pin]),
        "expected identity",
    );
    let mut corrupt: Value = serde_json::from_slice(&fs::read(&certificate).unwrap()).unwrap();
    corrupt["genesis"]["policy"]["player_op"] = true.into();
    fs::write(&certificate, serde_json::to_vec(&corrupt).unwrap()).unwrap();
    rejected(invoke(&founder, &verify_args), "protected no-OP policy differs");
    fs::write(&certificate, vec![b' '; 65_537]).unwrap();
    rejected(invoke(&founder, &verify_args), "size limit");
    assert!(!founder.exists());
}

#[test]
fn ordinary_world_creation_remains_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("normal");
    let created = success(invoke(
        &data,
        &["world", "create", "--name", "Ordinary V1", "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"],
    ));
    assert!(String::from_utf8_lossy(&created).contains("World ID: scworld:"));
    assert_eq!(fs::read_dir(data.join("worlds")).unwrap().count(), 1);
}
