from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    p = Path(path)
    text = p.read_text()
    found = text.count(old)
    if found != count:
        raise SystemExit(f"{path}: expected {count}, found {found}: {old[:140]!r}")
    p.write_text(text.replace(old, new, count))


path = "crates/swarm-cli/tests/consensus_partition_safety.rs"
replace(
    path,
    "use swarm_core::{create_world_genesis, random_nonce, DataPaths, PeerIdentity};\n",
    "use swarm_core::{create_world_genesis_with_fingerprint, random_nonce, sign_world_config, DataPaths, PeerIdentity};\n",
)
replace(
    path,
    '''use swarm_protocol::{
    EpochMode, EpochRecordV1, InviteV1, JoinRequestV1, MembershipProposalV1, MembershipRecordV1, PeerId,
    SnapshotManifestV1, WorldDescriptorV1, WorldId, WorldMemberV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
''',
    '''use swarm_protocol::{
    AuthorityPolicyV1, EpochMode, EpochRecordV1, InviteV1, JoinRequestV1, MembershipPolicyV1,
    MembershipProposalV1, MembershipRecordV1, RuntimeCompatibilityManifestV1, SnapshotManifestV1,
    WorldConfigV1, WorldDescriptorV1, WorldId, WorldMemberV1, WorldPresentationV1, WorldVisibilityV1,
    PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
''',
)
replace(
    path,
    '''struct Seed<'a> {
    metadata: &'a WorldMetadataV1,
    descriptor: &'a WorldDescriptorV1,
''',
    '''struct Seed<'a> {
    metadata: &'a WorldMetadataV1,
    config: &'a WorldConfigV1,
    descriptor: &'a WorldDescriptorV1,
''',
)
replace(
    path,
    ''') -> (WorldMetadataV1, WorldDescriptorV1, MembershipRecordV1, EpochRecordV1, SnapshotManifestV1) {
    let (world, genesis) = create_world_genesis(
        &authority.identity,
        "26.1.2".into(),
        "0.19.3".into(),
        label.as_bytes(),
    )
    .unwrap();
''',
    ''') -> (WorldMetadataV1, WorldConfigV1, WorldDescriptorV1, MembershipRecordV1, EpochRecordV1, SnapshotManifestV1) {
    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: "26.1.2".into(),
        loader_id: "fabric".into(),
        loader_version: "0.19.3".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: env!("CARGO_PKG_VERSION").into(),
        required_server_mods: Vec::new(),
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let fingerprint = compatibility.fingerprint().unwrap();
    let (world, genesis) = create_world_genesis_with_fingerprint(
        &authority.identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        fingerprint,
    )
    .unwrap();
''',
)
replace(
    path,
    '''    authority.storage.create_world(&metadata).unwrap();
    let mut descriptor = WorldDescriptorV1 {
''',
    '''    authority.storage.create_world(&metadata).unwrap();
    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 {
            allow_solo_advancement: true,
            preferred_replication_factor: members.len() as u16,
        },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: label.into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: authority.identity.peer_id(),
        authority_public_key: authority.identity.public_key(),
        signature: Vec::new(),
    };
    sign_world_config(&authority.identity, &mut config).unwrap();
    authority.storage.save_world_config(&config).unwrap();
    let mut descriptor = WorldDescriptorV1 {
''',
)
replace(
    path,
    '''    authority.storage.commit_snapshot(&manifest).unwrap();
    let mut epoch = EpochRecordV1 {
''',
    '''    authority.storage.commit_snapshot(&manifest).unwrap();
    let manifest = manifest.manifest().clone();
    let mut epoch = EpochRecordV1 {
''',
)
replace(
    path,
    "    (metadata, descriptor, membership, epoch, manifest)\n",
    "    (metadata, config, descriptor, membership, epoch, manifest)\n",
)
replace(
    path,
    '''    peer.storage.save_world_descriptor(seed.descriptor).unwrap();
''',
    '''    peer.storage.save_world_config(seed.config).unwrap();
    peer.storage.save_world_descriptor(seed.descriptor).unwrap();
''',
)
replace(
    path,
    "    let (metadata, descriptor, membership, epoch, manifest) =\n",
    "    let (metadata, config, descriptor, membership, epoch, manifest) =\n",
    count=3,
)
replace(
    path,
    '''        metadata: &metadata,
        descriptor: &descriptor,
''',
    '''        metadata: &metadata,
        config: &config,
        descriptor: &descriptor,
''',
    count=3,
)
