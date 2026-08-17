use std::{collections::BTreeSet, fs, path::PathBuf};
use swarm_protocol::{
    AuthorityTransferV1, BlobDescriptor, BlobEncoding, Hash32, PeerId, SnapshotManifestV1, TransferPhase, WorldId,
    PROTOCOL_VERSION,
};
use swarm_storage::{retention::RetentionPolicy, SnapshotContext, Storage};

fn world() -> WorldId {
    WorldId([0x72; 32])
}

fn blob_path(storage: &Storage, descriptor: &BlobDescriptor) -> PathBuf {
    let suffix = match descriptor.encoding {
        BlobEncoding::Raw => "raw",
        BlobEncoding::Zstd => "zst",
    };
    storage.world_dir(world()).join("blobs").join(format!("{}.{}", descriptor.hash.to_hex(), suffix))
}

fn snapshot(
    storage: &Storage,
    source: &std::path::Path,
    number: u64,
    previous_snapshot_hash: Option<Hash32>,
) -> SnapshotManifestV1 {
    let mut manifest = storage
        .snapshot_directory(
            source,
            SnapshotContext {
                world: world(),
                snapshot_number: number,
                epoch: 1,
                sequence: number,
                previous_snapshot_hash,
                authority_peer_id: PeerId([1; 32]),
                authority_public_key: [2; 32],
            },
        )
        .unwrap();
    manifest.signature = vec![0; 64];
    storage.commit_snapshot(&manifest).unwrap();
    manifest
}

#[test]
fn gc_never_removes_blobs_referenced_by_committed_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-world-state").unwrap();

    let storage = Storage::open(temp.path().join("store")).unwrap();
    let manifest = snapshot(&storage, &source, 1, None);
    let referenced = manifest.entries[0].blob.clone();
    let orphan = storage.put_blob(world(), b"unreferenced-orphan").unwrap();

    let report = storage.garbage_collect_blobs(world()).unwrap();

    assert!(blob_path(&storage, &referenced).exists());
    assert!(!blob_path(&storage, &orphan).exists());
    assert_eq!(report.removed_blobs, 1);
    storage.verify_snapshot(&manifest).unwrap();
}

#[test]
fn orphaned_blobs_are_reclaimed_after_snapshot_retention_prunes_their_last_reference() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();

    fs::write(source.join("level.dat"), b"snapshot-one").unwrap();
    let first = snapshot(&storage, &source, 1, None);
    let first_blob = first.entries[0].blob.clone();

    fs::write(source.join("level.dat"), b"snapshot-two").unwrap();
    let second = snapshot(&storage, &source, 2, Some(first.manifest_hash().unwrap()));
    let second_blob = second.entries[0].blob.clone();

    let report = storage
        .apply_retention(world(), &RetentionPolicy { keep_latest: 1, protected_snapshots: BTreeSet::new() })
        .unwrap();

    assert_eq!(report.removed_snapshots, vec![1]);
    assert_eq!(report.retained_snapshots, vec![2]);
    assert!(!blob_path(&storage, &first_blob).exists());
    assert!(blob_path(&storage, &second_blob).exists());
    assert!(storage.load_snapshot(world(), 1).is_err());
    storage.verify_snapshot(&second).unwrap();
}

#[test]
fn interrupted_cleanup_between_prune_and_gc_preserves_recoverability() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    let store_root = temp.path().join("store");
    let storage = Storage::open(&store_root).unwrap();

    fs::write(source.join("level.dat"), b"old-state").unwrap();
    let first = snapshot(&storage, &source, 1, None);
    fs::write(source.join("level.dat"), b"latest-recovery-state").unwrap();
    let latest = snapshot(&storage, &source, 2, Some(first.manifest_hash().unwrap()));

    let prune = storage
        .prune_snapshots(world(), &RetentionPolicy { keep_latest: 1, protected_snapshots: BTreeSet::new() })
        .unwrap();
    assert_eq!(prune.removed_snapshots, vec![1]);

    // Simulate process interruption after the manifest-prune phase and before
    // the blob sweep. Reopening must leave the latest recovery point intact.
    drop(storage);
    let reopened = Storage::open(&store_root).unwrap();
    let loaded = reopened.latest_snapshot(world()).unwrap().unwrap();
    assert_eq!(loaded.manifest_hash().unwrap(), latest.manifest_hash().unwrap());
    reopened.verify_snapshot(&loaded).unwrap();

    let restored = temp.path().join("restored");
    reopened.restore_snapshot(&loaded, &restored).unwrap();
    assert_eq!(fs::read(restored.join("level.dat")).unwrap(), b"latest-recovery-state");

    // A later GC may reclaim the old orphan, but the recovery point remains exact.
    reopened.garbage_collect_blobs(world()).unwrap();
    reopened.verify_snapshot(&loaded).unwrap();
    assert_eq!(reopened.latest_snapshot(world()).unwrap().unwrap(), latest);
}

#[test]
fn active_replication_pins_prevent_gc_from_reclaiming_in_flight_complete_blobs() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let descriptor = storage.put_blob(world(), b"completed-before-manifest-commit").unwrap();
    let path = blob_path(&storage, &descriptor);
    assert!(path.exists());

    let lease = storage.pin_replication_hashes(world(), &[descriptor.hash]).unwrap();
    assert_eq!(lease.pinned_blobs(), 1);
    let first_gc = storage.garbage_collect_blobs(world()).unwrap();
    assert_eq!(first_gc.removed_blobs, 0);
    assert!(path.exists());

    drop(lease);
    let second_gc = storage.garbage_collect_blobs(world()).unwrap();
    assert_eq!(second_gc.removed_blobs, 1);
    assert!(!path.exists());
}

#[test]
fn authority_transfer_base_is_a_mandatory_retention_root() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();

    fs::write(source.join("level.dat"), b"transfer-base").unwrap();
    let first = snapshot(&storage, &source, 1, None);
    let first_blob = first.entries[0].blob.clone();

    fs::write(source.join("level.dat"), b"newer-canonical-state").unwrap();
    let second = snapshot(&storage, &source, 2, Some(first.manifest_hash().unwrap()));

    storage
        .save_transfer_record(&AuthorityTransferV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world(),
            from_peer_id: PeerId([1; 32]),
            to_peer_id: PeerId([2; 32]),
            base_snapshot_hash: first.manifest_hash().unwrap(),
            next_epoch: 2,
            next_fencing_token: 2,
            phase: TransferPhase::Prepared,
            signer_peer_id: PeerId([1; 32]),
            signer_public_key: [3; 32],
            signature: vec![0; 64],
        })
        .unwrap();

    let report = storage
        .apply_retention(world(), &RetentionPolicy { keep_latest: 1, protected_snapshots: BTreeSet::new() })
        .unwrap();

    assert_eq!(report.retained_snapshots, vec![1, 2]);
    assert!(report.removed_snapshots.is_empty());
    assert!(blob_path(&storage, &first_blob).exists());
    storage.verify_snapshot(&first).unwrap();
    storage.verify_snapshot(&second).unwrap();
}

#[test]
fn latest_snapshot_is_retained_even_when_keep_latest_is_zero() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"only-snapshot").unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let manifest = snapshot(&storage, &source, 1, None);

    let report = storage
        .apply_retention(world(), &RetentionPolicy { keep_latest: 0, protected_snapshots: BTreeSet::new() })
        .unwrap();

    assert_eq!(report.retained_snapshots, vec![1]);
    assert!(report.removed_snapshots.is_empty());
    storage.verify_snapshot(&manifest).unwrap();
}
