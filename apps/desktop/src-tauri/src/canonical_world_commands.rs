use crate::canonical_commands::{
    canonicalize_modpack, CanonicalModpackResponse, CanonicalizationFailure, CanonicalizeModpackRequest,
};
use serde::{Deserialize, Serialize};
use swarm_catalog::CatalogService;
use swarm_core::{create_world_genesis_with_fingerprint, sign_world_config, DataPaths, PeerIdentity};
use swarm_protocol::{
    validate_runtime_selection, AuthorityPolicyV1, MembershipPolicyV1, MembershipRecordV1, WorldConfigV1,
    WorldDescriptorV1, WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
    STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, WorldMetadataV1};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCanonicalWorldRequest {
    pub name: String,
    pub visibility: String,
    pub modpack: CanonicalizeModpackRequest,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCanonicalWorldResponse {
    pub world_id: String,
    pub canonical: CanonicalModpackResponse,
}

#[tauri::command]
pub fn create_canonical_world(
    request: CreateCanonicalWorldRequest,
) -> Result<CreateCanonicalWorldResponse, CanonicalizationFailure> {
    let name = request.name.trim().to_owned();
    if name.is_empty() {
        return Err(failure("invalid_world_name", "World name is required"));
    }
    if !request.modpack.loader_id.trim().eq_ignore_ascii_case("fabric") {
        return Err(failure("unsupported_loader", "SwarmCraft v1 currently supports the Fabric loader contract"));
    }

    validate_runtime_selection(
        request.modpack.minecraft_version.trim(),
        request.modpack.loader_version.trim(),
        None,
    )
    .map_err(|error| failure("unsupported_runtime_adapter", error.to_string()))?;
    validate_catalog_selection(&request.modpack.minecraft_version, &request.modpack.loader_version)?;
    let canonical = canonicalize_modpack(request.modpack)?;
    let visibility = parse_visibility(&request.visibility)?;

    let paths = DataPaths::discover().map_err(|error| failure("storage_unavailable", error.to_string()))?;
    paths.ensure().map_err(|error| failure("storage_unavailable", error.to_string()))?;
    let storage =
        Storage::open(paths.root.clone()).map_err(|error| failure("storage_unavailable", error.to_string()))?;
    let identity =
        PeerIdentity::load_or_create(&paths).map_err(|error| failure("identity_unavailable", error.to_string()))?;

    let minecraft = canonical.manifest.minecraft_version.clone();
    let loader = canonical.manifest.loader.version.clone();
    let fingerprint = canonical
        .compatibility
        .fingerprint()
        .map_err(|error| failure("canonical_fingerprint_failed", error.to_string()))?;
    let (world_id, genesis) = create_world_genesis_with_fingerprint(&identity, minecraft, loader, fingerprint)
        .map_err(|error| failure("world_genesis_failed", error.to_string()))?;

    storage
        .create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: name.clone(),
            world_id,
            genesis: genesis.clone(),
        })
        .map_err(|error| failure("world_storage_failed", error.to_string()))?;

    let local_member = WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    };
    let descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id,
        compatibility_fingerprint: genesis.compatibility_fingerprint,
        members: vec![local_member.clone()],
        preferred_replication_factor: 2,
    };
    storage.save_world_descriptor(&descriptor).map_err(|error| failure("world_storage_failed", error.to_string()))?;

    let mut membership = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id,
        epoch: 0,
        sequence: 0,
        previous_membership_hash: None,
        members: vec![local_member],
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    identity.sign_membership(&mut membership).map_err(|error| failure("membership_sign_failed", error.to_string()))?;
    storage.save_membership_record(&membership).map_err(|error| failure("world_storage_failed", error.to_string()))?;

    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id,
        sequence: 1,
        previous_config_hash: None,
        compatibility: canonical.compatibility.clone(),
        visibility,
        authority_policy: AuthorityPolicyV1 {
            allow_solo_advancement: true,
            preferred_replication_factor: descriptor.preferred_replication_factor,
        },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name,
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    sign_world_config(&identity, &mut config)
        .map_err(|error| failure("world_config_sign_failed", error.to_string()))?;
    storage.save_world_config(&config).map_err(|error| failure("world_storage_failed", error.to_string()))?;

    let initial_source = paths.root.join("initial-world").join(world_id.to_hex());
    std::fs::create_dir_all(&initial_source).map_err(|error| failure("snapshot_failed", error.to_string()))?;
    let snapshot_result = (|| {
        let mut snapshot = storage.snapshot_directory(
            &initial_source,
            SnapshotContext {
                world: world_id,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
            },
        )?;
        identity.sign_snapshot(&mut snapshot)?;
        storage.commit_snapshot(&snapshot)?;
        Ok::<(), anyhow::Error>(())
    })();
    let _ = std::fs::remove_dir_all(&initial_source);
    snapshot_result.map_err(|error| failure("snapshot_failed", error.to_string()))?;

    Ok(CreateCanonicalWorldResponse { world_id: world_id.to_string(), canonical })
}

fn validate_catalog_selection(minecraft: &str, loader: &str) -> Result<(), CanonicalizationFailure> {
    let service = CatalogService::discover().map_err(|error| failure(error.code(), error.to_string()))?;
    let minecraft_catalog =
        service.minecraft_versions(true, false).map_err(|error| failure(error.code(), error.to_string()))?;
    if !minecraft_catalog.versions.iter().any(|version| version.supported && version.id == minecraft) {
        return Err(failure(
            "unsupported_minecraft_version",
            format!("Minecraft {minecraft} is not present in the authoritative Mojang catalog"),
        ));
    }
    service
        .validate_fabric_selection(minecraft, loader, false)
        .map_err(|error| failure(error.code(), error.to_string()))?;
    Ok(())
}

fn parse_visibility(value: &str) -> Result<WorldVisibilityV1, CanonicalizationFailure> {
    match value.trim().to_ascii_lowercase().as_str() {
        "private" => Ok(WorldVisibilityV1::Private),
        "unlisted" => Ok(WorldVisibilityV1::Unlisted),
        "public" => Ok(WorldVisibilityV1::Public),
        other => Err(failure("invalid_visibility", format!("unsupported world visibility {other}"))),
    }
}

fn failure(code: &str, message: impl Into<String>) -> CanonicalizationFailure {
    CanonicalizationFailure { code: code.into(), message: message.into(), artifact_id: None }
}
