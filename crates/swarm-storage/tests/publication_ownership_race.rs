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
    world: WorldId,
    snapshot_number: u64,
    previous_snapshot_hash: Hash32,
    start: Arc<Barrier>,
    commit: Arc<Barrier>,
    published: mpsc::Sender<(SnapshotManifestV1, String)>,
) -> thread::JoinHandle<()> {
    let source = source.to_path_buf();
    thread::spawn(move || {
        start.wait();
        let mut publication = storage
            .snapshot_directory_streaming(&source, context(world, snapshot_number, Some(previous_snapshot_hash)))
            .expect("local publisher should publish complete blobs");
        // Both publishers intentionally produce the exact same immutable manifest.
        publication.signature = vec![snapshot_number as u8; 64];
        published
            .send((publication.manifest().clone(), publication.publication_id().to_owned()))
            .expect("race coordinator should receive publication metadata");

        commit.wait();
        storage.commit_snapshot_streaming(&publication).expect("local publication commit should succeed idempotently");
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

    // Establish a valid direct-parent chain before starting the publication race.
    let mut first = storage
        .snapshot_directory_streaming(&shared_source, context(world, 1, None))
        .expect("first snapshot should publish");
    first.signature = vec![1; 64];
    storage.commit_snapshot_streaming(&first).unwrap();
    let first_hash = first.manifest_hash().unwrap();
    let shared_hash = first.entries[0].blob.hash;

    let mut replica = first.manifest().clone();
    replica.snapshot_number = 2;
    replica.sequence = 2;
    replica.previous_snapshot_hash = Some(first_hash);
    replica.signature = vec![2; 64];
    storage.finalize_replica(&replica).expect("replica manifest should commit over the shared complete blob");
    let replica_hash = replica.manifest_hash().unwrap();

    let mut latest = storage
        .snapshot_directory_streaming(&latest_source, context(world, 3, Some(replica_hash)))
        .expect("newer committed snapshot should publish");
    latest.signature = vec![3; 64];
    storage.commit_snapshot_streaming(&latest).unwrap();
    let latest_hash = latest.manifest_hash().unwrap();

    // Two live local publishers now prepare the same direct successor. Their
    // ownership pins must remain independent until each exact-idempotent commit.
    let start = Arc::new(Barrier::new(3));
    let commit = Arc::new(Barrier::new(3));
    let (published_tx, published_rx) = mpsc::channel();
    let left = spawn_publisher(
        storage.clone(),
        &shared_source,
        world,
        4,
        latest_hash,
        start.clone(),
        commit.clone(),
        published_tx.clone(),
    );
    let right = spawn_publisher(
        storage.clone(),
        &shared_source,
        world,
        4,
        latest_hash,
        start.clone(),
        commit.clone(),
        published_tx,
    );

    start.wait();
    let mut locals = [published_rx.recv().unwrap(), published_rx.recv().unwrap()];
    locals.sort_by(|left, right| left.1.cmp(&right.1));
    assert_eq!(locals[0].0.manifest_hash().unwrap(), locals[1].0.manifest_hash().unwrap());
    assert_ne!(locals[0].1, locals[1].1);
    assert!(publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    let during = storage
        .apply_retention(world, &RetentionPolicy { keep_latest: 1, protected_snapshots: BTreeSet::new() })
        .expect("GC and retention should coexist with in-flight publication owners");
    assert_eq!(
        during.removed_blobs, 0,
        "shared blob must remain live because both local publication owners pin it after older manifests are pruned"
    );
    assert_eq!(
        storage.list_snapshots(world).unwrap().iter().map(|manifest| manifest.snapshot_number).collect::<Vec<_>>(),
        vec![3]
    );
    assert!(publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    commit.wait();
    left.join().expect("first publisher thread should complete");
    right.join().expect("second publisher thread should complete");
    assert!(!publication_has_pin(&storage, world, &locals[0].1, shared_hash));
    assert!(!publication_has_pin(&storage, world, &locals[1].1, shared_hash));

    let after = storage
        .apply_retention(world, &RetentionPolicy { keep_latest: 2, protected_snapshots: BTreeSet::new() })
        .expect("post-publication retention should complete");
    assert_eq!(after.removed_blobs, 0, "retained local manifest must keep the shared blob live");

    let committed = storage.list_snapshots(world).unwrap();
    assert_eq!(committed.last().unwrap().snapshot_number, 4);
    for manifest in committed {
        storage
            .verify_snapshot_streaming(&manifest)
            .expect("no committed manifest may reference a missing or corrupt blob");
    }
}
