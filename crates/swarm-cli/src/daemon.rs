use anyhow::{anyhow, Context, Result};
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use swarm_cli::{
    authority_permit::{clear_permit, refresh_permit, PERMIT_HEARTBEAT_INTERVAL},
    host_readiness::{self, PeerReadinessObservation},
};
use swarm_consensus::{
    elect_authority, has_quorum, membership_vote_for, reconcile_solo_history, validate_membership_certificate_shape,
    validate_membership_proposal_shape, validate_recovery_certificate_shape, AuthorityCandidate, AuthorityGeneration,
    MembershipConsensusError, SoloReconciliation,
};
use swarm_core::{
    lifecycle::{verify_join_request_signature, verify_leave_request_signature, verify_sleep_record_signature},
    protocol_v2::{
        sign_recovery_ballot, sign_recovery_vote, sign_solo_branch, verify_recovery_ballot_signature,
        verify_recovery_vote_signature, verify_solo_branch_signature, verify_world_config_signature,
    },
    random_nonce, verify_invite_signature, verify_lease_signature, verify_membership_signature, verify_signature,
    verify_snapshot_signature, verify_transfer_signature, DataPaths, PeerIdentity,
};
use swarm_network::{
    load_or_create_transport_key, validate_invite_dial_address, BlobResumeV1, HostCapabilityV1, NetworkEvent,
    ReplicaAckV1, ResponseChannel, SwarmNode, TransportPeerId, WireRequest, WireResponse, MAX_BLOB_CHUNK,
};
use swarm_protocol::{
    peer_id_from_public_key, AuthorityLeaseGrantV1, BlobDescriptor, EpochMode, EpochRecordV1, Hash32,
    MembershipCertificateV1, MembershipProposalV1, MembershipRecordV1, MembershipVoteV1, PeerId, RecoveryBallotV1,
    RecoveryCertificateV1, RecoveryVoteV1, SnapshotManifestV1, SoloBranchV1, TransferPhase, WorldDescriptorV1, WorldId,
    WorldStatusV1, PROTOCOL_VERSION,
};
use swarm_storage::{DurableMembershipPromiseV1, MembershipPromiseResult, RecoveryPromiseResult, Storage};
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const AUTHORITY_LEASE_DURATION_MS: u64 = 5_000;
const RECOVERY_SETTLE_DELAY: Duration = Duration::from_secs(2);
const STATUS_FRESHNESS: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
enum OutboundContext {
    Manifest { world: WorldId, snapshot_number: u64 },
    Lease { world: WorldId, peer: PeerId, generation: AuthorityGeneration },
    RecoveryBallot { world: WorldId, peer: PeerId, ballot_hash: Hash32 },
    MembershipProposal { world: WorldId, peer: PeerId, proposal_hash: Hash32 },
    MembershipCommit { world: WorldId, peer: PeerId, sequence: u64 },
    Epoch { world: WorldId, peer: PeerId, generation: AuthorityGeneration },
    Status { world: WorldId, peer: PeerId },
    HostCapability { world: WorldId, peer: PeerId },
}

#[derive(Debug, Clone, Copy)]
struct LeaseAck {
    generation: AuthorityGeneration,
    observed_at: Instant,
}

#[derive(Debug, Clone, Copy)]
struct InboundLease {
    generation: AuthorityGeneration,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct ObservedStatus {
    status: WorldStatusV1,
    observed_at: Instant,
}

#[derive(Debug, Clone)]
struct ObservedCapability {
    capability: HostCapabilityV1,
    observed_at: Instant,
}

#[derive(Debug, Default)]
struct LeaseRuntime {
    authenticated_peers: HashMap<TransportPeerId, PeerId>,
    lease_acks: HashMap<(WorldId, PeerId), LeaseAck>,
    recovery_ballots: HashMap<WorldId, RecoveryBallotV1>,
    recovery_votes: HashMap<(WorldId, PeerId), RecoveryVoteV1>,
    membership_votes: HashMap<(WorldId, PeerId), MembershipVoteV1>,
    recovery_round_floor: HashMap<WorldId, u64>,
    epoch_acks: HashMap<(WorldId, PeerId), AuthorityGeneration>,
    permit_heartbeats: HashMap<WorldId, u64>,
    inbound_leases: HashMap<WorldId, InboundLease>,
    peer_status: HashMap<(WorldId, PeerId), ObservedStatus>,
    peer_capability: HashMap<(WorldId, PeerId), ObservedCapability>,
    recovery_not_before: HashMap<WorldId, Instant>,
    recovery_replication_sent: HashSet<(WorldId, PeerId)>,
}

struct HandlerContext<'a> {
    paths: &'a DataPaths,
    identity: &'a PeerIdentity,
    storage: &'a Storage,
}

struct RequestState<'a> {
    pending_manifests: &'a mut HashMap<WorldId, SnapshotManifestV1>,
    outbound: &'a mut HashMap<String, OutboundContext>,
    leases: &'a mut LeaseRuntime,
    now: Instant,
}

struct LocalAuthorityContext<'a> {
    paths: &'a DataPaths,
    storage: &'a Storage,
    identity: &'a PeerIdentity,
    descriptor: &'a WorldDescriptorV1,
    epoch: &'a EpochRecordV1,
    generation: AuthorityGeneration,
    now: Instant,
}

pub async fn run(paths: &DataPaths, storage: &Storage, listen: &str) -> Result<()> {
    let identity = PeerIdentity::load_or_create(paths)?;
    let transport_key = load_or_create_transport_key(&paths.transport_key())?;
    let hello = identity.signed_peer_hello(vec![
        "snapshot-replication-v1".into(),
        "membership-v1".into(),
        "membership-leave-v1".into(),
        "membership-joint-consensus-v1".into(),
        "authority-transfer-v1".into(),
        "authority-lease-v1".into(),
        "epoch-v1".into(),
        "sleep-wake-v1".into(),
        "relay-dcutr-v1".into(),
        "recovery-ballot-v1".into(),
        "world-config-v1".into(),
        "solo-history-v1".into(),
        "background-replica-v1".into(),
        "host-readiness-v1".into(),
    ])?;
    let mut node = SwarmNode::new(transport_key, hello, identity.network_signing_key())?;
    node.listen(listen.parse().context("invalid listen multiaddress")?)?;
    dial_pending_invite_bootstraps(storage, &mut node)?;
    info!(peer = %identity.peer_id(), %listen, "SwarmCraft daemon starting");

    let mut pending_manifests: HashMap<WorldId, SnapshotManifestV1> = HashMap::new();
    let mut outbound: HashMap<String, OutboundContext> = HashMap::new();
    let mut leases = LeaseRuntime::default();
    let mut lease_tick = tokio::time::interval(PERMIT_HEARTBEAT_INTERVAL);
    lease_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = lease_tick.tick() => {
                maintain_authority_leases(
                    paths,
                    storage,
                    &identity,
                    &mut node,
                    &mut outbound,
                    &mut leases,
                    Instant::now(),
                )?;
            }
            event = node.next_event() => {
                match event? {
                    NetworkEvent::Listening { address } => info!(%address, "daemon listening"),
                    NetworkEvent::Authenticated { transport_peer, application_peer } => {
                        leases.authenticated_peers.insert(transport_peer, application_peer);
                        info!(transport = %transport_peer, peer = %application_peer, "peer authenticated");
                        push_pending_membership_requests(storage, &mut node, &transport_peer, application_peer)?;
                        push_known_worlds(
                            storage,
                            &mut node,
                            &transport_peer,
                            application_peer,
                            identity.peer_id(),
                            &mut outbound,
                        )?;
                    }
                    NetworkEvent::InboundRequest { transport_peer, request, channel } => {
                        let application_peer = node
                            .application_peer(&transport_peer)
                            .context("authenticated request lost application peer mapping")?;
                        let context = HandlerContext { paths, identity: &identity, storage };
                        let mut state = RequestState {
                            pending_manifests: &mut pending_manifests,
                        outbound: &mut outbound,
                            leases: &mut leases,
                            now: Instant::now(),
                        };
            if let Err(error) = handle_request(
                &context,
                &mut node,
                transport_peer,
                application_peer,
                request,
                channel,
                &mut state,
            ) {
                warn!(peer = %application_peer, %error, "inbound authenticated request rejected");
            }
                    }
                    NetworkEvent::Response { transport_peer, request_id, response } => {
                        // Receiving HelloAccepted means the remote has authenticated our
                        // signed PeerHello. If we have also authenticated theirs, both sides
                        // can now safely accept canonical world synchronization requests.
                        // Re-push here because the earlier Authenticated event can race the
                        // remote handshake and be rejected with HANDSHAKE_REQUIRED.
                        if matches!(response, WireResponse::HelloAccepted { .. }) {
                            if let Some(application_peer) = node.application_peer(&transport_peer) {
                                leases.authenticated_peers.insert(transport_peer, application_peer);
                                push_pending_membership_requests(
                                    storage,
                                    &mut node,
                                    &transport_peer,
                                    application_peer,
                                )?;
                                push_known_worlds(
                                    storage,
                                    &mut node,
                                    &transport_peer,
                                    application_peer,
                                    identity.peer_id(),
                                    &mut outbound,
                                )?;
                            }
                        }
                        let context = outbound.remove(&request_key(&request_id));
                        handle_response(
                            storage,
                            &mut node,
                            &transport_peer,
                            context,
                            response,
                            &mut outbound,
                            &mut leases,
                        )?;
                    }
                    NetworkEvent::OutboundFailure { transport_peer, request_id, error } => {
                        if let Some(context) = outbound.remove(&request_key(&request_id)) {
                            match context {
                                OutboundContext::Lease { world, peer, .. } => {
                                    leases.lease_acks.remove(&(world, peer));
                                }
                                OutboundContext::RecoveryBallot { world, peer, .. } => {
                                    leases.recovery_votes.remove(&(world, peer));
                                }
                                OutboundContext::MembershipProposal { world, peer, .. } => {
                                    leases.membership_votes.remove(&(world, peer));
                                }
                                OutboundContext::MembershipCommit { .. } => {}
                                OutboundContext::Epoch { world, peer, .. } => {
                                    leases.epoch_acks.remove(&(world, peer));
                                }
                                OutboundContext::Status { world, peer } => {
                                    leases.peer_status.remove(&(world, peer));
                                }
                                OutboundContext::HostCapability { world, peer } => {
                                    leases.peer_capability.remove(&(world, peer));
                                }
                                OutboundContext::Manifest { .. } => {}
                            }
                        }
                        warn!(transport = %transport_peer, %error, "outbound peer request failed; replication will renegotiate after reconnect");
                    }
                    NetworkEvent::Disconnected { transport_peer } => {
                        if let Some(application_peer) = leases.authenticated_peers.remove(&transport_peer) {
                            leases.lease_acks.retain(|(_, peer), _| *peer != application_peer);
                            leases.recovery_votes.retain(|(_, peer), _| *peer != application_peer);
                            leases.membership_votes.retain(|(_, peer), _| *peer != application_peer);
                            leases.epoch_acks.retain(|(_, peer), _| *peer != application_peer);
                            leases.peer_status.retain(|(_, peer), _| *peer != application_peer);
                            leases.peer_capability.retain(|(_, peer), _| *peer != application_peer);
                        }
                        info!(transport = %transport_peer, "peer disconnected");
                    }
                    NetworkEvent::Connected { transport_peer } => {
                        info!(transport = %transport_peer, "transport connected; waiting for connection-bound application proof");
                    }
                    NetworkEvent::Discovered { transport_peer, address } => {
                        info!(transport = %transport_peer, %address, "peer discovered");
                    }
                }
            }
        }
    }
}

fn maintain_authority_leases(
    paths: &DataPaths,
    storage: &Storage,
    identity: &PeerIdentity,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
    now: Instant,
) -> Result<()> {
    let recovery_initial_delay = Duration::from_millis(AUTHORITY_LEASE_DURATION_MS) + RECOVERY_SETTLE_DELAY;
    for metadata in storage.list_worlds()? {
        let world = metadata.world_id;
        recover_committed_membership(storage, identity, world)?;
        if let Err(error) = publish_host_readiness_snapshot(paths, storage, identity, runtime, world, now) {
            warn!(%world, %error, "host-readiness snapshot could not be published");
        }
        if storage.load_sleep_record(world).is_ok() {
            clear_permit(paths, world)?;
            clear_runtime_world(runtime, world);
            continue;
        }
        if let Ok(promise) = storage.load_membership_promise(world) {
            clear_permit(paths, world)?;
            runtime.permit_heartbeats.remove(&world);
            maintain_membership_transition(storage, identity, node, outbound, runtime, &promise)?;
            continue;
        }

        let Ok(descriptor) = storage.load_world_descriptor(world) else {
            clear_permit(paths, world)?;
            continue;
        };
        let Some(local_member) = descriptor.member(identity.peer_id()) else {
            clear_permit(paths, world)?;
            continue;
        };
        if local_member.banned || local_member.public_key != identity.public_key() {
            clear_permit(paths, world)?;
            continue;
        }

        let Ok(epoch) = storage.load_epoch_record(world) else {
            clear_permit(paths, world)?;
            continue;
        };
        let generation = AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token };
        let member_count = descriptor.members.iter().filter(|member| !member.banned).count();
        runtime.recovery_not_before.entry(world).or_insert(now + recovery_initial_delay);

        if epoch.authority_peer_id == identity.peer_id() && epoch.authority_public_key == identity.public_key() {
            if epoch.mode == EpochMode::Recovery {
                ensure_recovery_artifacts(storage, identity, &epoch)?;
            }
            request_world_statuses(storage, node, outbound, runtime, &descriptor, identity.peer_id())?;
            request_host_capabilities(node, outbound, runtime, &descriptor, identity.peer_id())?;
            let context = LocalAuthorityContext {
                paths,
                storage,
                identity,
                descriptor: &descriptor,
                epoch: &epoch,
                generation,
                now,
            };
            maintain_local_authority(&context, node, outbound, runtime)?;
            continue;
        }

        clear_permit(paths, world)?;
        runtime.permit_heartbeats.remove(&world);
        if !local_member.authority_eligible || member_count <= 1 {
            continue;
        }

        request_world_statuses(storage, node, outbound, runtime, &descriptor, identity.peer_id())?;
        request_host_capabilities(node, outbound, runtime, &descriptor, identity.peer_id())?;
        if runtime.authenticated_peers.values().any(|peer| *peer == epoch.authority_peer_id) {
            continue;
        }
        if !recovery_window_open(runtime, world, generation, now) {
            continue;
        }

        let Some(latest) = storage.latest_snapshot(world)? else {
            continue;
        };
        storage.verify_snapshot(&latest)?;
        verify_snapshot_signature(&latest)?;
        let latest_hash = latest.manifest_hash()?;
        let mut visible_peers = vec![identity.peer_id()];
        let mut candidates = vec![AuthorityCandidate {
            peer_id: identity.peer_id(),
            accepted_epoch: epoch.epoch_number,
            canonical_sequence: latest.sequence,
            snapshot_complete: true,
            compatible: true,
            authority_eligible: true,
            banned: false,
        }];

        for member in descriptor.members.iter().filter(|member| member.peer_id != identity.peer_id() && !member.banned)
        {
            if !runtime.authenticated_peers.values().any(|peer| *peer == member.peer_id) {
                continue;
            }
            let Some(observed) = runtime.peer_status.get(&(world, member.peer_id)) else {
                continue;
            };
            if now.saturating_duration_since(observed.observed_at) > STATUS_FRESHNESS {
                continue;
            }
            let status = &observed.status;
            if status.world_id != world
                || status.epoch != epoch.epoch_number
                || status.sequence != latest.sequence
                || status.latest_snapshot != Some(latest_hash)
                || status.state_hash != Some(latest.state_root)
                || status.compatibility_fingerprint != descriptor.compatibility_fingerprint
            {
                continue;
            }
            visible_peers.push(member.peer_id);
            if member.authority_eligible && status.authority_eligible {
                candidates.push(AuthorityCandidate {
                    peer_id: member.peer_id,
                    accepted_epoch: status.epoch,
                    canonical_sequence: status.sequence,
                    snapshot_complete: true,
                    compatible: true,
                    authority_eligible: true,
                    banned: false,
                });
            }
        }

        if !has_quorum(member_count, visible_peers.len()) || candidates.is_empty() {
            continue;
        }
        if elect_authority(&candidates)? != identity.peer_id() {
            continue;
        }

        let recovery_generation =
            generation.checked_next().context("authority generation exhausted during crash recovery")?;
        drive_recovery_ballot(
            RecoveryAttempt {
                storage,
                identity,
                descriptor: &descriptor,
                previous: &epoch,
                latest: &latest,
                visible_peers: &visible_peers,
                recovery_generation,
            },
            node,
            outbound,
            runtime,
        )?;
    }
    Ok(())
}

struct RecoveryAttempt<'a> {
    storage: &'a Storage,
    identity: &'a PeerIdentity,
    descriptor: &'a WorldDescriptorV1,
    previous: &'a EpochRecordV1,
    latest: &'a SnapshotManifestV1,
    visible_peers: &'a [PeerId],
    recovery_generation: AuthorityGeneration,
}

fn drive_recovery_ballot(
    attempt: RecoveryAttempt<'_>,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
) -> Result<()> {
    let RecoveryAttempt { storage, identity, descriptor, previous, latest, visible_peers, recovery_generation } =
        attempt;
    let world = descriptor.world_id;
    let base_snapshot_hash = latest.manifest_hash()?;
    let membership = storage.load_membership_record(world)?;
    verify_membership_signature(&membership)?;
    let membership_hash = membership.record_hash()?;
    let durable_round = storage.load_recovery_promise(world).map_or(0, |promise| promise.ballot.round);
    let floor = runtime.recovery_round_floor.get(&world).copied().unwrap_or(0).max(durable_round);
    let active_is_usable = runtime.recovery_ballots.get(&world).is_some_and(|ballot| {
        ballot.base_epoch == previous.epoch_number
            && ballot.base_fencing_token == previous.fencing_token
            && ballot.target_epoch == recovery_generation.epoch
            && ballot.target_fencing_token == recovery_generation.fencing_token
            && ballot.candidate_peer_id == identity.peer_id()
            && ballot.base_snapshot_hash == base_snapshot_hash
            && ballot.base_state_hash == latest.state_root
            && ballot.membership_hash == membership_hash
            && ballot.round >= floor
    });

    if !active_is_usable {
        let round = floor.checked_add(1).context("recovery round counter exhausted")?.max(1);
        let mut ballot = RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            base_epoch: previous.epoch_number,
            base_fencing_token: previous.fencing_token,
            target_epoch: recovery_generation.epoch,
            target_fencing_token: recovery_generation.fencing_token,
            round,
            candidate_peer_id: identity.peer_id(),
            candidate_public_key: identity.public_key(),
            base_snapshot_hash,
            base_state_hash: latest.state_root,
            membership_hash,
            signature: Vec::new(),
        };
        sign_recovery_ballot(identity, &mut ballot)?;
        let mut local_vote = RecoveryVoteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            ballot_hash: ballot.ballot_hash()?,
            base_epoch: ballot.base_epoch,
            target_epoch: ballot.target_epoch,
            round: ballot.round,
            candidate_peer_id: ballot.candidate_peer_id,
            voter_peer_id: identity.peer_id(),
            voter_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        sign_recovery_vote(identity, &mut local_vote)?;
        match storage.promise_recovery_ballot(&ballot, &local_vote)? {
            RecoveryPromiseResult::Accepted => {}
            RecoveryPromiseResult::Idempotent => {
                local_vote = storage.load_recovery_promise(world)?.vote;
            }
            RecoveryPromiseResult::Rejected { highest_round } => {
                runtime.recovery_round_floor.insert(world, highest_round);
                return Ok(());
            }
        }
        runtime.recovery_votes.retain(|(vote_world, _), _| *vote_world != world);
        runtime.recovery_votes.insert((world, identity.peer_id()), local_vote);
        runtime.recovery_ballots.insert(world, ballot);
    }

    let ballot = runtime.recovery_ballots.get(&world).context("active recovery ballot disappeared")?.clone();
    let ballot_hash = ballot.ballot_hash()?;
    for (transport_peer, application_peer) in &runtime.authenticated_peers {
        if *application_peer == identity.peer_id() || !visible_peers.contains(application_peer) {
            continue;
        }
        if recovery_ballot_request_pending(outbound, world, *application_peer, ballot_hash) {
            continue;
        }
        let request_id = node.send_request(transport_peer, WireRequest::RecoveryBallot(Box::new(ballot.clone())))?;
        outbound.insert(
            request_key(&request_id),
            OutboundContext::RecoveryBallot { world, peer: *application_peer, ballot_hash },
        );
    }

    let members =
        descriptor.members.iter().filter(|member| !member.banned).map(|member| member.peer_id).collect::<Vec<_>>();
    let votes = runtime
        .recovery_votes
        .iter()
        .filter(|((vote_world, _), vote)| *vote_world == world && vote.matches_ballot(&ballot).unwrap_or(false))
        .map(|(_, vote)| vote.clone())
        .collect::<Vec<_>>();
    if !has_quorum(members.len(), votes.len()) {
        return Ok(());
    }
    let certificate = RecoveryCertificateV1 { ballot: ballot.clone(), votes };
    validate_recovery_certificate_shape(&certificate, &members)?;
    for vote in &certificate.votes {
        verify_recovery_vote_signature(vote)?;
    }

    // Persist the quorum proof before the epoch. A crash between these writes can
    // safely retry promotion; the proof itself never grants an older round power.
    storage.save_recovery_certificate(&certificate)?;
    #[cfg(debug_assertions)]
    if let Ok(delay_ms) = std::env::var("SWARMCRAFT_TEST_PAUSE_AFTER_RECOVERY_CERTIFICATE_MS") {
        if let Ok(delay_ms) = delay_ms.parse::<u64>() {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }
    let next = promote_recovery_epoch(storage, identity, previous, latest)?;
    let _ = storage.clear_recovery_promise_after_epoch_advance(world, next.epoch_number)?;
    runtime.lease_acks.retain(|(ack_world, _), _| *ack_world != world);
    runtime.epoch_acks.retain(|(ack_world, _), _| *ack_world != world);
    runtime.inbound_leases.remove(&world);
    runtime.recovery_replication_sent.retain(|(replica_world, _)| *replica_world != world);
    runtime.recovery_ballots.remove(&world);
    runtime.recovery_votes.retain(|(vote_world, _), _| *vote_world != world);

    for (transport_peer, application_peer) in &runtime.authenticated_peers {
        if *application_peer == identity.peer_id() || !visible_peers.contains(application_peer) {
            continue;
        }
        let request_id = node.send_request(
            transport_peer,
            WireRequest::RecoveryEpoch { record: next.clone(), certificate: Box::new(certificate.clone()) },
        )?;
        outbound.insert(
            request_key(&request_id),
            OutboundContext::Epoch { world, peer: *application_peer, generation: recovery_generation },
        );
    }
    info!(world = %world, epoch = next.epoch_number, round = ballot.round, peer = %identity.peer_id(), "authority recovered with durable quorum ballot");
    Ok(())
}

fn maintain_local_authority(
    context: &LocalAuthorityContext<'_>,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
) -> Result<()> {
    let world = context.descriptor.world_id;
    if context.epoch.mode == EpochMode::Recovery && !maintain_recovery_epoch_quorum(context, node, outbound, runtime)? {
        clear_permit(context.paths, world)?;
        return Ok(());
    }

    let lease = signed_lease(context.identity, world, context.generation)?;
    for (transport_peer, application_peer) in &runtime.authenticated_peers {
        let Some(member) = context.descriptor.member(*application_peer) else {
            continue;
        };
        if member.banned || *application_peer == context.identity.peer_id() {
            continue;
        }
        if lease_request_pending(outbound, world, *application_peer, context.generation) {
            continue;
        }
        let request_id = node.send_request(transport_peer, WireRequest::LeaseGrant(lease.clone()))?;
        outbound.insert(
            request_key(&request_id),
            OutboundContext::Lease { world, peer: *application_peer, generation: context.generation },
        );
    }

    let member_count = context.descriptor.members.iter().filter(|member| !member.banned).count();
    let fresh_window = Duration::from_millis(AUTHORITY_LEASE_DURATION_MS);
    let confirmed = 1 + context
        .descriptor
        .members
        .iter()
        .filter(|member| member.peer_id != context.identity.peer_id() && !member.banned)
        .filter(|member| runtime.authenticated_peers.values().any(|peer| *peer == member.peer_id))
        .filter(|member| {
            runtime.lease_acks.get(&(world, member.peer_id)).is_some_and(|ack| {
                ack.generation == context.generation
                    && context.now.saturating_duration_since(ack.observed_at) < fresh_window
            })
        })
        .count();
    if has_quorum(member_count, confirmed) {
        if context.epoch.mode == EpochMode::Solo {
            refresh_solo_branch(context.storage, context.identity, context.epoch)?;
            promote_solo_to_quorum(context, node, outbound, runtime)?;
            clear_permit(context.paths, world)?;
            return Ok(());
        }
        let heartbeat = runtime.permit_heartbeats.entry(world).or_default();
        *heartbeat = heartbeat.saturating_add(1);
        refresh_permit(context.paths, world, context.generation, *heartbeat)?;
    } else {
        clear_permit(context.paths, world)?;
        request_world_statuses(
            context.storage,
            node,
            outbound,
            runtime,
            context.descriptor,
            context.identity.peer_id(),
        )?;
    }
    context.storage.clear_recovery_reservation(world)?;
    let _ = context.storage.clear_recovery_promise_after_epoch_advance(world, context.epoch.epoch_number)?;
    Ok(())
}

fn solo_mode_allowed(storage: &Storage, world: WorldId) -> Result<bool> {
    let Ok(config) = storage.load_world_config(world) else {
        return Ok(false);
    };
    verify_world_config_signature(&config)?;
    Ok(config.authority_policy.allow_solo_advancement)
}

fn refresh_solo_branch(storage: &Storage, identity: &PeerIdentity, epoch: &EpochRecordV1) -> Result<()> {
    if epoch.mode != EpochMode::Solo {
        return Ok(());
    }
    let latest = storage.latest_snapshot(epoch.world_id)?.context("solo epoch has no snapshot")?;
    verify_snapshot_signature(&latest)?;
    let mut branch =
        storage.load_solo_branch(epoch.world_id).context("solo epoch is missing durable branch ancestry")?;
    verify_solo_branch_signature(&branch)?;
    if branch.authority_peer_id != identity.peer_id() || branch.world_id != epoch.world_id {
        return Err(anyhow!("solo branch authority does not match the local accepted authority"));
    }
    let latest_hash = latest.manifest_hash()?;
    if branch.head_snapshot_hash == latest_hash && branch.state_hash == latest.state_root {
        return Ok(());
    }
    branch.head_snapshot_hash = latest_hash;
    branch.head_epoch = epoch.epoch_number;
    branch.head_sequence = latest.sequence;
    branch.state_hash = latest.state_root;
    sign_solo_branch(identity, &mut branch)?;
    storage.save_solo_branch(&branch)?;
    Ok(())
}

fn promote_solo_to_quorum(
    context: &LocalAuthorityContext<'_>,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
) -> Result<EpochRecordV1> {
    let latest = context
        .storage
        .latest_snapshot(context.descriptor.world_id)?
        .context("cannot restore quorum without a solo head snapshot")?;
    let next_generation =
        context.generation.checked_next().context("authority generation exhausted while restoring quorum")?;
    let mut next = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: context.descriptor.world_id,
        epoch_number: next_generation.epoch,
        previous_epoch_hash: Some(epoch_record_hash(context.epoch)?),
        base_state_hash: latest.state_root,
        authority_peer_id: context.identity.peer_id(),
        authority_public_key: context.identity.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: next_generation.fencing_token,
        reason: "quorum restored after explicit solo history replication".into(),
        signature: Vec::new(),
    };
    next.signature = context.identity.sign(&next.signing_bytes()?);
    context.storage.save_epoch_record(&next)?;
    let generation = AuthorityGeneration { epoch: next.epoch_number, fencing_token: next.fencing_token };
    for (transport_peer, application_peer) in &runtime.authenticated_peers {
        let Some(member) = context.descriptor.member(*application_peer) else { continue };
        if member.banned || *application_peer == context.identity.peer_id() {
            continue;
        }
        let request_id = node.send_request(transport_peer, WireRequest::Epoch(next.clone()))?;
        outbound.insert(
            request_key(&request_id),
            OutboundContext::Epoch { world: next.world_id, peer: *application_peer, generation },
        );
    }
    info!(world = %next.world_id, epoch = next.epoch_number, "quorum restored after solo mode");
    Ok(next)
}

fn maintain_recovery_epoch_quorum(
    context: &LocalAuthorityContext<'_>,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
) -> Result<bool> {
    let world = context.descriptor.world_id;
    let certificate = context
        .storage
        .load_recovery_certificate(world)
        .context("accepted recovery epoch is missing its durable quorum certificate")?;
    if certificate.ballot.target_epoch != context.generation.epoch
        || certificate.ballot.target_fencing_token != context.generation.fencing_token
        || certificate.ballot.candidate_peer_id != context.identity.peer_id()
    {
        return Err(anyhow!("durable recovery certificate does not match the accepted recovery generation"));
    }
    for (transport_peer, application_peer) in &runtime.authenticated_peers {
        let Some(member) = context.descriptor.member(*application_peer) else {
            continue;
        };
        if member.banned || *application_peer == context.identity.peer_id() {
            continue;
        }
        if runtime.epoch_acks.get(&(world, *application_peer)) == Some(&context.generation)
            || epoch_request_pending(outbound, world, *application_peer, context.generation)
        {
            continue;
        }
        let request_id = node.send_request(
            transport_peer,
            WireRequest::RecoveryEpoch { record: context.epoch.clone(), certificate: Box::new(certificate.clone()) },
        )?;
        outbound.insert(
            request_key(&request_id),
            OutboundContext::Epoch { world, peer: *application_peer, generation: context.generation },
        );
    }

    let member_count = context.descriptor.members.iter().filter(|member| !member.banned).count();
    let confirmed = 1 + context
        .descriptor
        .members
        .iter()
        .filter(|member| member.peer_id != context.identity.peer_id() && !member.banned)
        .filter(|member| runtime.authenticated_peers.values().any(|peer| *peer == member.peer_id))
        .filter(|member| runtime.epoch_acks.get(&(world, member.peer_id)) == Some(&context.generation))
        .count();
    Ok(has_quorum(member_count, confirmed))
}

fn signed_lease(
    identity: &PeerIdentity,
    world: WorldId,
    generation: AuthorityGeneration,
) -> Result<AuthorityLeaseGrantV1> {
    let mut lease = AuthorityLeaseGrantV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch: generation.epoch,
        fencing_token: generation.fencing_token,
        lease_duration_ms: AUTHORITY_LEASE_DURATION_MS,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        nonce: random_nonce(),
        signature: Vec::new(),
    };
    identity.sign_lease(&mut lease)?;
    Ok(lease)
}

fn request_world_statuses(
    storage: &Storage,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &LeaseRuntime,
    descriptor: &WorldDescriptorV1,
    local_peer: PeerId,
) -> Result<()> {
    storage.load_world(descriptor.world_id)?;
    for (transport_peer, application_peer) in &runtime.authenticated_peers {
        if *application_peer == local_peer || descriptor.member(*application_peer).is_none() {
            continue;
        }
        if status_request_pending(outbound, descriptor.world_id, *application_peer) {
            continue;
        }
        let request_id =
            node.send_request(transport_peer, WireRequest::WorldStatus { world_id: descriptor.world_id })?;
        outbound.insert(
            request_key(&request_id),
            OutboundContext::Status { world: descriptor.world_id, peer: *application_peer },
        );
    }
    Ok(())
}

fn request_host_capabilities(
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &LeaseRuntime,
    descriptor: &WorldDescriptorV1,
    local_peer: PeerId,
) -> Result<()> {
    for (transport_peer, application_peer) in &runtime.authenticated_peers {
        if *application_peer == local_peer {
            continue;
        }
        let Some(member) = descriptor.member(*application_peer) else {
            continue;
        };
        if member.banned || capability_request_pending(outbound, descriptor.world_id, *application_peer) {
            continue;
        }
        let request_id =
            node.send_request(transport_peer, WireRequest::HostCapability { world_id: descriptor.world_id })?;
        outbound.insert(
            request_key(&request_id),
            OutboundContext::HostCapability { world: descriptor.world_id, peer: *application_peer },
        );
    }
    Ok(())
}

fn peer_readiness_observations(
    descriptor: &WorldDescriptorV1,
    local_peer: PeerId,
    runtime: &LeaseRuntime,
    now: Instant,
) -> Vec<PeerReadinessObservation> {
    descriptor
        .members
        .iter()
        .filter(|member| member.peer_id != local_peer && !member.banned)
        .map(|member| {
            let peer = member.peer_id;
            let reachable = runtime.authenticated_peers.values().any(|value| *value == peer);
            let status = runtime
                .peer_status
                .get(&(descriptor.world_id, peer))
                .filter(|observed| now.saturating_duration_since(observed.observed_at) <= STATUS_FRESHNESS)
                .map(|observed| observed.status.clone());
            let capability = runtime
                .peer_capability
                .get(&(descriptor.world_id, peer))
                .filter(|observed| now.saturating_duration_since(observed.observed_at) <= STATUS_FRESHNESS)
                .map(|observed| observed.capability.clone());
            PeerReadinessObservation { peer_id: peer, reachable, status, capability }
        })
        .collect()
}

fn local_recovery_quorum_without_authority(
    storage: &Storage,
    world: WorldId,
    local_peer: PeerId,
    runtime: &LeaseRuntime,
    now: Instant,
) -> Result<bool> {
    let Ok(descriptor) = storage.load_world_descriptor(world) else {
        return Ok(false);
    };
    let Ok(epoch) = storage.load_epoch_record(world) else {
        return Ok(false);
    };
    let Some(latest) = storage.latest_snapshot(world)? else {
        return Ok(false);
    };
    let observations = peer_readiness_observations(&descriptor, local_peer, runtime, now);
    host_readiness::surviving_recovery_quorum(&descriptor, &epoch, &latest, local_peer, &observations)
}

fn publish_host_readiness_snapshot(
    paths: &DataPaths,
    storage: &Storage,
    identity: &PeerIdentity,
    runtime: &LeaseRuntime,
    world: WorldId,
    now: Instant,
) -> Result<()> {
    let observations = storage
        .load_world_descriptor(world)
        .map(|descriptor| peer_readiness_observations(&descriptor, identity.peer_id(), runtime, now))
        .unwrap_or_default();
    let report = host_readiness::evaluate_from_storage(storage, identity.peer_id(), world, &observations)?;
    host_readiness::publish_report(paths, world, &report)
}

fn recovery_window_open(runtime: &LeaseRuntime, world: WorldId, generation: AuthorityGeneration, now: Instant) -> bool {
    if let Some(lease) = runtime.inbound_leases.get(&world) {
        if lease.generation > generation {
            return false;
        }
        if lease.generation == generation {
            return now >= lease.expires_at + RECOVERY_SETTLE_DELAY;
        }
    }
    runtime.recovery_not_before.get(&world).is_some_and(|deadline| now >= *deadline)
}

fn promote_recovery_epoch(
    storage: &Storage,
    identity: &PeerIdentity,
    previous: &EpochRecordV1,
    latest: &SnapshotManifestV1,
) -> Result<EpochRecordV1> {
    let next_generation = AuthorityGeneration { epoch: previous.epoch_number, fencing_token: previous.fencing_token }
        .checked_next()
        .context("authority generation exhausted during recovery promotion")?;
    let mut next = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: previous.world_id,
        epoch_number: next_generation.epoch,
        previous_epoch_hash: Some(epoch_record_hash(previous)?),
        base_state_hash: latest.state_root,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        mode: EpochMode::Recovery,
        fencing_token: next_generation.fencing_token,
        reason: "automatic crash recovery after durable quorum ballot".into(),
        signature: Vec::new(),
    };
    next.signature = identity.sign(&next.signing_bytes()?);
    storage.save_epoch_record(&next)?;
    Ok(next)
}

fn ensure_recovery_artifacts(storage: &Storage, identity: &PeerIdentity, epoch: &EpochRecordV1) -> Result<()> {
    let latest = storage.latest_snapshot(epoch.world_id)?.context("recovery epoch has no base snapshot")?;
    if latest.epoch < epoch.epoch_number {
        if latest.epoch.checked_add(1) != Some(epoch.epoch_number) || latest.state_root != epoch.base_state_hash {
            return Err(anyhow!("recovery epoch does not directly promote the latest canonical snapshot"));
        }
        let mut promoted = SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: epoch.world_id,
            snapshot_number: storage.next_snapshot_number(epoch.world_id)?,
            epoch: epoch.epoch_number,
            sequence: latest.sequence.checked_add(1).context("snapshot sequence counter exhausted")?,
            previous_snapshot_hash: Some(latest.manifest_hash()?),
            entries: latest.entries.clone(),
            state_root: latest.state_root,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        identity.sign_snapshot(&mut promoted)?;
        storage.commit_snapshot(&promoted)?;
    } else if latest.epoch != epoch.epoch_number
        || latest.authority_peer_id != identity.peer_id()
        || latest.authority_public_key != identity.public_key()
        || latest.state_root != epoch.base_state_hash
    {
        return Err(anyhow!("latest snapshot conflicts with the accepted recovery epoch"));
    }

    let membership = storage.load_membership_record(epoch.world_id)?;
    if membership.epoch > epoch.epoch_number {
        return Err(anyhow!("membership is ahead of the accepted recovery epoch"));
    }
    if membership.epoch != epoch.epoch_number
        || membership.authority_peer_id != identity.peer_id()
        || membership.authority_public_key != identity.public_key()
    {
        let mut promoted = MembershipRecordV1 {
            protocol_version: membership.protocol_version,
            world_id: epoch.world_id,
            epoch: epoch.epoch_number,
            sequence: membership.sequence.checked_add(1).context("membership sequence counter exhausted")?,
            previous_membership_hash: Some(membership.record_hash()?),
            members: membership.members.clone(),
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        identity.sign_membership(&mut promoted)?;
        storage.save_membership_record(&promoted)?;
    }
    Ok(())
}

fn clear_runtime_world(runtime: &mut LeaseRuntime, world: WorldId) {
    runtime.lease_acks.retain(|(ack_world, _), _| *ack_world != world);
    runtime.recovery_ballots.remove(&world);
    runtime.recovery_votes.retain(|(ack_world, _), _| *ack_world != world);
    runtime.membership_votes.retain(|(ack_world, _), _| *ack_world != world);
    runtime.recovery_round_floor.remove(&world);
    runtime.epoch_acks.retain(|(ack_world, _), _| *ack_world != world);
    runtime.peer_status.retain(|(status_world, _), _| *status_world != world);
    runtime.peer_capability.retain(|(status_world, _), _| *status_world != world);
    runtime.permit_heartbeats.remove(&world);
    runtime.inbound_leases.remove(&world);
    runtime.recovery_not_before.remove(&world);
    runtime.recovery_replication_sent.retain(|(replica_world, _)| *replica_world != world);
}

fn lease_request_pending(
    outbound: &HashMap<String, OutboundContext>,
    world: WorldId,
    peer: PeerId,
    generation: AuthorityGeneration,
) -> bool {
    outbound.values().any(|context| {
        matches!(
            context,
            OutboundContext::Lease {
                world: request_world,
                peer: request_peer,
                generation: request_generation,
            } if *request_world == world && *request_peer == peer && *request_generation == generation
        )
    })
}

fn recovery_ballot_request_pending(
    outbound: &HashMap<String, OutboundContext>,
    world: WorldId,
    peer: PeerId,
    ballot_hash: Hash32,
) -> bool {
    outbound.values().any(|context| {
        matches!(
            context,
            OutboundContext::RecoveryBallot {
                world: request_world,
                peer: request_peer,
                ballot_hash: request_hash,
            } if *request_world == world && *request_peer == peer && *request_hash == ballot_hash
        )
    })
}

fn epoch_request_pending(
    outbound: &HashMap<String, OutboundContext>,
    world: WorldId,
    peer: PeerId,
    generation: AuthorityGeneration,
) -> bool {
    outbound.values().any(|context| {
        matches!(
            context,
            OutboundContext::Epoch {
                world: request_world,
                peer: request_peer,
                generation: request_generation,
            } if *request_world == world && *request_peer == peer && *request_generation == generation
        )
    })
}

fn status_request_pending(outbound: &HashMap<String, OutboundContext>, world: WorldId, peer: PeerId) -> bool {
    outbound.values().any(|context| {
        matches!(context, OutboundContext::Status { world: request_world, peer: request_peer } if *request_world == world && *request_peer == peer)
    })
}

fn capability_request_pending(outbound: &HashMap<String, OutboundContext>, world: WorldId, peer: PeerId) -> bool {
    outbound.values().any(|context| {
        matches!(context, OutboundContext::HostCapability { world: request_world, peer: request_peer } if *request_world == world && *request_peer == peer)
    })
}

fn dial_pending_invite_bootstraps(storage: &Storage, node: &mut SwarmNode) -> Result<()> {
    for metadata in storage.list_worlds()? {
        let Ok(request) = storage.load_pending_join(metadata.world_id) else { continue };
        for value in request.invite.bootstrap_addrs {
            match value.parse() {
                Ok(address) => {
                    if let Err(error) = validate_invite_dial_address(&address) {
                        warn!(world = %metadata.world_id, %value, %error, "invite bootstrap DNS scope validation failed");
                        continue;
                    }
                    if let Err(error) = node.dial(address) {
                        warn!(world = %metadata.world_id, %value, %error, "invite bootstrap dial failed");
                    }
                }
                Err(error) => warn!(world = %metadata.world_id, %value, %error, "invalid invite bootstrap address"),
            }
        }
    }
    Ok(())
}

fn push_pending_membership_requests(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    application_peer: PeerId,
) -> Result<()> {
    for metadata in storage.list_worlds()? {
        if let Ok(join) = storage.load_pending_join(metadata.world_id) {
            if join.invite.inviter_peer_id == application_peer {
                node.send_request(transport_peer, WireRequest::JoinRequest(Box::new(join)))?;
            }
        }
        if let Ok(leave) = storage.load_pending_leave(metadata.world_id) {
            if let Ok(membership) = storage.load_membership_record(metadata.world_id) {
                if membership.authority_peer_id == application_peer {
                    node.send_request(transport_peer, WireRequest::LeaveRequest(Box::new(leave)))?;
                }
            }
        }
    }
    Ok(())
}

fn push_known_worlds(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    application_peer: PeerId,
    local_peer: PeerId,
    outbound: &mut HashMap<String, OutboundContext>,
) -> Result<()> {
    for metadata in storage.list_worlds()? {
        let world = metadata.world_id;
        let Ok(descriptor) = storage.load_world_descriptor(world) else { continue };
        let local_is_authority =
            storage.load_epoch_record(world).is_ok_and(|epoch| epoch.authority_peer_id == local_peer);
        if !local_is_authority && !storage.background_seeding_enabled(world)? {
            continue;
        }

        if let Ok(membership) = storage.load_membership_record(world) {
            if let Ok(certificate) = storage.load_membership_certificate(world) {
                if certificate.proposal.proposed.record_hash()? == membership.record_hash()?
                    && proposal_member(&certificate.proposal, application_peer).is_some()
                {
                    let id = node.send_request(transport_peer, WireRequest::MembershipCommit(Box::new(certificate)))?;
                    outbound.insert(
                        request_key(&id),
                        OutboundContext::MembershipCommit {
                            world,
                            peer: application_peer,
                            sequence: membership.sequence,
                        },
                    );
                    continue;
                }
            }
        }
        if authorized_descriptor_member(&descriptor, application_peer).is_err() {
            continue;
        }
        push_committed_world_payload(storage, node, transport_peer, world, outbound, true)?;
    }
    Ok(())
}

fn push_committed_world_payload(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    world: WorldId,
    outbound: &mut HashMap<String, OutboundContext>,
    include_membership: bool,
) -> Result<()> {
    if let Ok(config) = storage.load_world_config(world) {
        node.send_request(transport_peer, WireRequest::WorldConfig(Box::new(config)))?;
    }
    if let Ok(branch) = storage.load_solo_branch(world) {
        node.send_request(transport_peer, WireRequest::SoloBranch(Box::new(branch)))?;
    }
    if let Ok(epoch) = storage.load_epoch_record(world) {
        if epoch.mode == EpochMode::Recovery {
            if let Ok(certificate) = storage.load_recovery_certificate(world) {
                node.send_request(
                    transport_peer,
                    WireRequest::RecoveryEpoch { record: epoch, certificate: Box::new(certificate) },
                )?;
            }
        } else {
            node.send_request(transport_peer, WireRequest::Epoch(epoch))?;
        }
    }
    if include_membership {
        if let Ok(membership) = storage.load_membership_record(world) {
            node.send_request(transport_peer, WireRequest::Membership(membership))?;
        }
    }
    if let Ok(transfer) = storage.load_transfer_record(world) {
        node.send_request(transport_peer, WireRequest::AuthorityTransfer(transfer))?;
    }
    if let Ok(sleep) = storage.load_sleep_record(world) {
        node.send_request(transport_peer, WireRequest::Sleep(sleep))?;
    }
    if let Some(manifest) = storage.latest_snapshot(world)? {
        verify_snapshot_signature(&manifest)?;
        let id = node.send_request(transport_peer, WireRequest::SnapshotManifest(manifest.clone()))?;
        outbound
            .insert(request_key(&id), OutboundContext::Manifest { world, snapshot_number: manifest.snapshot_number });
    }
    Ok(())
}

fn handle_request(
    context: &HandlerContext<'_>,
    node: &mut SwarmNode,
    transport_peer: TransportPeerId,
    application_peer: PeerId,
    request: WireRequest,
    channel: ResponseChannel<WireResponse>,
    state: &mut RequestState<'_>,
) -> Result<()> {
    let identity = context.identity;
    let storage = context.storage;
    if let Some(world_id) = request.membership_world_id() {
        authorize_member(storage, world_id, application_peer)?;
    }
    match request {
        WireRequest::Ping { nonce } => node.respond(channel, WireResponse::Pong { nonce })?,
        WireRequest::WorldStatus { world_id } => {
            let status = world_status(storage, world_id, identity.peer_id())?;
            node.respond(channel, WireResponse::WorldStatus(status))?;
        }
        WireRequest::HostCapability { world_id } => {
            let recovery_quorum = local_recovery_quorum_without_authority(
                storage,
                world_id,
                identity.peer_id(),
                state.leases,
                state.now,
            )?;
            let capability = host_readiness::local_host_capability(context.paths, storage, world_id, recovery_quorum)?;
            node.respond(channel, WireResponse::HostCapability(capability))?;
        }
        WireRequest::WorldDescriptor { world_id } => {
            let descriptor = storage.load_world_descriptor(world_id).ok();
            node.respond(channel, WireResponse::WorldDescriptor(descriptor))?;
        }
        WireRequest::JoinRequest(request) => {
            let request = *request;
            if application_peer != request.joining_member.peer_id {
                return Err(anyhow!("join request transport identity does not match joining peer"));
            }
            verify_join_request_signature(&request)?;
            verify_invite_signature(&request.invite)?;
            crate::invite::validate_invite_expiry(request.invite.expires_unix_ms, unix_millis()?)
                .context("join invite lifetime is invalid")?;
            let world = request.world_id;
            let metadata = storage.load_world(world)?;
            if request.invite.genesis.world_id()? != world
                || request.invite.genesis.compatibility_fingerprint != metadata.genesis.compatibility_fingerprint
            {
                return Err(anyhow!("invite does not match local world genesis"));
            }
            let current = storage.load_membership_record(world)?;
            verify_membership_signature(&current)?;
            if current.authority_peer_id != identity.peer_id() || current.authority_public_key != identity.public_key()
            {
                return Err(anyhow!("only the current local authority may accept a join request"));
            }
            if request.invite.inviter_peer_id != current.authority_peer_id
                || request.invite.inviter_public_key != current.authority_public_key
            {
                return Err(anyhow!("join invite was not issued by the current authority"));
            }
            let descriptor = storage.load_world_descriptor(world)?;
            let inviter = descriptor
                .member(request.invite.inviter_peer_id)
                .context("invite signer is not a current world member")?;
            if inviter.banned || inviter.public_key != request.invite.inviter_public_key {
                return Err(anyhow!("invite signer is banned or key does not match current membership"));
            }
            if let Some(member) = descriptor.member(request.joining_member.peer_id) {
                if member.public_key != request.joining_member.public_key || member.banned {
                    return Err(anyhow!("joining peer conflicts with existing membership"));
                }
                node.respond(channel, WireResponse::JoinAccepted { membership_sequence: current.sequence })?;
                push_known_worlds(
                    storage,
                    node,
                    &transport_peer,
                    application_peer,
                    identity.peer_id(),
                    state.outbound,
                )?;
                return Ok(());
            }
            if let Ok(promise) = storage.load_membership_promise(world) {
                let proposed_member = promise
                    .proposal
                    .proposed
                    .members
                    .iter()
                    .find(|member| member.peer_id == request.joining_member.peer_id);
                if promise.proposal.previous.record_hash()? == current.record_hash()?
                    && proposed_member == Some(&request.joining_member)
                {
                    let id = node.send_request(
                        &transport_peer,
                        WireRequest::MembershipProposal(Box::new(promise.proposal.clone())),
                    )?;
                    state.outbound.insert(
                        request_key(&id),
                        OutboundContext::MembershipProposal {
                            world,
                            peer: application_peer,
                            proposal_hash: promise.proposal.proposal_hash()?,
                        },
                    );
                    node.respond(
                        channel,
                        WireResponse::JoinAccepted { membership_sequence: promise.proposal.proposed.sequence },
                    )?;
                    return Ok(());
                }
                return Err(anyhow!("another membership transition is already durably prepared"));
            }
            let mut proposed_descriptor = descriptor.clone();
            proposed_descriptor.members.push(request.joining_member.clone());
            proposed_descriptor.normalize();
            let mut next = MembershipRecordV1 {
                protocol_version: current.protocol_version,
                world_id: world,
                epoch: current.epoch,
                sequence: current.sequence.checked_add(1).context("membership sequence counter exhausted")?,
                previous_membership_hash: Some(current.record_hash()?),
                members: proposed_descriptor.members,
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            identity.sign_membership(&mut next)?;
            let proposal = MembershipProposalV1 { previous: current, proposed: next };
            validate_membership_proposal_shape(&proposal)?;
            let vote = sign_membership_vote(identity, &proposal)?;
            match storage.promise_membership_proposal(&proposal, &vote)? {
                MembershipPromiseResult::Accepted | MembershipPromiseResult::Idempotent => {}
                MembershipPromiseResult::Rejected => {
                    return Err(anyhow!("membership proposal conflicts with a durable prepare"))
                }
            }
            state.leases.membership_votes.insert((world, identity.peer_id()), vote);
            clear_permit(context.paths, world)?;
            state.leases.permit_heartbeats.remove(&world);
            let id = node.send_request(&transport_peer, WireRequest::MembershipProposal(Box::new(proposal.clone())))?;
            state.outbound.insert(
                request_key(&id),
                OutboundContext::MembershipProposal {
                    world,
                    peer: application_peer,
                    proposal_hash: proposal.proposal_hash()?,
                },
            );
            node.respond(channel, WireResponse::JoinAccepted { membership_sequence: proposal.proposed.sequence })?;
        }
        WireRequest::LeaveRequest(request) => {
            let request = *request;
            if application_peer != request.leaving_peer_id {
                return Err(anyhow!("leave request transport identity does not match leaving peer"));
            }
            verify_leave_request_signature(&request)?;
            let current = storage.load_membership_record(request.world_id)?;
            verify_membership_signature(&current)?;
            if current.authority_peer_id != identity.peer_id() || current.authority_public_key != identity.public_key()
            {
                return Err(anyhow!("only the current local authority may accept a leave request"));
            }
            if request.leaving_peer_id == current.authority_peer_id {
                return Err(anyhow!("authority must transfer authority before leaving"));
            }
            if current.record_hash()? != request.membership_hash {
                return Err(anyhow!("leave request references stale membership"));
            }
            let descriptor = storage.load_world_descriptor(request.world_id)?;
            let leaving =
                descriptor.member(request.leaving_peer_id).context("leaving peer is not a current world member")?;
            if leaving.banned || leaving.public_key != request.leaving_public_key {
                return Err(anyhow!("leaving peer key does not match current membership"));
            }
            if let Ok(promise) = storage.load_membership_promise(request.world_id) {
                let absent =
                    promise.proposal.proposed.members.iter().all(|member| member.peer_id != request.leaving_peer_id);
                if promise.proposal.previous.record_hash()? == current.record_hash()? && absent {
                    let id = node.send_request(
                        &transport_peer,
                        WireRequest::MembershipProposal(Box::new(promise.proposal.clone())),
                    )?;
                    state.outbound.insert(
                        request_key(&id),
                        OutboundContext::MembershipProposal {
                            world: request.world_id,
                            peer: application_peer,
                            proposal_hash: promise.proposal.proposal_hash()?,
                        },
                    );
                    node.respond(
                        channel,
                        WireResponse::LeaveAccepted { membership_sequence: promise.proposal.proposed.sequence },
                    )?;
                    return Ok(());
                }
                return Err(anyhow!("another membership transition is already durably prepared"));
            }
            let mut proposed_descriptor = descriptor.clone();
            proposed_descriptor.members.retain(|member| member.peer_id != request.leaving_peer_id);
            proposed_descriptor.normalize();
            let mut next = MembershipRecordV1 {
                protocol_version: current.protocol_version,
                world_id: current.world_id,
                epoch: current.epoch,
                sequence: current.sequence.checked_add(1).context("membership sequence counter exhausted")?,
                previous_membership_hash: Some(current.record_hash()?),
                members: proposed_descriptor.members,
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            identity.sign_membership(&mut next)?;
            let proposal = MembershipProposalV1 { previous: current, proposed: next };
            validate_membership_proposal_shape(&proposal)?;
            let vote = sign_membership_vote(identity, &proposal)?;
            match storage.promise_membership_proposal(&proposal, &vote)? {
                MembershipPromiseResult::Accepted | MembershipPromiseResult::Idempotent => {}
                MembershipPromiseResult::Rejected => {
                    return Err(anyhow!("membership proposal conflicts with a durable prepare"))
                }
            }
            state.leases.membership_votes.insert((request.world_id, identity.peer_id()), vote);
            clear_permit(context.paths, request.world_id)?;
            state.leases.permit_heartbeats.remove(&request.world_id);
            let id = node.send_request(&transport_peer, WireRequest::MembershipProposal(Box::new(proposal.clone())))?;
            state.outbound.insert(
                request_key(&id),
                OutboundContext::MembershipProposal {
                    world: request.world_id,
                    peer: application_peer,
                    proposal_hash: proposal.proposal_hash()?,
                },
            );
            node.respond(channel, WireResponse::LeaveAccepted { membership_sequence: proposal.proposed.sequence })?;
        }
        WireRequest::SnapshotManifest(manifest) => {
            authorize_manifest(storage, application_peer, &manifest)?;
            let missing = storage
                .missing_blobs(&manifest)
                .into_iter()
                .map(|descriptor| {
                    Ok(BlobResumeV1 {
                        hash: descriptor.hash,
                        offset: storage.partial_blob_offset(manifest.world_id, &descriptor)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            state.pending_manifests.insert(manifest.world_id, manifest.clone());
            node.respond(
                channel,
                WireResponse::ManifestAccepted { snapshot_number: manifest.snapshot_number, missing: missing.clone() },
            )?;
            if missing.is_empty() {
                finalize_and_ack(storage, node, &transport_peer, &manifest)?;
                state.pending_manifests.remove(&manifest.world_id);
            }
        }
        WireRequest::BlobChunk { world_id, hash, encoding, offset, data, finished } => {
            authorize_member(storage, world_id, application_peer)?;
            let manifest =
                state.pending_manifests.get(&world_id).context("blob chunk arrived without a negotiated manifest")?;
            let descriptor =
                find_descriptor(manifest, hash).context("blob hash is not referenced by negotiated manifest")?;
            if descriptor.encoding != encoding {
                return Err(anyhow!("blob encoding does not match manifest"));
            }
            let next_offset = storage.receive_blob_chunk(world_id, descriptor, offset, &data, finished)?;
            node.respond(channel, WireResponse::BlobChunkAccepted { hash, next_offset })?;
            if finished && storage.missing_blobs(manifest).is_empty() {
                let manifest = manifest.clone();
                finalize_and_ack(storage, node, &transport_peer, &manifest)?;
                state.pending_manifests.remove(&world_id);
            }
        }
        WireRequest::MissingBlobs { world_id, snapshot_number, hashes } => {
            authorize_member(storage, world_id, application_peer)?;
            let manifest = storage.load_snapshot(world_id, snapshot_number)?;
            let missing = hashes
                .into_iter()
                .filter_map(|hash| find_descriptor(&manifest, hash))
                .map(|descriptor| BlobResumeV1 { hash: descriptor.hash, offset: 0 })
                .collect();
            node.respond(channel, WireResponse::MissingBlobs(missing))?;
        }
        WireRequest::ReplicaAck(ack) => {
            authorize_member(storage, ack.world_id, application_peer)?;
            if !ack.complete {
                return Err(anyhow!("incomplete replica acknowledgement rejected"));
            }
            info!(peer = %application_peer, world = %ack.world_id, snapshot = ack.snapshot_number, "replica verified snapshot");
            node.respond(channel, WireResponse::ReplicaAckAccepted)?;
        }
        WireRequest::MembershipProposal(proposal) => {
            let proposal = *proposal;
            if application_peer != proposal.proposed.authority_peer_id {
                return Err(anyhow!("membership proposal sender is not its signed authority"));
            }
            verify_membership_signature(&proposal.previous)?;
            verify_membership_signature(&proposal.proposed)?;
            validate_membership_proposal_shape(&proposal)?;
            validate_membership_proposal_for_local(storage, identity, &proposal)?;
            let vote = sign_membership_vote(identity, &proposal)?;
            let durable_vote = match storage.promise_membership_proposal(&proposal, &vote)? {
                MembershipPromiseResult::Accepted => vote,
                MembershipPromiseResult::Idempotent => {
                    storage.load_membership_promise(proposal.proposed.world_id)?.vote
                }
                MembershipPromiseResult::Rejected => {
                    return Err(anyhow!("membership proposal conflicts with this peer's durable prepare"))
                }
            };
            clear_permit(context.paths, proposal.proposed.world_id)?;
            state.leases.permit_heartbeats.remove(&proposal.proposed.world_id);
            node.respond(channel, WireResponse::MembershipVote(Box::new(durable_vote)))?;
        }
        WireRequest::MembershipCommit(certificate) => {
            let certificate = *certificate;
            let world = certificate.proposal.proposed.world_id;
            if application_peer != certificate.proposal.proposed.authority_peer_id {
                return Err(anyhow!("membership commit sender is not its signed authority"));
            }
            validate_membership_certificate_for_local(storage, identity, &certificate)?;
            storage.save_membership_certificate(&certificate)?;
            apply_membership_certificate(storage, &certificate)?;
            clear_permit(context.paths, world)?;
            state.leases.permit_heartbeats.remove(&world);
            state.leases.membership_votes.retain(|(vote_world, _), _| *vote_world != world);
            node.respond(
                channel,
                WireResponse::MembershipCommitAccepted { sequence: certificate.proposal.proposed.sequence },
            )?;
        }
        WireRequest::Membership(record) => {
            verify_membership_signature(&record)?;
            authorize_member(storage, record.world_id, record.authority_peer_id)?;
            if let Ok(epoch) = storage.load_epoch_record(record.world_id) {
                if record.epoch > epoch.epoch_number {
                    return Err(anyhow!("membership cannot advance before its authority epoch is accepted"));
                }
                if record.epoch == epoch.epoch_number
                    && (record.authority_peer_id != epoch.authority_peer_id
                        || record.authority_public_key != epoch.authority_public_key)
                {
                    return Err(anyhow!("membership authority does not match the accepted epoch"));
                }
            }
            if let Ok(current) = storage.load_membership_record(record.world_id) {
                if record == current {
                    node.respond(channel, WireResponse::MembershipAccepted { sequence: record.sequence })?;
                    return Ok(());
                }
                if record.members != current.members {
                    return Err(anyhow!("membership voter-set changes require a joint membership certificate"));
                }
                if record.previous_membership_hash != Some(current.record_hash()?)
                    || record.sequence
                        != current.sequence.checked_add(1).context("membership sequence counter exhausted")?
                {
                    return Err(anyhow!("same-voter membership record must directly extend the committed membership"));
                }
                if record.epoch < current.epoch {
                    return Err(anyhow!("stale membership record rejected"));
                }
            } else if record.sequence != 1 || record.previous_membership_hash.is_some() {
                return Err(anyhow!("non-genesis membership requires a joint membership certificate"));
            }
            let mut descriptor = storage.load_world_descriptor(record.world_id)?;
            descriptor.members = record.members.clone();
            descriptor.normalize();
            storage.save_membership_record(&record)?;
            storage.save_world_descriptor(&descriptor)?;
            clear_satisfied_pending_membership(storage, &descriptor)?;
            node.respond(channel, WireResponse::MembershipAccepted { sequence: record.sequence })?;
        }
        WireRequest::WorldConfig(config) => {
            let config = *config;
            verify_world_config_signature(&config)?;
            let metadata = storage.load_world(config.world_id)?;
            let descriptor = storage.load_world_descriptor(config.world_id)?;
            let fingerprint = config.compatibility_fingerprint()?;
            if fingerprint != metadata.genesis.compatibility_fingerprint
                || fingerprint != descriptor.compatibility_fingerprint
            {
                return Err(anyhow!("world config compatibility fingerprint does not match canonical genesis"));
            }
            authorize_member(storage, config.world_id, application_peer)?;
            authorize_member(storage, config.world_id, config.authority_peer_id)?;
            if application_peer != config.authority_peer_id {
                return Err(anyhow!("world config must be sent by its signed authority"));
            }
            authorize_world_config_current_authority(storage, application_peer, &config)?;
            storage.save_world_config(&config)?;
            node.respond(channel, WireResponse::WorldConfigAccepted { sequence: config.sequence })?;
        }
        WireRequest::SoloBranch(branch) => {
            let branch = *branch;
            verify_solo_branch_signature(&branch)?;
            authorize_member(storage, branch.world_id, application_peer)?;
            authorize_member(storage, branch.world_id, branch.authority_peer_id)?;
            if application_peer != branch.authority_peer_id {
                return Err(anyhow!("solo branch must be sent by its signed authority"));
            }
            if let Ok(local) = storage.load_solo_branch(branch.world_id) {
                match reconcile_solo_history(&local, &branch)? {
                    SoloReconciliation::Equivalent | SoloReconciliation::KeepLocal => {}
                    SoloReconciliation::AdoptRemote => storage.save_solo_branch(&branch)?,
                    SoloReconciliation::Conflict => {
                        storage.preserve_solo_conflict(&local)?;
                        storage.preserve_solo_conflict(&branch)?;
                        node.respond(
                            channel,
                            WireResponse::Error {
                                code: "SOLO_HISTORY_CONFLICT".into(),
                                message: "independently advanced solo histories were preserved; manual resolution is required".into(),
                            },
                        )?;
                        return Ok(());
                    }
                }
            } else {
                storage.save_solo_branch(&branch)?;
            }
            node.respond(channel, WireResponse::SoloBranchAccepted)?;
        }
        WireRequest::RecoveryBallot(ballot) => {
            let ballot = *ballot;
            ensure_no_membership_prepare(storage, ballot.world_id)?;
            if application_peer != ballot.candidate_peer_id {
                return Err(anyhow!("recovery ballot sender is not the signed candidate"));
            }
            verify_recovery_ballot_signature(&ballot)?;
            let descriptor = storage.load_world_descriptor(ballot.world_id)?;
            let candidate =
                descriptor.member(ballot.candidate_peer_id).context("recovery candidate is not a member")?;
            if candidate.banned || !candidate.authority_eligible || candidate.public_key != ballot.candidate_public_key
            {
                return Err(anyhow!(
                    "recovery candidate is not authority eligible or its key does not match membership"
                ));
            }
            let current = storage.load_epoch_record(ballot.world_id)?;
            let expected_generation =
                AuthorityGeneration { epoch: current.epoch_number, fencing_token: current.fencing_token }
                    .checked_next()
                    .context("accepted authority generation is exhausted")?;
            if ballot.base_epoch != current.epoch_number
                || ballot.base_fencing_token != current.fencing_token
                || ballot.target_epoch != expected_generation.epoch
                || ballot.target_fencing_token != expected_generation.fencing_token
            {
                return Err(anyhow!("recovery ballot does not target the next accepted generation"));
            }
            if state.leases.authenticated_peers.values().any(|peer| *peer == current.authority_peer_id) {
                return Err(anyhow!("cannot vote for recovery while the accepted authority is connected"));
            }
            let generation = AuthorityGeneration { epoch: current.epoch_number, fencing_token: current.fencing_token };
            if !recovery_window_open(state.leases, ballot.world_id, generation, state.now) {
                return Err(anyhow!("cannot vote for recovery before the accepted authority lease expires"));
            }
            let latest =
                storage.latest_snapshot(ballot.world_id)?.context("recovery ballot has no canonical base snapshot")?;
            if latest.manifest_hash()? != ballot.base_snapshot_hash || latest.state_root != ballot.base_state_hash {
                return Err(anyhow!("recovery ballot canonical base does not match the latest verified snapshot"));
            }
            let membership = storage.load_membership_record(ballot.world_id)?;
            verify_membership_signature(&membership)?;
            if membership.record_hash()? != ballot.membership_hash {
                return Err(anyhow!("recovery ballot membership hash is stale"));
            }
            let mut vote = RecoveryVoteV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: ballot.world_id,
                ballot_hash: ballot.ballot_hash()?,
                base_epoch: ballot.base_epoch,
                target_epoch: ballot.target_epoch,
                round: ballot.round,
                candidate_peer_id: ballot.candidate_peer_id,
                voter_peer_id: identity.peer_id(),
                voter_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            sign_recovery_vote(identity, &mut vote)?;
            match storage.promise_recovery_ballot(&ballot, &vote)? {
                RecoveryPromiseResult::Accepted => node.respond(channel, WireResponse::RecoveryVote(Box::new(vote)))?,
                RecoveryPromiseResult::Idempotent => {
                    let durable = storage.load_recovery_promise(ballot.world_id)?;
                    node.respond(channel, WireResponse::RecoveryVote(Box::new(durable.vote)))?;
                }
                RecoveryPromiseResult::Rejected { highest_round } => node.respond(
                    channel,
                    WireResponse::RecoveryRejected {
                        highest_round,
                        reason: "durable recovery promise rejects stale or conflicting ballot".into(),
                    },
                )?,
            }
        }
        WireRequest::RecoveryEpoch { record, certificate } => {
            let certificate = *certificate;
            ensure_no_membership_prepare(storage, record.world_id)?;
            validate_recovery_epoch(storage, application_peer, &record, &certificate)?;
            if let Ok(current) = storage.load_epoch_record(record.world_id) {
                if record == current {
                    storage.save_recovery_certificate(&certificate)?;
                    let _ = storage.clear_recovery_promise_after_epoch_advance(record.world_id, record.epoch_number)?;
                    node.respond(
                        channel,
                        WireResponse::EpochAccepted { epoch: record.epoch_number, fencing_token: record.fencing_token },
                    )?;
                    return Ok(());
                }
                let expected_generation =
                    AuthorityGeneration { epoch: current.epoch_number, fencing_token: current.fencing_token }
                        .checked_next()
                        .context("accepted authority generation is exhausted")?;
                if record.epoch_number != expected_generation.epoch
                    || record.fencing_token != expected_generation.fencing_token
                    || record.previous_epoch_hash != Some(epoch_record_hash(&current)?)
                {
                    return Err(anyhow!("certified recovery epoch does not directly extend the accepted epoch"));
                }
            }
            if let Ok(promise) = storage.load_recovery_promise(record.world_id) {
                if promise.ballot.round > certificate.ballot.round {
                    return Err(anyhow!("certified recovery epoch is stale relative to a newer durable promise"));
                }
                if promise.ballot.round == certificate.ballot.round
                    && promise.ballot.ballot_hash()? != certificate.ballot.ballot_hash()?
                {
                    return Err(anyhow!(
                        "certified recovery epoch conflicts with this peer's durable same-round promise"
                    ));
                }
            }
            storage.save_recovery_certificate(&certificate)?;
            storage.save_epoch_record(&record)?;
            storage.clear_sleep_record(record.world_id)?;
            let _ = storage.clear_recovery_promise_after_epoch_advance(record.world_id, record.epoch_number)?;
            state.leases.inbound_leases.remove(&record.world_id);
            node.respond(
                channel,
                WireResponse::EpochAccepted { epoch: record.epoch_number, fencing_token: record.fencing_token },
            )?;
        }
        WireRequest::Epoch(record) => {
            ensure_no_membership_prepare(storage, record.world_id)?;
            if record.mode == EpochMode::Recovery {
                return Err(anyhow!("recovery epoch requires a durable quorum certificate"));
            }
            authorize_epoch(storage, application_peer, &record)?;
            if let Ok(current) = storage.load_epoch_record(record.world_id) {
                if record == current {
                    node.respond(
                        channel,
                        WireResponse::EpochAccepted { epoch: record.epoch_number, fencing_token: record.fencing_token },
                    )?;
                    return Ok(());
                }
                let expected_generation =
                    AuthorityGeneration { epoch: current.epoch_number, fencing_token: current.fencing_token }
                        .checked_next()
                        .context("accepted authority generation is exhausted")?;
                if record.epoch_number != expected_generation.epoch
                    || record.fencing_token != expected_generation.fencing_token
                    || record.previous_epoch_hash != Some(epoch_record_hash(&current)?)
                {
                    return Err(anyhow!("epoch and fencing token must advance exactly once from the accepted epoch"));
                }
                validate_non_recovery_epoch_transition(storage, &current, &record)?;
            } else if let Some(latest) = storage.latest_snapshot(record.world_id)? {
                if record.epoch_number < latest.epoch || record.fencing_token == 0 {
                    return Err(anyhow!("epoch record is older than the local canonical snapshot"));
                }
            }
            storage.save_epoch_record(&record)?;
            storage.clear_sleep_record(record.world_id)?;
            node.respond(
                channel,
                WireResponse::EpochAccepted { epoch: record.epoch_number, fencing_token: record.fencing_token },
            )?;
        }
        WireRequest::AuthorityTransfer(transfer) => {
            ensure_no_membership_prepare(storage, transfer.world_id)?;
            verify_transfer_signature(&transfer)?;
            authorize_member(storage, transfer.world_id, transfer.signer_peer_id)?;
            validate_transfer(storage, &transfer)?;
            storage.save_transfer_record(&transfer)?;
            node.respond(channel, WireResponse::TransferAccepted)?;
        }
        WireRequest::LeaseGrant(lease) => {
            ensure_no_membership_prepare(storage, lease.world_id)?;
            verify_lease_signature(&lease)?;
            if application_peer != lease.authority_peer_id {
                return Err(anyhow!("lease sender is not the signed authority"));
            }
            authorize_member(storage, lease.world_id, lease.authority_peer_id)?;
            let descriptor = storage.load_world_descriptor(lease.world_id)?;
            let member = descriptor.member(lease.authority_peer_id).context("lease authority is not a member")?;
            if !member.authority_eligible || member.banned || member.public_key != lease.authority_public_key {
                return Err(anyhow!("lease authority is not eligible or key does not match membership"));
            }
            if lease.lease_duration_ms != AUTHORITY_LEASE_DURATION_MS {
                return Err(anyhow!("lease duration does not match the authority policy"));
            }
            let epoch = storage.load_epoch_record(lease.world_id)?;
            let current_generation =
                AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token };
            let received_generation = AuthorityGeneration { epoch: lease.epoch, fencing_token: lease.fencing_token };
            if received_generation != current_generation {
                return Err(anyhow!("future authority generations require a recovery ballot, not a lease reservation"));
            }
            if lease.authority_peer_id != epoch.authority_peer_id
                || lease.authority_public_key != epoch.authority_public_key
            {
                return Err(anyhow!("lease does not match the accepted authority epoch"));
            }
            let expires_at = state.now + Duration::from_millis(lease.lease_duration_ms);
            state
                .leases
                .inbound_leases
                .insert(lease.world_id, InboundLease { generation: current_generation, expires_at });
            state.leases.recovery_not_before.insert(lease.world_id, expires_at + RECOVERY_SETTLE_DELAY);
            node.respond(
                channel,
                WireResponse::LeaseAccepted { epoch: lease.epoch, fencing_token: lease.fencing_token },
            )?;
        }
        WireRequest::Sleep(record) => {
            ensure_no_membership_prepare(storage, record.world_id)?;
            if application_peer != record.authority_peer_id {
                return Err(anyhow!("sleep sender is not the signed authority"));
            }
            verify_sleep_record_signature(&record)?;
            authorize_member(storage, record.world_id, record.authority_peer_id)?;
            let descriptor = storage.load_world_descriptor(record.world_id)?;
            let member = descriptor.member(record.authority_peer_id).context("sleep authority is not a member")?;
            if member.public_key != record.authority_public_key || !member.authority_eligible || member.banned {
                return Err(anyhow!("sleep authority is not eligible or key does not match membership"));
            }
            let epoch = storage.load_epoch_record(record.world_id)?;
            if record.epoch != epoch.epoch_number
                || record.fencing_token != epoch.fencing_token
                || record.authority_peer_id != epoch.authority_peer_id
            {
                return Err(anyhow!("sleep record does not match the accepted authority generation"));
            }
            let latest =
                storage.latest_snapshot(record.world_id)?.context("cannot sleep a world without a snapshot")?;
            if latest.manifest_hash()? != record.latest_snapshot_hash {
                return Err(anyhow!("sleep record does not reference the exact latest snapshot"));
            }
            storage.save_sleep_record(&record)?;
            node.respond(
                channel,
                WireResponse::SleepAccepted { epoch: record.epoch, fencing_token: record.fencing_token },
            )?;
        }
        WireRequest::DiscoveryPublic { .. }
        | WireRequest::DiscoveryResolve { .. }
        | WireRequest::FriendPresence { .. } => {
            node.respond(
                channel,
                WireResponse::Error {
                    code: "DISCOVERY_ENDPOINT_REQUIRED".into(),
                    message: "discovery requests are handled by the discovery service".into(),
                },
            )?;
        }
        WireRequest::Hello(_) | WireRequest::HelloChallenge { .. } | WireRequest::HelloProof(_) => {
            return Err(anyhow!("application handshake requests are handled by the network authentication layer"));
        }
    }
    Ok(())
}

fn handle_response(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    context: Option<OutboundContext>,
    response: WireResponse,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
) -> Result<()> {
    let now = Instant::now();
    match (context, response) {
        (
            Some(OutboundContext::Manifest { world, snapshot_number }),
            WireResponse::ManifestAccepted { snapshot_number: accepted, missing },
        ) => {
            if accepted != snapshot_number {
                return Err(anyhow!("manifest response snapshot number mismatch"));
            }
            let manifest = storage.load_snapshot(world, snapshot_number)?;
            for resume in missing {
                let descriptor = find_descriptor(&manifest, resume.hash)
                    .context("peer requested blob not referenced by manifest")?;
                let mut offset = resume.offset;
                loop {
                    let (data, finished) =
                        storage.read_encoded_blob_chunk(world, descriptor, offset, MAX_BLOB_CHUNK)?;
                    let chunk_len = data.len() as u64;
                    node.send_request(
                        transport_peer,
                        WireRequest::BlobChunk {
                            world_id: world,
                            hash: descriptor.hash,
                            encoding: descriptor.encoding,
                            offset,
                            data,
                            finished,
                        },
                    )?;
                    if finished {
                        break;
                    }
                    offset = offset.saturating_add(chunk_len);
                }
            }
        }
        (
            Some(OutboundContext::Lease { world, peer, generation }),
            WireResponse::LeaseAccepted { epoch, fencing_token },
        ) => {
            validate_generation_response(generation, epoch, fencing_token, "lease")?;
            runtime.lease_acks.insert((world, peer), LeaseAck { generation, observed_at: now });
            if let Ok(epoch_record) = storage.load_epoch_record(world) {
                if epoch_record.mode == EpochMode::Recovery
                    && epoch_record.epoch_number == generation.epoch
                    && epoch_record.fencing_token == generation.fencing_token
                    && runtime.recovery_replication_sent.insert((world, peer))
                {
                    if let Ok(membership) = storage.load_membership_record(world) {
                        node.send_request(transport_peer, WireRequest::Membership(membership))?;
                    }
                    if let Some(manifest) = storage.latest_snapshot(world)? {
                        if manifest.epoch == generation.epoch {
                            node.send_request(transport_peer, WireRequest::SnapshotManifest(manifest))?;
                        }
                    }
                }
            }
        }
        (Some(OutboundContext::RecoveryBallot { world, peer, ballot_hash }), WireResponse::RecoveryVote(vote)) => {
            let vote = *vote;
            verify_recovery_vote_signature(&vote)?;
            let Some(ballot) = runtime.recovery_ballots.get(&world) else {
                return Ok(());
            };
            if ballot.ballot_hash()? != ballot_hash || !vote.matches_ballot(ballot)? || vote.voter_peer_id != peer {
                return Err(anyhow!("recovery vote does not match the active ballot or authenticated peer"));
            }
            runtime.recovery_votes.insert((world, peer), vote);
        }
        (
            Some(OutboundContext::RecoveryBallot { world, ballot_hash, .. }),
            WireResponse::RecoveryRejected { highest_round, reason },
        ) => {
            warn!(world = %world, %highest_round, %reason, "peer rejected recovery ballot");
            let active_round = runtime
                .recovery_ballots
                .get(&world)
                .filter(|ballot| ballot.ballot_hash().ok() == Some(ballot_hash))
                .map_or(0, |ballot| ballot.round);
            if highest_round >= active_round {
                runtime
                    .recovery_round_floor
                    .entry(world)
                    .and_modify(|round| *round = (*round).max(highest_round))
                    .or_insert(highest_round);
                runtime.recovery_ballots.remove(&world);
                runtime.recovery_votes.retain(|(vote_world, _), _| *vote_world != world);
            }
        }
        (
            Some(OutboundContext::MembershipProposal { world, peer, proposal_hash }),
            WireResponse::MembershipVote(vote),
        ) => {
            let vote = *vote;
            verify_membership_vote_signature(&vote)?;
            let Ok(promise) = storage.load_membership_promise(world) else {
                // A certificate may have committed and cleared the durable prepare
                // while this response was in flight. Late votes are then stale, not fatal.
                return Ok(());
            };
            if promise.proposal.proposal_hash()? != proposal_hash
                || !vote.matches_proposal(&promise.proposal)?
                || vote.voter_peer_id != peer
            {
                return Err(anyhow!("membership vote does not match the active proposal or authenticated peer"));
            }
            runtime.membership_votes.insert((world, peer), vote);
        }
        (
            Some(OutboundContext::MembershipCommit { world, peer, sequence }),
            WireResponse::MembershipCommitAccepted { sequence: accepted },
        ) => {
            if accepted != sequence {
                return Err(anyhow!("membership commit acknowledgement sequence mismatch"));
            }
            let descriptor = storage.load_world_descriptor(world)?;
            if descriptor.member(peer).is_some_and(|member| !member.banned) {
                push_committed_world_payload(storage, node, transport_peer, world, outbound, false)?;
            }
            info!(world = %world, peer = %peer, sequence, "peer acknowledged committed membership configuration");
        }
        (
            Some(OutboundContext::Epoch { world, peer, generation }),
            WireResponse::EpochAccepted { epoch, fencing_token },
        ) => {
            validate_generation_response(generation, epoch, fencing_token, "epoch")?;
            runtime.epoch_acks.insert((world, peer), generation);
        }
        (Some(OutboundContext::Status { world, peer }), WireResponse::WorldStatus(Some(status))) => {
            if status.world_id != world {
                return Err(anyhow!("world status response references the wrong world"));
            }
            runtime.peer_status.insert((world, peer), ObservedStatus { status, observed_at: now });
        }
        (Some(OutboundContext::Status { world, peer }), WireResponse::WorldStatus(None)) => {
            runtime.peer_status.remove(&(world, peer));
        }
        (Some(OutboundContext::HostCapability { world, peer }), WireResponse::HostCapability(Some(capability))) => {
            if capability.world_id != world {
                return Err(anyhow!("host-capability response references the wrong world"));
            }
            runtime.peer_capability.insert((world, peer), ObservedCapability { capability, observed_at: now });
        }
        (Some(OutboundContext::HostCapability { world, peer }), WireResponse::HostCapability(None)) => {
            runtime.peer_capability.remove(&(world, peer));
        }
        (_, WireResponse::Error { code, message }) => warn!(%code, %message, "peer rejected request"),
        _ => {}
    }
    Ok(())
}

fn sign_membership_vote(identity: &PeerIdentity, proposal: &MembershipProposalV1) -> Result<MembershipVoteV1> {
    let mut vote = membership_vote_for(proposal, identity.peer_id(), identity.public_key())?;
    vote.signature = identity.sign(&vote.signing_bytes()?);
    Ok(vote)
}

fn verify_membership_vote_signature(vote: &MembershipVoteV1) -> Result<()> {
    verify_signature(vote.voter_peer_id, vote.voter_public_key, &vote.signing_bytes()?, &vote.signature)?;
    Ok(())
}

fn ensure_no_membership_prepare(storage: &Storage, world: WorldId) -> Result<()> {
    if storage.load_membership_promise(world).is_ok() {
        return Err(anyhow!("authority/recovery transition is fenced by a durable membership prepare"));
    }
    Ok(())
}

fn proposal_member(proposal: &MembershipProposalV1, peer: PeerId) -> Option<&swarm_protocol::WorldMemberV1> {
    proposal
        .previous
        .members
        .iter()
        .chain(proposal.proposed.members.iter())
        .find(|member| member.peer_id == peer && !member.banned)
}

fn validate_membership_proposal_for_local(
    storage: &Storage,
    identity: &PeerIdentity,
    proposal: &MembershipProposalV1,
) -> Result<()> {
    let world = proposal.proposed.world_id;
    let local_member = proposal_member(proposal, identity.peer_id())
        .context("local peer is not an active voter in either membership configuration")?;
    if local_member.public_key != identity.public_key() {
        return Err(anyhow!("local peer key does not match the membership proposal"));
    }
    if let Ok(current) = storage.load_membership_record(world) {
        verify_membership_signature(&current)?;
        if current.record_hash()? != proposal.previous.record_hash()? {
            return Err(anyhow!("membership proposal does not extend the locally committed configuration"));
        }
        return Ok(());
    }
    let join = storage
        .load_pending_join(world)
        .context("new membership voter has neither committed membership nor a pending join")?;
    verify_join_request_signature(&join)?;
    verify_invite_signature(&join.invite)?;
    if join.invite.expires_unix_ms < unix_millis()? {
        return Err(anyhow!("pending join invite expired before membership commit"));
    }
    if join.joining_member.peer_id != identity.peer_id()
        || join.joining_member.public_key != identity.public_key()
        || proposal.proposed.members.iter().find(|m| m.peer_id == identity.peer_id()) != Some(&join.joining_member)
        || join.invite.inviter_peer_id != proposal.previous.authority_peer_id
        || join.invite.inviter_public_key != proposal.previous.authority_public_key
    {
        return Err(anyhow!("membership proposal does not match the locally staged join"));
    }
    Ok(())
}

fn validate_membership_certificate_signatures(certificate: &MembershipCertificateV1) -> Result<()> {
    verify_membership_signature(&certificate.proposal.previous)?;
    verify_membership_signature(&certificate.proposal.proposed)?;
    validate_membership_certificate_shape(certificate)?;
    for vote in &certificate.votes {
        verify_membership_vote_signature(vote)?;
    }
    Ok(())
}

fn validate_membership_certificate_for_local(
    storage: &Storage,
    identity: &PeerIdentity,
    certificate: &MembershipCertificateV1,
) -> Result<()> {
    validate_membership_certificate_signatures(certificate)?;
    let proposal = &certificate.proposal;
    let world = proposal.proposed.world_id;
    if let Ok(current) = storage.load_membership_record(world) {
        verify_membership_signature(&current)?;
        if current != proposal.proposed && current.record_hash()? != proposal.previous.record_hash()? {
            return Err(anyhow!("membership certificate does not extend the locally committed configuration"));
        }
    } else {
        validate_membership_proposal_for_local(storage, identity, proposal)?;
    }
    if let Ok(promise) = storage.load_membership_promise(world) {
        if promise.proposal.proposal_hash()? != proposal.proposal_hash()? {
            return Err(anyhow!("membership certificate conflicts with this peer's durable prepare"));
        }
    }
    Ok(())
}

fn clear_satisfied_pending_membership(storage: &Storage, descriptor: &WorldDescriptorV1) -> Result<()> {
    if let Ok(join) = storage.load_pending_join(descriptor.world_id) {
        if descriptor
            .member(join.joining_member.peer_id)
            .is_some_and(|m| m.public_key == join.joining_member.public_key && !m.banned)
        {
            storage.clear_pending_join(descriptor.world_id)?;
        }
    }
    if let Ok(leave) = storage.load_pending_leave(descriptor.world_id) {
        if descriptor.member(leave.leaving_peer_id).is_none() {
            storage.clear_pending_leave(descriptor.world_id)?;
        }
    }
    Ok(())
}

fn apply_membership_certificate(storage: &Storage, certificate: &MembershipCertificateV1) -> Result<()> {
    validate_membership_certificate_signatures(certificate)?;
    let proposal = &certificate.proposal;
    let world = proposal.proposed.world_id;
    if let Ok(current) = storage.load_membership_record(world) {
        if current != proposal.proposed && current.record_hash()? != proposal.previous.record_hash()? {
            return Err(anyhow!("cannot apply membership certificate over an unrelated committed configuration"));
        }
    } else {
        let join = storage
            .load_pending_join(world)
            .context("cannot bootstrap non-genesis membership without a pending join")?;
        if proposal.proposed.members.iter().find(|m| m.peer_id == join.joining_member.peer_id)
            != Some(&join.joining_member)
        {
            return Err(anyhow!("membership certificate does not contain the locally pending join"));
        }
    }
    let mut descriptor = storage.load_world_descriptor(world)?;
    descriptor.members = proposal.proposed.members.clone();
    descriptor.normalize();
    storage.save_membership_record(&proposal.proposed)?;
    storage.save_world_descriptor(&descriptor)?;
    let _ = storage.clear_membership_promise_after_commit(world, proposal.proposed.record_hash()?)?;
    clear_satisfied_pending_membership(storage, &descriptor)?;
    Ok(())
}

fn recover_committed_membership(storage: &Storage, identity: &PeerIdentity, world: WorldId) -> Result<()> {
    let Ok(certificate) = storage.load_membership_certificate(world) else {
        return Ok(());
    };
    if let Ok(current) = storage.load_membership_record(world) {
        verify_membership_signature(&current)?;
        if current == certificate.proposal.proposed {
            let _ = storage.clear_membership_promise_after_commit(world, current.record_hash()?)?;
            return Ok(());
        }
        if (current.epoch, current.sequence)
            > (certificate.proposal.proposed.epoch, certificate.proposal.proposed.sequence)
        {
            // Membership certificates are durable historical evidence. A later same-world
            // membership generation (for example recovery authority promotion) supersedes
            // this certificate and must not be rolled back on every daemon tick.
            return Ok(());
        }
    }
    validate_membership_certificate_for_local(storage, identity, &certificate)?;
    apply_membership_certificate(storage, &certificate)
}

fn membership_proposal_request_pending(
    outbound: &HashMap<String, OutboundContext>,
    world: WorldId,
    peer: PeerId,
    proposal_hash: Hash32,
) -> bool {
    outbound.values().any(|context| {
        matches!(context,
        OutboundContext::MembershipProposal { world: w, peer: p, proposal_hash: h }
        if *w == world && *p == peer && *h == proposal_hash)
    })
}

fn maintain_membership_transition(
    storage: &Storage,
    identity: &PeerIdentity,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
    promise: &DurableMembershipPromiseV1,
) -> Result<()> {
    let proposal = &promise.proposal;
    let world = proposal.proposed.world_id;
    if proposal.proposed.authority_peer_id != identity.peer_id()
        || proposal.proposed.authority_public_key != identity.public_key()
    {
        return Ok(());
    }
    let current = storage.load_membership_record(world)?;
    if current == proposal.proposed {
        let _ = storage.clear_membership_promise_after_commit(world, current.record_hash()?)?;
        return Ok(());
    }
    if current.record_hash()? != proposal.previous.record_hash()? {
        return Err(anyhow!("durable membership proposal no longer extends the committed membership"));
    }
    verify_membership_vote_signature(&promise.vote)?;
    runtime.membership_votes.insert((world, identity.peer_id()), promise.vote.clone());
    let proposal_hash = proposal.proposal_hash()?;
    let mut sent = HashSet::new();
    for member in proposal.previous.members.iter().chain(proposal.proposed.members.iter()) {
        if member.banned || member.peer_id == identity.peer_id() || !sent.insert(member.peer_id) {
            continue;
        }
        let Some((transport_peer, _)) =
            runtime.authenticated_peers.iter().find(|(_, application_peer)| **application_peer == member.peer_id)
        else {
            continue;
        };
        if membership_proposal_request_pending(outbound, world, member.peer_id, proposal_hash) {
            continue;
        }
        let id = node.send_request(transport_peer, WireRequest::MembershipProposal(Box::new(proposal.clone())))?;
        outbound.insert(
            request_key(&id),
            OutboundContext::MembershipProposal { world, peer: member.peer_id, proposal_hash },
        );
    }
    let votes = runtime
        .membership_votes
        .iter()
        .filter_map(|((w, _), vote)| {
            (*w == world && vote.matches_proposal(proposal).ok() == Some(true)).then_some(vote.clone())
        })
        .collect::<Vec<_>>();
    let certificate = MembershipCertificateV1 { proposal: proposal.clone(), votes };
    match validate_membership_certificate_shape(&certificate) {
        Ok(()) => {}
        Err(
            MembershipConsensusError::OldQuorumUnavailable { .. }
            | MembershipConsensusError::NewQuorumUnavailable { .. },
        ) => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    for vote in &certificate.votes {
        verify_membership_vote_signature(vote)?;
    }
    storage.save_membership_certificate(&certificate)?;
    apply_membership_certificate(storage, &certificate)?;
    runtime.membership_votes.retain(|(w, _), _| *w != world);
    let mut notified = HashSet::new();
    for member in certificate.proposal.previous.members.iter().chain(certificate.proposal.proposed.members.iter()) {
        if member.peer_id == identity.peer_id() || !notified.insert(member.peer_id) {
            continue;
        }
        let Some((transport_peer, _)) =
            runtime.authenticated_peers.iter().find(|(_, application_peer)| **application_peer == member.peer_id)
        else {
            continue;
        };
        let id = node.send_request(transport_peer, WireRequest::MembershipCommit(Box::new(certificate.clone())))?;
        outbound.insert(
            request_key(&id),
            OutboundContext::MembershipCommit {
                world,
                peer: member.peer_id,
                sequence: certificate.proposal.proposed.sequence,
            },
        );
    }
    info!(world=%world, sequence=certificate.proposal.proposed.sequence, "joint membership configuration committed");
    Ok(())
}

fn validate_generation_response(
    generation: AuthorityGeneration,
    epoch: u64,
    fencing_token: u64,
    kind: &str,
) -> Result<()> {
    if epoch != generation.epoch || fencing_token != generation.fencing_token {
        return Err(anyhow!("{kind} acknowledgement generation mismatch"));
    }
    Ok(())
}

fn authorize_world_config_current_authority(
    storage: &Storage,
    sender: PeerId,
    config: &swarm_protocol::WorldConfigV1,
) -> Result<()> {
    if sender != config.authority_peer_id {
        return Err(anyhow!("world config sender does not match its signed authority"));
    }
    if let Ok(epoch) = storage.load_epoch_record(config.world_id) {
        if config.authority_peer_id != epoch.authority_peer_id
            || config.authority_public_key != epoch.authority_public_key
        {
            return Err(anyhow!("world config is not signed by the current accepted authority"));
        }
    } else {
        let membership = storage.load_membership_record(config.world_id)?;
        verify_membership_signature(&membership)?;
        if config.authority_peer_id != membership.authority_peer_id
            || config.authority_public_key != membership.authority_public_key
        {
            return Err(anyhow!("world config is not signed by the bootstrap authority"));
        }
    }
    Ok(())
}

fn authorize_manifest(storage: &Storage, sender: PeerId, manifest: &SnapshotManifestV1) -> Result<()> {
    storage.load_world(manifest.world_id)?;
    authorize_member(storage, manifest.world_id, sender)?;
    let descriptor = storage.load_world_descriptor(manifest.world_id)?;
    let authority = descriptor
        .member(manifest.authority_peer_id)
        .context("snapshot authority is not an authorized world member")?;
    if authority.banned || !authority.authority_eligible || authority.public_key != manifest.authority_public_key {
        return Err(anyhow!("snapshot authority is not eligible or public key does not match membership"));
    }
    verify_snapshot_signature(manifest)?;
    storage.validate_replica_history(manifest)?;
    if let Ok(epoch) = storage.load_epoch_record(manifest.world_id) {
        if manifest.epoch != epoch.epoch_number
            || manifest.authority_peer_id != epoch.authority_peer_id
            || manifest.authority_public_key != epoch.authority_public_key
        {
            return Err(anyhow!("snapshot does not belong to the accepted authority epoch"));
        }
    }
    Ok(())
}

fn validate_recovery_epoch(
    storage: &Storage,
    sender: PeerId,
    record: &EpochRecordV1,
    certificate: &RecoveryCertificateV1,
) -> Result<()> {
    if record.mode != EpochMode::Recovery {
        return Err(anyhow!("recovery certificate attached to a non-recovery epoch"));
    }
    authorize_epoch(storage, sender, record)?;
    let ballot = &certificate.ballot;
    verify_recovery_ballot_signature(ballot)?;
    if sender != ballot.candidate_peer_id
        || record.authority_peer_id != ballot.candidate_peer_id
        || record.authority_public_key != ballot.candidate_public_key
        || record.epoch_number != ballot.target_epoch
        || record.fencing_token != ballot.target_fencing_token
        || record.base_state_hash != ballot.base_state_hash
    {
        return Err(anyhow!("recovery epoch does not match its signed ballot"));
    }
    let latest = storage.latest_snapshot(record.world_id)?.context("recovery epoch has no canonical base snapshot")?;
    if latest.manifest_hash()? != ballot.base_snapshot_hash || latest.state_root != ballot.base_state_hash {
        return Err(anyhow!("recovery certificate canonical base does not match local verified history"));
    }
    let membership = storage.load_membership_record(record.world_id)?;
    verify_membership_signature(&membership)?;
    if membership.record_hash()? != ballot.membership_hash {
        return Err(anyhow!("recovery certificate membership hash is stale"));
    }
    let canonical_members =
        membership.members.iter().filter(|member| !member.banned).map(|member| member.peer_id).collect::<Vec<_>>();
    validate_recovery_certificate_shape(certificate, &canonical_members)?;
    for vote in &certificate.votes {
        verify_recovery_vote_signature(vote)?;
    }
    Ok(())
}

fn validate_non_recovery_epoch_transition(
    storage: &Storage,
    current: &EpochRecordV1,
    next: &EpochRecordV1,
) -> Result<()> {
    if next.mode == EpochMode::Recovery {
        return Err(anyhow!("recovery transition requires a quorum certificate"));
    }
    if next.authority_peer_id == current.authority_peer_id {
        if next.authority_public_key != current.authority_public_key {
            return Err(anyhow!("same authority peer cannot change its canonical public key"));
        }
        if next.mode == EpochMode::Solo {
            if !solo_mode_allowed(storage, next.world_id)? {
                return Err(anyhow!("solo advancement is disabled by the signed world configuration"));
            }
            let descriptor = storage.load_world_descriptor(next.world_id)?;
            let member_count = descriptor.members.iter().filter(|member| !member.banned).count();
            if member_count > 1 {
                return Err(anyhow!(
                    "multi-member worlds cannot enter writable solo mode without a committed clean relinquishment"
                ));
            }
        }
        return Ok(());
    }

    let transfer = storage
        .load_transfer_record(next.world_id)
        .context("authority change requires a persisted authority transfer")?;
    if transfer.phase != TransferPhase::Committed
        || transfer.from_peer_id != current.authority_peer_id
        || transfer.to_peer_id != next.authority_peer_id
        || transfer.next_epoch != next.epoch_number
        || transfer.next_fencing_token != next.fencing_token
    {
        return Err(anyhow!("epoch authority change does not match the committed transfer"));
    }
    Ok(())
}

fn authorize_epoch(storage: &Storage, sender: PeerId, record: &EpochRecordV1) -> Result<()> {
    record.validate_semantics()?;
    authorize_member(storage, record.world_id, sender)?;
    authorize_member(storage, record.world_id, record.authority_peer_id)?;
    let descriptor = storage.load_world_descriptor(record.world_id)?;
    let authority = descriptor.member(record.authority_peer_id).context("epoch authority is not a member")?;
    if authority.banned || !authority.authority_eligible || authority.public_key != record.authority_public_key {
        return Err(anyhow!("epoch authority is not eligible or key does not match membership"));
    }
    verify_signature(
        record.authority_peer_id,
        record.authority_public_key,
        &record.signing_bytes()?,
        &record.signature,
    )?;
    let latest = storage
        .latest_snapshot(record.world_id)?
        .context("cannot accept an authority epoch without a base snapshot")?;
    if latest.state_root != record.base_state_hash {
        return Err(anyhow!("epoch base state hash does not match the latest verified snapshot"));
    }
    Ok(())
}

fn validate_transfer(storage: &Storage, transfer: &swarm_protocol::AuthorityTransferV1) -> Result<()> {
    let descriptor = storage.load_world_descriptor(transfer.world_id)?;
    let from = descriptor.member(transfer.from_peer_id).context("transfer source is not a world member")?;
    let to = descriptor.member(transfer.to_peer_id).context("transfer target is not a world member")?;
    if from.banned || to.banned || !to.authority_eligible {
        return Err(anyhow!("transfer participants are banned or target is not authority eligible"));
    }
    let expected_signer = match transfer.phase {
        TransferPhase::Prepared | TransferPhase::Committed => transfer.from_peer_id,
        TransferPhase::Accepted => transfer.to_peer_id,
    };
    if transfer.signer_peer_id != expected_signer {
        return Err(anyhow!("transfer phase was signed by the wrong participant"));
    }
    let signer = descriptor.member(transfer.signer_peer_id).context("transfer signer is not a member")?;
    if signer.public_key != transfer.signer_public_key {
        return Err(anyhow!("transfer signer key does not match membership"));
    }
    let latest = storage.latest_snapshot(transfer.world_id)?.context("cannot transfer authority without a snapshot")?;
    if latest.manifest_hash()? != transfer.base_snapshot_hash {
        return Err(anyhow!("transfer does not reference the exact latest snapshot"));
    }
    if let Ok(epoch) = storage.load_epoch_record(transfer.world_id) {
        let expected = AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token }
            .checked_next()
            .context("accepted authority generation is exhausted")?;
        if transfer.next_epoch != expected.epoch || transfer.next_fencing_token != expected.fencing_token {
            return Err(anyhow!("transfer generation does not advance the accepted epoch exactly once"));
        }
    }
    if let Ok(previous) = storage.load_transfer_record(transfer.world_id) {
        let valid_progression = matches!(
            (previous.phase, transfer.phase),
            (TransferPhase::Prepared, TransferPhase::Accepted) | (TransferPhase::Accepted, TransferPhase::Committed)
        );
        let same_transfer = previous.from_peer_id == transfer.from_peer_id
            && previous.to_peer_id == transfer.to_peer_id
            && previous.base_snapshot_hash == transfer.base_snapshot_hash
            && previous.next_epoch == transfer.next_epoch
            && previous.next_fencing_token == transfer.next_fencing_token;
        if !valid_progression || !same_transfer {
            return Err(anyhow!("authority transfer does not continue the persisted transfer state"));
        }
    } else if transfer.phase != TransferPhase::Prepared {
        return Err(anyhow!("authority transfer must begin in the prepared phase"));
    }
    Ok(())
}

fn authorized_descriptor_member(
    descriptor: &WorldDescriptorV1,
    peer: PeerId,
) -> Result<&swarm_protocol::WorldMemberV1> {
    let member = descriptor.member(peer).context("peer is not an authorized member of this world")?;
    if peer_id_from_public_key(&member.public_key) != peer {
        return Err(anyhow!("world membership public key does not match peer identity"));
    }
    if member.banned {
        return Err(anyhow!("peer is banned from this world"));
    }
    Ok(member)
}

fn authorize_member(storage: &Storage, world: WorldId, peer: PeerId) -> Result<()> {
    let descriptor = storage.load_world_descriptor(world)?;
    authorized_descriptor_member(&descriptor, peer)?;
    Ok(())
}

fn finalize_and_ack(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    manifest: &SnapshotManifestV1,
) -> Result<()> {
    verify_snapshot_signature(manifest)?;
    storage.finalize_replica(manifest)?;
    let ack = ReplicaAckV1 {
        world_id: manifest.world_id,
        snapshot_number: manifest.snapshot_number,
        manifest_hash: manifest.manifest_hash()?,
        state_root: manifest.state_root,
        complete: true,
    };
    node.send_request(transport_peer, WireRequest::ReplicaAck(ack))?;
    Ok(())
}

fn epoch_record_hash(record: &EpochRecordV1) -> Result<Hash32> {
    let encoded = postcard::to_allocvec(record)?;
    Ok(Hash32::from_domain_bytes(b"swarmcraft/epoch-record/v1\0", &encoded))
}

fn world_status(storage: &Storage, world: WorldId, local_peer: PeerId) -> Result<Option<WorldStatusV1>> {
    let Ok(metadata) = storage.load_world(world) else { return Ok(None) };
    let latest = storage.latest_snapshot(world)?;
    let eligible = storage
        .load_world_descriptor(world)
        .ok()
        .and_then(|descriptor| descriptor.member(local_peer).cloned())
        .is_some_and(|member| member.authority_eligible && !member.banned);
    let epoch = storage.load_epoch_record(world).ok();
    Ok(Some(WorldStatusV1 {
        world_id: world,
        epoch: epoch
            .as_ref()
            .map_or_else(|| latest.as_ref().map_or(0, |manifest| manifest.epoch), |record| record.epoch_number),
        sequence: latest.as_ref().map_or(0, |manifest| manifest.sequence),
        latest_snapshot: latest.as_ref().map(|manifest| manifest.manifest_hash()).transpose()?,
        state_hash: latest.as_ref().map(|manifest| manifest.state_root),
        compatibility_fingerprint: metadata.genesis.compatibility_fingerprint,
        authority_eligible: eligible,
    }))
}

fn find_descriptor(manifest: &SnapshotManifestV1, hash: Hash32) -> Option<&BlobDescriptor> {
    manifest.entries.iter().find(|entry| entry.blob.hash == hash).map(|entry| &entry.blob)
}

fn unix_millis() -> Result<u64> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).context("system clock is before Unix epoch")?;
    Ok(duration.as_millis().try_into().unwrap_or(u64::MAX))
}

fn request_key(value: &impl Debug) -> String {
    format!("{value:?}")
}

#[cfg(test)]
mod authorization_matrix_tests {
    use super::*;

    fn member(key: [u8; 32], banned: bool) -> swarm_protocol::WorldMemberV1 {
        swarm_protocol::WorldMemberV1 {
            peer_id: peer_id_from_public_key(&key),
            public_key: key,
            authority_eligible: true,
            banned,
        }
    }

    fn descriptor(members: Vec<swarm_protocol::WorldMemberV1>) -> WorldDescriptorV1 {
        WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([9; 32]),
            compatibility_fingerprint: Hash32([8; 32]),
            members,
            preferred_replication_factor: 2,
        }
    }

    #[test]
    fn current_member_is_authorized_but_stranger_and_removed_member_are_not() {
        let current = member([1; 32], false);
        let current_peer = current.peer_id;
        let stranger_peer = peer_id_from_public_key(&[2; 32]);
        let current_descriptor = descriptor(vec![current]);
        assert!(authorized_descriptor_member(&current_descriptor, current_peer).is_ok());
        assert!(authorized_descriptor_member(&current_descriptor, stranger_peer).is_err());

        let removed_descriptor = descriptor(Vec::new());
        assert!(authorized_descriptor_member(&removed_descriptor, current_peer).is_err());
    }

    #[test]
    fn banned_and_key_mismatched_members_are_not_authorized() {
        let banned = member([3; 32], true);
        let banned_peer = banned.peer_id;
        assert!(authorized_descriptor_member(&descriptor(vec![banned]), banned_peer).is_err());

        let claimed_peer = peer_id_from_public_key(&[4; 32]);
        let mismatched = swarm_protocol::WorldMemberV1 {
            peer_id: claimed_peer,
            public_key: [5; 32],
            authority_eligible: true,
            banned: false,
        };
        assert!(authorized_descriptor_member(&descriptor(vec![mismatched]), claimed_peer).is_err());
    }
}


#[cfg(test)]
#[path = "daemon_protocol_tests.rs"]
mod protocol_acceptance_tests;
