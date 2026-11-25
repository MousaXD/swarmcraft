//! Exercise the production acquisition and manual-supply commands on an
//! isolated canonical world. This deliberately makes no external requests.
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const JAR: &str = "504b030414000000000000002100472ecb0d46000000460000000f0000006661627269632e6d6f642e6a736f6e7b22736368656d6156657273696f6e223a312c226964223a227368617265646d6f64222c2276657273696f6e223a22312e30222c22656e7669726f6e6d656e74223a222a227d504b03041400000000000000210005abd66905000000050000000a0000006d61726b65722e7478746578616374504b0102140314000000000000002100472ecb0d46000000460000000f00000000000000000000008001000000006661627269632e6d6f642e6a736f6e504b010214031400000000000000210005abd66905000000050000000a00000000000000000000008001730000006d61726b65722e747874504b0506000000000200020075000000a00000000000";

fn invoke(bin: &str, data: &Path, args: &[&str]) -> Output {
    Command::new(bin).arg("--data-dir").arg(data).args(args).output().unwrap()
}
fn success(output: Output) -> Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn real_manual_fallback_is_exact_idempotent_and_never_starts_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let save = tmp.path().join("save");
    fs::create_dir(&save).unwrap();
    fs::write(save.join("level.dat"), b"isolated save").unwrap();
    fs::write(save.join("session.lock"), b"fixture lock").unwrap();
    let jar = tmp.path().join("mod.jar");
    fs::write(&jar, hex::decode(JAR).unwrap()).unwrap();
    let imported = success(invoke(
        env!("CARGO_BIN_EXE_swarmcraft-import"),
        &data,
        &[
            "--source",
            save.to_str().unwrap(),
            "--name",
            "Acquisition fixture",
            "--minecraft",
            "26.1.2",
            "--fabric-loader",
            "0.19.3",
            "--server-mod",
            jar.to_str().unwrap(),
        ],
    ));
    let world = imported["world_id"].as_str().unwrap();
    let id: swarm_protocol::WorldId = world.parse().unwrap();
    let storage = swarm_storage::Storage::open(data.clone()).unwrap();
    let config = storage.load_world_config(id).unwrap();
    let checkpoint = storage.latest_snapshot(id).unwrap().unwrap();
    let cli = env!("CARGO_BIN_EXE_swarmcraft");
    let runtime = env!("CARGO_BIN_EXE_swarmcraft-runtime");
    assert!(invoke(cli, &data, &["world", "mods-remove", world, "sharedmod"]).status.success());
    let report = success(invoke(runtime, &data, &["acquire-mods", world]));
    assert_eq!(report["world_id"], world);
    assert_eq!(report["ready"], false);
    assert_eq!(report["artifacts"][0]["state"], "manual_required");
    assert_eq!(report["artifacts"][0]["provider"], Value::Null);
    assert_eq!(
        report["artifacts"][0]["artifact_hash"],
        config.compatibility.required_server_mods[0].artifact_hash.to_hex()
    );
    let wrong = tmp.path().join("wrong.jar");
    fs::write(&wrong, b"wrong bytes").unwrap();
    assert!(!invoke(cli, &data, &["world", "mods-add", world, wrong.to_str().unwrap()]).status.success());
    assert!(invoke(cli, &data, &["world", "mods-add", world, jar.to_str().unwrap()]).status.success());
    for _ in 0..2 {
        let report = success(invoke(runtime, &data, &["acquire-mods", world]));
        assert_eq!(report["ready"], true);
        assert_eq!(report["artifacts"][0]["state"], "already_verified");
    }
    assert_eq!(storage.load_world_config(id).unwrap(), config);
    assert_eq!(storage.latest_snapshot(id).unwrap().unwrap(), checkpoint);
    assert!(!data.join("runtime").exists());
    assert!(!data.join("control").join(id.to_hex()).join("hosting-consent.json").exists());
    assert_eq!(fs::read(&jar).unwrap(), hex::decode(JAR).unwrap());
}
