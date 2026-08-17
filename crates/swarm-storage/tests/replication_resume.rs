use std::fs;
use swarm_protocol::{BlobDescriptor, BlobEncoding, Hash32, PeerId, WorldId, BLOB_HASH_DOMAIN};
use swarm_storage::{replica::ReplicationError, SnapshotContext, Storage, StorageError};

#[test]
fn replication_resumes_after_storage_restart() {
    let temp = tempfile::tempdir().unwrap();
    let world = WorldId([9; 32]);
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("region")).unwrap();
    let region = (0..65_536).map(|index| (index % 251) as u8).collect::<Vec<_>>();
    fs::write(source.join("region/r.0.0.mca"), &region).unwrap();

    let authority = Storage::open(temp.path().join("authority")).unwrap();
    let mut manifest = authority
        .snapshot_directory(
            &source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 1,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: PeerId([1; 32]),
                authority_public_key: [2; 32],
            },
        )
        .unwrap();
    manifest.signature = vec![0; 64];
    authority.commit_snapshot(&manifest).unwrap();

    let replica_root = temp.path().join("replica");
    let replica = Storage::open(&replica_root).unwrap();
    for descriptor in replica.missing_blobs(&manifest) {
        let (first, finished) = authority.read_encoded_blob_chunk(world, &descriptor, 0, 1).unwrap();
        assert!(!finished);
        let mut offset = replica.receive_blob_chunk(world, &descriptor, 0, &first, false).unwrap();

        let resumed = Storage::open(&replica_root).unwrap();
        assert_eq!(resumed.partial_blob_offset(world, &descriptor).unwrap(), offset);
        loop {
            let (chunk, finished) = authority.read_encoded_blob_chunk(world, &descriptor, offset, 4096).unwrap();
            offset = resumed.receive_blob_chunk(world, &descriptor, offset, &chunk, finished).unwrap();
            if finished {
                break;
            }
        }
    }

    let replica = Storage::open(&replica_root).unwrap();
    replica.finalize_replica(&manifest).unwrap();
    let restored = temp.path().join("restored");
    replica.restore_snapshot(&manifest, &restored).unwrap();
    assert_eq!(fs::read(restored.join("region/r.0.0.mca")).unwrap(), region);
}

#[test]
fn replica_rejects_zstd_amplification_without_publishing_poisoned_blob() {
    let temp = tempfile::tempdir().unwrap();
    let world = WorldId([7; 32]);
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("tiny.bin"), [0u8]).unwrap();

    let authority = Storage::open(temp.path().join("authority")).unwrap();
    let mut manifest = authority
        .snapshot_directory(
            &source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 1,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: PeerId([1; 32]),
                authority_public_key: [2; 32],
            },
        )
        .unwrap();

    let expanded = vec![0u8; 8 * 1024 * 1024];
    let encoded = zstd::stream::encode_all(expanded.as_slice(), 3).unwrap();
    let mut hasher = blake3::Hasher::new();
    hasher.update(BLOB_HASH_DOMAIN);
    hasher.update(&[0]);
    let descriptor = BlobDescriptor {
        hash: Hash32(*hasher.finalize().as_bytes()),
        uncompressed_size: 1,
        encoded_size: encoded.len() as u64,
        encoding: BlobEncoding::Zstd,
    };
    manifest.entries[0].blob = descriptor.clone();

    let replica = Storage::open(temp.path().join("replica")).unwrap();
    let error = replica.receive_blob_chunk(world, &descriptor, 0, &encoded, true).unwrap_err();
    assert!(matches!(
        error,
        ReplicationError::Storage(StorageError::BlobCorrupt(hash)) if hash == descriptor.hash
    ));
    assert!(!replica.has_complete_blob(world, &descriptor));
    assert_eq!(replica.partial_blob_offset(world, &descriptor).unwrap(), 0);
    assert!(matches!(
        replica.finalize_replica(&manifest),
        Err(ReplicationError::Incomplete(snapshot)) if snapshot == manifest.snapshot_number
    ));
    assert!(replica.latest_snapshot(world).unwrap().is_none());
}
