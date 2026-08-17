use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{mpsc, Arc, Barrier},
    thread,
};
use swarm_protocol::{PeerId, SnapshotManifestV1, WorldId};
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

fn publication_pin_count(storage: &Storage, world: WorldId) -> usize {
    let pins = storage.world_dir(world).join(".snapshot-publication-pins");
    match fs::read_dir(pins) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("pin"))
            .count(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => panic!("failed to inspect publication pins: {error}"),
    }
}

fn spawn_publisher(
    storage: Storage,
    source: &Path,
    world: WorldId,
    snapshot_number: u64,
    start: Arc<Barrier>,
    commit: Arc<Barrier>,
    manifests: mpsc::Sender<SnapshotManifestV1>,
) -> thread::JoinHandle<()> {
    let source = source.to_path_buf();
    thread::spawn(move || {
        start.wait();
        let mut manifest = storage
            .snapshot_directory_streaming(&source, context(world, snapshot_number))
            .expect("local publisher should publish complete blobs");
        manifest.signature = vec![snapshot_number as u8; 64];
        manifests.send(manifest.clone()).expect("race coordinator should receive manifest");

        // Keep this transaction's durable publication owner and pins alive while
        // the replica commit, GC, and retention all execute.
        commit.wait();
        storage.commit_snapshot_streaming(&manifest).expect("local manifest commit should succeed");
    })
}

#[test]
fn concurrent_local_publishers_keep_distinct_ownership_through_replica_gc_and_retention() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    // A shared blob large enough that both publisher threads execute the
    // streaming path concurrently, while still keeping this CI test lightweight.
    fs::write(source.join("level.dat"), vec![0x5a; 2 * 1024 * 1024]).unwrap();

    let storage = Storage::open(temp.path().join("store")).unwrap();
    let world = WorldId([0x77; 32]);
    let start = Arc::new(Barrier::new(3));
    let commit = Arc::new(Barrier::new(3));
    let (manifest_tx, manifest_rx) = mpsc::channel();

    let first = spawn_publisher(
        storage.clone(),
        &source,
        world,
        1,
        start.clone(),
        commit.clone(),
        manifest_tx.clone(),
    );
    let second = spawn_publisher(
        storage.clone(),
        &source,
        world,
        2,
        start.clone(),
        commit.clone(),
        manifest_tx,
    );

    start.wait();
    let mut local = vec![manifest_rx.recv().unwrap(), manifest_rx.recv().unwrap()];
    local.sort_by_key(|manifest| manifest.snapshot_number);
    assert_eq!(local[0].entries[0].blob.hash, local[1].entries[0].blob.hash);
    assert_eq!(
        publication_pin_count(&storage, world),
        2,
        "each uncommitted local publication must retain its own owner pin for the shared blob"
    );

    // Reuse the already-complete blob through the real replica finalization
    // path. This deliberately has the same blob hash as both in-flight local
    // publishers but a different snapshot identity.
    let mut replica_manifest = local[0].clone();
    replica_manifest.snapshot_number = 3;
    replica_manifest.sequence = 3;
    replica_manifest.signature = vec![0x63; 64];
    storage.finalize_replica(&replica_manifest).expect("replica manifest should commit over complete shared blob");
    assert_eq!(
        publication_pin_count(&storage, world),
        2,
        "replica commit must not consume either local publisher's owner pin"
    );

    let during = storage
        .apply_retention(
            world,
            &RetentionPolicy { keep_latest: 1, protected_snapshots: BTreeSet::new() },
        )
        .expect("GC and retention should coexist with in-flight publication owners");
    assert_eq!(during.removed_blobs, 0);
    assert_eq!(publication_pin_count(&storage, world), 2);

    commit.wait();
    first.join().expect("first publisher thread should complete");
    second.join().expect("second publisher thread should complete");
    assert_eq!(publication_pin_count(&storage, world), 0, "each commit should release exactly its own owner pin");

    let after = storage
        .apply_retention(
            world,
            &RetentionPolicy { keep_latest: 2, protected_snapshots: BTreeSet::new() },
        )
        .expect("post-publication retention should complete");
    assert_eq!(after.removed_blobs, 0, "shared blob is still referenced by retained manifests");

    let committed = storage.list_snapshots(world).unwrap();
    assert!(!committed.is_empty());
    for manifest in committed {
        storage
            .verify_snapshot_streaming(&manifest)
            .expect("no committed manifest may reference a missing or corrupt blob");
    }
}
