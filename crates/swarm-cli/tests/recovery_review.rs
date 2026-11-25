use serde_json::Value;
use std::{fs, path::Path, process::Command};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{Hash32, SnapshotManifestV1, SoloBranchV1, WorldId, PROTOCOL_VERSION};
use swarm_storage::{SnapshotContext, Storage};

fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(root)
        .args(args)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap()
}

fn success(root: &Path, args: &[&str]) -> String {
    let output = cli(root, args);
    assert!(output.status.success(), "args={args:?} stderr={}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn json(root: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&success(root, args)).unwrap()
}

fn denied(root: &Path, args: &[&str]) {
    let output = cli(root, args);
    assert!(!output.status.success(), "unexpected success: {args:?}");
}

fn checkpoint(
    storage: &Storage,
    identity: &PeerIdentity,
    world: WorldId,
    source: &Path,
    bytes: &[u8],
) -> SnapshotManifestV1 {
    fs::create_dir_all(source).unwrap();
    fs::write(source.join("level.dat"), bytes).unwrap();
    let prior = storage.latest_snapshot(world).unwrap().unwrap();
    let mut publication = storage
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
    identity.sign_snapshot(&mut publication).unwrap();
    storage.commit_snapshot(&publication).unwrap();
    publication.manifest().clone()
}

fn branch(identity: &PeerIdentity, base: &SnapshotManifestV1, head: &SnapshotManifestV1) -> SoloBranchV1 {
    let mut branch = SoloBranchV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: base.world_id,
        base_snapshot_hash: base.manifest_hash().unwrap(),
        base_epoch: base.epoch,
        head_snapshot_hash: head.manifest_hash().unwrap(),
        head_epoch: head.epoch,
        head_sequence: head.sequence,
        state_hash: head.state_root,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: vec![],
    };
    swarm_core::sign_solo_branch(identity, &mut branch).unwrap();
    branch
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &path);
        } else {
            assert!(entry.file_type().unwrap().is_file());
            fs::copy(entry.path(), path).unwrap();
        }
    }
}

#[test]
fn exact_checkpoint_and_both_divergent_holders_export_without_resolving_original_history() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("a");
    let other = temp.path().join("b");
    let created = success(
        &root,
        &["world", "create", "--name", "Recovery source", "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"],
    );
    let world: WorldId = created.lines().find_map(|line| line.strip_prefix("World ID: ")).unwrap().parse().unwrap();
    let id = world.to_string();
    let storage = Storage::open(&root).unwrap();
    let identity = PeerIdentity::load_or_create(&DataPaths::from_root(root.clone())).unwrap();
    let base = checkpoint(&storage, &identity, world, &temp.path().join("common"), b"common original");
    // Controlled cloned fixture models two independently advanced copies of
    // the same signed world. Neither store has the other's fork payload.
    copy_tree(&root, &other);
    let peer = Storage::open(&other).unwrap();
    let a = checkpoint(&storage, &identity, world, &temp.path().join("a-source"), b"branch A bytes");
    let b = checkpoint(&peer, &identity, world, &temp.path().join("b-source"), b"branch B bytes");
    let branch_a = branch(&identity, &base, &a);
    let branch_b = branch(&identity, &base, &b);
    let hash_a = branch_a.branch_hash().unwrap().to_string();
    let hash_b = branch_b.branch_hash().unwrap().to_string();
    let expected_a = storage.list_snapshots(world).unwrap();
    let expected_b = peer.list_snapshots(world).unwrap();
    for store in [&storage, &peer] {
        store.preserve_solo_conflict(&branch_a).unwrap();
        store.preserve_solo_conflict(&branch_b).unwrap();
    }

    let review_a = json(&root, &["world", "recovery-review", &id]);
    let review_b = json(&other, &["world", "recovery-review", &id]);
    let digest = review_a["conflict_digest"].as_str().unwrap();
    assert_eq!(review_b["conflict_digest"], digest);
    assert_eq!(review_a["checkpoints"].as_array().unwrap().len(), 3);
    let choices = |review: &Value, hash: &str| {
        review["conflicts"].as_array().unwrap().iter().find(|entry| entry["branch_hash"] == hash).unwrap().clone()
    };
    assert_eq!(choices(&review_a, &hash_a)["local_history"], "matching_signed_manifests");
    assert_eq!(choices(&review_a, &hash_b)["local_history"], "evidence_only");
    assert_eq!(choices(&review_b, &hash_b)["local_history"], "matching_signed_manifests");
    assert_eq!(choices(&review_b, &hash_a)["local_history"], "evidence_only");
    let old_hash = base.manifest_hash().unwrap().to_string();
    let copy_args = ["world", "recover-copy", &id, "--checkpoint", &old_hash, "--confirm-new-identity", "--json"];
    denied(&root, &copy_args);
    denied(
        &root,
        &[
            "world",
            "recover-copy",
            &id,
            "--checkpoint",
            &old_hash,
            "--confirm-new-identity",
            "--acknowledge-conflicts",
            &Hash32([9; 32]).to_string(),
            "--json",
        ],
    );

    let absent = temp.path().join("absent-files");
    denied(
        &root,
        &[
            "world",
            "conflict-export",
            &id,
            "--branch",
            &hash_b,
            "--acknowledge-conflicts",
            digest,
            "--destination",
            absent.to_str().unwrap(),
        ],
    );
    assert!(!absent.exists());

    let evidence = temp.path().join("evidence");
    let receipt = json(
        &root,
        &[
            "world",
            "conflict-export",
            &id,
            "--branch",
            &hash_b,
            "--acknowledge-conflicts",
            digest,
            "--evidence-only",
            "--destination",
            evidence.to_str().unwrap(),
        ],
    );
    assert_eq!(receipt["evidence"], "signed_branch_evidence_only");
    assert_eq!(receipt["files_exported"], false);
    assert!(!evidence.join("world").exists());
    let exact: SoloBranchV1 =
        postcard::from_bytes(&fs::read(evidence.join("signed-branch.postcard")).unwrap()).unwrap();
    assert_eq!(exact, branch_b);
    assert!(!evidence.join(".swarmcraft-export-incomplete").exists());

    for (holder, branch_hash, bytes, name) in
        [(&root, &hash_a, b"branch A bytes", "export-a"), (&other, &hash_b, b"branch B bytes", "export-b")]
    {
        let path = temp.path().join(name);
        let receipt = json(
            holder,
            &[
                "world",
                "conflict-export",
                &id,
                "--branch",
                branch_hash,
                "--acknowledge-conflicts",
                digest,
                "--destination",
                path.to_str().unwrap(),
            ],
        );
        assert_eq!(receipt["files_exported"], true);
        assert_eq!(receipt["evidence"], "signed_branch_head_files_not_canonical_authority");
        assert_eq!(receipt["manifest_count"], 2);
        assert_eq!(fs::read(path.join("world/level.dat")).unwrap(), bytes);
        assert!(!path.join(".swarmcraft-export-incomplete").exists());
        denied(
            holder,
            &[
                "world",
                "conflict-export",
                &id,
                "--branch",
                branch_hash,
                "--acknowledge-conflicts",
                digest,
                "--destination",
                path.to_str().unwrap(),
            ],
        );
        assert_eq!(fs::read(path.join("world/level.dat")).unwrap(), bytes);
    }

    let inside = root.join("must-not-be-created");
    denied(
        &root,
        &[
            "world",
            "recovery-export",
            &id,
            "--checkpoint",
            &old_hash,
            "--acknowledge-conflicts",
            digest,
            "--destination",
            inside.to_str().unwrap(),
        ],
    );
    assert!(!inside.exists());
    denied(&root, &["world", "recover", &id, "2", inside.to_str().unwrap()]);
    denied(&root, &["world", "export", &id, inside.to_str().unwrap()]);
    assert!(!inside.exists());

    #[cfg(unix)]
    {
        let link = temp.path().join("linked-root");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let redirected = link.join("redirected-export");
        denied(
            &root,
            &[
                "world",
                "recovery-export",
                &id,
                "--checkpoint",
                &old_hash,
                "--acknowledge-conflicts",
                digest,
                "--destination",
                redirected.to_str().unwrap(),
            ],
        );
        denied(&root, &["world", "recover", &id, "2", redirected.to_str().unwrap()]);
        assert!(!redirected.exists());
    }

    let exported = temp.path().join("old-checkpoint");
    let receipt = json(
        &root,
        &[
            "world",
            "recovery-export",
            &id,
            "--checkpoint",
            &old_hash,
            "--acknowledge-conflicts",
            digest,
            "--destination",
            exported.to_str().unwrap(),
        ],
    );
    assert_eq!(receipt["checkpoint_hash"], old_hash);
    assert_eq!(receipt["manifest_count"], 3);
    assert_eq!(fs::read(exported.join("world/level.dat")).unwrap(), b"common original");

    let copy = json(
        &root,
        &[
            "world",
            "recover-copy",
            &id,
            "--checkpoint",
            &old_hash,
            "--acknowledge-conflicts",
            digest,
            "--confirm-new-identity",
            "--json",
        ],
    );
    assert_eq!(copy["source_snapshot_hash"], old_hash);
    assert_eq!(copy["source_snapshot_number"], 2);
    assert_eq!(copy["source_conflict_digest"], digest);
    let copy_world: WorldId = copy["world_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(copy_world, world);
    let recovered = storage.latest_snapshot(copy_world).unwrap().unwrap();
    assert_eq!(recovered.state_root, base.state_root);
    assert_eq!(recovered.entries, base.entries);
    storage.verify_snapshot(&recovered).unwrap();
    assert_eq!(storage.list_snapshots(world).unwrap(), expected_a);
    assert_eq!(peer.list_snapshots(world).unwrap(), expected_b);
    for store in [&storage, &peer] {
        assert_eq!(store.list_solo_conflicts(world).unwrap().len(), 2);
    }
    assert!(swarm_cli::hosting_budget::load_consent(&DataPaths::from_root(root.clone()), copy_world)
        .unwrap()
        .is_none());
    assert_eq!(storage.load_world_config(copy_world).unwrap().visibility, swarm_protocol::WorldVisibilityV1::Private);
    // No new action using an earlier conflict-set acknowledgement.
    let mut third = branch_a.clone();
    third.state_hash = Hash32([5; 32]);
    swarm_core::sign_solo_branch(&identity, &mut third).unwrap();
    storage.preserve_solo_conflict(&third).unwrap();
    denied(
        &root,
        &[
            "world",
            "recover-copy",
            &id,
            "--checkpoint",
            &old_hash,
            "--acknowledge-conflicts",
            digest,
            "--confirm-new-identity",
            "--json",
        ],
    );
    assert_eq!(storage.list_worlds().unwrap().len(), 2);
    let changed = json(&root, &["world", "recovery-review", &id]);
    assert_ne!(changed["conflict_digest"], digest);
    let changed_digest = changed["conflict_digest"].as_str().unwrap();
    // A matching signed manifest is only a selection, not evidence that its
    // payload is still complete. Corrupt the other holder's unique head.
    let blob = &b.entries[0].blob;
    let suffix = match blob.encoding {
        swarm_protocol::BlobEncoding::Raw => "raw",
        swarm_protocol::BlobEncoding::Zstd => "zst",
    };
    let blob_path = peer.world_dir(world).join("blobs").join(format!("{}.{suffix}", blob.hash));
    let original_blob = fs::read(&blob_path).unwrap();
    fs::write(&blob_path, b"corrupt branch payload").unwrap();
    let corrupt_destination = temp.path().join("must-not-export-corrupt");
    denied(
        &other,
        &[
            "world",
            "conflict-export",
            &id,
            "--branch",
            &hash_b,
            "--acknowledge-conflicts",
            digest,
            "--destination",
            corrupt_destination.to_str().unwrap(),
        ],
    );
    assert!(!corrupt_destination.exists());
    fs::write(&blob_path, original_blob).unwrap();
    peer.verify_snapshot(&b).unwrap();

    // A valid signed record whose claimed state differs from its head is
    // explicitly unverified for data export, not silently reinterpreted.
    let third_hash = third.branch_hash().unwrap().to_string();
    assert_eq!(choices(&changed, &third_hash)["local_history"], "unverified_local_history");
    // A corrupted signature is not treated as absence or a new conflict set.
    let record = storage
        .world_dir(world)
        .join("recovery/solo-conflicts")
        .join(format!("{}.postcard", branch_a.branch_hash().unwrap()));
    let exact = fs::read(&record).unwrap();
    let mut forged = branch_a.clone();
    forged.signature[0] ^= 1;
    fs::write(&record, postcard::to_allocvec(&forged).unwrap()).unwrap();
    denied(&root, &["world", "recovery-review", &id]);
    denied(
        &root,
        &[
            "world",
            "recover-copy",
            &id,
            "--checkpoint",
            &old_hash,
            "--acknowledge-conflicts",
            changed_digest,
            "--confirm-new-identity",
            "--json",
        ],
    );
    fs::write(&record, exact).unwrap();
    assert_eq!(storage.list_snapshots(world).unwrap(), expected_a);
}
