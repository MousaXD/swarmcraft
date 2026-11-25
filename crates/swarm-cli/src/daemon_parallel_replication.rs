//! Receiver-driven ordinary replication from authenticated current members.
//! Historical replay remains on its independently fenced protocol.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use swarm_network::{ReplicaPullTargetV1, REPLICA_PULL_CAPABILITY};

const MAX_PULLS: usize = 4;
const MAX_WORLDS: usize = 4;
const MAX_BLOBS: usize = swarm_network::MAX_MISSING_BLOBS;
const MAX_FAILED_SOURCES: usize = 8;
const SOURCE_RETRY: Duration = Duration::from_secs(3);
const BAD_DATA_RETRY: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(45);
const LEGACY_FALLBACK: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub(super) struct PullRequest {
    pub target: ReplicaPullTargetV1,
    pub transport: TransportPeerId,
    peer: PeerId,
    hash: Hash32,
    offset: u64,
    max_bytes: usize,
    sent_at: Instant,
    paced: bool,
}

#[derive(Debug)]
struct BlobWork {
    descriptor: BlobDescriptor,
    offset: u64,
    preferred: Option<PeerId>,
    attempt_start: u64,
    clean_retry: bool,
    counted_resume: bool,
    failed: BTreeMap<PeerId, Instant>,
}

impl BlobWork {
    fn fail(&mut self, peer: PeerId, until: Instant) {
        self.defer(peer, until);
        self.preferred = None;
        self.clean_retry = false;
    }

    fn defer(&mut self, peer: PeerId, until: Instant) {
        if self.failed.len() >= MAX_FAILED_SOURCES && !self.failed.contains_key(&peer) {
            if let Some(oldest) = self.failed.iter().min_by_key(|(_, until)| *until).map(|(peer, _)| *peer) {
                self.failed.remove(&oldest);
            }
        }
        self.failed.insert(peer, until);
    }

    fn start(&mut self, peer: PeerId) {
        if self.preferred != Some(peer) {
            self.preferred = Some(peer);
            self.attempt_start = self.offset;
            self.clean_retry = false;
        }
    }

    fn available(&self, peer: PeerId, now: Instant) -> bool {
        self.failed.get(&peer).is_none_or(|until| *until <= now)
    }
}

#[derive(Debug)]
struct Download {
    target: ReplicaPullTargetV1,
    authority: PeerId,
    initiator: PeerId,
    blobs: BTreeMap<Hash32, BlobWork>,
    last_progress: Instant,
    last_scheduled: Option<Instant>,
    pause_until: Option<Instant>,
    bytes: u64,
    resumed: usize,
    failures: usize,
    sources: BTreeSet<PeerId>,
    max_parallel: usize,
}

#[derive(Debug, Default)]
pub(super) struct PullRuntime {
    downloads: HashMap<WorldId, Download>,
    fallback_until: HashMap<WorldId, Instant>,
    pub(super) pacer: ReplicationPacer,
}

impl PullRuntime {
    fn mark_fallback(&mut self, world: WorldId, now: Instant) {
        if self.fallback_until.len() >= 64 && !self.fallback_until.contains_key(&world) {
            if let Some(oldest) = self.fallback_until.iter().min_by_key(|(_, until)| *until).map(|(world, _)| *world) {
                self.fallback_until.remove(&oldest);
            }
        }
        self.fallback_until.insert(world, now + LEGACY_FALLBACK);
    }

    pub(super) fn active(&self, world: WorldId) -> bool {
        self.downloads.contains_key(&world)
    }

    pub(super) fn has_work(&self) -> bool {
        !self.downloads.is_empty()
    }

    pub(super) fn cancel(&mut self, outbound: &mut HashMap<String, OutboundContext>, world: WorldId) {
        self.downloads.remove(&world);
        outbound.retain(
            |_, context| !matches!(context, OutboundContext::PullChunk(request) if request.target.world_id == world),
        );
    }

    pub(super) fn disconnect(
        &mut self,
        outbound: &mut HashMap<String, OutboundContext>,
        transport: TransportPeerId,
        now: Instant,
    ) {
        outbound.retain(|_, context| {
            if let OutboundContext::PullChunk(request) = context {
                if request.transport == transport {
                    self.failure(request, now, SOURCE_RETRY);
                    return false;
                }
            }
            true
        });
    }

    pub(super) fn failure(&mut self, request: &PullRequest, now: Instant, backoff: Duration) {
        if let Some(download) = self.downloads.get_mut(&request.target.world_id) {
            if download.target == request.target {
                if let Some(blob) = download.blobs.get_mut(&request.hash) {
                    if blob.offset == request.offset {
                        blob.fail(request.peer, now + backoff);
                        download.failures += 1;
                    }
                }
            }
        }
    }
}

/// Only already accepted, ordinary quorum/recovery worlds. A pull capability
/// is a transport hint, never authorization to discover or advance a world.
fn scope(storage: &Storage, world: WorldId, local: PeerId) -> Result<(MembershipRecordV1, EpochRecordV1)> {
    if storage.stale_publication_pending(world)?
        || storage.crash_retirement_pending(world)?
        || storage.first_contact_replay_pinned(world)?
        || awaiting_committed_join(storage, world)?
        || requires_historical_replay(storage, world)?
        || !storage.list_solo_conflicts(world)?.is_empty()
    {
        bail!("ordinary replica pull is fenced");
    }
    authorize_committed_member(storage, world, local)?;
    let membership = storage.load_membership_record(world)?;
    verify_membership_signature(&membership)?;
    let descriptor = storage.load_world_descriptor(world)?;
    if descriptor.members != membership.members
        || !membership.members.iter().any(|member| !member.banned && member.peer_id == local)
    {
        bail!("replica pull local membership changed");
    }
    let epoch = storage.load_epoch_record(world)?;
    if membership.epoch != epoch.epoch_number
        || membership.authority_peer_id != epoch.authority_peer_id
        || membership.authority_public_key != epoch.authority_public_key
        || !membership.members.iter().any(|member| {
            !member.banned
                && member.authority_eligible
                && member.peer_id == epoch.authority_peer_id
                && member.public_key == epoch.authority_public_key
        })
    {
        bail!("ordinary replica pull membership/epoch is not synchronized");
    }
    swarm_cli::live_authority::verify_playable_epoch(storage, &epoch)?;
    Ok((membership, epoch))
}

fn matches_epoch(manifest: &SnapshotManifestV1, epoch: &EpochRecordV1) -> bool {
    manifest.world_id == epoch.world_id
        && manifest.epoch == epoch.epoch_number
        && manifest.authority_peer_id == epoch.authority_peer_id
        && manifest.authority_public_key == epoch.authority_public_key
}

fn exact_target(
    target: ReplicaPullTargetV1,
    manifest: &SnapshotManifestV1,
    membership: &MembershipRecordV1,
    epoch: &EpochRecordV1,
) -> Result<()> {
    if target.world_id != manifest.world_id
        || target.snapshot_number != manifest.snapshot_number
        || target.manifest_hash != manifest.manifest_hash()?
        || target.membership_hash != membership.record_hash()?
        || target.epoch_hash != epoch.record_hash()?
    {
        bail!("replica pull target changed");
    }
    Ok(())
}

fn current_candidates(
    node: &SwarmNode,
    runtime: &LeaseRuntime,
    membership: &MembershipRecordV1,
) -> Vec<(PeerId, TransportPeerId)> {
    let mut candidates = runtime
        .authenticated_peers
        .iter()
        .filter_map(|(transport, peer)| {
            (node.application_peer(transport) == Some(*peer)
                && node.application_supports(transport, REPLICA_PULL_CAPABILITY)
                && membership.members.iter().any(|member| !member.banned && member.peer_id == *peer))
            .then_some((*peer, *transport))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup_by_key(|(peer, _)| *peer);
    candidates
}

fn descriptors_consistent(manifest: &SnapshotManifestV1) -> Result<()> {
    swarm_storage::streaming::validate_manifest_shape(manifest)?;
    let mut descriptors = BTreeMap::new();
    for entry in &manifest.entries {
        if let Some(previous) = descriptors.insert(entry.blob.hash, &entry.blob) {
            if previous != &entry.blob {
                bail!("conflicting descriptors in replica pull manifest");
            }
        }
    }
    Ok(())
}

/// Admission never modifies canonical metadata. Negotiate only when the
/// manifest sender advertised support and two distinct signed members are
/// connected. One remaining source is enough to finish an existing pull.
pub(super) fn negotiate(
    context: &HandlerContext<'_>,
    node: &SwarmNode,
    source: (TransportPeerId, PeerId),
    manifest: &SnapshotManifestV1,
    missing: &[BlobResumeV1],
    state: &mut RequestState<'_>,
) -> Result<Option<ReplicaPullTargetV1>> {
    let (transport, sender) = source;
    let (storage, identity) = (context.storage, context.identity);
    let world = manifest.world_id;
    let now = state.now;
    state.leases.pulls.fallback_until.retain(|_, until| *until > now);
    if let Some(download) = state.leases.pulls.downloads.get(&world) {
        let valid = scope(storage, world, identity.peer_id()).is_ok_and(|(membership, epoch)| {
            matches_epoch(manifest, &epoch)
                && exact_target(download.target, manifest, &membership, &epoch).is_ok()
                && state.pending_manifests.get(&world) == Some(manifest)
        });
        if valid {
            if !node.application_supports(&transport, REPLICA_PULL_CAPABILITY) {
                bail!("ordinary push cannot overlap active receiver-driven replication");
            }
            return Ok(Some(download.target));
        }
        // A new ordinary manifest cannot replace an unfinished valid cursor.
        // A changed scope/epoch invalidates the old cursor on the drive tick.
        bail!("ordinary manifest conflicts with active receiver-driven replication");
    }
    if missing.is_empty()
        || missing.len() > MAX_BLOBS
        || state.leases.pulls.downloads.len() >= MAX_WORLDS
        || state.leases.pulls.fallback_until.contains_key(&world)
        || !node.application_supports(&transport, REPLICA_PULL_CAPABILITY)
    {
        return Ok(None);
    }
    let Ok((membership, epoch)) = scope(storage, world, identity.peer_id()) else { return Ok(None) };
    if !matches_epoch(manifest, &epoch)
        || epoch.authority_peer_id == identity.peer_id()
        || current_candidates(node, state.leases, &membership).len() < 2
    {
        return Ok(None);
    }
    descriptors_consistent(manifest)?;
    let target = ReplicaPullTargetV1 {
        world_id: world,
        membership_hash: membership.record_hash()?,
        epoch_hash: epoch.record_hash()?,
        snapshot_number: manifest.snapshot_number,
        manifest_hash: manifest.manifest_hash()?,
    };
    let mut blobs = BTreeMap::new();
    let mut resumed = 0;
    for resume in missing {
        let descriptor = find_descriptor(manifest, resume.hash).context("missing pull descriptor")?.clone();
        if resume.offset >= descriptor.encoded_size {
            bail!("unfinishable replica pull offset");
        }
        resumed += usize::from(resume.offset > 0);
        blobs.insert(
            resume.hash,
            BlobWork {
                descriptor,
                offset: resume.offset,
                preferred: None,
                attempt_start: resume.offset,
                clean_retry: false,
                counted_resume: resume.offset > 0,
                failed: BTreeMap::new(),
            },
        );
    }
    state.leases.pulls.downloads.insert(
        world,
        Download {
            target,
            authority: epoch.authority_peer_id,
            initiator: sender,
            blobs,
            last_progress: now,
            last_scheduled: None,
            pause_until: None,
            bytes: 0,
            resumed,
            failures: 0,
            sources: BTreeSet::new(),
            max_parallel: 0,
        },
    );
    info!(%world, snapshot = target.snapshot_number, missing = missing.len(), resumed, "receiver-driven replica pull negotiated");
    Ok(Some(target))
}

fn unavailable() -> WireResponse {
    WireResponse::Error { code: "REPLICA_BLOB_UNAVAILABLE".into(), message: "Replica payload unavailable".into() }
}

pub(super) fn serve(
    storage: &Storage,
    identity: &PeerIdentity,
    node: &mut SwarmNode,
    sender: PeerId,
    request: WireRequest,
    channel: ResponseChannel<WireResponse>,
    state: &mut RequestState<'_>,
) -> Result<()> {
    let WireRequest::ReplicaBlobRead { target, hash, offset, max_bytes } = request else { unreachable!() };
    let response = (|| -> Result<WireResponse> {
        let (membership, epoch) = scope(storage, target.world_id, identity.peer_id())?;
        authorize_committed_member(storage, target.world_id, sender)?;
        if epoch.authority_peer_id != identity.peer_id() && !storage.background_seeding_enabled(target.world_id)? {
            bail!("background seeding is not enabled");
        }
        let manifest = storage.load_snapshot(target.world_id, target.snapshot_number)?;
        exact_target(target, &manifest, &membership, &epoch)?;
        if !matches_epoch(&manifest, &epoch) {
            bail!("requested replica payload is not in accepted generation");
        }
        verify_snapshot_signature(&manifest)?;
        let descriptor = find_descriptor(&manifest, hash).context("blob not in exact manifest")?;
        if offset >= descriptor.encoded_size || max_bytes == 0 || max_bytes as usize > MAX_BLOB_CHUNK {
            bail!("invalid replica pull chunk");
        }
        let (limit, paced) =
            match state.leases.replication_pacer.plan(state.now, node.game_traffic_active(), max_bytes as usize) {
                ChunkPlan::Ready { max_bytes, paced } => (max_bytes, paced),
                ChunkPlan::Deferred(_) => {
                    return Ok(WireResponse::Error {
                        code: "REPLICA_PACED".into(),
                        message: "Replica is pacing background payloads".into(),
                    })
                }
            };
        let (data, finished) = storage.read_encoded_blob_chunk(target.world_id, descriptor, offset, limit)?;
        if data.is_empty() || data.len() > limit {
            bail!("replica returned an invalid local chunk");
        }
        // Recheck scope after disk access and before disclosing bytes.
        let (current, current_epoch) = scope(storage, target.world_id, identity.peer_id())?;
        exact_target(target, &manifest, &current, &current_epoch)?;
        if current_epoch != epoch
            || !current.members.iter().any(|member| !member.banned && member.peer_id == sender)
            || (current_epoch.authority_peer_id != identity.peer_id()
                && !storage.background_seeding_enabled(target.world_id)?)
        {
            bail!("replica source eligibility changed");
        }
        state.leases.replication_pacer.sent(state.now, data.len(), paced);
        Ok(WireResponse::ReplicaBlobData { target, hash, offset, data, finished })
    })()
    .unwrap_or_else(|error| {
        tracing::debug!(%sender, %error, "replica pull read refused");
        unavailable()
    });
    node.respond(channel, response)
}

fn eligible_current(
    storage: &Storage,
    identity: &PeerIdentity,
    manifest: &SnapshotManifestV1,
    download: &Download,
) -> Result<MembershipRecordV1> {
    let (membership, epoch) = scope(storage, manifest.world_id, identity.peer_id())?;
    exact_target(download.target, manifest, &membership, &epoch)?;
    if !matches_epoch(manifest, &epoch)
        || epoch.authority_peer_id != download.authority
        || epoch.authority_peer_id == identity.peer_id()
    {
        bail!("replica pull scope advanced");
    }
    storage.validate_replica_history(manifest)?;
    Ok(membership)
}

fn choose_blob(download: &Download, peer: PeerId, busy: &HashSet<(WorldId, Hash32)>, now: Instant) -> Option<Hash32> {
    let world = download.target.world_id;
    // Finish an existing partial first; do not queue the same blob twice.
    download
        .blobs
        .iter()
        .filter(|(hash, blob)| !busy.contains(&(world, **hash)) && blob.available(peer, now))
        .min_by_key(|(hash, blob)| (blob.preferred != Some(peer), blob.preferred.is_some(), blob.offset == 0, **hash))
        .map(|(hash, _)| *hash)
}

pub(super) fn drive(
    context: &HandlerContext<'_>,
    node: &mut SwarmNode,
    pending: &mut HashMap<WorldId, SnapshotManifestV1>,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
    now: Instant,
) -> Result<()> {
    let mut worlds =
        runtime.pulls.downloads.iter().map(|(world, download)| (download.last_scheduled, *world)).collect::<Vec<_>>();
    worlds.sort_unstable();
    runtime.pulls.fallback_until.retain(|_, until| *until > now);
    let active_game = node.game_traffic_active();
    let capacity = if active_game { 1 } else { MAX_PULLS };
    let mut busy = HashSet::new();
    let mut peers = HashSet::new();
    let mut active = 0;
    for context in outbound.values() {
        if let OutboundContext::PullChunk(request) = context {
            busy.insert((request.target.world_id, request.hash));
            peers.insert(request.peer);
            active += 1;
        }
    }
    for (_, world) in worlds {
        let Some(manifest) = pending.get(&world) else {
            runtime.pulls.cancel(outbound, world);
            continue;
        };
        let download = &runtime.pulls.downloads[&world];
        let Ok(membership) = eligible_current(context.storage, context.identity, manifest, download) else {
            runtime.pulls.cancel(outbound, world);
            pending.remove(&world);
            continue;
        };
        // A stalled pull never permanently disables ordinary push. Keep all
        // complete blobs and partial offsets; a later manifest renegotiates.
        if now.saturating_duration_since(download.last_progress) >= STALL_TIMEOUT && !download.blobs.is_empty() {
            warn!(%world, "replica pull stalled; preserving payloads and allowing legacy fallback");
            runtime.pulls.cancel(outbound, world);
            runtime.pulls.mark_fallback(world, now);
            continue;
        }
        if download.blobs.is_empty() {
            if download.pause_until.is_some_and(|until| until > now) {
                continue;
            }
            let result = (|| -> Result<()> {
                verify_snapshot_signature(manifest)?;
                let head = context.storage.canonical_snapshot_head(world)?.head;
                let (membership, epoch) = scope(context.storage, world, context.identity.peer_id())?;
                exact_target(download.target, manifest, &membership, &epoch)?;
                if !matches_epoch(manifest, &epoch) {
                    bail!("replica pull generation changed before publication");
                }
                context.storage.commit_replica_snapshot_fenced(
                    manifest,
                    swarm_storage::SnapshotCommitFence {
                        expected_epoch: epoch.epoch_number,
                        expected_fencing_token: epoch.fencing_token,
                        expected_head: head,
                    },
                    download.target.epoch_hash,
                    download.target.membership_hash,
                    context.identity.peer_id(),
                )?;
                Ok(())
            })();
            if let Err(error) = result {
                if error.downcast_ref::<StorageError>().is_some_and(StorageError::is_storage_pressure) {
                    runtime.pulls.downloads.get_mut(&world).expect("completed pull exists").pause_until =
                        Some(now + REPLICATION_RATE_LIMIT_BACKOFF);
                } else {
                    warn!(%world, %error, "replica pull publication deferred to ordinary negotiation");
                    runtime.pulls.cancel(outbound, world);
                    runtime.pulls.mark_fallback(world, now);
                }
                continue;
            }
            let download = runtime.pulls.downloads.remove(&world).expect("completed pull exists");
            let ack = ReplicaAckV1 {
                world_id: world,
                snapshot_number: manifest.snapshot_number,
                manifest_hash: manifest.manifest_hash()?,
                state_root: manifest.state_root,
                complete: true,
            };
            for (transport, peer) in &runtime.authenticated_peers {
                if (*peer == download.authority || *peer == download.initiator)
                    && node.application_peer(transport) == Some(*peer)
                    && scope(context.storage, world, context.identity.peer_id()).is_ok_and(|(membership, epoch)| {
                        exact_target(download.target, manifest, &membership, &epoch).is_ok()
                            && membership.members.iter().any(|member| !member.banned && member.peer_id == *peer)
                    })
                {
                    let _ = node.send_request(transport, WireRequest::ReplicaAck(ack.clone()));
                }
            }
            info!(%world, snapshot = download.target.snapshot_number, sources = download.sources.len(),
                bytes = download.bytes, resumed = download.resumed, failures = download.failures,
                max_parallel = download.max_parallel, "receiver-driven replica pull completed");
            pending.remove(&world);
            continue;
        }
        if active >= capacity || download.pause_until.is_some_and(|until| until > now) {
            continue;
        }
        // Ambiguous local I/O can leave a full partial after the last write.
        // Renegotiate through the durable resume/promotion path instead of
        // requesting zero bytes or subtracting an invalid offset.
        if download.blobs.values().any(|blob| blob.offset >= blob.descriptor.encoded_size) {
            runtime.pulls.cancel(outbound, world);
            runtime.pulls.mark_fallback(world, now);
            continue;
        }
        let mut candidates = current_candidates(node, runtime, &membership);
        candidates.sort_by_key(|(peer, _)| {
            // Status is a hint only. Exact source admission is repeated by
            // the serving peer and every received byte is verified locally.
            let exact = runtime.peer_status.get(&(world, *peer)).is_some_and(|observed| {
                now.saturating_duration_since(observed.observed_at) < STATUS_FRESHNESS
                    && observed.status.latest_snapshot == Some(download.target.manifest_hash)
            });
            (!exact, *peer != download.initiator, *peer)
        });
        for (peer, transport) in candidates {
            if active >= capacity {
                break;
            }
            if peers.contains(&peer) {
                continue;
            }
            let Some(hash) = choose_blob(&runtime.pulls.downloads[&world], peer, &busy, now) else { continue };
            let (max_bytes, paced) = match runtime.pulls.pacer.plan(now, active_game, MAX_BLOB_CHUNK) {
                ChunkPlan::Ready { max_bytes, paced } => (max_bytes, paced),
                ChunkPlan::Deferred(_) => break,
            };
            let download = runtime.pulls.downloads.get_mut(&world).expect("active pull exists");
            let blob = download.blobs.get_mut(&hash).expect("selected pull blob exists");
            if blob.offset > 0 && blob.preferred != Some(peer) && !blob.counted_resume {
                download.resumed += 1;
                blob.counted_resume = true;
            }
            blob.start(peer);
            let limit = max_bytes.min((blob.descriptor.encoded_size - blob.offset).min(usize::MAX as u64) as usize);
            let request = PullRequest {
                target: download.target,
                transport,
                peer,
                hash,
                offset: blob.offset,
                max_bytes: limit,
                sent_at: now,
                paced,
            };
            let id = match node.send_request(
                &transport,
                WireRequest::ReplicaBlobRead {
                    target: request.target,
                    hash,
                    offset: request.offset,
                    max_bytes: limit as u32,
                },
            ) {
                Ok(id) => id,
                Err(error) => {
                    tracing::debug!(%peer, %error, "replica pull request could not be sent");
                    runtime.pulls.failure(&request, now, SOURCE_RETRY);
                    continue;
                }
            };
            outbound.insert(request_key(&id), OutboundContext::PullChunk(request));
            runtime.pulls.pacer.sent(now, limit, paced);
            download.last_scheduled = Some(now);
            busy.insert((world, hash));
            peers.insert(peer);
            active += 1;
            download.max_parallel = download.max_parallel.max(busy.iter().filter(|(w, _)| *w == world).count());
        }
    }
    Ok(())
}

fn exact_chunk(request: &PullRequest, response: WireResponse, descriptor: &BlobDescriptor) -> Result<(Vec<u8>, bool)> {
    response.validate_limits()?;
    let WireResponse::ReplicaBlobData { target, hash, offset, data, finished } = response else {
        bail!("unexpected replica pull response");
    };
    let end = offset.checked_add(data.len() as u64).context("pull offset overflow")?;
    if target != request.target
        || hash != request.hash
        || offset != request.offset
        || data.is_empty()
        || data.len() > request.max_bytes
        || end > descriptor.encoded_size
        || finished != (end == descriptor.encoded_size)
    {
        bail!("replica pull response does not match the exact requested range");
    }
    Ok((data, finished))
}

pub(super) fn response(
    context: &HandlerContext<'_>,
    node: &SwarmNode,
    reply: (TransportPeerId, PullRequest, WireResponse),
    pending: &HashMap<WorldId, SnapshotManifestV1>,
    runtime: &mut LeaseRuntime,
    now: Instant,
) {
    let (transport, request, response) = reply;
    let world = request.target.world_id;
    let Some(download) = runtime.pulls.downloads.get_mut(&world) else { return };
    let Some(manifest) = pending.get(&world) else { return };
    if download.target != request.target {
        return;
    }
    let valid_source = transport == request.transport
        && node.application_peer(&transport) == Some(request.peer)
        && node.application_supports(&transport, REPLICA_PULL_CAPABILITY)
        && eligible_current(context.storage, context.identity, manifest, download).is_ok_and(|membership| {
            membership.members.iter().any(|member| !member.banned && member.peer_id == request.peer)
        });
    if !valid_source {
        runtime.pulls.failure(&request, now, SOURCE_RETRY);
        return;
    }
    let Some(blob) = download.blobs.get_mut(&request.hash) else { return };
    if blob.offset != request.offset || blob.preferred != Some(request.peer) {
        return;
    }
    if let WireResponse::Error { ref code, .. } = response {
        let backoff = match code.as_str() {
            "RATE_LIMITED" | "STORAGE_PAUSED" => REPLICATION_RATE_LIMIT_BACKOFF,
            "REPLICA_PACED" => Duration::from_millis(250),
            _ => SOURCE_RETRY,
        };
        if code == "REPLICA_PACED" {
            // Pacing is normal backpressure, not corruption.
            blob.defer(request.peer, now + backoff);
        } else {
            runtime.pulls.failure(&request, now, backoff);
        }
        return;
    }
    let (data, finished) = match exact_chunk(&request, response, &blob.descriptor) {
        Ok(chunk) => chunk,
        Err(error) => {
            warn!(%world, peer = %request.peer, %error, "replica pull response rejected");
            runtime.pulls.failure(&request, now, BAD_DATA_RETRY);
            return;
        }
    };
    let accepted = context.storage.receive_blob_chunk(world, &blob.descriptor, request.offset, &data, finished);
    match accepted {
        Ok(offset) => {
            blob.offset = offset;
            download.bytes = download.bytes.saturating_add(data.len() as u64);
            if download.sources.insert(request.peer) {
                info!(%world, peer = %request.peer, snapshot = download.target.snapshot_number, "receiver-driven replica source contributing");
            }
            download.last_progress = now;
            if finished {
                download.blobs.remove(&request.hash);
            }
            runtime.pulls.pacer.acknowledge(now.saturating_duration_since(request.sent_at), request.paced);
        }
        Err(ReplicationError::Storage(ref error)) if error.is_storage_pressure() => {
            download.pause_until = Some(now + REPLICATION_RATE_LIMIT_BACKOFF);
            download.last_progress = now; // local storage pause is not a failed source
        }
        Err(error) => {
            // A concurrent writer or a failure after durable promotion may
            // have already completed this exact content. Keep the verified
            // blob rather than recreating its partial file.
            if context.storage.has_complete_blob(world, &blob.descriptor) {
                download.blobs.remove(&request.hash);
                download.last_progress = now;
                return;
            }
            let actual = context.storage.partial_blob_offset(world, &blob.descriptor);
            if let Ok(offset) = actual {
                blob.offset = offset;
                // A bad resumed prefix is not evidence that the final
                // honest source was bad. Retry it once from clean offset.
                if finished && offset == 0 && blob.attempt_start > 0 && !blob.clean_retry {
                    blob.clean_retry = true;
                    blob.attempt_start = 0;
                    download.failures += 1;
                    warn!(%world, peer = %request.peer, blob = %request.hash, "resumed replica pull failed verification; clean retry");
                    return;
                }
            }
            blob.fail(request.peer, now + BAD_DATA_RETRY);
            download.failures += 1;
            warn!(%world, peer = %request.peer, blob = %request.hash, %error, "replica pull blob rejected; retaining unrelated progress");
        }
    }
}
