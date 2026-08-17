//! Multi-source, bounded snapshot reconstruction.
//!
//! This module deliberately stays transport-agnostic. Network code can populate a
//! [`ReplicaInventory`] and use [`BlobSourceSelector`], while local/in-process
//! callers can use [`ReplicationScheduler`] directly with [`BlobSource`]
//! implementations.

use crate::{
    replica::ReplicationError,
    retention::RetentionError,
    Storage, StorageError,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use swarm_protocol::{BlobDescriptor, Hash32, PeerId, SnapshotManifestV1, WorldId};
use thiserror::Error;
use tracing::{debug, warn};

pub const DEFAULT_PARALLEL_BLOBS: usize = 4;
pub const MAX_PARALLEL_BLOBS: usize = 32;
pub const DEFAULT_REPLICATION_CHUNK_SIZE: usize = 256 * 1024;
pub const MAX_REPLICATION_CHUNK_SIZE: usize = 4 * 1024 * 1024;

pub trait BlobSource: Send + Sync {
    fn peer_id(&self) -> PeerId;

    fn has_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool;

    fn read_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, bool), ReplicationError>;
}

#[derive(Debug, Clone)]
pub struct LocalReplicaSource {
    peer_id: PeerId,
    storage: Storage,
}

impl LocalReplicaSource {
    pub fn new(peer_id: PeerId, storage: Storage) -> Self {
        Self { peer_id, storage }
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }
}

impl BlobSource for LocalReplicaSource {
    fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    fn has_blob(&self, world: WorldId, descriptor: &BlobDescriptor) -> bool {
        self.storage.has_complete_blob(world, descriptor)
    }

    fn read_blob_chunk(
        &self,
        world: WorldId,
        descriptor: &BlobDescriptor,
        offset: u64,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, bool), ReplicationError> {
        self.storage.read_encoded_blob_chunk(world, descriptor, offset, max_bytes)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplicaInventory {
    by_blob: BTreeMap<Hash32, BTreeSet<PeerId>>,
}

impl ReplicaInventory {
    pub fn record_complete_blob(&mut self, peer: PeerId, hash: Hash32) {
        self.by_blob.entry(hash).or_default().insert(peer);
    }

    pub fn remove_peer(&mut self, peer: PeerId) {
        for peers in self.by_blob.values_mut() {
            peers.remove(&peer);
        }
        self.by_blob.retain(|_, peers| !peers.is_empty());
    }

    pub fn sources_for(&self, hash: Hash32) -> Vec<PeerId> {
        self.by_blob.get(&hash).map_or_else(Vec::new, |peers| peers.iter().copied().collect())
    }

    pub fn contains(&self, peer: PeerId, hash: Hash32) -> bool {
        self.by_blob.get(&hash).is_some_and(|peers| peers.contains(&peer))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobAssignment {
    pub descriptor: BlobDescriptor,
    pub candidates: Vec<PeerId>,
}

#[derive(Debug, Clone, Default)]
pub struct BlobSourceSelector;

impl BlobSourceSelector {
    pub fn assign(
        &self,
        missing: &[BlobDescriptor],
        inventory: &ReplicaInventory,
    ) -> Result<Vec<BlobAssignment>, SchedulerError> {
        let mut assigned_bytes: BTreeMap<PeerId, u64> = BTreeMap::new();
        let mut assignments = Vec::with_capacity(missing.len());

        for descriptor in missing {
            let mut candidates = inventory.sources_for(descriptor.hash);
            if candidates.is_empty() {
                return Err(SchedulerError::NoSource(descriptor.hash));
            }
            candidates.sort_by_key(|peer| (assigned_bytes.get(peer).copied().unwrap_or(0), *peer));
            let primary = candidates[0];
            *assigned_bytes.entry(primary).or_default() = assigned_bytes
                .get(&primary)
                .copied()
                .unwrap_or(0)
                .saturating_add(descriptor.encoded_size);

            // Keep the chosen primary first. Re-rank fallbacks by their current
            // assigned load so a failed source does not funnel all work onto one peer.
            let mut fallbacks = candidates[1..].to_vec();
            fallbacks.sort_by_key(|peer| (assigned_bytes.get(peer).copied().unwrap_or(0), *peer));
            let mut ordered = Vec::with_capacity(candidates.len());
            ordered.push(primary);
            ordered.extend(fallbacks);
            assignments.push(BlobAssignment { descriptor: descriptor.clone(), candidates: ordered });
        }

        Ok(assignments)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationOptions {
    pub max_parallel_blobs: usize,
    pub chunk_size: usize,
}

impl Default for ReplicationOptions {
    fn default() -> Self {
        Self {
            max_parallel_blobs: DEFAULT_PARALLEL_BLOBS,
            chunk_size: DEFAULT_REPLICATION_CHUNK_SIZE,
        }
    }
}

impl ReplicationOptions {
    fn validate(self) -> Result<Self, SchedulerError> {
        if self.max_parallel_blobs == 0 || self.max_parallel_blobs > MAX_PARALLEL_BLOBS {
            return Err(SchedulerError::InvalidOptions(format!(
                "max_parallel_blobs must be in 1..={MAX_PARALLEL_BLOBS}"
            )));
        }
        if self.chunk_size == 0 || self.chunk_size > MAX_REPLICATION_CHUNK_SIZE {
            return Err(SchedulerError::InvalidOptions(format!(
                "chunk_size must be in 1..={MAX_REPLICATION_CHUNK_SIZE}"
            )));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceReplicationStats {
    pub attempted_blobs: usize,
    pub completed_blobs: usize,
    pub failures: usize,
    pub corrupt_rejections: usize,
    pub bytes_received: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplicationReport {
    pub total_blobs: usize,
    pub completed_blobs: usize,
    pub resumed_blobs: usize,
    pub source_failures: usize,
    pub corrupt_rejections: usize,
    pub bytes_received: u64,
    pub max_parallel_blobs_observed: usize,
    pub sources_used: BTreeSet<PeerId>,
    pub per_source: BTreeMap<PeerId, SourceReplicationStats>,
}

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Replication(#[from] ReplicationError),
    #[error(transparent)]
    Retention(#[from] RetentionError),
    #[error("invalid replication options: {0}")]
    InvalidOptions(String),
    #[error("multiple BlobSource instances use peer id {0}")]
    DuplicateSource(PeerId),
    #[error("snapshot contains conflicting descriptors for blob {0}")]
    ConflictingDescriptor(Hash32),
    #[error("no replica advertises blob {0}")]
    NoSource(Hash32),
    #[error("all replica sources failed for blob {hash}: {last_error}")]
    AllSourcesFailed { hash: Hash32, last_error: String },
    #[error("replication worker panicked")]
    WorkerPanicked,
}

#[derive(Debug, Clone, Default)]
pub struct ReplicationScheduler {
    selector: BlobSourceSelector,
}

impl ReplicationScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inventory(
        &self,
        destination: &Storage,
        manifest: &SnapshotManifestV1,
        sources: &[Arc<dyn BlobSource>],
    ) -> Result<ReplicaInventory, SchedulerError> {
        validate_descriptor_consistency(manifest)?;
        let missing = destination.missing_blobs(manifest);
        let mut inventory = ReplicaInventory::default();
        let mut peers = BTreeSet::new();

        for source in sources {
            let peer = source.peer_id();
            if !peers.insert(peer) {
                return Err(SchedulerError::DuplicateSource(peer));
            }
            for descriptor in &missing {
                if source.has_blob(manifest.world_id, descriptor) {
                    inventory.record_complete_blob(peer, descriptor.hash);
                }
            }
        }
        Ok(inventory)
    }

    pub fn plan(
        &self,
        destination: &Storage,
        manifest: &SnapshotManifestV1,
        inventory: &ReplicaInventory,
    ) -> Result<Vec<BlobAssignment>, SchedulerError> {
        validate_descriptor_consistency(manifest)?;
        self.selector.assign(&destination.missing_blobs(manifest), inventory)
    }

    pub fn reconstruct(
        &self,
        destination: &Storage,
        manifest: &SnapshotManifestV1,
        sources: Vec<Arc<dyn BlobSource>>,
        options: ReplicationOptions,
    ) -> Result<ReplicationReport, SchedulerError> {
        let options = options.validate()?;
        validate_descriptor_consistency(manifest)?;

        let missing = destination.missing_blobs(manifest);
        if missing.is_empty() {
            destination.finalize_replica(manifest)?;
            return Ok(ReplicationReport::default());
        }

        let inventory = self.inventory(destination, manifest, &sources)?;
        let assignments = self.selector.assign(&missing, &inventory)?;
        let source_map = sources
            .into_iter()
            .map(|source| (source.peer_id(), source))
            .collect::<BTreeMap<_, _>>();

        let hashes = missing.iter().map(|descriptor| descriptor.hash).collect::<Vec<_>>();
        let _lease = destination.pin_replication_hashes(manifest.world_id, &hashes)?;

        let queue = Arc::new(Mutex::new(VecDeque::from(assignments)));
        let report = Arc::new(Mutex::new(ReplicationReport { total_blobs: missing.len(), ..Default::default() }));
        let fatal = Arc::new(Mutex::new(None::<(Hash32, String)>));
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let workers = options.max_parallel_blobs.min(missing.len());

        debug!(
            world = %manifest.world_id,
            snapshot = manifest.snapshot_number,
            blobs = missing.len(),
            workers,
            "starting multi-source snapshot reconstruction"
        );

        let scope_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    let queue = Arc::clone(&queue);
                    let report = Arc::clone(&report);
                    let fatal = Arc::clone(&fatal);
                    let stop = Arc::clone(&stop);
                    let active = Arc::clone(&active);
                    let max_active = Arc::clone(&max_active);
                    let source_map = &source_map;
                    scope.spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            let Some(assignment) = queue.lock().expect("replication queue poisoned").pop_front() else {
                                break;
                            };
                            let now_active = active.fetch_add(1, Ordering::AcqRel) + 1;
                            max_active.fetch_max(now_active, Ordering::AcqRel);
                            let outcome = transfer_assignment(
                                destination,
                                manifest.world_id,
                                &assignment,
                                source_map,
                                options.chunk_size,
                                &report,
                            );
                            active.fetch_sub(1, Ordering::AcqRel);

                            if let Err((hash, message)) = outcome {
                                let mut slot = fatal.lock().expect("replication error slot poisoned");
                                if slot.is_none() {
                                    *slot = Some((hash, message));
                                }
                                stop.store(true, Ordering::Release);
                            }
                        }
                    });
                }
            });
        }));

        if scope_result.is_err() {
            return Err(SchedulerError::WorkerPanicked);
        }

        let observed = max_active.load(Ordering::Acquire);
        report.lock().expect("replication report poisoned").max_parallel_blobs_observed = observed;

        if let Some((hash, last_error)) = fatal.lock().expect("replication error slot poisoned").take() {
            return Err(SchedulerError::AllSourcesFailed { hash, last_error });
        }

        destination.finalize_replica(manifest)?;
        let report = Arc::try_unwrap(report)
            .expect("replication workers released report")
            .into_inner()
            .expect("replication report poisoned");
        debug!(
            world = %manifest.world_id,
            snapshot = manifest.snapshot_number,
            completed = report.completed_blobs,
            sources = report.sources_used.len(),
            resumed = report.resumed_blobs,
            failures = report.source_failures,
            "snapshot reconstruction complete"
        );
        Ok(report)
    }
}

fn transfer_assignment(
    destination: &Storage,
    world: WorldId,
    assignment: &BlobAssignment,
    source_map: &BTreeMap<PeerId, Arc<dyn BlobSource>>,
    chunk_size: usize,
    report: &Arc<Mutex<ReplicationReport>>,
) -> Result<(), (Hash32, String)> {
    let descriptor = &assignment.descriptor;
    let mut last_error = String::from("no candidate source remained");
    let mut resumed_recorded = false;

    for peer in &assignment.candidates {
        let Some(source) = source_map.get(peer) else {
            continue;
        };
        {
            let mut report = report.lock().expect("replication report poisoned");
            report.per_source.entry(*peer).or_default().attempted_blobs += 1;
        }

        let mut offset = match destination.partial_blob_offset(world, descriptor) {
            Ok(offset) => offset,
            Err(error) => return Err((descriptor.hash, error.to_string())),
        };
        if offset > 0 && !resumed_recorded {
            report.lock().expect("replication report poisoned").resumed_blobs += 1;
            resumed_recorded = true;
        }

        loop {
            let chunk = source.read_blob_chunk(world, descriptor, offset, chunk_size);
            let (data, finished) = match chunk {
                Ok(value) => value,
                Err(error) => {
                    record_source_failure(report, *peer, &error);
                    last_error = error.to_string();
                    warn!(source = %peer, blob = %descriptor.hash, %last_error, "replica source failed; trying fallback");
                    break;
                }
            };

            if data.is_empty() && !finished {
                last_error = "source returned an empty unfinished chunk".into();
                record_plain_source_failure(report, *peer);
                warn!(source = %peer, blob = %descriptor.hash, "replica source stalled; trying fallback");
                break;
            }

            let received = destination.receive_blob_chunk(world, descriptor, offset, &data, finished);
            let next_offset = match received {
                Ok(next_offset) => next_offset,
                Err(error) => {
                    record_source_failure(report, *peer, &error);
                    last_error = error.to_string();
                    warn!(source = %peer, blob = %descriptor.hash, %last_error, "replica data rejected; trying fallback");
                    break;
                }
            };

            {
                let mut report = report.lock().expect("replication report poisoned");
                report.bytes_received = report.bytes_received.saturating_add(data.len() as u64);
                report.sources_used.insert(*peer);
                let stats = report.per_source.entry(*peer).or_default();
                stats.bytes_received = stats.bytes_received.saturating_add(data.len() as u64);
            }

            offset = next_offset;
            if finished {
                let mut report = report.lock().expect("replication report poisoned");
                report.completed_blobs += 1;
                report.sources_used.insert(*peer);
                report.per_source.entry(*peer).or_default().completed_blobs += 1;
                return Ok(());
            }
        }
    }

    Err((descriptor.hash, last_error))
}

fn record_source_failure(report: &Arc<Mutex<ReplicationReport>>, peer: PeerId, error: &ReplicationError) {
    let corrupt = matches!(error, ReplicationError::Storage(StorageError::BlobCorrupt(_)));
    let mut report = report.lock().expect("replication report poisoned");
    report.source_failures += 1;
    let stats = report.per_source.entry(peer).or_default();
    stats.failures += 1;
    if corrupt {
        report.corrupt_rejections += 1;
        stats.corrupt_rejections += 1;
    }
}

fn record_plain_source_failure(report: &Arc<Mutex<ReplicationReport>>, peer: PeerId) {
    let mut report = report.lock().expect("replication report poisoned");
    report.source_failures += 1;
    report.per_source.entry(peer).or_default().failures += 1;
}

fn validate_descriptor_consistency(manifest: &SnapshotManifestV1) -> Result<(), SchedulerError> {
    let mut descriptors: BTreeMap<Hash32, &BlobDescriptor> = BTreeMap::new();
    for entry in &manifest.entries {
        if let Some(existing) = descriptors.insert(entry.blob.hash, &entry.blob) {
            if existing != &entry.blob {
                return Err(SchedulerError::ConflictingDescriptor(entry.blob.hash));
            }
        }
    }
    Ok(())
}
