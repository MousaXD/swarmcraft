use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use swarm_protocol::{BlobDescriptor, BlobEncoding, PeerId, SnapshotManifestV1, WorldId};
use swarm_storage::{
    replica::ReplicationError,
    scheduler::{BlobSource, LocalReplicaSource, ReplicationOptions, ReplicationScheduler},
    SnapshotContext, Storage, StorageError,
};

fn world() -> WorldId {
    WorldId([0x61; 32])
}

fn pseudo_random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed | 1;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push((state >> 24) as u8);
    }
    bytes
}

fn blob_path(storage: &Storage, descriptor: &BlobDescriptor) -> PathBuf {
    let suffix = match descriptor.encoding {
        BlobEncoding::Raw => "raw",
        BlobEncoding::Zstd => "zst",
    };
    storage.world_dir(world()).join("blobs").join(format!("{}.{}", descriptor.hash.to_hex(), suffix))
}

fn copy_blob(source: &Storage, destination: &Storage, descriptor: &BlobDescriptor) {
    let mut offset = destination.partial_blob_offset(world(), descriptor).unwrap();
    loop {
        let (data, finished) = source.read_encoded_blob_chunk(world(), descriptor, offset, 4096).unwrap();
        offset = destination.receive_blob_chunk(world(), descriptor, offset, &data, finished).unwrap();
        if finished {
            break;
        }
    }
}

fn copy_snapshot(source: &Storage, destination: &Storage, manifest: &SnapshotManifestV1) {
    for descriptor in destination.missing_blobs(manifest) {
        copy_blob(source, destination, &descriptor);
    }
    destination.finalize_replica(manifest).unwrap();
}

fn fixture(temp: &tempfile::TempDir, file_count: usize) -> (PathBuf, Storage, SnapshotManifestV1, Storage, Storage) {
    let source_world = temp.path().join("source-world");
    fs::create_dir_all(source_world.join("region")).unwrap();
    for index in 0..file_count {
        let relative =
            if index == 0 { PathBuf::from("level.dat") } else { PathBuf::from(format!("region/r.{index}.mca")) };
        fs::write(
            source_world.join(relative),
            pseudo_random_bytes(0xCAFE_BABE + index as u64, 96 * 1024 + index * 4096),
        )
        .unwrap();
    }

    let authority = Storage::open(temp.path().join("authority")).unwrap();
    let mut publication = authority
        .snapshot_directory(
            &source_world,
            SnapshotContext {
                world: world(),
                snapshot_number: 1,
                epoch: 1,
                sequence: 9,
                previous_snapshot_hash: None,
                authority_peer_id: PeerId([9; 32]),
                authority_public_key: [8; 32],
            },
        )
        .unwrap();
    publication.signature = vec![0; 64];
    authority.commit_snapshot(&publication).unwrap();
    let manifest = publication.manifest().clone();

    let replica_a = Storage::open(temp.path().join("replica-a")).unwrap();
    let replica_b = Storage::open(temp.path().join("replica-b")).unwrap();
    copy_snapshot(&authority, &replica_a, &manifest);
    copy_snapshot(&authority, &replica_b, &manifest);
    (source_world, authority, manifest, replica_a, replica_b)
}

fn assert_exact_restore(storage: &Storage, manifest: &SnapshotManifestV1, source_world: &Path, destination: &Path) {
    storage.restore_snapshot(manifest, destination).unwrap();
    for entry in &manifest.entries {
        assert_eq!(
            fs::read(destination.join(&entry.path)).unwrap(),
            fs::read(source_world.join(&entry.path)).unwrap(),
            "restored bytes differ for {}",
            entry.path
        );
    }
}

struct SlowSource {
    inner: LocalReplicaSource,
    delay: Duration,
}

impl BlobSource for SlowSource {
    fn peer_id(&self) -> PeerId {
        self.inner.peer_id()
    }

    fn has_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool {
        self.inner.has_blob(world, descriptor)
    }

    fn read_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, bool), ReplicationError> {
        std::thread::sleep(self.delay);
        self.inner.read_blob_chunk(world, descriptor, offset, max_bytes)
    }
}

struct DisappearingSource {
    inner: LocalReplicaSource,
    successful_chunks: usize,
    reads: AtomicUsize,
}

impl BlobSource for DisappearingSource {
    fn peer_id(&self) -> PeerId {
        self.inner.peer_id()
    }

    fn has_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool {
        self.inner.has_blob(world, descriptor)
    }

    fn read_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, bool), ReplicationError> {
        if self.reads.fetch_add(1, Ordering::AcqRel) >= self.successful_chunks {
            return Err(StorageError::Io {
                path: PathBuf::from("simulated-disconnected-replica"),
                source: std::io::Error::new(std::io::ErrorKind::ConnectionReset, "replica disappeared"),
            }
            .into());
        }
        self.inner.read_blob_chunk(world, descriptor, offset, max_bytes)
    }
}

struct PoisonThenDisappearSource {
    inner: LocalReplicaSource,
    reads: AtomicUsize,
}

impl BlobSource for PoisonThenDisappearSource {
    fn peer_id(&self) -> PeerId {
        self.inner.peer_id()
    }

    fn has_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool {
        self.inner.has_blob(world, descriptor)
    }

    fn read_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, bool), ReplicationError> {
        if self.reads.fetch_add(1, Ordering::AcqRel) != 0 {
            return Err(StorageError::Io {
                path: PathBuf::from("simulated-poisoned-disconnected-replica"),
                source: std::io::Error::new(std::io::ErrorKind::ConnectionReset, "replica disappeared"),
            }
            .into());
        }
        let (mut data, finished) = self.inner.read_blob_chunk(world, descriptor, offset, max_bytes)?;
        if let Some(first) = data.first_mut() {
            *first ^= 0x6D;
        }
        Ok((data, finished))
    }
}

struct CorruptingSource {
    inner: LocalReplicaSource,
}

impl BlobSource for CorruptingSource {
    fn peer_id(&self) -> PeerId {
        self.inner.peer_id()
    }

    fn has_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool {
        self.inner.has_blob(world, descriptor)
    }

    fn read_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, bool), ReplicationError> {
        let (mut data, finished) = self.inner.read_blob_chunk(world, descriptor, offset, max_bytes)?;
        if let Some(first) = data.first_mut() {
            *first ^= 0xA5;
        }
        Ok((data, finished))
    }
}

#[test]
fn new_peer_reconstructs_one_snapshot_from_multiple_replicas_concurrently() {
    let temp = tempfile::tempdir().unwrap();
    let (source_world, _authority, manifest, replica_a, replica_b) = fixture(&temp, 6);
    let destination = Storage::open(temp.path().join("destination")).unwrap();
    let scheduler = ReplicationScheduler::new();

    let sources: Vec<Arc<dyn BlobSource>> = vec![
        Arc::new(SlowSource {
            inner: LocalReplicaSource::new(PeerId([1; 32]), replica_a),
            delay: Duration::from_millis(2),
        }),
        Arc::new(SlowSource {
            inner: LocalReplicaSource::new(PeerId([2; 32]), replica_b),
            delay: Duration::from_millis(2),
        }),
    ];
    let report = scheduler
        .reconstruct(&destination, &manifest, sources, ReplicationOptions { max_parallel_blobs: 4, chunk_size: 2048 })
        .unwrap();

    assert_eq!(report.completed_blobs, manifest.entries.len());
    assert!(report.sources_used.contains(&PeerId([1; 32])));
    assert!(report.sources_used.contains(&PeerId([2; 32])));
    assert!(report.per_source[&PeerId([1; 32])].completed_blobs > 0);
    assert!(report.per_source[&PeerId([2; 32])].completed_blobs > 0);
    assert!(report.max_parallel_blobs_observed >= 2);
    destination.verify_snapshot(&manifest).unwrap();
    assert_exact_restore(&destination, &manifest, &source_world, &temp.path().join("restored"));
}

#[test]
fn source_disappearance_mid_blob_resumes_from_another_replica() {
    let temp = tempfile::tempdir().unwrap();
    let (source_world, _authority, manifest, replica_a, replica_b) = fixture(&temp, 1);
    let destination = Storage::open(temp.path().join("destination")).unwrap();
    let scheduler = ReplicationScheduler::new();

    let sources: Vec<Arc<dyn BlobSource>> = vec![
        Arc::new(DisappearingSource {
            inner: LocalReplicaSource::new(PeerId([1; 32]), replica_a),
            successful_chunks: 1,
            reads: AtomicUsize::new(0),
        }),
        Arc::new(LocalReplicaSource::new(PeerId([2; 32]), replica_b)),
    ];
    let report = scheduler
        .reconstruct(&destination, &manifest, sources, ReplicationOptions { max_parallel_blobs: 1, chunk_size: 1024 })
        .unwrap();

    assert!(report.source_failures >= 1);
    assert_eq!(report.resumed_blobs, 1);
    assert_eq!(report.per_source[&PeerId([2; 32])].completed_blobs, 1);
    destination.verify_snapshot(&manifest).unwrap();
    assert_exact_restore(&destination, &manifest, &source_world, &temp.path().join("restored"));
}

#[test]
fn corrupt_source_is_rejected_without_poisoning_snapshot_reconstruction() {
    let temp = tempfile::tempdir().unwrap();
    let (source_world, _authority, manifest, replica_a, replica_b) = fixture(&temp, 1);
    let destination = Storage::open(temp.path().join("destination")).unwrap();
    let scheduler = ReplicationScheduler::new();

    let sources: Vec<Arc<dyn BlobSource>> = vec![
        Arc::new(CorruptingSource { inner: LocalReplicaSource::new(PeerId([1; 32]), replica_a) }),
        Arc::new(LocalReplicaSource::new(PeerId([2; 32]), replica_b)),
    ];
    let report = scheduler
        .reconstruct(&destination, &manifest, sources, ReplicationOptions { max_parallel_blobs: 1, chunk_size: 4096 })
        .unwrap();

    assert!(report.corrupt_rejections >= 1);
    assert!(report.source_failures >= 1);
    assert_eq!(report.per_source[&PeerId([2; 32])].completed_blobs, 1);
    destination.verify_snapshot(&manifest).unwrap();
    assert_exact_restore(&destination, &manifest, &source_world, &temp.path().join("restored"));
}

#[test]
fn existing_partial_transfer_resumes_from_a_different_replica() {
    let temp = tempfile::tempdir().unwrap();
    let (source_world, _authority, manifest, replica_a, replica_b) = fixture(&temp, 1);
    let destination = Storage::open(temp.path().join("destination")).unwrap();
    let descriptor = manifest.entries[0].blob.clone();

    let (first, finished) = replica_a.read_encoded_blob_chunk(world(), &descriptor, 0, 777).unwrap();
    assert!(!finished);
    let offset = destination.receive_blob_chunk(world(), &descriptor, 0, &first, false).unwrap();
    assert_eq!(destination.partial_blob_offset(world(), &descriptor).unwrap(), offset);

    let sources: Vec<Arc<dyn BlobSource>> = vec![Arc::new(LocalReplicaSource::new(PeerId([2; 32]), replica_b))];
    let report = ReplicationScheduler::new()
        .reconstruct(&destination, &manifest, sources, ReplicationOptions { max_parallel_blobs: 1, chunk_size: 2048 })
        .unwrap();

    assert_eq!(report.resumed_blobs, 1);
    assert_eq!(report.completed_blobs, 1);
    destination.verify_snapshot(&manifest).unwrap();
    assert_exact_restore(&destination, &manifest, &source_world, &temp.path().join("restored"));
}

#[test]
fn corrupt_partial_from_disappearing_source_does_not_blame_the_healthy_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let (source_world, _authority, manifest, replica_a, replica_b) = fixture(&temp, 1);
    let destination = Storage::open(temp.path().join("destination")).unwrap();

    let sources: Vec<Arc<dyn BlobSource>> = vec![
        Arc::new(PoisonThenDisappearSource {
            inner: LocalReplicaSource::new(PeerId([1; 32]), replica_a),
            reads: AtomicUsize::new(0),
        }),
        Arc::new(LocalReplicaSource::new(PeerId([2; 32]), replica_b)),
    ];
    let report = ReplicationScheduler::new()
        .reconstruct(&destination, &manifest, sources, ReplicationOptions { max_parallel_blobs: 1, chunk_size: 1024 })
        .unwrap();

    assert_eq!(report.resumed_blobs, 1);
    assert_eq!(report.resume_verification_retries, 1);
    assert!(report.corrupt_rejections >= 1);
    assert_eq!(report.per_source[&PeerId([2; 32])].completed_blobs, 1);
    destination.verify_snapshot(&manifest).unwrap();
    assert_exact_restore(&destination, &manifest, &source_world, &temp.path().join("restored"));
}

#[test]
fn source_inventory_excludes_a_corrupt_local_replica() {
    let temp = tempfile::tempdir().unwrap();
    let (_source_world, _authority, manifest, replica_a, replica_b) = fixture(&temp, 1);
    let descriptor = &manifest.entries[0].blob;
    let path = blob_path(&replica_a, descriptor);
    let mut encoded = fs::read(&path).unwrap();
    let middle = encoded.len() / 2;
    encoded[middle] ^= 0x33;
    fs::write(path, encoded).unwrap();

    let destination = Storage::open(temp.path().join("destination")).unwrap();
    let sources: Vec<Arc<dyn BlobSource>> = vec![
        Arc::new(LocalReplicaSource::new(PeerId([1; 32]), replica_a)),
        Arc::new(LocalReplicaSource::new(PeerId([2; 32]), replica_b)),
    ];
    let inventory = ReplicationScheduler::new().inventory(&destination, &manifest, &sources).unwrap();

    assert!(!inventory.contains(PeerId([1; 32]), descriptor.hash));
    assert!(inventory.contains(PeerId([2; 32]), descriptor.hash));
}
