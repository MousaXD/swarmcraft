from pathlib import Path
import re

path = Path('crates/swarm-cli/src/daemon.rs')
text = path.read_text()


def replace(old: str, new: str, label: str) -> None:
    global text
    if old not in text:
        raise SystemExit(f'missing daemon patch anchor: {label}')
    text = text.replace(old, new, 1)

replace(
    'use swarm_consensus::{elect_authority, has_quorum, AuthorityCandidate, AuthorityGeneration};',
    'use swarm_consensus::{\n    elect_authority, has_quorum, reconcile_solo_history, validate_recovery_certificate_shape, AuthorityCandidate,\n    AuthorityGeneration, SoloReconciliation,\n};',
    'consensus imports',
)
replace(
    '    lifecycle::{verify_join_request_signature, verify_leave_request_signature, verify_sleep_record_signature},\n    random_nonce, verify_invite_signature, verify_lease_signature, verify_membership_signature, verify_signature,\n    verify_snapshot_signature, verify_transfer_signature, DataPaths, PeerIdentity,',
    '    lifecycle::{verify_join_request_signature, verify_leave_request_signature, verify_sleep_record_signature},\n    protocol_v2::{\n        sign_recovery_ballot, sign_recovery_vote, verify_recovery_ballot_signature, verify_recovery_vote_signature,\n        verify_solo_branch_signature, verify_world_config_signature,\n    },\n    random_nonce, verify_invite_signature, verify_lease_signature, verify_membership_signature, verify_signature,\n    verify_snapshot_signature, verify_transfer_signature, DataPaths, PeerIdentity,',
    'core imports',
)
replace(
    '    AuthorityLeaseGrantV1, BlobDescriptor, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, PeerId,\n    SnapshotManifestV1, TransferPhase, WorldDescriptorV1, WorldId, WorldStatusV1, PROTOCOL_VERSION,',
    '    AuthorityLeaseGrantV1, BlobDescriptor, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, PeerId,\n    RecoveryBallotV1, RecoveryCertificateV1, RecoveryVoteV1, SnapshotManifestV1, TransferPhase, WorldDescriptorV1,\n    WorldId, WorldStatusV1, PROTOCOL_VERSION,',
    'protocol imports',
)
replace('use swarm_storage::Storage;', 'use swarm_storage::{RecoveryPromiseResult, Storage};', 'storage imports')

replace(
    '    Reservation { world: WorldId, peer: PeerId, generation: AuthorityGeneration },',
    '    RecoveryBallot { world: WorldId, peer: PeerId, ballot_hash: Hash32 },',
    'outbound recovery context',
)
replace(
    '    reservation_acks: HashMap<(WorldId, PeerId), AuthorityGeneration>,',
    '    recovery_ballots: HashMap<WorldId, RecoveryBallotV1>,\n    recovery_votes: HashMap<(WorldId, PeerId), RecoveryVoteV1>,\n    recovery_round_floor: HashMap<WorldId, u64>,',
    'runtime recovery fields',
)
replace(
    '        "relay-dcutr-v1".into(),\n',
    '        "relay-dcutr-v1".into(),\n        "recovery-ballot-v1".into(),\n        "world-config-v1".into(),\n        "solo-history-v1".into(),\n        "background-replica-v1".into(),\n',
    'capabilities',
)
replace(
    '                                OutboundContext::Reservation { world, peer, .. } => {\n                                    leases.reservation_acks.remove(&(world, peer));\n                                }',
    '                                OutboundContext::RecoveryBallot { world, peer, .. } => {\n                                    leases.recovery_votes.remove(&(world, peer));\n                                }',
    'outbound failure recovery',
)
replace(
    '                            leases.reservation_acks.retain(|(_, peer), _| *peer != application_peer);',
    '                            leases.recovery_votes.retain(|(_, peer), _| *peer != application_peer);',
    'disconnect recovery votes',
)

pattern = re.compile(
    r'''        let recovery_generation = AuthorityGeneration \{\n            epoch: generation\.epoch\.saturating_add\(1\),\n            fencing_token: generation\.fencing_token\.saturating_add\(1\),\n        \};\n        let reservation = signed_lease\(identity, world, recovery_generation\)\?;\n        if !storage\.reserve_recovery\(&reservation\)\? \{\n            continue;\n        \}\n\n        for \(transport_peer, application_peer\) in &runtime\.authenticated_peers \{.*?        info!\(world = %world, epoch = next\.epoch_number, peer = %identity\.peer_id\(\), "authority recovered after quorum reservation"\);\n''',
    re.S,
)
replacement = '''        let recovery_generation = AuthorityGeneration {\n            epoch: generation.epoch.saturating_add(1),\n            fencing_token: generation.fencing_token.saturating_add(1),\n        };\n        drive_recovery_ballot(\n            storage,\n            identity,\n            node,\n            outbound,\n            runtime,\n            &descriptor,\n            &epoch,\n            &latest,\n            &visible_peers,\n            recovery_generation,\n        )?;\n'''
text, count = pattern.subn(replacement, text, count=1)
if count != 1:
    raise SystemExit(f'candidate recovery block replacement count={count}')

replace(
    '    context.storage.clear_recovery_reservation(world)?;\n    Ok(())',
    '    context.storage.clear_recovery_reservation(world)?;\n    let _ = context.storage.clear_recovery_promise_after_epoch_advance(world, context.epoch.epoch_number)?;\n    Ok(())',
    'local authority cleanup',
)
replace(
    '    let world = context.descriptor.world_id;\n    for (transport_peer, application_peer) in &runtime.authenticated_peers {',
    '    let world = context.descriptor.world_id;\n    let certificate = context\n        .storage\n        .load_recovery_certificate(world)\n        .context("accepted recovery epoch is missing its durable quorum certificate")?;\n    if certificate.ballot.target_epoch != context.generation.epoch\n        || certificate.ballot.target_fencing_token != context.generation.fencing_token\n        || certificate.ballot.candidate_peer_id != context.identity.peer_id()\n    {\n        return Err(anyhow!("durable recovery certificate does not match the accepted recovery generation"));\n    }\n    for (transport_peer, application_peer) in &runtime.authenticated_peers {',
    'recovery quorum certificate load',
)
replace(
    '        let request_id = node.send_request(transport_peer, WireRequest::Epoch(context.epoch.clone()))?;',
    '        let request_id = node.send_request(\n            transport_peer,\n            WireRequest::RecoveryEpoch { record: context.epoch.clone(), certificate: Box::new(certificate.clone()) },\n        )?;',
    'recovery epoch send certificate',
)

# The status-only catch-up proof was an unsafe substitute for a durable quorum certificate.
text, count = re.subn(
    r'\nfn recovery_catchup_has_quorum\(.*?\n\}\n\nfn promote_recovery_epoch',
    '\nfn promote_recovery_epoch',
    text,
    count=1,
    flags=re.S,
)
if count != 1:
    raise SystemExit(f'recovery catchup removal count={count}')

replace(
    '        reason: "automatic crash recovery after authority lease expiry".into(),',
    '        reason: "automatic crash recovery after durable quorum ballot".into(),',
    'recovery epoch reason',
)
replace(
    '    runtime.reservation_acks.retain(|(ack_world, _), _| *ack_world != world);',
    '    runtime.recovery_ballots.remove(&world);\n    runtime.recovery_votes.retain(|(ack_world, _), _| *ack_world != world);\n    runtime.recovery_round_floor.remove(&world);',
    'clear runtime recovery',
)

old_helper = '''fn reservation_request_pending(
    outbound: &HashMap<String, OutboundContext>,
    world: WorldId,
    peer: PeerId,
    generation: AuthorityGeneration,
) -> bool {
    outbound.values().any(|context| {
        matches!(
            context,
            OutboundContext::Reservation {
                world: request_world,
                peer: request_peer,
                generation: request_generation,
            } if *request_world == world && *request_peer == peer && *request_generation == generation
        )
    })
}
'''
new_helper = '''fn recovery_ballot_request_pending(
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
'''
replace(old_helper, new_helper, 'pending recovery ballot helper')

old_push_epoch = '''        if let Ok(epoch) = storage.load_epoch_record(metadata.world_id) {
            let _ = node.send_request(transport_peer, WireRequest::Epoch(epoch))?;
        }
'''
new_push_epoch = '''        if let Ok(config) = storage.load_world_config(metadata.world_id) {
            let _ = node.send_request(transport_peer, WireRequest::WorldConfig(Box::new(config)))?;
        }
        if let Ok(branch) = storage.load_solo_branch(metadata.world_id) {
            let _ = node.send_request(transport_peer, WireRequest::SoloBranch(Box::new(branch)))?;
        }
        if let Ok(epoch) = storage.load_epoch_record(metadata.world_id) {
            if epoch.mode == EpochMode::Recovery {
                if let Ok(certificate) = storage.load_recovery_certificate(metadata.world_id) {
                    let _ = node.send_request(
                        transport_peer,
                        WireRequest::RecoveryEpoch { record: epoch, certificate: Box::new(certificate) },
                    )?;
                }
            } else {
                let _ = node.send_request(transport_peer, WireRequest::Epoch(epoch))?;
            }
        }
'''
replace(old_push_epoch, new_push_epoch, 'push world v2 control state')

# Add world config and solo reconciliation handlers immediately before epoch handling.
epoch_anchor = '        WireRequest::Epoch(record) => {'
if epoch_anchor not in text:
    raise SystemExit('missing epoch handler anchor')
handlers = '''        WireRequest::WorldConfig(config) => {
            let config = *config;
            verify_world_config_signature(&config)?;
            authorize_member(storage, config.world_id, application_peer)?;
            authorize_member(storage, config.world_id, config.authority_peer_id)?;
            if application_peer != config.authority_peer_id {
                return Err(anyhow!("world config must be sent by its signed authority"));
            }
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
            if application_peer != ballot.candidate_peer_id {
                return Err(anyhow!("recovery ballot sender is not the signed candidate"));
            }
            verify_recovery_ballot_signature(&ballot)?;
            let descriptor = storage.load_world_descriptor(ballot.world_id)?;
            let candidate = descriptor.member(ballot.candidate_peer_id).context("recovery candidate is not a member")?;
            if candidate.banned || !candidate.authority_eligible || candidate.public_key != ballot.candidate_public_key {
                return Err(anyhow!("recovery candidate is not authority eligible or its key does not match membership"));
            }
            let current = storage.load_epoch_record(ballot.world_id)?;
            if ballot.base_epoch != current.epoch_number
                || ballot.base_fencing_token != current.fencing_token
                || ballot.target_epoch != current.epoch_number.saturating_add(1)
                || ballot.target_fencing_token != current.fencing_token.saturating_add(1)
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
            let latest = storage.latest_snapshot(ballot.world_id)?.context("recovery ballot has no canonical base snapshot")?;
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
                if record.epoch_number != current.epoch_number.saturating_add(1)
                    || record.fencing_token != current.fencing_token.saturating_add(1)
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
                    return Err(anyhow!("certified recovery epoch conflicts with this peer's durable same-round promise"));
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
'''
text = text.replace(epoch_anchor, handlers + epoch_anchor, 1)

# Plain Epoch can no longer smuggle a recovery generation without its certificate.
pattern = re.compile(r'        WireRequest::Epoch\(record\) => \{.*?\n        \}\n        WireRequest::AuthorityTransfer', re.S)
plain_epoch = '''        WireRequest::Epoch(record) => {
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
                if record.epoch_number != current.epoch_number.saturating_add(1)
                    || record.fencing_token != current.fencing_token.saturating_add(1)
                {
                    return Err(anyhow!("epoch and fencing token must advance exactly once"));
                }
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
        WireRequest::AuthorityTransfer'''
text, count = pattern.subn(plain_epoch, text, count=1)
if count != 1:
    raise SystemExit(f'plain epoch replacement count={count}')

# Current-generation leases remain heartbeats. Future-generation LeaseGrant reservations are retired.
pattern = re.compile(r'        WireRequest::LeaseGrant\(lease\) => \{.*?\n        \}\n        WireRequest::Sleep', re.S)
lease_handler = '''        WireRequest::LeaseGrant(lease) => {
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
            let current_generation = AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token };
            let received_generation = AuthorityGeneration { epoch: lease.epoch, fencing_token: lease.fencing_token };
            if received_generation != current_generation {
                return Err(anyhow!("future authority generations require a recovery ballot, not a lease reservation"));
            }
            if lease.authority_peer_id != epoch.authority_peer_id || lease.authority_public_key != epoch.authority_public_key {
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
        WireRequest::Sleep'''
text, count = pattern.subn(lease_handler, text, count=1)
if count != 1:
    raise SystemExit(f'lease handler replacement count={count}')

# Replace old reservation response with ballot votes/rejections.
pattern = re.compile(
    r'''        \(\n            Some\(OutboundContext::Reservation \{ world, peer, generation \}\),\n            WireResponse::LeaseAccepted \{ epoch, fencing_token \},\n        \) => \{\n            validate_generation_response\(generation, epoch, fencing_token, "recovery reservation"\)\?;\n            runtime\.reservation_acks\.insert\(\(world, peer\), generation\);\n        \}\n'''
)
recovery_response = '''        (
            Some(OutboundContext::RecoveryBallot { world, peer, ballot_hash }),
            WireResponse::RecoveryVote(vote),
        ) => {
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
                runtime.recovery_round_floor.entry(world).and_modify(|round| *round = (*round).max(highest_round)).or_insert(highest_round);
                runtime.recovery_ballots.remove(&world);
                runtime.recovery_votes.retain(|(vote_world, _), _| *vote_world != world);
            }
        }
'''
text, count = pattern.subn(recovery_response, text, count=1)
if count != 1:
    raise SystemExit(f'recovery response replacement count={count}')

# Insert ballot driver before local-authority maintenance.
anchor = '\nfn maintain_local_authority(\n'
if anchor not in text:
    raise SystemExit('missing maintain_local_authority anchor')
driver = r'''
fn drive_recovery_ballot(
    storage: &Storage,
    identity: &PeerIdentity,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
    descriptor: &WorldDescriptorV1,
    previous: &EpochRecordV1,
    latest: &SnapshotManifestV1,
    visible_peers: &[PeerId],
    recovery_generation: AuthorityGeneration,
) -> Result<()> {
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
        let round = floor.saturating_add(1).max(1);
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

    let members = descriptor
        .members
        .iter()
        .filter(|member| !member.banned)
        .map(|member| member.peer_id)
        .collect::<Vec<_>>();
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
'''
text = text.replace(anchor, driver + anchor, 1)

# Add certified recovery validation before normal epoch authorization.
authorize_anchor = '\nfn authorize_epoch(storage: &Storage, sender: PeerId, record: &EpochRecordV1) -> Result<()> {'
if authorize_anchor not in text:
    raise SystemExit('missing authorize_epoch anchor')
validator = r'''
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
    let canonical_members = membership
        .members
        .iter()
        .filter(|member| !member.banned)
        .map(|member| member.peer_id)
        .collect::<Vec<_>>();
    validate_recovery_certificate_shape(certificate, &canonical_members)?;
    for vote in &certificate.votes {
        verify_recovery_vote_signature(vote)?;
    }
    Ok(())
}
'''
text = text.replace(authorize_anchor, validator + authorize_anchor, 1)

path.write_text(text)
