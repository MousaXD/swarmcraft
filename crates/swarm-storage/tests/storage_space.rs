use std::{collections::BTreeSet, fs, path::Path};
use swarm_protocol::{
    peer_id_from_public_key, BlobDescriptor, BlobEncoding, Hash32, SnapshotManifestV1, WorldGenesisV1, WorldId,
    PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{
    space::DEFAULT_FREE_RESERVE_BYTES, RetentionPolicy, SnapshotContext, Storage, WorldMetadataV1, WorldStoragePolicy,
};

fn fixture() -> (tempfile::TempDir, Storage, WorldId) {
    let temp = tempfile::tempdir().unwrap();
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "26.1.2".into(),
        fabric_loader_version: "0.19.3".into(),
        compatibility_fingerprint: Hash32([4; 32]),
        creation_nonce: [8; 32],
        creator_public_key: [6; 32],
        initial_membership: vec![peer_id_from_public_key(&[6; 32])],
    };
    let world = genesis.world_id().unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    storage
        .create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "space regression".into(),
            world_id: world,
            genesis,
        })
        .unwrap();
    (temp, storage, world)
}

fn context(world: WorldId, prior: Option<&SnapshotManifestV1>) -> SnapshotContext {
    let n = prior.map_or(1, |manifest| manifest.snapshot_number + 1);
    SnapshotContext {
        world,
        snapshot_number: n,
        epoch: 1,
        sequence: n,
        previous_snapshot_hash: prior.map(|manifest| manifest.manifest_hash().unwrap()),
        authority_peer_id: peer_id_from_public_key(&[6; 32]),
        authority_public_key: [6; 32],
    }
}

fn checkpoint(
    storage: &Storage,
    world: WorldId,
    source: &Path,
    prior: Option<&SnapshotManifestV1>,
) -> SnapshotManifestV1 {
    let mut publication = storage.snapshot_directory(source, context(world, prior)).unwrap();
    publication.signature = vec![0; 64];
    storage.commit_snapshot(&publication).unwrap();
    publication.manifest().clone()
}

fn policy(max_world_bytes: Option<u64>) -> WorldStoragePolicy {
    WorldStoragePolicy { max_world_bytes, reserve_free_bytes: DEFAULT_FREE_RESERVE_BYTES }
}

#[test]
fn local_replica_policy_is_durable_and_invalid_records_fail_closed() {
    use swarm_storage::replica_policy::{ReplicaPolicy, UtcWindow};
    let (_temp, storage, world) = fixture();
    assert_eq!(storage.world_replica_policy(world).unwrap(), ReplicaPolicy::default());
    let restricted = ReplicaPolicy {
        follow: false,
        hot: true,
        utc_window: Some(UtcWindow { start_minute: 1320, end_minute: 120 }),
        max_kib_per_second: Some(64),
    };
    storage.set_world_replica_policy(world, restricted).unwrap();
    let reopened = Storage::open(storage.root()).unwrap();
    assert_eq!(reopened.world_replica_policy(world).unwrap(), restricted);
    assert!(reopened
        .set_world_replica_policy(world, ReplicaPolicy { max_kib_per_second: Some(0), ..restricted })
        .is_err());
    assert_eq!(reopened.world_replica_policy(world).unwrap(), restricted);
    let path = reopened.world_dir(world).join("metadata/local-replica-policy.json");
    fs::write(&path, b"{\"follow\":true,\"unknown\":1}").unwrap();
    assert!(reopened.world_replica_policy(world).is_err());
    fs::write(&path, vec![0u8; 4097]).unwrap();
    assert!(reopened.world_replica_policy(world).is_err());
    #[cfg(unix)]
    {
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(path.with_extension("missing"), &path).unwrap();
        assert!(reopened.world_replica_policy(world).is_err());
        assert!(reopened.set_world_replica_policy(world, restricted).is_err());
    }
}

#[test]
fn quota_pauses_local_checkpoint_without_changing_head_or_leaking_owned_temp() {
    let (temp, storage, world) = fixture();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"accepted world").unwrap();
    let first = checkpoint(&storage, world, &source, None);
    fs::write(source.join("level.dat"), b"new unsaved world").unwrap();
    storage.set_world_storage_policy(world, policy(Some(0))).unwrap();
    assert!(storage.world_storage_status(world).unwrap().payload_admission_paused);
    let denied = storage.snapshot_directory(&source, context(world, Some(&first))).unwrap_err();
    assert!(denied.is_storage_pressure(), "{denied:?}");
    assert_eq!(storage.latest_snapshot(world).unwrap().unwrap(), first);
    storage.verify_snapshot(&first).unwrap();
    assert_eq!(fs::read(source.join("level.dat")).unwrap(), b"new unsaved world");
    let dir = storage.world_dir(world).join("blobs");
    assert!(fs::read_dir(dir).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".blob-")));
    assert!(!storage.world_dir(world).join("metadata/snapshot-commit.intent.postcard").exists());

    let reopened = Storage::open(storage.root()).unwrap();
    assert_eq!(reopened.world_storage_policy(world).unwrap(), policy(Some(0)));
    reopened.set_world_storage_policy(world, policy(None)).unwrap();
    let second = checkpoint(&reopened, world, &source, Some(&first));
    assert_eq!(second.snapshot_number, 2);
    reopened.verify_snapshot(&first).unwrap();
    reopened.verify_snapshot(&second).unwrap();
}

#[test]
fn physical_reserve_blocks_replica_and_restore_before_mutating_existing_data() {
    let (temp, storage, world) = fixture();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"accepted").unwrap();
    let first = checkpoint(&storage, world, &source, None);
    let dest = temp.path().join("restore");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("keep.txt"), b"keep me").unwrap();
    storage
        .set_world_storage_policy(world, WorldStoragePolicy { max_world_bytes: None, reserve_free_bytes: u64::MAX })
        .unwrap();
    assert!(storage.world_storage_status(world).unwrap().payload_admission_paused);
    assert!(storage.restore_snapshot(&first, &dest).unwrap_err().is_storage_pressure());
    assert_eq!(fs::read(dest.join("keep.txt")).unwrap(), b"keep me");
    assert!(!dest.join(".swarmcraft-restore-incomplete").exists());
    let bytes = b"another file";
    let desc = BlobDescriptor {
        hash: BlobDescriptor::hash_uncompressed(bytes),
        encoding: BlobEncoding::Raw,
        uncompressed_size: bytes.len() as u64,
        encoded_size: bytes.len() as u64,
    };
    assert!(storage.receive_blob_chunk(world, &desc, 0, bytes, true).is_err());
    assert_eq!(storage.partial_blob_offset(world, &desc).unwrap(), 0);
    assert_eq!(storage.latest_snapshot(world).unwrap().unwrap(), first);
}

#[test]
fn paused_replication_preserves_partial_offset_and_can_resume_after_policy_change() {
    let (_temp, storage, world) = fixture();
    let bytes = b"complete verified data";
    let desc = BlobDescriptor {
        hash: BlobDescriptor::hash_uncompressed(bytes),
        encoding: BlobEncoding::Raw,
        uncompressed_size: bytes.len() as u64,
        encoded_size: bytes.len() as u64,
    };
    let pin = storage.pin_replication_hashes(world, &[desc.hash]).unwrap();
    assert_eq!(storage.receive_blob_chunk(world, &desc, 0, &bytes[..8], false).unwrap(), 8);
    storage.set_world_storage_policy(world, policy(Some(0))).unwrap();
    let denied = storage.receive_blob_chunk(world, &desc, 8, &bytes[8..], true).unwrap_err();
    assert!(matches!(denied, swarm_storage::replica::ReplicationError::Storage(ref err) if err.is_storage_pressure()));
    assert_eq!(storage.partial_blob_offset(world, &desc).unwrap(), 8);
    storage.garbage_collect_blobs(world).unwrap();
    assert_eq!(storage.partial_blob_offset(world, &desc).unwrap(), 8);
    storage.set_world_storage_policy(world, policy(None)).unwrap();
    assert_eq!(storage.receive_blob_chunk(world, &desc, 8, &bytes[8..], true).unwrap(), bytes.len() as u64);
    assert!(storage.has_complete_blob(world, &desc));
    assert_eq!(storage.garbage_collect_blobs(world).unwrap().removed_blobs, 0);
    drop(pin);
    assert_eq!(storage.garbage_collect_blobs(world).unwrap().removed_blobs, 1);
}

#[test]
fn real_world_retention_preserves_entire_chain_and_reclaims_only_unreferenced_bytes() {
    let (temp, storage, world) = fixture();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"one").unwrap();
    let first = checkpoint(&storage, world, &source, None);
    fs::write(source.join("level.dat"), b"two").unwrap();
    let second = checkpoint(&storage, world, &source, Some(&first));
    let orphan = storage.put_blob(world, b"orphan").unwrap();
    let pinned = storage.put_blob(world, b"in flight").unwrap();
    let pin = storage.pin_replication_hashes(world, &[pinned.hash]).unwrap();
    storage.set_world_storage_policy(world, policy(Some(0))).unwrap();
    let result = storage
        .apply_retention(world, &RetentionPolicy { keep_latest: 0, protected_snapshots: BTreeSet::new() })
        .unwrap();
    assert!(result.removed_snapshots.is_empty());
    assert_eq!(result.retained_snapshots, vec![1, 2]);
    assert_eq!(result.removed_blobs, 1);
    assert!(!storage.has_complete_blob(world, &orphan));
    assert!(storage.has_complete_blob(world, &pinned));
    storage.verify_snapshot(&first).unwrap();
    storage.verify_snapshot(&second).unwrap();
    drop(pin);
}

#[test]
fn corrupt_policy_refuses_new_payload_and_explicit_replacement_repairs_it() {
    let (_temp, storage, world) = fixture();
    fs::write(storage.world_dir(world).join("metadata/local-storage-policy.json"), b"invalid").unwrap();
    assert!(storage.put_blob(world, b"not admitted").is_err());
    assert!(storage.world_storage_status(world).is_err());
    storage.set_world_storage_policy(world, policy(None)).unwrap();
    assert!(storage.put_blob(world, b"admitted").is_ok());
}

#[test]
fn missing_historical_manifest_blocks_gc_instead_of_treating_its_blobs_as_orphans() {
    let (temp, storage, world) = fixture();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"first historical save").unwrap();
    let first = checkpoint(&storage, world, &source, None);
    fs::write(source.join("level.dat"), b"second historical save").unwrap();
    let second = checkpoint(&storage, world, &source, Some(&first));
    let first_blob = &first.entries[0].blob;
    let orphan = storage.put_blob(world, b"orphan").unwrap();
    fs::remove_file(storage.world_dir(world).join("snapshots/00000000000000000001.postcard")).unwrap();
    assert!(matches!(storage.garbage_collect_blobs(world),
        Err(swarm_storage::RetentionError::IncompleteCanonicalHistory(id)) if id == world));
    assert!(storage.has_complete_blob(world, first_blob));
    assert!(storage.has_complete_blob(world, &orphan));
    storage.verify_snapshot(&second).unwrap();
}

#[test]
fn concurrent_payload_writers_cannot_both_spend_the_same_world_quota() {
    let (_temp, storage, world) = fixture();
    let baseline = storage.world_storage_status(world).unwrap().world_bytes;
    let limit = baseline + 1024 * 1024 + 4096;
    storage.set_world_storage_policy(world, policy(Some(limit))).unwrap();
    let start = std::sync::Barrier::new(3);
    let results = std::thread::scope(|scope| {
        let threads = [0x35u8, 0x47].map(|byte| {
            let storage = storage.clone();
            let start = &start;
            scope.spawn(move || {
                let data = vec![byte; 1024 * 1024];
                let descriptor = BlobDescriptor {
                    hash: BlobDescriptor::hash_uncompressed(&data),
                    encoding: BlobEncoding::Raw,
                    uncompressed_size: data.len() as u64,
                    encoded_size: data.len() as u64,
                };
                start.wait();
                storage.receive_blob_chunk(world, &descriptor, 0, &data, true)
            })
        });
        start.wait();
        threads.map(|thread| thread.join().unwrap())
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1, "{results:?}");
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result,
        Err(swarm_storage::replica::ReplicationError::Storage(error)) if error.is_storage_pressure()))
            .count(),
        1
    );
    assert!(storage.world_storage_status(world).unwrap().world_bytes <= limit);
}
