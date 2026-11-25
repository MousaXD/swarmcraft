//! Real sidecar checks of creator-anchored compatibility and exact local JAR bytes.
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const EXACT: &str = "504b030414000000000000002100472ecb0d46000000460000000f0000006661627269632e6d6f642e6a736f6e7b22736368656d6156657273696f6e223a312c226964223a227368617265646d6f64222c2276657273696f6e223a22312e30222c22656e7669726f6e6d656e74223a222a227d504b03041400000000000000210005abd66905000000050000000a0000006d61726b65722e7478746578616374504b0102140314000000000000002100472ecb0d46000000460000000f00000000000000000000008001000000006661627269632e6d6f642e6a736f6e504b010214031400000000000000210005abd66905000000050000000a00000000000000000000008001730000006d61726b65722e747874504b0506000000000200020075000000a00000000000";
const ALTERED: &str = "504b030414000000000000002100472ecb0d46000000460000000f0000006661627269632e6d6f642e6a736f6e7b22736368656d6156657273696f6e223a312c226964223a227368617265646d6f64222c2276657273696f6e223a22312e30222c22656e7669726f6e6d656e74223a222a227d504b030414000000000000002100ad9011ca07000000070000000a0000006d61726b65722e747874616c7465726564504b0102140314000000000000002100472ecb0d46000000460000000f00000000000000000000008001000000006661627269632e6d6f642e6a736f6e504b0102140314000000000000002100ad9011ca07000000070000000a00000000000000000000008001730000006d61726b65722e747874504b0506000000000200020075000000a20000000000";
const DIFFERENT_VERSION: &str = "504b0304140000000000000021007a172e7b46000000460000000f0000006661627269632e6d6f642e6a736f6e7b22736368656d6156657273696f6e223a312c226964223a227368617265646d6f64222c2276657273696f6e223a22322e30222c22656e7669726f6e6d656e74223a222a227d504b03041400000000000000210005abd66905000000050000000a0000006d61726b65722e7478746578616374504b01021403140000000000000021007a172e7b46000000460000000f00000000000000000000008001000000006661627269632e6d6f642e6a736f6e504b010214031400000000000000210005abd66905000000050000000a00000000000000000000008001730000006d61726b65722e747874504b0506000000000200020075000000a00000000000";

fn profile(data: &Path, world: &str, folder: Option<&Path>) -> Value {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_swarmcraft-runtime"));
    cmd.arg("--data-dir").arg(data).args(["client-profile", world]);
    if let Some(folder) = folder {
        cmd.arg("--mods-dir").arg(folder);
    }
    let output = cmd.output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["world_id"], world);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(data.to_str().unwrap()));
    result
}

#[test]
fn real_client_profile_is_advisory_exact_and_preserves_launcher_files() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let source = temp.path().join("source");
    let mods = temp.path().join("launcher instance").join("mods");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&mods).unwrap();
    fs::write(source.join("level.dat"), b"fixture save").unwrap();
    fs::write(source.join("session.lock"), b"fixture lock").unwrap();
    let original = temp.path().join("sharedmod.jar");
    fs::write(&original, hex::decode(EXACT).unwrap()).unwrap();
    let imported = Command::new(env!("CARGO_BIN_EXE_swarmcraft-import"))
        .arg("--data-dir")
        .arg(&data)
        .arg("--source")
        .arg(&source)
        .args(["--name", "Client test", "--minecraft", "26.1.2", "--fabric-loader", "0.19.3", "--server-mod"])
        .arg(&original)
        .output()
        .unwrap();
    assert!(imported.status.success(), "{}", String::from_utf8_lossy(&imported.stderr));
    let imported: Value = serde_json::from_slice(&imported.stdout).unwrap();
    let world = imported["world_id"].as_str().unwrap();
    let world_id: swarm_protocol::WorldId = world.parse().unwrap();
    let storage = swarm_storage::Storage::open(data.clone()).unwrap();
    let config = storage.load_world_config(world_id).unwrap();
    let checkpoint = storage.latest_snapshot(world_id).unwrap().unwrap();
    let account = mods.parent().unwrap().join("launcher_accounts.json");
    let settings = mods.parent().unwrap().join("options.txt");
    fs::write(&account, b"synthetic-account-secret-not-for-inspection").unwrap();
    fs::write(&settings, b"synthetic launcher settings").unwrap();

    let unchecked = profile(&data, world, None);
    assert_eq!(unchecked["inventory"], "not_checked");
    assert_eq!(unchecked["minimum_java_major"], 25);
    assert_eq!(unchecked["minecraft_version"], "26.1.2");
    assert_eq!(unchecked["required_mods"][0]["mod_id"], "sharedmod");
    assert_eq!(unchecked["compatibility_fingerprint"], imported["compatibility_fingerprint"]);
    assert_eq!(profile(&data, world, Some(&mods))["issues"][0]["code"], "missing_required");

    let jar = mods.join("sharedmod.jar");
    let exact = hex::decode(EXACT).unwrap();
    fs::write(&jar, &exact).unwrap();
    assert_eq!(profile(&data, world, Some(&mods))["inventory"], "exact_match");
    fs::write(&jar, hex::decode(ALTERED).unwrap()).unwrap();
    assert_eq!(profile(&data, world, Some(&mods))["issues"][0]["code"], "hash_mismatch");
    fs::write(&jar, hex::decode(DIFFERENT_VERSION).unwrap()).unwrap();
    assert_eq!(profile(&data, world, Some(&mods))["issues"][0]["code"], "version_mismatch");
    fs::write(&jar, &exact).unwrap();
    fs::write(mods.join("duplicate.jar"), &exact).unwrap();
    assert_eq!(profile(&data, world, Some(&mods))["issues"][0]["code"], "duplicate_mod");
    fs::remove_file(mods.join("duplicate.jar")).unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&account, mods.join("not-a-mod.jar")).unwrap();
        assert_eq!(profile(&data, world, Some(&mods))["issues"][0]["code"], "invalid_jar");
        fs::remove_file(mods.join("not-a-mod.jar")).unwrap();
    }
    assert_eq!(profile(&data, world, Some(&mods))["inventory"], "exact_match");
    assert_eq!(fs::read(&jar).unwrap(), exact);
    assert_eq!(fs::read(&account).unwrap(), b"synthetic-account-secret-not-for-inspection");
    assert_eq!(fs::read(&settings).unwrap(), b"synthetic launcher settings");
    assert_eq!(storage.load_world_config(world_id).unwrap(), config);
    assert_eq!(storage.latest_snapshot(world_id).unwrap().unwrap(), checkpoint);
    assert!(!data.join("runtime").exists());

    // A signed local config with the wrong compatibility must not become a
    // client preparation profile. Use an isolated fixture to exercise denial.
    let mut incompatible = config.clone();
    incompatible.compatibility.minecraft_version = "26.1.3".into();
    incompatible.sequence += 1;
    incompatible.previous_config_hash = Some(config.config_hash().unwrap());
    let identity = swarm_core::PeerIdentity::load_existing(&swarm_core::DataPaths::from_root(data.clone())).unwrap();
    swarm_core::sign_world_config(&identity, &mut incompatible).unwrap();
    storage.save_world_config(&incompatible).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_swarmcraft-runtime"))
        .arg("--data-dir")
        .arg(&data)
        .args(["client-profile", world])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
}
