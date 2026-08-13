use std::fs;
use swarm_protocol::{PeerId, WorldId};
use swarm_storage::{SnapshotContext, Storage};

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
