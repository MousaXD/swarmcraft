use std::fs;

use swarm_protocol::{snapshot_state_root, Hash32, PeerId, WorldId};
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

fn snapshot_fixture() -> (tempfile::TempDir, Storage, swarm_protocol::SnapshotManifestV1) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("region")).unwrap();
    fs::write(source.join("level.dat"), b"level-data").unwrap();
    fs::write(source.join("region/r.0.0.mca"), b"region-data").unwrap();

    let storage = Storage::open(temp.path().join("store")).unwrap();
    let manifest = storage.snapshot_directory(&source, context(WorldId([9; 32]))).unwrap();
    (temp, storage, manifest)
}

#[test]
fn truncated_manifest_is_rejected_instead_of_becoming_canonical() {
    let (_temp, storage, manifest) = snapshot_fixture();
    storage.commit_snapshot(&manifest).unwrap();

    let path = storage
        .world_dir(manifest.world_id)
        .join("snapshots")
        .join(format!("{:020}.postcard", manifest.snapshot_number));
    let encoded = fs::read(&path).unwrap();
    fs::write(&path, &encoded[..encoded.len() / 2]).unwrap();

    assert!(matches!(storage.load_snapshot(manifest.world_id, manifest.snapshot_number), Err(StorageError::Decode(_))));
}

#[test]
fn restore_rejects_traversal_before_writing_outside_destination() {
    let (temp, storage, mut manifest) = snapshot_fixture();
    manifest.entries[0].path = "../escaped.dat".into();

    let destination = temp.path().join("restore");
    let escaped = temp.path().join("escaped.dat");
    assert!(matches!(
        storage.restore_snapshot(&manifest, &destination),
        Err(StorageError::UnsafeRelativePath(path)) if path == "../escaped.dat"
    ));
    assert!(!escaped.exists());
}

#[cfg(unix)]
#[test]
fn restore_rejects_symlinked_parent_before_writing_outside_destination() {
    use std::os::unix::fs::symlink;

    let (temp, storage, mut manifest) = snapshot_fixture();
    manifest.entries[0].path = "redirect/escaped.dat".into();
    manifest.state_root = snapshot_state_root(&manifest.entries).unwrap();

    let destination = temp.path().join("restore");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&destination).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, destination.join("redirect")).unwrap();

    assert!(matches!(
        storage.restore_snapshot(&manifest, &destination),
        Err(StorageError::SymlinkUnsupported(path)) if path == destination.join("redirect")
    ));
    assert!(!outside.join("escaped.dat").exists());
}

#[test]
fn duplicate_manifest_paths_are_rejected_even_with_a_matching_state_root() {
    let (_temp, storage, mut manifest) = snapshot_fixture();
    let duplicate = manifest.entries[0].clone();
    manifest.entries.push(duplicate);
    manifest.state_root = snapshot_state_root(&manifest.entries).unwrap();

    assert!(matches!(
        storage.verify_snapshot(&manifest),
        Err(StorageError::UnsafeRelativePath(path)) if path == manifest.entries[0].path
    ));
}

#[test]
fn forged_state_root_is_rejected_before_restore() {
    let (temp, storage, mut manifest) = snapshot_fixture();
    manifest.state_root = Hash32([0xee; 32]);
    let destination = temp.path().join("restore");

    assert!(matches!(storage.restore_snapshot(&manifest, &destination), Err(StorageError::StateRootMismatch)));
    assert!(!destination.join("level.dat").exists());
}

#[test]
fn filesystem_shape_error_is_reported_as_io_failure_without_panicking() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let world = WorldId([7; 32]);
    let world_dir = storage.world_dir(world);
    fs::create_dir_all(&world_dir).unwrap();
    fs::write(world_dir.join("blobs"), b"not-a-directory").unwrap();

    assert!(matches!(storage.put_blob(world, b"payload"), Err(StorageError::Io { .. })));
}
