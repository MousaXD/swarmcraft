from pathlib import Path


def replace(path, old, new, count=1):
    p = Path(path); s = p.read_text(); n = s.count(old)
    if n != count: raise SystemExit(f"{path}: expected {count}, found {n}: {old[:100]!r}")
    p.write_text(s.replace(old, new, count))


def between(path, start, end, new):
    p=Path(path); s=p.read_text(); a=s.find(start); b=s.find(end, a+len(start)) if a>=0 else -1
    if a<0 or b<0: raise SystemExit(f"markers missing in {path}: {start!r} / {end!r}")
    p.write_text(s[:a]+new+s[b:])


path="crates/swarm-cli/src/daemon.rs"
replace(path,
"use swarm_consensus::{\n    elect_authority, has_quorum, reconcile_solo_history, validate_recovery_certificate_shape, AuthorityCandidate,\n    AuthorityGeneration, SoloReconciliation,\n};\n",
"use swarm_consensus::{\n    elect_authority, has_quorum, membership_vote_for, reconcile_solo_history, validate_membership_certificate_shape,\n    validate_membership_proposal_shape, validate_recovery_certificate_shape, AuthorityCandidate, AuthorityGeneration,\n    MembershipConsensusError, SoloReconciliation,\n};\n")
replace(path,
"    AuthorityLeaseGrantV1, BlobDescriptor, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, PeerId,\n    RecoveryBallotV1, RecoveryCertificateV1, RecoveryVoteV1, SnapshotManifestV1, SoloBranchV1, TransferPhase,\n",
"    AuthorityLeaseGrantV1, BlobDescriptor, EpochMode, EpochRecordV1, Hash32, MembershipCertificateV1,\n    MembershipProposalV1, MembershipRecordV1, MembershipVoteV1, PeerId, RecoveryBallotV1, RecoveryCertificateV1,\n    RecoveryVoteV1, SnapshotManifestV1, SoloBranchV1, TransferPhase,\n")
replace(path,"use swarm_storage::{RecoveryPromiseResult, Storage};\n","use swarm_storage::{DurableMembershipPromiseV1, MembershipPromiseResult, RecoveryPromiseResult, Storage};\n")
replace(path,
"    RecoveryBallot { world: WorldId, peer: PeerId, ballot_hash: Hash32 },\n    Epoch { world: WorldId, peer: PeerId, generation: AuthorityGeneration },\n",
"    RecoveryBallot { world: WorldId, peer: PeerId, ballot_hash: Hash32 },\n    MembershipProposal { world: WorldId, peer: PeerId, proposal_hash: Hash32 },\n    MembershipCommit { world: WorldId, peer: PeerId, sequence: u64 },\n    Epoch { world: WorldId, peer: PeerId, generation: AuthorityGeneration },\n")
replace(path,
"    recovery_votes: HashMap<(WorldId, PeerId), RecoveryVoteV1>,\n    recovery_round_floor: HashMap<WorldId, u64>,\n",
"    recovery_votes: HashMap<(WorldId, PeerId), RecoveryVoteV1>,\n    membership_votes: HashMap<(WorldId, PeerId), MembershipVoteV1>,\n    recovery_round_floor: HashMap<WorldId, u64>,\n")
replace(path,'        "membership-leave-v1".into(),\n','        "membership-leave-v1".into(),\n        "membership-joint-consensus-v1".into(),\n')
replace(path,
"                            response,\n                            &mut leases,\n                            Instant::now(),\n",
"                            response,\n                            identity.peer_id(),\n                            &mut outbound,\n                            &mut leases,\n                            Instant::now(),\n")
replace(path,
"                                OutboundContext::RecoveryBallot { world, peer, .. } => {\n                                    leases.recovery_votes.remove(&(world, peer));\n                                }\n                                OutboundContext::Epoch { world, peer, .. } => {\n",
"                                OutboundContext::RecoveryBallot { world, peer, .. } => {\n                                    leases.recovery_votes.remove(&(world, peer));\n                                }\n                                OutboundContext::MembershipProposal { world, peer, .. } => {\n                                    leases.membership_votes.remove(&(world, peer));\n                                }\n                                OutboundContext::MembershipCommit { .. } => {}\n                                OutboundContext::Epoch { world, peer, .. } => {\n")
replace(path,
"                            leases.recovery_votes.retain(|(_, peer), _| *peer != application_peer);\n                            leases.epoch_acks.retain(|(_, peer), _| *peer != application_peer);\n",
"                            leases.recovery_votes.retain(|(_, peer), _| *peer != application_peer);\n                            leases.membership_votes.retain(|(_, peer), _| *peer != application_peer);\n                            leases.epoch_acks.retain(|(_, peer), _| *peer != application_peer);\n")
replace(path,
"        let world = metadata.world_id;\n        if let Err(error) = publish_host_readiness_snapshot(paths, storage, identity, runtime, world, now) {\n",
"        let world = metadata.world_id;\n        recover_committed_membership(storage, identity, world)?;\n        if let Err(error) = publish_host_readiness_snapshot(paths, storage, identity, runtime, world, now) {\n")
replace(path,
"        if storage.load_sleep_record(world).is_ok() {\n            clear_permit(paths, world)?;\n            clear_runtime_world(runtime, world);\n            continue;\n        }\n\n        let Ok(descriptor) = storage.load_world_descriptor(world) else {\n",
"        if storage.load_sleep_record(world).is_ok() {\n            clear_permit(paths, world)?;\n            clear_runtime_world(runtime, world);\n            continue;\n        }\n        if let Ok(promise) = storage.load_membership_promise(world) {\n            clear_permit(paths, world)?;\n            runtime.permit_heartbeats.remove(&world);\n            maintain_membership_transition(storage, identity, node, outbound, runtime, &promise)?;\n            continue;\n        }\n\n        let Ok(descriptor) = storage.load_world_descriptor(world) else {\n")
replace(path,
"    runtime.recovery_votes.retain(|(ack_world, _), _| *ack_world != world);\n    runtime.recovery_round_floor.remove(&world);\n",
"    runtime.recovery_votes.retain(|(ack_world, _), _| *ack_world != world);\n    runtime.membership_votes.retain(|(ack_world, _), _| *ack_world != world);\n    runtime.recovery_round_floor.remove(&world);\n")

between(path,"fn push_known_worlds(\n","fn handle_request(\n",'''fn push_known_worlds(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    application_peer: PeerId,
    local_peer: PeerId,
    outbound: &mut HashMap<String, OutboundContext>,
) -> Result<()> {
    for metadata in storage.list_worlds()? {
        let Ok(descriptor) = storage.load_world_descriptor(metadata.world_id) else { continue };
        if descriptor.member(application_peer).is_none() { continue; }
        let local_is_authority = storage
            .load_epoch_record(metadata.world_id)
            .is_ok_and(|epoch| epoch.authority_peer_id == local_peer);
        if !local_is_authority && !storage.background_seeding_enabled(metadata.world_id)? { continue; }

        if let Ok(membership) = storage.load_membership_record(metadata.world_id) {
            if let Ok(certificate) = storage.load_membership_certificate(metadata.world_id) {
                if certificate.proposal.proposed.record_hash()? == membership.record_hash()? {
                    let id = node.send_request(transport_peer, WireRequest::MembershipCommit(Box::new(certificate)))?;
                    outbound.insert(request_key(&id), OutboundContext::MembershipCommit {
                        world: metadata.world_id,
                        peer: application_peer,
                        sequence: membership.sequence,
                    });
                    continue;
                }
            }
        }
        push_committed_world_payload(storage, node, transport_peer, metadata.world_id, outbound, true)?;
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
                node.send_request(transport_peer, WireRequest::RecoveryEpoch { record: epoch, certificate: Box::new(certificate) })?;
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
        outbound.insert(request_key(&id), OutboundContext::Manifest { world, snapshot_number: manifest.snapshot_number });
    }
    Ok(())
}

''')
