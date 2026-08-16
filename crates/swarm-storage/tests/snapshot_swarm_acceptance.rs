use std::{fs, path::PathBuf};
use swarm_protocol::{BlobDescriptor, BlobEncoding, PeerId, SnapshotManifestV1, WorldId};
use swarm_storage::{ReplicationError, SnapshotContext, Storage};

fn world() -> WorldId {
    WorldId([0x42; 32])
}

fn synthetic_bytes(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| seed.wrapping_add((index as u8).wrapping_mul(31)).rotate_left((index % 7) as u32))
        .collect()
}

fn unique_descriptors(manifest: &SnapshotManifestV1) -> Vec<BlobDescriptor> {
    let mut descriptors = Vec::new();
    for entry in &manifest.entries {
        if !descriptors.iter().any(|descriptor: &BlobDescriptor| descriptor.hash == entry.blob.hash) {
            descriptors.push(entry.blob.clone());
        }
    }
    descriptors
}

fn blob_path(storage: &Storage, descriptor: &BlobDescriptor) -> PathBuf {
    let suffix = match descriptor.encoding {
        BlobEncoding::Raw => "raw",
        BlobEncoding::Zstd => "zst",
    };
    storage
        .root()
        .join("worlds")
        .join(world().to_hex())
        .join("blobs")
        .join(format!("{}.{}", descriptor.hash.to_hex(), suffix))
}

fn copy_blob(source: &Storage, destination: &Storage, descriptor: &BlobDescriptor) -> Result<(), ReplicationError> {
    let mut offset = destination.partial_blob_offset(world(), descriptor)?;
    loop {
        let (data, finished) = source.read_encoded_blob_chunk(world(), descriptor, offset, 4096)?;
        offset = destination.receive_blob_chunk(world(), descriptor, offset, &data, finished)?;
        if finished {
            return Ok(());
        }
    }
}

fn replicate_snapshot(source: &Storage, destination: &Storage, manifest: &SnapshotManifestV1) {
    for descriptor in destination.missing_blobs(manifest) {
        copy_blob(source, destination, &descriptor).unwrap();
    }
    destination.finalize_replica(manifest).unwrap();
    destination.verify_snapshot(manifest).unwrap();
}

#[test]
fn fourth_peer_reconstructs_from_surviving_replicas_and_skips_corrupt_source() {
    let temp = tempfile::tempdir().unwrap();
    let source_world = temp.path().join("source-world");
    fs::create_dir_all(source_world.join("region")).unwrap();
    fs::write(source_world.join("level.dat"), synthetic_bytes(1, 32 * 1024)).unwrap();
    fs::write(source_world.join("region/r.0.0.mca"), synthetic_bytes(2, 96 * 1024)).unwrap();
    fs::write(source_world.join("region/r.0.1.mca"), synthetic_bytes(3, 128 * 1024)).unwrap();

    let peer_a_root = temp.path().join("peer-a");
    let peer_a = Storage::open(&peer_a_root).unwrap();
    let mut manifest = peer_a
        .snapshot_directory(
            &source_world,
            SnapshotContext {
                world: world(),
                snapshot_number: 1,
                epoch: 1,
                sequence: 11,
                previous_snapshot_hash: None,
                authority_peer_id: PeerId([1; 32]),
                authority_public_key: [2; 32],
            },
        )
        .unwrap();
    manifest.signature = vec![0; 64];
    peer_a.commit_snapshot(&manifest).unwrap();
    peer_a.verify_snapshot(&manifest).unwrap();

    let peer_b = Storage::open(temp.path().join("peer-b")).unwrap();
    let peer_c = Storage::open(temp.path().join("peer-c")).unwrap();
    replicate_snapshot(&peer_a, &peer_b, &manifest);
    replicate_snapshot(&peer_a, &peer_c, &manifest);

    let descriptors = unique_descriptors(&manifest);
    assert!(descriptors.len() >= 3, "fixture must create at least three distinct blobs");

    // The original peer disappears completely. The fourth peer must recover only from B and C.
    drop(peer_a);
    fs::remove_dir_all(&peer_a_root).unwrap();
    assert!(!peer_a_root.exists());

    // B loses one blob. C still has it, so no single surviving replica is sufficient.
    fs::remove_file(blob_path(&peer_b, &descriptors[0])).unwrap();
    assert!(!peer_b.has_complete_blob(world(), &descriptors[0]));

    // C keeps a same-size but corrupt encoding for a different blob. This is important:
    // metadata/length checks alone must not allow the bad replica to poison a new peer.
    let corrupt_path = blob_path(&peer_c, &descriptors[1]);
    let mut corrupt = fs::read(&corrupt_path).unwrap();
    assert!(!corrupt.is_empty());
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 0x5A;
    fs::write(&corrupt_path, corrupt).unwrap();
    assert!(!peer_c.has_complete_blob(world(), &descriptors[1]));

    let peer_d = Storage::open(temp.path().join("peer-d")).unwrap();
    for (index, descriptor) in descriptors.iter().enumerate() {
        let copied = if index == 0 {
            // B is missing this blob, so D must fall through to C.
            copy_blob(&peer_b, &peer_d, descriptor).is_ok() || copy_blob(&peer_c, &peer_d, descriptor).is_ok()
        } else if index == 1 {
            // C serves corrupt bytes first. The failed final verification must discard the
            // poisoned partial so B can restart this blob from offset zero.
            assert!(copy_blob(&peer_c, &peer_d, descriptor).is_err());
            assert_eq!(peer_d.partial_blob_offset(world(), descriptor).unwrap(), 0);
            copy_blob(&peer_b, &peer_d, descriptor).is_ok()
        } else {
            copy_blob(&peer_b, &peer_d, descriptor).is_ok() || copy_blob(&peer_c, &peer_d, descriptor).is_ok()
        };
        assert!(copied, "surviving replicas should provide blob {}", descriptor.hash);
    }

    peer_d.finalize_replica(&manifest).unwrap();
    peer_d.verify_snapshot(&manifest).unwrap();
    assert!(peer_d.missing_blobs(&manifest).is_empty());

    let restored = temp.path().join("restored-on-peer-d");
    peer_d.restore_snapshot(&manifest, &restored).unwrap();
    assert_eq!(fs::read(restored.join("level.dat")).unwrap(), fs::read(source_world.join("level.dat")).unwrap());
    assert_eq!(
        fs::read(restored.join("region/r.0.0.mca")).unwrap(),
        fs::read(source_world.join("region/r.0.0.mca")).unwrap()
    );
    assert_eq!(
        fs::read(restored.join("region/r.0.1.mca")).unwrap(),
        fs::read(source_world.join("region/r.0.1.mca")).unwrap()
    );
}
