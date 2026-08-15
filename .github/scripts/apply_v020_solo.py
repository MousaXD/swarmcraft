from pathlib import Path

path = Path('crates/swarm-cli/src/daemon.rs')
text = path.read_text()


def replace(old: str, new: str, label: str) -> None:
    global text
    if old not in text:
        raise SystemExit(f'missing solo patch anchor: {label}')
    text = text.replace(old, new, 1)

replace(
'''        sign_recovery_ballot, sign_recovery_vote, verify_recovery_ballot_signature, verify_recovery_vote_signature,
        verify_solo_branch_signature, verify_world_config_signature,
''',
'''        sign_recovery_ballot, sign_recovery_vote, sign_solo_branch, verify_recovery_ballot_signature,
        verify_recovery_vote_signature, verify_solo_branch_signature, verify_world_config_signature,
''',
'import solo signer',
)
replace(
'''    RecoveryBallotV1, RecoveryCertificateV1, RecoveryVoteV1, SnapshotManifestV1, TransferPhase, WorldDescriptorV1,
    WorldId, WorldStatusV1, PROTOCOL_VERSION,
''',
'''    RecoveryBallotV1, RecoveryCertificateV1, RecoveryVoteV1, SnapshotManifestV1, SoloBranchV1, TransferPhase,
    WorldDescriptorV1, WorldId, WorldStatusV1, PROTOCOL_VERSION,
''',
'import solo branch',
)

old = '''    if has_quorum(member_count, confirmed) {
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
'''
new = '''    if has_quorum(member_count, confirmed) {
        if context.epoch.mode == EpochMode::Solo {
            refresh_solo_branch(context.storage, context.identity, context.epoch)?;
            promote_solo_to_quorum(context, node, outbound, runtime)?;
            clear_permit(context.paths, world)?;
            return Ok(());
        }
        let heartbeat = runtime.permit_heartbeats.entry(world).or_default();
        *heartbeat = heartbeat.saturating_add(1);
        refresh_permit(context.paths, world, context.generation, *heartbeat)?;
    } else if solo_mode_allowed(context.storage, world)? {
        request_world_statuses(
            context.storage,
            node,
            outbound,
            runtime,
            context.descriptor,
            context.identity.peer_id(),
        )?;
        if context.epoch.mode != EpochMode::Solo {
            promote_to_solo(context.storage, context.identity, context.epoch)?;
            clear_permit(context.paths, world)?;
            return Ok(());
        }
        refresh_solo_branch(context.storage, context.identity, context.epoch)?;
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
'''
replace(old, new, 'maintain authority solo policy')

anchor = '''fn maintain_recovery_epoch_quorum(
'''
if anchor not in text:
    raise SystemExit('missing recovery quorum anchor')
helpers = r'''fn solo_mode_allowed(storage: &Storage, world: WorldId) -> Result<bool> {
    let Ok(config) = storage.load_world_config(world) else {
        return Ok(false);
    };
    verify_world_config_signature(&config)?;
    Ok(config.authority_policy.allow_solo_advancement)
}

fn promote_to_solo(storage: &Storage, identity: &PeerIdentity, previous: &EpochRecordV1) -> Result<EpochRecordV1> {
    if previous.authority_peer_id != identity.peer_id() || previous.authority_public_key != identity.public_key() {
        return Err(anyhow!("only the accepted authority may enter solo mode"));
    }
    if !solo_mode_allowed(storage, previous.world_id)? {
        return Err(anyhow!("solo advancement is disabled by the signed world configuration"));
    }
    let latest = storage.latest_snapshot(previous.world_id)?.context("cannot enter solo mode without a canonical snapshot")?;
    verify_snapshot_signature(&latest)?;
    let next_epoch = previous.epoch_number.saturating_add(1);
    let mut branch = SoloBranchV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: previous.world_id,
        base_snapshot_hash: latest.manifest_hash()?,
        base_epoch: previous.epoch_number,
        head_snapshot_hash: latest.manifest_hash()?,
        head_epoch: next_epoch,
        head_sequence: latest.sequence,
        state_hash: latest.state_root,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    sign_solo_branch(identity, &mut branch)?;
    // Preserve ancestry before making the solo epoch current. A crash can leave an
    // inert future branch, but never an active solo epoch with forgotten ancestry.
    storage.save_solo_branch(&branch)?;

    let mut next = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: previous.world_id,
        epoch_number: next_epoch,
        previous_epoch_hash: Some(epoch_record_hash(previous)?),
        base_state_hash: latest.state_root,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        mode: EpochMode::Solo,
        fencing_token: previous.fencing_token.saturating_add(1),
        reason: "solo advancement permitted by signed world policy while quorum is unavailable".into(),
        signature: Vec::new(),
    };
    next.signature = identity.sign(&next.signing_bytes()?);
    storage.save_epoch_record(&next)?;
    info!(world = %previous.world_id, epoch = next.epoch_number, "entered explicit solo mode");
    Ok(next)
}

fn refresh_solo_branch(storage: &Storage, identity: &PeerIdentity, epoch: &EpochRecordV1) -> Result<()> {
    if epoch.mode != EpochMode::Solo {
        return Ok(());
    }
    let latest = storage.latest_snapshot(epoch.world_id)?.context("solo epoch has no snapshot")?;
    verify_snapshot_signature(&latest)?;
    let mut branch = storage.load_solo_branch(epoch.world_id).context("solo epoch is missing durable branch ancestry")?;
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
    let mut next = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: context.descriptor.world_id,
        epoch_number: context.epoch.epoch_number.saturating_add(1),
        previous_epoch_hash: Some(epoch_record_hash(context.epoch)?),
        base_state_hash: latest.state_root,
        authority_peer_id: context.identity.peer_id(),
        authority_public_key: context.identity.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: context.epoch.fencing_token.saturating_add(1),
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

'''
text = text.replace(anchor, helpers + anchor, 1)

# Background seeding is optional for non-authority replicas.
replace(
'''                        push_known_worlds(storage, &mut node, &transport_peer, application_peer, &mut outbound)?;''',
'''                        push_known_worlds(
                            storage,
                            &mut node,
                            &transport_peer,
                            application_peer,
                            identity.peer_id(),
                            &mut outbound,
                        )?;''',
'push worlds authenticated call',
)
replace(
'''            push_known_worlds(storage, node, &transport_peer, application_peer, state.outbound)?;''',
'''            push_known_worlds(
                storage,
                node,
                &transport_peer,
                application_peer,
                identity.peer_id(),
                state.outbound,
            )?;''',
'push worlds join call',
)
replace(
'''fn push_known_worlds(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    application_peer: PeerId,
    outbound: &mut HashMap<String, OutboundContext>,
) -> Result<()> {''',
'''fn push_known_worlds(
    storage: &Storage,
    node: &mut SwarmNode,
    transport_peer: &TransportPeerId,
    application_peer: PeerId,
    local_peer: PeerId,
    outbound: &mut HashMap<String, OutboundContext>,
) -> Result<()> {''',
'push worlds signature',
)
replace(
'''        let Ok(descriptor) = storage.load_world_descriptor(metadata.world_id) else { continue };
        if descriptor.member(application_peer).is_none() {
            continue;
        }''',
'''        let Ok(descriptor) = storage.load_world_descriptor(metadata.world_id) else { continue };
        if descriptor.member(application_peer).is_none() {
            continue;
        }
        let local_is_authority = storage
            .load_epoch_record(metadata.world_id)
            .is_ok_and(|epoch| epoch.authority_peer_id == local_peer);
        if !local_is_authority && !storage.background_seeding_enabled(metadata.world_id)? {
            continue;
        }''',
'background seed gate',
)

# Config fingerprints must be anchored to genesis, never presentation metadata.
replace(
'''            verify_world_config_signature(&config)?;
            authorize_member(storage, config.world_id, application_peer)?;''',
'''            verify_world_config_signature(&config)?;
            let metadata = storage.load_world(config.world_id)?;
            let descriptor = storage.load_world_descriptor(config.world_id)?;
            let fingerprint = config.compatibility_fingerprint()?;
            if fingerprint != metadata.genesis.compatibility_fingerprint
                || fingerprint != descriptor.compatibility_fingerprint
            {
                return Err(anyhow!("world config compatibility fingerprint does not match canonical genesis"));
            }
            authorize_member(storage, config.world_id, application_peer)?;''',
'config fingerprint anchor',
)

# Tighten non-recovery epoch transitions.
replace(
'''                if record.epoch_number != current.epoch_number.saturating_add(1)
                    || record.fencing_token != current.fencing_token.saturating_add(1)
                {
                    return Err(anyhow!("epoch and fencing token must advance exactly once"));
                }
''',
'''                if record.epoch_number != current.epoch_number.saturating_add(1)
                    || record.fencing_token != current.fencing_token.saturating_add(1)
                    || record.previous_epoch_hash != Some(epoch_record_hash(&current)?)
                {
                    return Err(anyhow!("epoch and fencing token must advance exactly once from the accepted epoch"));
                }
                validate_non_recovery_epoch_transition(storage, &current, &record)?;
''',
'plain epoch transition validation',
)

anchor = '''fn authorize_epoch(storage: &Storage, sender: PeerId, record: &EpochRecordV1) -> Result<()> {'''
if anchor not in text:
    raise SystemExit('missing authorize epoch anchor')
validator = r'''fn validate_non_recovery_epoch_transition(
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
        if next.mode == EpochMode::Solo && !solo_mode_allowed(storage, next.world_id)? {
            return Err(anyhow!("solo advancement is disabled by the signed world configuration"));
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

'''
text = text.replace(anchor, validator + anchor, 1)

path.write_text(text)
