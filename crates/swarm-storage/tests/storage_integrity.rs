use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};
use swarm_protocol::{
    peer_id_from_public_key, snapshot_state_root, BlobDescriptor, BlobEncoding, EpochMode, EpochRecordV1, Hash32,
    PeerId, RecoveryBallotV1, RecoveryVoteV1, SnapshotEntry, SnapshotManifestV1, WorldId, PROTOCOL_VERSION,
};
use swarm_storage::{RecoveryPromiseResult, SnapshotCommitFence, SnapshotContext, Storage, StorageError};

fn context(
    world: WorldId,
    number: u64,
    sequence: u64,
    previous_snapshot_hash: Option<Hash32>,
    authority_peer_id: PeerId,
    authority_public_key: [u8; 32],
) -> SnapshotContext {
    SnapshotContext {
        world,
        snapshot_number: number,
        epoch: 1,
        sequence,
        previous_snapshot_hash,
        authority_peer_id,
        authority_public_key,
    }
}

fn snapshot(
    storage: &Storage,
    source: &Path,
    world: WorldId,
    number: u64,
    sequence: u64,
    previous_snapshot_hash: Option<Hash32>,
    authority_peer_id: PeerId,
    authority_public_key: [u8; 32],
) -> swarm_storage::SnapshotPublication {
    let mut publication = storage
        .snapshot_directory(
            source,
            context(world, number, sequence, previous_snapshot_hash, authority_peer_id, authority_public_key),
        )
        .unwrap();
    publication.signature = vec![number as u8; 64];
    publication
}

#[test]
fn missing_canonical_head_target_fails_closed_without_number_reuse() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"one").unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let world = WorldId([0x21; 32]);
    let authority_peer_id = PeerId([2; 32]);
    let authority_public_key = [3; 32];

    let first = snapshot(&storage, &source, world, 1, 1, None, authority_peer_id, authority_public_key);
    storage.commit_snapshot(&first).unwrap();
    let first_hash = first.manifest_hash().unwrap();
    fs::write(source.join("level.dat"), b"two").unwrap();
    let second = snapshot(&storage, &source, world, 2, 2, Some(first_hash), authority_peer_id, authority_public_key);
    storage.commit_snapshot(&second).unwrap();

    let newest_path = storage.world_dir(world).join("snapshots").join(format!("{:020}.postcard", 2));
    fs::remove_file(&newest_path).unwrap();

    assert!(matches!(
        storage.latest_snapshot(world),
        Err(StorageError::MissingCanonicalHeadTarget { snapshot_number: 2, .. })
    ));
    assert!(matches!(
        storage.next_snapshot_number(world),
        Err(StorageError::MissingCanonicalHeadTarget { snapshot_number: 2, .. })
    ));

    let replacement =
        snapshot(&storage, &source, world, 2, 2, Some(first_hash), authority_peer_id, authority_public_key);
    assert!(matches!(
        storage.commit_snapshot(&replacement),
        Err(StorageError::MissingCanonicalHeadTarget { snapshot_number: 2, .. })
    ));
}

#[test]
fn durable_fence_rejects_same_epoch_after_fencing_token_changes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"one").unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let world = WorldId([0x22; 32]);
    let authority_peer_id = PeerId([4; 32]);
    let authority_public_key = [5; 32];

    let epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 1,
        previous_epoch_hash: None,
        base_state_hash: Hash32([8; 32]),
        authority_peer_id,
        authority_public_key,
        mode: EpochMode::Quorum,
        fencing_token: 10,
        reason: "initial".into(),
        signature: vec![1; 64],
    };
    storage.save_epoch_record(&epoch).unwrap();

    let first = snapshot(&storage, &source, world, 1, 1, None, authority_peer_id, authority_public_key);
    storage
        .commit_snapshot_fenced(
            &first,
            SnapshotCommitFence { expected_epoch: 1, expected_fencing_token: 10, expected_head: None },
        )
        .unwrap();
    let first_hash = first.manifest_hash().unwrap();
    let observed_head = storage.canonical_snapshot_head(world).unwrap().head;

    fs::write(source.join("level.dat"), b"two").unwrap();
    let second = snapshot(&storage, &source, world, 2, 2, Some(first_hash), authority_peer_id, authority_public_key);
    let mut superseding = epoch.clone();
    superseding.fencing_token = 11;
    superseding.reason = "supersede stale writer".into();
    storage.save_epoch_record(&superseding).unwrap();

    assert!(matches!(
        storage.commit_snapshot_fenced(
            &second,
            SnapshotCommitFence { expected_epoch: 1, expected_fencing_token: 10, expected_head: observed_head },
        ),
        Err(StorageError::SnapshotFenceMismatch { expected_epoch: 1, expected_fencing_token: 10, .. })
    ));
    assert_eq!(storage.latest_snapshot(world).unwrap().unwrap().snapshot_number, 1);
}

#[test]
fn load_snapshot_rejects_embedded_namespace_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("store")).unwrap();
    let world = WorldId([0x23; 32]);
    storage.canonical_snapshot_head(world).unwrap();
    let snapshots = storage.world_dir(world).join("snapshots");
    fs::create_dir_all(&snapshots).unwrap();
    let wrong = SnapshotManifestV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId([0x24; 32]),
        snapshot_number: 1,
        epoch: 1,
        sequence: 1,
        previous_snapshot_hash: None,
        entries: Vec::new(),
        state_root: snapshot_state_root(&[]).unwrap(),
        authority_peer_id: PeerId([2; 32]),
        authority_public_key: [3; 32],
        signature: vec![0; 64],
    };
    fs::write(snapshots.join(format!("{:020}.postcard", 1)), postcard::to_allocvec(&wrong).unwrap()).unwrap();
    assert!(matches!(storage.load_snapshot(world, 1), Err(StorageError::WorldMetadataMismatch)));
}

#[test]
fn portable_case_aliases_are_rejected_before_blob_access() {
    let descriptor =
        BlobDescriptor { hash: Hash32([9; 32]), uncompressed_size: 1, encoded_size: 1, encoding: BlobEncoding::Raw };
    let entries = vec![
        SnapshotEntry { path: "region/Foo.dat".into(), blob: descriptor.clone() },
        SnapshotEntry { path: "region/foo.dat".into(), blob: descriptor },
    ];
    let manifest = SnapshotManifestV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId([0x25; 32]),
        snapshot_number: 1,
        epoch: 1,
        sequence: 1,
        previous_snapshot_hash: None,
        state_root: snapshot_state_root(&entries).unwrap(),
        entries,
        authority_peer_id: PeerId([2; 32]),
        authority_public_key: [3; 32],
        signature: vec![0; 64],
    };
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path()).unwrap();
    assert!(matches!(
        storage.verify_snapshot(&manifest),
        Err(StorageError::PortablePathCollision(path)) if path == "region/foo.dat"
    ));
}

#[test]
fn crash_temp_debris_is_reported_without_automatic_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path()).unwrap();
    let world = WorldId([0x26; 32]);
    let debris = storage.world_dir(world).join("metadata").join(".atomic-dead-process-7.tmp");
    fs::create_dir_all(debris.parent().unwrap()).unwrap();
    fs::write(&debris, b"partial").unwrap();
    let report = storage.storage_temp_debris().unwrap();
    assert_eq!(report, vec![debris.clone()]);
    assert!(debris.exists());
}

fn recovery_ballot(world: WorldId, candidate: u8) -> RecoveryBallotV1 {
    let candidate_public_key = [candidate; 32];
    RecoveryBallotV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        base_epoch: 4,
        base_fencing_token: 8,
        target_epoch: 5,
        target_fencing_token: 9,
        round: 7,
        candidate_peer_id: peer_id_from_public_key(&candidate_public_key),
        candidate_public_key,
        base_snapshot_hash: Hash32([3; 32]),
        base_state_hash: Hash32([4; 32]),
        membership_hash: Hash32([5; 32]),
        signature: Vec::new(),
    }
}

fn recovery_vote(ballot: &RecoveryBallotV1) -> RecoveryVoteV1 {
    let voter_public_key = [6; 32];
    RecoveryVoteV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: ballot.world_id,
        ballot_hash: ballot.ballot_hash().unwrap(),
        base_epoch: ballot.base_epoch,
        target_epoch: ballot.target_epoch,
        round: ballot.round,
        candidate_peer_id: ballot.candidate_peer_id,
        voter_peer_id: peer_id_from_public_key(&voter_public_key),
        voter_public_key,
        signature: Vec::new(),
    }
}

fn wait_for_file(path: &Path) {
    for _ in 0..400 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {}", path.display());
}

#[test]
fn recovery_promise_child_process() {
    if std::env::var_os("SWARMCRAFT_STORAGE_RACE_CHILD").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("SWARMCRAFT_STORAGE_RACE_ROOT").unwrap());
    let candidate: u8 = std::env::var("SWARMCRAFT_STORAGE_RACE_CANDIDATE").unwrap().parse().unwrap();
    let ready = PathBuf::from(std::env::var_os("SWARMCRAFT_STORAGE_RACE_READY").unwrap());
    let start = PathBuf::from(std::env::var_os("SWARMCRAFT_STORAGE_RACE_START").unwrap());
    let output = PathBuf::from(std::env::var_os("SWARMCRAFT_STORAGE_RACE_OUTPUT").unwrap());
    let storage = Storage::open(root).unwrap();
    fs::write(&ready, b"ready").unwrap();
    wait_for_file(&start);
    let ballot = recovery_ballot(WorldId([0x7a; 32]), candidate);
    let result = storage.promise_recovery_ballot(&ballot, &recovery_vote(&ballot)).unwrap();
    let text = match result {
        RecoveryPromiseResult::Accepted => "accepted",
        RecoveryPromiseResult::Idempotent => "idempotent",
        RecoveryPromiseResult::Rejected { .. } => "rejected",
    };
    fs::write(output, text).unwrap();
}

#[test]
fn two_processes_cannot_accept_conflicting_equal_round_recovery_promises() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let start = temp.path().join("start");
    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    let mut outputs = Vec::new();
    let mut ready_files = Vec::new();

    for candidate in [2u8, 3u8] {
        let output = temp.path().join(format!("result-{candidate}"));
        let ready = temp.path().join(format!("ready-{candidate}"));
        let child = Command::new(&exe)
            .arg("--exact")
            .arg("recovery_promise_child_process")
            .arg("--nocapture")
            .env("SWARMCRAFT_STORAGE_RACE_CHILD", "1")
            .env("SWARMCRAFT_STORAGE_RACE_ROOT", &root)
            .env("SWARMCRAFT_STORAGE_RACE_CANDIDATE", candidate.to_string())
            .env("SWARMCRAFT_STORAGE_RACE_READY", &ready)
            .env("SWARMCRAFT_STORAGE_RACE_START", &start)
            .env("SWARMCRAFT_STORAGE_RACE_OUTPUT", &output)
            .spawn()
            .unwrap();
        children.push(child);
        outputs.push(output);
        ready_files.push(ready);
    }

    for ready in &ready_files {
        wait_for_file(ready);
    }
    fs::write(&start, b"go").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let results = outputs.iter().map(|path| fs::read_to_string(path).unwrap()).collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.as_str() == "accepted").count(), 1);
    assert_eq!(results.iter().filter(|result| result.as_str() == "rejected").count(), 1);
}
