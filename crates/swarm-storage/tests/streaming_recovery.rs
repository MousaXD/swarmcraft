use std::{fs, path::Path};
use swarm_protocol::{BlobEncoding, PeerId, WorldId, PROTOCOL_VERSION};
use swarm_storage::{SnapshotContext, Storage, StorageError};

fn context(world: WorldId) -> SnapshotContext {
    SnapshotContext {
        world,
        snapshot_number: 1,
        epoch: 1,
        sequence: 1,
        previous_snapshot_hash: None,
        authority_peer_id: PeerId([2; 32]),
        authority_public_key: [3; 32],
    }
}

fn blob_path(storage: &Storage, world: WorldId, manifest: &swarm_protocol::SnapshotManifestV1) -> std::path::PathBuf {
    let descriptor = &manifest.entries[0].blob;
    let suffix = match descriptor.encoding {
        BlobEncoding::Raw => "raw",
        BlobEncoding::Zstd => "zst",
    };
    storage.world_dir(world).join("blobs").join(format!("{}.{}", descriptor.hash.to_hex(), suffix))
}

fn create_snapshot(storage: &Storage, source: &Path, world: WorldId) -> swarm_protocol::SnapshotManifestV1 {
    fs::create_dir_all(source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-world-state").unwrap();
    let mut manifest = storage.snapshot_directory(source, context(world)).unwrap();
    manifest.signature = vec![0; 64];
    storage.commit_snapshot(&manifest).unwrap();
    manifest
}

#[test]
fn corrupt_restore_never_replaces_existing_destination_file() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let world = WorldId([11; 32]);
    let manifest = create_snapshot(&storage, &source, world);

    fs::create_dir_all(&destination).unwrap();
    fs::write(destination.join("level.dat"), b"previous-safe-copy").unwrap();

    let path = blob_path(&storage, world, &manifest);
    let original_len = fs::metadata(&path).unwrap().len();
    fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(original_len / 2).unwrap();

    assert!(matches!(storage.restore_snapshot(&manifest, &destination), Err(StorageError::BlobCorrupt(_))));
    assert_eq!(fs::read(destination.join("level.dat")).unwrap(), b"previous-safe-copy");
}

#[test]
fn stale_temporary_files_are_ignored_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let storage = Storage::open(&root).unwrap();
    let source = temp.path().join("source");
    let world = WorldId([12; 32]);
    let manifest = create_snapshot(&storage, &source, world);

    let blobs = storage.world_dir(world).join("blobs");
    let snapshots = storage.world_dir(world).join("snapshots");
    fs::create_dir_all(&blobs).unwrap();
    fs::create_dir_all(&snapshots).unwrap();
    fs::write(blobs.join(".blob-dead-process-1.zst"), b"partial").unwrap();
    fs::write(snapshots.join("00000000000000000002.postcard.tmp"), b"partial").unwrap();

    drop(storage);
    let reopened = Storage::open(&root).unwrap();
    assert_eq!(reopened.latest_snapshot(world).unwrap().unwrap().snapshot_number, manifest.snapshot_number);
    reopened.verify_snapshot(&manifest).unwrap();
}

#[test]
fn unsupported_protocol_is_rejected_before_restore() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let world = WorldId([13; 32]);
    let mut manifest = create_snapshot(&storage, &source, world);
    manifest.protocol_version = PROTOCOL_VERSION.saturating_add(1);

    assert!(matches!(
        storage.restore_snapshot(&manifest, &destination),
        Err(StorageError::UnsupportedProtocol(version)) if version == PROTOCOL_VERSION + 1
    ));
    assert!(!destination.exists());
}

#[test]
fn failed_snapshot_does_not_publish_a_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let source = temp.path().join("source");
    let world = WorldId([14; 32]);
    fs::create_dir_all(&source).unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("missing-target", source.join("unsafe-link")).unwrap();
        assert!(matches!(storage.snapshot_directory(&source, context(world)), Err(StorageError::SymlinkUnsupported(_))));
        assert!(storage.latest_snapshot(world).unwrap().is_none());
    }

    #[cfg(not(unix))]
    {
        fs::write(source.join("ordinary-file"), b"ordinary").unwrap();
        let manifest = storage.snapshot_directory(&source, context(world)).unwrap();
        assert!(storage.latest_snapshot(world).unwrap().is_none());
        assert_eq!(manifest.snapshot_number, 1);
    }
}
