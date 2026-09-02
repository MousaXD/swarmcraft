use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{mpsc, Arc, Barrier},
    thread,
};
use swarm_protocol::{Hash32, PeerId, SnapshotManifestV1, WorldId};
use swarm_storage::{RetentionPolicy, SnapshotContext, Storage};

fn context(world: WorldId, snapshot_number: u64, previous_snapshot_hash: Option<Hash32>) -> SnapshotContext {
    SnapshotContext {
        world,
        snapshot_number,
        epoch: 7,
        sequence: snapshot_number,
        previous_snapshot_hash,
        authority_peer_id: PeerId([0x31; 32]),
        authority_public_key: [0x42; 32],
    }
}

fn publication_has_pin(storage: &Storage, world: WorldId, publication_id: &str, hash: Hash32) -> bool {
    storage
        .world_dir(world)
        .join(".snapshot-publication-pins")
        .join(publication_id)
        .join(format!("{}.pin", hash.to_hex()))
        .is_file()
}

fn spawn_publisher(
    storage: Storage,
    source: &Path,
    context: SnapshotContext,
    start: Arc<Barrier>,
    release: Arc<Barrier>,
    published: mpsc::Sender<(SnapshotManifestV1, String)>,
) -> thread::JoinHandle<()> {
    let source = source.to_path_buf();
    thread::spawn(move || {
        start.wait();
        let mut publication = storage
            .snapshot_directory_streaming(&source, context)
            .expect("local publisher should publish complete blobs");
        publication.signature = vec![context.snapshot_number as u8; 64];
        published
            .send((publication.manifest().clone(), publication.publication_id().to_owned()))
            .expect("race coordinator should receive publication metadata");

        release.wait();
        drop(publication);
    })
}

#[test]
fn simultaneous_local_publishers_survive_replica_commit_gc_and_retention() {
    let temp = tempfile::tempdir().unwrap();
    let parent_source = temp.path().join("parent-source");
    let shared_source = temp.path().join("shared-source");
    let latest_source = temp.path().join("latest-source");
    fs::create_dir_all(&parent_source).unwrap();
    fs::create_dir_all(&shared_source).unwrap();
    fs::create_dir_all(&latest_source).unwrap();
    fs::write(parent_source.join("level.dat"), b"canonical-parent").unwrap();
    fs::write(shared_source.join("level.dat"), vec![0x5a; 2 * 1024 * 1024]).unwrap();
    fs::write(latest_source.join("level.dat"), b"newer-committed-snapshot").unwrap();

    let storage = Storage::open(temp.path().join("store")).unwrap();
    let world = WorldId([0x77; 32]);

    let mut parent = storage.snapshot_directory_streaming(&parent_source, context(world, 1, None)).unwrap();
    parent.signature = vec![0x61; 64];
    storage.commit_snapshot_streaming(&parent).unwrap();
    let parent_hash = parent.manifest_hash().unwrap();

    let start = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(3));
    let (published_tx, published_rx) = mpsc::channel();
    let child_context = context(world, 2, Some(parent_hash));

    let first = spawn_publisher(
        storage.clone(),
        &shared_source,
        child_context,
        start.clone(),
        release.clone(),
        published_tx.clone(),
    );
    let second = spawn_publisher(storage.clone(), &shared_source, child_context, start.clone(), release.clone(), published_tx);

    start.wait();
    let mut locals = [published_rx.recv().unwrap(), published_rx.recv().unwrap()];
    locals.sort_by(|a, b| a.1.cmp(&b.1));
    let shared_blob = locals[0].0.entries[0].blob.clone();
    let shared_hash = shared_blob.hash;
    assert_eq!(shared_hash, locals[1].0.entries[0].blob.hash);
    assert_ne!(locals[0].1, locals[1].1);
    assert!(publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    let mut replica = locals[0].0.clone();
    replica.signature = vec![0x63; 64];
    storage.finalize_replica(&replica).expect("direct-child replica should commit over the shared complete blob");
    assert!(publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    let replica_hash = replica.manifest_hash().unwrap();
    let mut latest = storage
        .snapshot_directory_streaming(&latest_source, context(world, 3, Some(replica_hash)))
        .expect("newer direct-child snapshot should publish");
    latest.signature = vec![0x64; 64];
    storage.commit_snapshot_streaming(&latest).unwrap();

    storage
        .apply_retention(world, &RetentionPolicy { keep_latest: 1, protected_snapshots: BTreeSet::new() })
        .expect("GC and retention should coexist with in-flight publication owners");
    assert!(!storage.list_snapshots(world).unwrap().iter().any(|manifest| manifest.snapshot_number == 2));
    assert!(
        storage.read_blob(world, &shared_blob).is_ok(),
        "shared blob must remain live because both stale local publication owners still pin it"
    );
    assert!(publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    release.wait();
    first.join().expect("first publisher thread should exit cleanly");
    second.join().expect("second publisher thread should exit cleanly");

    storage
        .apply_retention(world, &RetentionPolicy { keep_latest: 1, protected_snapshots: BTreeSet::new() })
        .expect("post-publication retention should reclaim abandoned stale-publication data");
    assert!(
        storage.read_blob(world, &shared_blob).is_err(),
        "once both stale publishers exit, their unreferenced shared blob should be collectible"
    );

    let committed = storage.list_snapshots(world).unwrap();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].snapshot_number, 3);
    storage
        .verify_snapshot_streaming(&committed[0])
        .expect("the retained canonical manifest must reference complete, uncorrupted blobs");
}
