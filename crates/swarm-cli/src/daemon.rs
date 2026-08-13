use anyhow::{anyhow, Context, Result};
use std::{collections::HashMap, fmt::Debug};
use swarm_core::{
    verify_lease_signature, verify_membership_signature, verify_snapshot_signature, verify_transfer_signature,
    DataPaths, PeerIdentity,
};
use swarm_network::{
    load_or_create_transport_key, BlobResumeV1, NetworkEvent, ReplicaAckV1, ResponseChannel, SwarmNode,
    TransportPeerId, WireRequest, WireResponse, MAX_BLOB_CHUNK,
};
use swarm_protocol::{BlobDescriptor, Hash32, PeerId, SnapshotManifestV1, WorldId, WorldStatusV1};
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
                if record.epoch < current.epoch
                    || (record.epoch == current.epoch && record.sequence <= current.sequence)
                {
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
        WireRequest::AuthorityTransfer(transfer) => {
            verify_transfer_signature(&transfer)?;
            authorize_member(storage, transfer.world_id, transfer.signer_peer_id)?;
            node.respond(channel, WireResponse::TransferAccepted)?;
        }
        WireRequest::LeaseGrant(lease) => {
            verify_lease_signature(&lease)?;
            authorize_member(storage, lease.world_id, lease.authority_peer_id)?;
            node.respond(
                channel,
                WireResponse::LeaseAccepted { epoch: lease.epoch, fencing_token: lease.fencing_token },
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
    Ok(Some(WorldStatusV1 {
        world_id: world,
        epoch: latest.as_ref().map_or(0, |manifest| manifest.epoch),
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

fn request_key(value: &impl Debug) -> String {
    format!("{value:?}")
}
