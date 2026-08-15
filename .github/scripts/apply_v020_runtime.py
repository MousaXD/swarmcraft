from pathlib import Path


def replace(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f'missing patch anchor: {label}')
    return text.replace(old, new, 1)

# CLI: canonical compatibility configuration, visibility, background seeding, conflict inspection.
path = Path('crates/swarm-cli/src/main.rs')
text = path.read_text()
text = replace(
    text,
    '''use swarm_core::{
    create_world_genesis, random_nonce, verify_membership_signature, verify_snapshot_signature, DataPaths, PeerIdentity,
};''',
    '''use swarm_core::{
    create_world_genesis_with_fingerprint, random_nonce, sign_world_config, verify_membership_signature,
    verify_snapshot_signature, DataPaths, PeerIdentity,
};''',
    'cli core imports',
)
text = replace(
    text,
    '''use swarm_protocol::{
    InviteV1, JoinRequestV1, LeaveRequestV1, MembershipRecordV1, WorldDescriptorV1, WorldId, WorldMemberV1,
    PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};''',
    '''use swarm_protocol::{
    ArtifactRequirementV1, ArtifactSideV1, AuthorityPolicyV1, EpochMode, Hash32, InviteV1, JoinRequestV1,
    LeaveRequestV1, MembershipPolicyV1, MembershipRecordV1, RuntimeCompatibilityManifestV1, WorldConfigV1,
    WorldDescriptorV1, WorldId, WorldMemberV1, WorldPresentationV1, WorldSafetyLevelV1, WorldVisibilityV1,
    PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};''',
    'cli protocol imports',
)
text = replace(
    text,
    '''        /// Stable compatibility material, for example a sorted mod/datapack fingerprint string.
        #[arg(long, default_value = "vanilla-fabric")]
        compatibility: String,
    },''',
    '''        /// Legacy exact compatibility material retained for CLI compatibility. Prefer the canonical manifest shown by `world compatibility`.
        #[arg(long, default_value = "vanilla-fabric")]
        compatibility: String,
        /// private, unlisted, or public.
        #[arg(long, default_value = "private")]
        visibility: String,
    },''',
    'create flags',
)
text = replace(
    text,
    '''    /// Show local world and snapshot status.
    Status { world: String },''',
    '''    /// Show local world and snapshot status.
    Status { world: String },
    /// Inspect the canonical execution compatibility manifest and authority eligibility.
    Compatibility { world: String },
    /// Enable or disable background replica seeding while Minecraft is off.
    Seed { world: String, enabled: bool },
    /// List preserved conflicting solo-history branches requiring manual resolution.
    Conflicts { world: String },''',
    'world subcommands',
)
text = replace(
    text,
    '''        WorldCommand::Create { name, minecraft, fabric_loader, compatibility } => {
            let identity = PeerIdentity::load_or_create(paths)?;
            let (world_id, genesis) =
                create_world_genesis(&identity, minecraft, fabric_loader, compatibility.as_bytes())?;''',
    '''        WorldCommand::Create { name, minecraft, fabric_loader, compatibility, visibility } => {
            let identity = PeerIdentity::load_or_create(paths)?;
            let visibility = parse_visibility(&visibility)?;
            let legacy_hash = Hash32::from_domain_bytes(
                b"swarmcraft/legacy-compatibility/v1\\0",
                compatibility.as_bytes(),
            );
            let manifest = RuntimeCompatibilityManifestV1 {
                minecraft_version: minecraft.clone(),
                loader_id: "fabric".into(),
                loader_version: fabric_loader.clone(),
                swarmcraft_protocol_version: PROTOCOL_VERSION,
                fabric_adapter_version: env!("CARGO_PKG_VERSION").into(),
                required_server_mods: vec![ArtifactRequirementV1 {
                    artifact_id: "swarmcraft.legacy-compatibility".into(),
                    version: "1".into(),
                    artifact_hash: legacy_hash,
                    side: ArtifactSideV1::Server,
                    provider_hint: None,
                }],
                required_client_mods: Vec::new(),
                datapacks: Vec::new(),
            };
            let fingerprint = manifest.fingerprint()?;
            let (world_id, genesis) = create_world_genesis_with_fingerprint(
                &identity,
                minecraft,
                fabric_loader,
                fingerprint,
            )?;''',
    'world create manifest',
)
text = replace(
    text,
    '''            identity.sign_membership(&mut membership)?;
            storage.save_membership_record(&membership)?;
            println!("Created world: {name}");''',
    '''            identity.sign_membership(&mut membership)?;
            storage.save_membership_record(&membership)?;
            let mut config = WorldConfigV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id,
                sequence: 1,
                previous_config_hash: None,
                compatibility: manifest,
                visibility,
                authority_policy: AuthorityPolicyV1 {
                    allow_solo_advancement: true,
                    preferred_replication_factor: descriptor.preferred_replication_factor,
                },
                membership_policy: MembershipPolicyV1::InviteOnly,
                presentation: WorldPresentationV1 {
                    name: name.clone(),
                    description: String::new(),
                    tags: Vec::new(),
                    icon_hash: None,
                    approximate_region: None,
                },
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            sign_world_config(&identity, &mut config)?;
            storage.save_world_config(&config)?;
            println!("Created world: {name}");''',
    'save world config',
)
text = replace(
    text,
    '''                joining_member: local_member(&identity),''',
    '''                joining_member: local_storage_member(&identity),''',
    'join replica-only member',
)
status_anchor = '''        WorldCommand::Snapshot { world, source, epoch, sequence } => {'''
if status_anchor not in text:
    raise SystemExit('missing snapshot anchor')
new_commands = '''        WorldCommand::Compatibility { world } => {
            let world = parse_world(&world)?;
            let metadata = storage.load_world(world)?;
            match storage.load_world_config(world) {
                Ok(config) => {
                    let fingerprint = config.compatibility_fingerprint()?;
                    println!("Compatibility fingerprint: {fingerprint}");
                    println!("Minecraft: {}", config.compatibility.minecraft_version);
                    println!("Loader: {} {}", config.compatibility.loader_id, config.compatibility.loader_version);
                    println!("Fabric adapter: {}", config.compatibility.fabric_adapter_version);
                    println!("Server mods: {}", config.compatibility.required_server_mods.len());
                    println!("Client mods: {}", config.compatibility.required_client_mods.len());
                    println!("Datapacks: {}", config.compatibility.datapacks.len());
                    println!(
                        "Genesis match: {}",
                        fingerprint == metadata.genesis.compatibility_fingerprint
                    );
                    let identity = PeerIdentity::load_or_create(paths)?;
                    let eligible = storage
                        .load_world_descriptor(world)
                        .ok()
                        .and_then(|descriptor| descriptor.member(identity.peer_id()).cloned())
                        .is_some_and(|member| member.authority_eligible && !member.banned);
                    if eligible && fingerprint == metadata.genesis.compatibility_fingerprint {
                        println!("Authority eligibility: Compatible");
                    } else {
                        println!("Authority eligibility: Replica only: not authority eligible");
                    }
                }
                Err(_) => {
                    println!("Compatibility manifest: not yet synchronized");
                    println!("Authority eligibility: Replica only: not authority eligible");
                }
            }
        }
        WorldCommand::Seed { world, enabled } => {
            let world = parse_world(&world)?;
            storage.set_background_seeding(world, enabled)?;
            println!("Background seeding: {}", if enabled { "enabled" } else { "disabled" });
        }
        WorldCommand::Conflicts { world } => {
            let world = parse_world(&world)?;
            let conflicts = storage.list_solo_conflicts(world)?;
            if conflicts.is_empty() {
                println!("Solo history conflicts: none");
            } else {
                println!("Solo history conflicts: {}", conflicts.len());
                for branch in conflicts {
                    println!(
                        "{} writer={} epoch={} seq={} state={}",
                        branch.branch_hash()?,
                        branch.authority_peer_id,
                        branch.head_epoch,
                        branch.head_sequence,
                        branch.state_hash
                    );
                }
                println!("No automatic Minecraft world merge is attempted; preserve both branches until manually resolved.");
            }
        }
'''
text = text.replace(status_anchor, new_commands + status_anchor, 1)

# Enrich status without making membership count pretend to be live replica count.
text = replace(
    text,
    '''            println!("Compatibility: {}", metadata.genesis.compatibility_fingerprint);
            if let Ok(descriptor) = storage.load_world_descriptor(world) {''',
    '''            println!("Compatibility: {}", metadata.genesis.compatibility_fingerprint);
            if let Ok(config) = storage.load_world_config(world) {
                println!("Visibility: {:?}", config.visibility);
                println!("Compatibility manifest: {}", config.compatibility_fingerprint()?);
            } else {
                println!("Visibility: unknown (configuration not synchronized)");
            }
            if let Ok(descriptor) = storage.load_world_descriptor(world) {''',
    'status config',
)
text = replace(
    text,
    '''            match latest {''',
    '''            let conflict = !storage.list_solo_conflicts(world)?.is_empty();
            let solo = storage
                .load_epoch_record(world)
                .is_ok_and(|epoch| epoch.mode == EpochMode::Solo)
                || storage.load_solo_branch(world).is_ok();
            let safety = if conflict {
                WorldSafetyLevelV1::Conflict
            } else if solo {
                WorldSafetyLevelV1::SoloUnreplicated
            } else {
                WorldSafetyLevelV1::Canonical
            };
            println!("Safety: {:?}", safety);
            println!("Background seeding: {}", storage.background_seeding_enabled(world)?);
            match latest {''',
    'status safety',
)
text = replace(
    text,
    '''fn local_member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}
''',
    '''fn local_member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn local_storage_member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: false,
        banned: false,
    }
}

fn parse_visibility(value: &str) -> Result<WorldVisibilityV1> {
    match value.to_ascii_lowercase().as_str() {
        "private" => Ok(WorldVisibilityV1::Private),
        "unlisted" => Ok(WorldVisibilityV1::Unlisted),
        "public" => Ok(WorldVisibilityV1::Public),
        _ => bail!("visibility must be private, unlisted, or public"),
    }
}
''',
    'member helpers',
)
path.write_text(text)

# Network node: record observable connectivity state while preserving libp2p as the source of transport decisions.
path = Path('crates/swarm-network/src/node.rs')
text = path.read_text()
text = replace(
    text,
    'use crate::{verify_peer_hello, wire::WireRequest, wire::WireResponse};',
    'use crate::{verify_peer_hello, wire::WireRequest, wire::WireResponse, ConnectivityDiagnosticsV1, NatStatusV1};',
    'node diagnostics imports',
)
text = replace(
    text,
    '''pub struct SwarmNode {
    swarm: Swarm<Behaviour>,
    local_hello: PeerHelloV1,
    authenticated: HashMap<TransportPeerId, PeerId>,
}''',
    '''pub struct SwarmNode {
    swarm: Swarm<Behaviour>,
    local_hello: PeerHelloV1,
    authenticated: HashMap<TransportPeerId, PeerId>,
    diagnostics: ConnectivityDiagnosticsV1,
}''',
    'node diagnostics field',
)
text = replace(
    text,
    '''        let mut node = Self { swarm, local_hello, authenticated: HashMap::new() };''',
    '''        let mut node = Self {
            swarm,
            local_hello,
            authenticated: HashMap::new(),
            diagnostics: ConnectivityDiagnosticsV1::default(),
        };''',
    'node constructor diagnostics',
)
text = replace(
    text,
    '''    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).copied()
    }
''',
    '''    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).copied()
    }

    pub fn connectivity_diagnostics(&self) -> ConnectivityDiagnosticsV1 {
        self.diagnostics.clone()
    }
''',
    'diagnostics getter',
)
text = replace(
    text,
    '''        let relay_address = ensure_peer_suffix(relay_address, relay_peer);''',
    '''        let relay_address = ensure_peer_suffix(relay_address, relay_peer);
        self.diagnostics.selected_relay = Some(relay_address.to_string());''',
    'selected relay',
)
text = replace(
    text,
    '''                SwarmEvent::NewListenAddr { address, .. } => {
                    info!(%address, "network listening");''',
    '''                SwarmEvent::NewListenAddr { address, .. } => {
                    self.diagnostics.record_local_address(address.to_string());
                    if address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit)) {
                        self.diagnostics.relay_connectivity = true;
                    }
                    info!(%address, "network listening");''',
    'listen diagnostics',
)
text = replace(
    text,
    '''                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    debug!(transport_peer = %peer_id, "peer connected");''',
    '''                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                    let endpoint_debug = format!("{endpoint:?}");
                    if endpoint_debug.contains("p2p-circuit") || endpoint_debug.contains("P2pCircuit") {
                        self.diagnostics.relay_connectivity = true;
                    } else {
                        self.diagnostics.record_direct_success();
                    }
                    debug!(transport_peer = %peer_id, "peer connected");''',
    'connection diagnostics',
)
text = replace(
    text,
    '''                })) => {
                    for address in info.listen_addrs {''',
    '''                })) => {
                    self.diagnostics.record_observed_address(info.observed_addr.to_string());
                    for address in info.listen_addrs {''',
    'identify observed address',
)
text = replace(
    text,
    '''                    request_response::Event::OutboundFailure { peer, request_id, error, .. } => {
                        return Ok(NetworkEvent::OutboundFailure {''',
    '''                    request_response::Event::OutboundFailure { peer, request_id, error, .. } => {
                        self.diagnostics.record_direct_failure(error.to_string());
                        return Ok(NetworkEvent::OutboundFailure {''',
    'outbound diagnostics',
)
text = replace(
    text,
    '''                SwarmEvent::Behaviour(BehaviourEvent::RelayClient(event)) => {
                    debug!(?event, "relay client event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::Dcutr(event)) => {
                    info!(?event, "DCUtR hole-punch event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::AutoNat(event)) => {
                    debug!(?event, "AutoNAT event");
                }''',
    '''                SwarmEvent::Behaviour(BehaviourEvent::RelayClient(event)) => {
                    let event_debug = format!("{event:?}");
                    if event_debug.contains("ReservationReqAccepted") || event_debug.contains("Reservation") {
                        self.diagnostics.relay_connectivity = true;
                    }
                    debug!(?event, "relay client event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::Dcutr(event)) => {
                    let event_debug = format!("{event:?}");
                    self.diagnostics.start_hole_punch();
                    if event_debug.contains("Success") {
                        self.diagnostics.finish_hole_punch(Ok::<(), String>(()));
                        self.diagnostics.direct_connectivity = true;
                    } else if event_debug.contains("Error") || event_debug.contains("Failed") {
                        self.diagnostics.finish_hole_punch(Err(event_debug.clone()));
                    }
                    info!(?event, "DCUtR hole-punch event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::AutoNat(event)) => {
                    let event_debug = format!("{event:?}");
                    if event_debug.contains("Public") {
                        self.diagnostics.nat_status = NatStatusV1::Public;
                    } else if event_debug.contains("Private") {
                        self.diagnostics.nat_status = NatStatusV1::Private;
                    }
                    debug!(?event, "AutoNAT event");
                }''',
    'behaviour diagnostics',
)
path.write_text(text)
