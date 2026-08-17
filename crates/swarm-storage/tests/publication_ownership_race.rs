use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{mpsc, Arc, Barrier},
    thread,
};
use swarm_protocol::{Hash32, PeerId, SnapshotManifestV1, WorldId};
use swarm_storage::{RetentionPolicy, SnapshotContext, Storage};

fn context(world: WorldId, snapshot_number: u64) -> SnapshotContext {
    SnapshotContext {
        world,
        snapshot_number,
        epoch: 7,
        sequence: snapshot_number,
        previous_snapshot_hash: None,
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
    world: WorldId,
    snapshot_number: u64,
    start: Arc<Barrier>,
    commit: Arc<Barrier>,
    published: mpsc::Sender<(SnapshotManifestV1, String)>,
) -> thread::JoinHandle<()> {
    let source = source.to_path_buf();
    thread::spawn(move || {
        start.wait();
        let mut publication = storage
            .snapshot_directory_streaming(&source, context(world, snapshot_number))
            .expect("local publisher should publish complete blobs");
        publication.signature = vec![snapshot_number as u8; 64];
        published
            .send((publication.manifest().clone(), publication.publication_id().to_owned()))
            .expect("race coordinator should receive publication metadata");

        commit.wait();
        storage
            .commit_snapshot_streaming(&publication)
            .expect("local publication commit should succeed");
    })
}

#[test]
fn simultaneous_local_publishers_survive_replica_commit_gc_and_retention() {
    let temp = tempfile::tempdir().unwrap();
    let shared_source = temp.path().join("shared-source");
    let latest_source = temp.path().join("latest-source");
    fs::create_dir_all(&shared_source).unwrap();
    fs::create_dir_all(&latest_source).unwrap();
    fs::write(shared_source.join("level.dat"), vec![0x5a; 2 * 1024 * 1024]).unwrap();
    fs::write(latest_source.join("level.dat"), b"newer-committed-snapshot").unwrap();

    let storage = Storage::open(temp.path().join("store")).unwrap();
    let world = WorldId([0x77; 32]);
    let start = Arc::new(Barrier::new(3));
    let commit = Arc::new(Barrier::new(3));
    let (published_tx, published_rx) = mpsc::channel();

    let first = spawn_publisher(
        storage.clone(),
        &shared_source,
        world,
        1,
        start.clone(),
        commit.clone(),
        published_tx.clone(),
    );
    let second = spawn_publisher(
        storage.clone(),
        &shared_source,
        world,
        2,
        start.clone(),
        commit.clone(),
        published_tx,
    );

    start.wait();
    let mut locals = vec![published_rx.recv().unwrap(), published_rx.recv().unwrap()];
    locals.sort_by_key(|(manifest, _)| manifest.snapshot_number);
    let shared_hash = locals[0].0.entries[0].blob.hash;
    assert_eq!(shared_hash, locals[1].0.entries[0].blob.hash);
    assert_ne!(locals[0].1, locals[1].1);
    assert!(publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    let mut replica = locals[0].0.clone();
    replica.snapshot_number = 3;
    replica.sequence = 3;
    replica.signature = vec![0x63; 64];
    storage.finalize_replica(&replica).expect("replica manifest should commit over the shared complete blob");
    assert!(publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    let mut latest = storage
        .snapshot_directory_streaming(&latest_source, context(world, 4))
        .expect("newer committed snapshot should publish");
    latest.signature = vec![0x64; 64];
    storage.commit_snapshot_streaming(&latest).unwrap();

    let during = storage
        .apply_retention(
            world,
            &RetentionPolicy { keep_latest: 1, protected_snapshots: BTreeSet::new() },
        )
        .expect("GC and retention should coexist with in-flight publication owners");
    assert_eq!(during.removed_blobs, 0, "shared blob must remain live only because both local publication owners pin it");
    assert!(!storage.list_snapshots(world).unwrap().iter().any(|manifest| manifest.snapshot_number == 3));
    assert!(publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    commit.wait();
    first.join().expect("first publisher thread should complete");
    second.join().expect("second publisher thread should complete");
    assert!(!publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(!publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    let after = storage
        .apply_retention(
            world,
            &RetentionPolicy { keep_latest: 2, protected_snapshots: BTreeSet::new() },
        )
        .expect("post-publication retention should complete");
    assert_eq!(after.removed_blobs, 0, "retained local manifest must keep the shared blob live");

    let committed = storage.list_snapshots(world).unwrap();
    assert!(!committed.is_empty());
    for manifest in committed {
        storage
            .verify_snapshot_streaming(&manifest)
            .expect("no committed manifest may reference a missing or corrupt blob");
    }
}
