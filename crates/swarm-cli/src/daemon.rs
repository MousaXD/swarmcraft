use anyhow::{anyhow, Context, Result};
use std::{
    collections::HashMap,
    fmt::Debug,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::{
    lifecycle::{verify_join_request_signature, verify_sleep_record_signature},
    verify_invite_signature, verify_lease_signature, verify_membership_signature, verify_signature, verify_snapshot_signature,
    verify_transfer_signature, DataPaths, PeerIdentity,
};
use swarm_network::{
    load_or_create_transport_key, BlobResumeV1, NetworkEvent, ReplicaAckV1, ResponseChannel, SwarmNode,
    TransportPeerId, WireRequest, WireResponse, MAX_BLOB_CHUNK,
};
use swarm_protocol::{
    BlobDescriptor, EpochRecordV1, Hash32, MembershipRecordV1, PeerId, SnapshotManifestV1, TransferPhase, WorldId,
    WorldStatusV1,
};
use swarm_storage::Storage;
use tracing::{info, warn};

#[derive(Debug, Clone)]
enum OutboundContext {
    Manifest { world: WorldId, snapshot_number: u64 },
}

pub async fn run(paths: &DataPaths, storage: &Storage, listen: &str) -> Result<()> {
    let identity = PeerIdentity::load_or_create(paths)?;
    let transport_key = load_or_create_transport_key(&paths.transport_key())?;
    let hello = identity.signed_peer_hello(vec![
        "snapshot-replication-v1".into(),
        "membership-v1".into(),
        "authority-transfer-v1".into(),
        "authority-lease-v1".into(),
        "epoch-v1".into(),
        "sleep-wake-v1".into(),
    ])?;
    let mut node = SwarmNode::new(transport_key, hello)?;
    node.listen(listen.parse().context("invalid QUIC listen multiaddress")?)?;
    info!(peer = %identity.peer_id(), %listen, "SwarmCraft daemon starting");

    let mut pending_manifests: HashMap<WorldId, SnapshotManifestV1> = HashMap::new();
    let mut outbound: HashMap<String, OutboundContext> = HashMap::new();

    loop {
        match node.next_event().await? {
            NetworkEvent::Listening { address } => info!(%address, "daemon listening"),
            NetworkEvent::Authenticated { transport_peer, application_peer } => {
                info!(transport = %transport_peer, peer = %application_peer, "peer authenticated");
                push_known_worlds(storage, &mut node, &transport_peer, application_peer, &mut outbound)?;
            }
            NetworkEvent::InboundRequest { transport_peer, request, channel } => {
                let application_peer = node
                    .application_peer(&transport_peer)
                    .context("authenticated request lost application peer mapping")?;
                handle_request(
                    &identity,
                    storage,
                    &mut node,
                    transport_peer,
                    application_peer,
                    request,
                    channel,
                    &mut pending_manifests,
                )?;
            }
            NetworkEvent::Response { transport_peer, request_id, response } => {
                let context = outbound.remove(&request_key(&request_id));
                handle_response(storage, &mut node, &transport_peer, context, response)?;
            }
            NetworkEvent::OutboundFailure { transport_peer, request_id, error } => {
                outbound.remove(&request_key(&request_id));
                warn!(transport = %transport_peer, %error, "outbound peer request failed; replication will renegotiate after reconnect");
            }
            NetworkEvent::Disconnected { transport_peer } => {
                info!(transport = %transport_peer, "peer disconnected");
            }
            NetworkEvent::Connected { transport_peer } => {
                info!(transport = %transport_peer, "transport connected; waiting for signed PeerHello");
            }
            NetworkEvent::Discovered { transport_peer, address } => {
                info!(transport = %transport_peer, %address, "peer discovered");
            }
        }
    }
}

fn push_known_worlds(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    application_peer: PeerId,
    outbound: &mut HashMap<String, OutboundContext>,
) -> Result<()> {
    for metadata in storage.list_worlds()? {
        let Ok(descriptor) = storage.load_world_descriptor(metadata.world_id) else { continue };
        if descriptor.member(application_peer).is_none() {
            continue;
        }
        if let Ok(membership) = storage.load_membership_record(metadata.world_id) {
            let _ = node.send_request(transport_peer, WireRequest::Membership(membership))?;
        }
        if let Ok(epoch) = storage.load_epoch_record(metadata.world_id) {
            let _ = node.send_request(transport_peer, WireRequest::Epoch(epoch))?;
        }
        if let Ok(transfer) = storage.load_transfer_record(metadata.world_id) {
            let _ = node.send_request(transport_peer, WireRequest::AuthorityTransfer(transfer))?;
        }
        if let Ok(sleep) = storage.load_sleep_record(metadata.world_id) {
            let _ = node.send_request(transport_peer, WireRequest::Sleep(sleep))?;
        }
        if let Some(manifest) = storage.latest_snapshot(metadata.world_id)? {
            verify_snapshot_signature(&manifest)?;
            let id = node.send_request(transport_peer, WireRequest::SnapshotManifest(manifest.clone()))?;
            outbound.insert(
                request_key(&id),
                OutboundContext::Manifest { world: metadata.world_id, snapshot_number: manifest.snapshot_number },
            );
        }
    }
    Ok(())
}

fn handle_request(
    identity: &PeerIdentity,
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: TransportPeerId,
    application_peer: PeerId,
    request: WireRequest,
    channel: ResponseChannel<WireResponse>,
    pending_manifests: &mut HashMap<WorldId, SnapshotManifestV1>,
) -> Result<()> {
    match request {
        WireRequest::Ping { nonce } => node.respond(channel, WireResponse::Pong { nonce })?,
        WireRequest::WorldStatus { world_id } => {
            let status = world_status(storage, world_id, application_peer)?;
            node.respond(channel, WireResponse::WorldStatus(status))?;
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
            if request.invite.expires_unix_ms < unix_millis()? {
                return Err(anyhow!("invite has expired"));
            }
            let world = request.world_id;
            let metadata = storage.load_world(world)?;
            if request.invite.genesis.world_id()? != world
                || request.invite.genesis.compatibility_fingerprint != metadata.genesis.compatibility_fingerprint
            {
                return Err(anyhow!("invite does not match local world genesis"));
            }
            let mut descriptor = storage.load_world_descriptor(world)?;
            let inviter = descriptor
                .member(request.invite.inviter_peer_id)
                .context("invite signer is not a current world member")?;
            if inviter.banned || inviter.public_key != request.invite.inviter_public_key {
                return Err(anyhow!("invite signer is banned or key does not match current membership"));
            }
            let current = storage.load_membership_record(world)?;
            verify_membership_signature(&current)?;
            if current.authority_peer_id != identity.peer_id() || current.authority_public_key != identity.public_key() {
                return Err(anyhow!("only the current local authority may accept a join request"));
            }
            if let Some(member) = descriptor.member(request.joining_member.peer_id) {
                if member.public_key != request.joining_member.public_key || member.banned {
                    return Err(anyhow!("joining peer conflicts with existing membership"));
                }
                node.respond(channel, WireResponse::JoinAccepted { membership_sequence: current.sequence })?;
            } else {
                let previous_hash = Some(current.record_hash()?);
                descriptor.members.push(request.joining_member.clone());
                descriptor.normalize();
                let mut next = MembershipRecordV1 {
                    protocol_version: current.protocol_version,
                    world_id: world,
                    epoch: current.epoch,
                    sequence: current.sequence.saturating_add(1),
                    previous_membership_hash: previous_hash,
                    members: descriptor.members.clone(),
                    authority_peer_id: identity.peer_id(),
                    authority_public_key: identity.public_key(),
                    signature: Vec::new(),
                };
                identity.sign_membership(&mut next)?;
                storage.save_world_descriptor(&descriptor)?;
                storage.save_membership_record(&next)?;
                node.respond(channel, WireResponse::JoinAccepted { membership_sequence: next.sequence })?;
            }
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
            pending_manifests.insert(manifest.world_id, manifest.clone());
            node.respond(
                channel,
                WireResponse::ManifestAccepted { snapshot_number: manifest.snapshot_number, missing: missing.clone() },
            )?;
            if missing.is_empty() {
                finalize_and_ack(storage, node, &transport_peer, &manifest)?;
                pending_manifests.remove(&manifest.world_id);
            }
        }
        WireRequest::BlobChunk { world_id, hash, encoding, offset, data, finished } => {
            authorize_member(storage, world_id, application_peer)?;
            let manifest =
                pending_manifests.get(&world_id).context("blob chunk arrived without a negotiated manifest")?;
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
                pending_manifests.remove(&world_id);
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
        WireRequest::Membership(record) => {
            verify_membership_signature(&record)?;
            authorize_member(storage, record.world_id, record.authority_peer_id)?;
            if let Ok(current) = storage.load_membership_record(record.world_id) {
                if record.epoch < current.epoch || (record.epoch == current.epoch && record.sequence <= current.sequence) {
                    return Err(anyhow!("stale membership record rejected"));
                }
            }
            let mut descriptor = storage.load_world_descriptor(record.world_id)?;
            descriptor.members = record.members.clone();
            descriptor.normalize();
            storage.save_membership_record(&record)?;
            storage.save_world_descriptor(&descriptor)?;
            node.respond(channel, WireResponse::MembershipAccepted { sequence: record.sequence })?;
        }
        WireRequest::Epoch(record) => {
            authorize_epoch(storage, application_peer, &record)?;
            if let Ok(current) = storage.load_epoch_record(record.world_id) {
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
        WireRequest::AuthorityTransfer(transfer) => {
            verify_transfer_signature(&transfer)?;
            authorize_member(storage, transfer.world_id, transfer.signer_peer_id)?;
            validate_transfer(storage, &transfer)?;
            storage.save_transfer_record(&transfer)?;
            node.respond(channel, WireResponse::TransferAccepted)?;
        }
        WireRequest::LeaseGrant(lease) => {
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
            if let Ok(epoch) = storage.load_epoch_record(lease.world_id) {
                if lease.epoch != epoch.epoch_number || lease.fencing_token != epoch.fencing_token {
                    return Err(anyhow!("lease generation does not match accepted epoch"));
                }
            }
            node.respond(
                channel,
                WireResponse::LeaseAccepted { epoch: lease.epoch, fencing_token: lease.fencing_token },
            )?;
        }
        WireRequest::Sleep(record) => {
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
            let latest = storage.latest_snapshot(record.world_id)?.context("cannot sleep a world without a snapshot")?;
            if latest.manifest_hash()? != record.latest_snapshot_hash {
                return Err(anyhow!("sleep record does not reference the exact latest snapshot"));
            }
            storage.save_sleep_record(&record)?;
            node.respond(
                channel,
                WireResponse::SleepAccepted { epoch: record.epoch, fencing_token: record.fencing_token },
            )?;
        }
        WireRequest::Hello(_) => return Err(anyhow!("PeerHello is handled by the network authentication layer")),
    }
    Ok(())
}

fn handle_response(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    context: Option<OutboundContext>,
    response: WireResponse,
) -> Result<()> {
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
        (_, WireResponse::Error { code, message }) => warn!(%code, %message, "peer rejected request"),
        _ => {}
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
    if let Some(current) = storage.latest_snapshot(manifest.world_id)? {
        if manifest.epoch < current.epoch || (manifest.epoch == current.epoch && manifest.sequence < current.sequence) {
            return Err(anyhow!("stale snapshot manifest rejected"));
        }
    }
    if let Ok(epoch) = storage.load_epoch_record(manifest.world_id) {
        if manifest.epoch != epoch.epoch_number || manifest.authority_peer_id != epoch.authority_peer_id {
            return Err(anyhow!("snapshot does not belong to the accepted authority epoch"));
        }
    }
    Ok(())
}

fn authorize_epoch(storage: &Storage, sender: PeerId, record: &EpochRecordV1) -> Result<()> {
    if sender != record.authority_peer_id {
        return Err(anyhow!("epoch sender is not the signed authority"));
    }
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
    let latest = storage.latest_snapshot(record.world_id)?.context("cannot accept an authority epoch without a base snapshot")?;
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
        if transfer.next_epoch != epoch.epoch_number.saturating_add(1)
            || transfer.next_fencing_token != epoch.fencing_token.saturating_add(1)
        {
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

fn authorize_member(storage: &Storage, world: WorldId, peer: PeerId) -> Result<()> {
    let descriptor = storage.load_world_descriptor(world)?;
    let member = descriptor.member(peer).context("peer is not an authorized member of this world")?;
    if member.banned {
        return Err(anyhow!("peer is banned from this world"));
    }
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
        epoch: epoch.as_ref().map_or_else(|| latest.as_ref().map_or(0, |manifest| manifest.epoch), |record| record.epoch_number),
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
