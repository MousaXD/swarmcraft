use rand_core::{OsRng, RngCore};
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{Hash32, WorldId};
use swarm_storage::{SnapshotContext, Storage};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(root)
        .args(args)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap()
}
fn success(root: &Path, args: &[&str]) -> String {
    let output = run(root, args);
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}
fn json(root: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&success(root, args)).unwrap()
}
fn denied(root: &Path, args: &[&str]) {
    let result = run(root, args);
    assert!(!result.status.success(), "unexpected success: {args:?}");
}
fn create(root: &Path, name: &str) -> WorldId {
    success(root, &["world", "create", "--name", name, "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"])
        .lines()
        .find_map(|line| line.strip_prefix("World ID: "))
        .unwrap()
        .parse()
        .unwrap()
}
fn commit(
    storage: &Storage,
    identity: &PeerIdentity,
    world: WorldId,
    source: &Path,
) -> swarm_protocol::SnapshotManifestV1 {
    let prior = storage.latest_snapshot(world).unwrap().unwrap();
    let mut snapshot = storage
        .snapshot_directory(
            source,
            SnapshotContext {
                world,
                snapshot_number: prior.snapshot_number + 1,
                epoch: prior.epoch,
                sequence: prior.sequence + 1,
                previous_snapshot_hash: Some(prior.manifest_hash().unwrap()),
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
            },
        )
        .unwrap();
    identity.sign_snapshot(&mut snapshot).unwrap();
    storage.commit_snapshot(&snapshot).unwrap();
    snapshot.manifest().clone()
}

#[test]
fn encrypted_transport_restores_pinned_old_checkpoint_without_opening_recipient_store() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("origin");
    let recipient = temp.path().join("recipient");
    let world = create(&root, "Private checkpoint backup");
    let world_id = world.to_string();
    let storage = Storage::open(&root).unwrap();
    let identity = PeerIdentity::load_or_create(&DataPaths::from_root(root.clone())).unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("region")).unwrap();
    let mut random_data = vec![0u8; 640 * 1024];
    OsRng.fill_bytes(&mut random_data);
    let private_text = b"unique world contents must stay private";
    fs::write(source.join("level.dat"), private_text).unwrap();
    fs::write(source.join("region/r.0.0.mca"), &random_data).unwrap();
    fs::write(source.join("region/duplicate.dat"), &random_data).unwrap();
    fs::write(source.join("empty.dat"), []).unwrap();
    fs::write(source.join("compressed.dat"), vec![0x5a; 500 * 1024]).unwrap();
    let old = commit(&storage, &identity, world, &source);
    fs::write(source.join("level.dat"), b"newer checkpoint must not be restored").unwrap();
    let latest = commit(&storage, &identity, world, &source);
    let checkpoint = old.manifest_hash().unwrap().to_string();
    let original_history = storage.list_snapshots(world).unwrap();
    let original_epoch = storage.load_epoch_record(world).unwrap();
    let original_membership = storage.load_membership_record(world).unwrap();
    let original_config = storage.load_world_config(world).unwrap();

    let key = temp.path().join("offline.key");
    let second_key = temp.path().join("wrong.key");
    json(&recipient, &["backup", "keygen", "--destination", key.to_str().unwrap()]);
    json(&recipient, &["backup", "keygen", "--destination", second_key.to_str().unwrap()]);
    assert!(!recipient.exists(), "keygen opened canonical storage");
    let secret = fs::read(&key).unwrap();
    assert_eq!(secret.len(), 32);
    assert_ne!(secret, fs::read(&second_key).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
    }
    denied(&recipient, &["backup", "keygen", "--destination", key.to_str().unwrap()]);
    assert_eq!(fs::read(&key).unwrap(), secret);

    let bundle = temp.path().join("checkpoint.scbackup");
    let export = json(
        &root,
        &[
            "backup",
            "export",
            &world_id,
            "--checkpoint",
            &checkpoint,
            "--key-file",
            key.to_str().unwrap(),
            "--destination",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(export["operation"], "encrypted_export");
    assert_eq!(export["source_snapshot_number"], 2);
    assert_eq!(export["historical_manifest_count"], 3);
    assert_eq!(export["authority_granted"], false);
    assert_eq!(export["registered_world"], false);
    assert_eq!(export["files"], 5);
    assert!(export["encoded_payload_bytes"].as_u64().unwrap() < export["restored_payload_bytes"].as_u64().unwrap());
    let encrypted = fs::read(&bundle).unwrap();
    assert!(encrypted.starts_with(b"SCBKUP01"));
    assert!(!encrypted.windows(private_text.len()).any(|bytes| bytes == private_text));
    assert!(!encrypted.windows(b"level.dat".len()).any(|bytes| bytes == b"level.dat"));
    assert!(!encrypted.windows(32).any(|bytes| bytes == &secret[..]));
    denied(
        &root,
        &[
            "backup",
            "export",
            &world_id,
            "--checkpoint",
            &checkpoint,
            "--key-file",
            key.to_str().unwrap(),
            "--destination",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(fs::read(&bundle).unwrap(), encrypted);

    let repeated = temp.path().join("repeated.scbackup");
    json(
        &root,
        &[
            "backup",
            "export",
            &world_id,
            "--checkpoint",
            &checkpoint,
            "--key-file",
            key.to_str().unwrap(),
            "--destination",
            repeated.to_str().unwrap(),
        ],
    );
    let second_ciphertext = fs::read(&repeated).unwrap();
    assert_eq!(encrypted.len(), second_ciphertext.len());
    assert_ne!(&encrypted[8..24], &second_ciphertext[8..24]);

    let restored = temp.path().join("restored");
    let import_args = |path: &Path, key: &Path, output: &Path, world: &str, hash: &str| {
        vec![
            "backup".to_string(),
            "import".into(),
            "--expected-world".into(),
            world.into(),
            "--expected-checkpoint".into(),
            hash.into(),
            "--key-file".into(),
            key.to_str().unwrap().into(),
            "--bundle".into(),
            path.to_str().unwrap().into(),
            "--destination".into(),
            output.to_str().unwrap().into(),
            "--confirm-data-only".into(),
        ]
    };
    let import = |root: &Path, args: &[String]| json(root, &args.iter().map(String::as_str).collect::<Vec<_>>());
    let refuse = |root: &Path, args: &[String]| denied(root, &args.iter().map(String::as_str).collect::<Vec<_>>());

    let args = import_args(&bundle, &key, &restored, &world_id, &checkpoint);
    refuse(&recipient, &args[..args.len() - 1]);
    assert!(!restored.exists());
    refuse(&recipient, &import_args(&bundle, &second_key, &restored, &world_id, &checkpoint));
    refuse(&recipient, &import_args(&bundle, &key, &restored, &world_id, &latest.manifest_hash().unwrap().to_string()));
    refuse(&recipient, &import_args(&bundle, &key, &restored, &WorldId([8; 32]).to_string(), &checkpoint));
    assert!(!restored.exists());
    assert!(!recipient.exists());

    // Model interrupted/modified file transport, including a complete prefix
    // lacking only the final authenticated terminator.
    for (index, length) in
        [0, 10, 24, 44, encrypted.len() / 2, encrypted.len() - 20, encrypted.len() - 1].into_iter().enumerate()
    {
        let partial = temp.path().join(format!("partial-{index}.scbackup"));
        fs::write(&partial, &encrypted[..length]).unwrap();
        refuse(&recipient, &import_args(&partial, &key, &restored, &world_id, &checkpoint));
        assert!(!restored.exists(), "truncated transfer published a destination");
        assert!(!recipient.exists());
    }
    let corrupted = temp.path().join("corrupt.scbackup");
    let mut modified = encrypted.clone();
    let middle = modified.len() / 2;
    modified[middle] ^= 0x80;
    fs::write(&corrupted, &modified).unwrap();
    refuse(&recipient, &import_args(&corrupted, &key, &restored, &world_id, &checkpoint));
    let appended = temp.path().join("appended.scbackup");
    modified = encrypted.clone();
    modified.extend_from_slice(b"trailing");
    fs::write(&appended, &modified).unwrap();
    refuse(&recipient, &import_args(&appended, &key, &restored, &world_id, &checkpoint));
    assert!(!restored.exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
        refuse(&recipient, &args);
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let alias = temp.path().join("origin-alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        let redirected = alias.join("not-a-backup");
        refuse(&root, &import_args(&bundle, &key, &redirected, &world_id, &checkpoint));
        assert!(!redirected.exists());
    }
    let output = import(&recipient, &args);
    assert_eq!(output["operation"], "data_only_import");
    assert_eq!(output["source_checkpoint_hash"], checkpoint);
    assert_eq!(output["state_root"], old.state_root.to_string());
    assert_eq!(output["compatibility"], export["compatibility"]);
    assert_eq!(output["registered_world"], false);
    assert_eq!(output["authority_granted"], false);
    assert!(!recipient.exists(), "backup import created a canonical store");
    assert_eq!(fs::read(restored.join("world/level.dat")).unwrap(), private_text);
    assert_eq!(fs::read(restored.join("world/region/r.0.0.mca")).unwrap(), random_data);
    assert_eq!(fs::read(restored.join("world/region/duplicate.dat")).unwrap(), random_data);
    assert_eq!(fs::read(restored.join("world/empty.dat")).unwrap(), b"");
    assert_eq!(fs::read(restored.join("world/compressed.dat")).unwrap(), vec![0x5a; 500 * 1024]);
    assert!(!restored.join(".swarmcraft-backup-import-incomplete").exists());
    let receipt: Value = serde_json::from_slice(&fs::read(restored.join("backup-receipt.json")).unwrap()).unwrap();
    assert_eq!(receipt, output);
    let signed: swarm_protocol::SnapshotManifestV1 =
        postcard::from_bytes(&fs::read(restored.join("signed-manifest.postcard")).unwrap()).unwrap();
    assert_eq!(signed, old);
    swarm_core::verify_snapshot_signature(&signed).unwrap();
    refuse(&recipient, &args);
    assert_eq!(fs::read(restored.join("world/level.dat")).unwrap(), private_text);

    // Import on the original device is still data-only and cannot change
    // its accepted head, runtime authorization, membership or configuration.
    let second_restore = temp.path().join("restore-on-origin");
    let result = import(&root, &import_args(&bundle, &key, &second_restore, &world_id, &checkpoint));
    assert_eq!(result["registered_world"], false);
    assert_eq!(storage.list_worlds().unwrap().len(), 1);
    assert_eq!(storage.list_snapshots(world).unwrap(), original_history);
    assert_eq!(storage.load_epoch_record(world).unwrap(), original_epoch);
    assert_eq!(storage.load_membership_record(world).unwrap(), original_membership);
    assert_eq!(storage.load_world_config(world).unwrap(), original_config);
    assert!(swarm_cli::hosting_budget::load_consent(&DataPaths::from_root(root.clone()), world).unwrap().is_none());
    let inside = root.join("forbidden");
    refuse(&root, &import_args(&bundle, &key, &inside, &world_id, &checkpoint));
    denied(
        &root,
        &[
            "backup",
            "export",
            &world_id,
            "--checkpoint",
            &checkpoint,
            "--key-file",
            key.to_str().unwrap(),
            "--destination",
            inside.to_str().unwrap(),
        ],
    );
    assert!(!inside.exists());
    assert!(!fs::read_dir(temp.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".swarmcraft-backup-import-")));
    assert_eq!(storage.latest_snapshot(world).unwrap(), Some(latest));
    assert_ne!(old.manifest_hash().unwrap(), Hash32::default());
}
