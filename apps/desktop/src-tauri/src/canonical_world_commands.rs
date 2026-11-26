use crate::canonical_commands::{
    canonicalize_modpack, CanonicalModpackResponse, CanonicalizationFailure, CanonicalizeModpackRequest,
};
use serde::{Deserialize, Serialize};
use swarm_catalog::CatalogService;
use swarm_core::{create_world_genesis_with_fingerprint, sign_world_config, DataPaths, PeerIdentity};
use swarm_protocol::{
    validate_new_world_selection, AuthorityPolicyV1, MembershipPolicyV1, MembershipRecordV1, WorldConfigV1,
    WorldDescriptorV1, WorldId, WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
    STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{
    genesis::{establish_initial_epoch, GENESIS_EPOCH},
    SnapshotContext, Storage, WorldMetadataV1,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCanonicalWorldRequest {
    pub name: String,
    pub visibility: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
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
    let mut history =
        crate::support_diagnostics::ActionGuard::begin(crate::support_diagnostics::Action::CreateWorld, None);
    let result = (|| {
        let name = request.name.trim().to_owned();
        if name.is_empty() {
            return Err(failure("invalid_world_name", "World name is required"));
        }
        let visibility = parse_visibility(&request.visibility)?;
        let presentation = swarm_protocol::new_listing_presentation(
            name.clone(),
            visibility,
            request.region.as_deref(),
            request.language.as_deref(),
        )
        .map_err(|error| failure("invalid_listing_metadata", error))?;
        if !request.modpack.loader_id.trim().eq_ignore_ascii_case("fabric") {
            return Err(failure("unsupported_loader", "SwarmCraft v1 currently supports the Fabric loader contract"));
        }

        validate_new_world_selection(request.modpack.minecraft_version.trim(), request.modpack.loader_version.trim())
            .map_err(|error| {
            failure(
                if matches!(error, swarm_protocol::RuntimeSupportError::UnqualifiedNewWorld { .. }) {
                    "unqualified_runtime"
                } else {
                    "unsupported_runtime_adapter"
                },
                error.to_string(),
            )
        })?;
        validate_catalog_selection(&request.modpack.minecraft_version, &request.modpack.loader_version)?;
        let canonical = canonicalize_modpack(request.modpack)?;

        let paths = DataPaths::discover().map_err(|error| failure("storage_unavailable", error.to_string()))?;
        let storage =
            Storage::open(paths.root.clone()).map_err(|error| failure("storage_unavailable", error.to_string()))?;
        let identity =
            PeerIdentity::load_or_create(&paths).map_err(|error| failure("identity_unavailable", error.to_string()))?;

        let world_id = persist_canonical_world(&storage, &identity, name, visibility, presentation, &canonical)?;

        Ok(CreateCanonicalWorldResponse { world_id: world_id.to_string(), canonical })
    })();
    if let Ok(receipt) = &result {
        history.associate_created_world(&receipt.world_id);
    }
    history.finish(&result);
    result
}

fn persist_canonical_world(
    destination: &Storage,
    identity: &PeerIdentity,
    name: String,
    visibility: WorldVisibilityV1,
    presentation: WorldPresentationV1,
    canonical: &CanonicalModpackResponse,
) -> Result<WorldId, CanonicalizationFailure> {
    let minecraft = canonical.manifest.minecraft_version.clone();
    let loader = canonical.manifest.loader.version.clone();
    let fingerprint = canonical
        .compatibility
        .fingerprint()
        .map_err(|error| failure("canonical_fingerprint_failed", error.to_string()))?;
    let (world_id, genesis) = create_world_genesis_with_fingerprint(identity, minecraft, loader, fingerprint)
        .map_err(|error| failure("world_genesis_failed", error.to_string()))?;

    // An active daemon must never discover a half-created epoch-zero world.
    // The fresh ID names a private staging directory; do not adopt old staging
    // or overwrite any existing destination, including a dangling symlink.
    let parent = destination.root().join(".create-staging");
    let staging = parent.join(world_id.to_hex());
    let final_world = destination.world_dir(world_id);
    swarm_core::create_private_dir_all(&parent).map_err(|e| failure("world_storage_failed", e.to_string()))?;
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&staging).map_err(|e| failure("world_storage_failed", e.to_string()))?;
    let result = (|| {
        let storage = Storage::open(staging.clone()).map_err(|e| failure("world_storage_failed", e.to_string()))?;
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
        storage
            .save_world_descriptor(&descriptor)
            .map_err(|error| failure("world_storage_failed", error.to_string()))?;

        let initial_source = staging.join("initial-world").join(world_id.to_hex());
        std::fs::create_dir_all(&initial_source).map_err(|error| failure("snapshot_failed", error.to_string()))?;
        let snapshot_result = (|| {
            let mut snapshot = storage.snapshot_directory(
                &initial_source,
                SnapshotContext {
                    world: world_id,
                    snapshot_number: 1,
                    epoch: GENESIS_EPOCH,
                    sequence: 1,
                    previous_snapshot_hash: None,
                    authority_peer_id: identity.peer_id(),
                    authority_public_key: identity.public_key(),
                },
            )?;
            identity.sign_snapshot(&mut snapshot)?;
            storage.commit_snapshot(&snapshot)?;
            establish_initial_epoch(&storage, identity, &snapshot)?;
            Ok::<(), anyhow::Error>(())
        })();
        let _ = std::fs::remove_dir_all(&initial_source);
        snapshot_result.map_err(|error| failure("snapshot_failed", error.to_string()))?;

        let mut membership = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id,
            epoch: GENESIS_EPOCH,
            sequence: 0,
            previous_membership_hash: None,
            members: vec![local_member],
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        identity
            .sign_membership(&mut membership)
            .map_err(|error| failure("membership_sign_failed", error.to_string()))?;
        storage
            .save_membership_record(&membership)
            .map_err(|error| failure("world_storage_failed", error.to_string()))?;

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
            presentation,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        sign_world_config(identity, &mut config)
            .map_err(|error| failure("world_config_sign_failed", error.to_string()))?;
        storage.save_world_config(&config).map_err(|error| failure("world_storage_failed", error.to_string()))?;

        let staged_world = storage.world_dir(world_id);
        match std::fs::symlink_metadata(&final_world) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(failure("world_storage_failed", e.to_string())),
            Ok(_) => return Err(failure("world_storage_failed", "fresh world destination already exists")),
        }
        std::fs::rename(&staged_world, &final_world).map_err(|e| failure("world_storage_failed", e.to_string()))?;
        #[cfg(unix)]
        if let Err(e) = std::fs::File::open(destination.worlds_dir()).and_then(|f| f.sync_all()) {
            // Match the import publication boundary: do not report a durable
            // success if syncing the visible parent failed.
            let _ = std::fs::rename(&final_world, &staged_world);
            return Err(failure("world_storage_failed", e.to_string()));
        }
        Ok(world_id)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("swarmcraft-create-test-{}", hex::encode(swarm_core::random_nonce())));
            swarm_core::create_private_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn canonical_create_publishes_verified_genesis_before_the_world_is_visible() {
        let dir = TestDir::new();
        let storage = Storage::open(dir.0.clone()).unwrap();
        let identity = PeerIdentity::generate();
        let name = "Canonical native genesis".to_owned();
        let visibility = WorldVisibilityV1::Private;
        let presentation = swarm_protocol::new_listing_presentation(name.clone(), visibility, None, None).unwrap();
        let canonical = canonicalize_modpack(CanonicalizeModpackRequest {
            minecraft_version: "26.1.2".into(),
            loader_id: "fabric".into(),
            loader_version: "0.19.3".into(),
            packages: Vec::new(),
            datapacks: Vec::new(),
        })
        .unwrap();

        assert!(storage.list_worlds().unwrap().is_empty());
        let world = persist_canonical_world(&storage, &identity, name, visibility, presentation, &canonical).unwrap();
        // Re-open the published store, not the staging path.
        let storage = Storage::open(dir.0.clone()).unwrap();
        assert_eq!(storage.list_worlds().unwrap().len(), 1);
        let metadata = storage.load_world(world).unwrap();
        let snapshot = storage.latest_snapshot(world).unwrap().unwrap();
        let epoch = storage.load_epoch_record(world).unwrap();
        let membership = storage.load_membership_record(world).unwrap();
        let config = storage.load_world_config(world).unwrap();
        let descriptor = storage.load_world_descriptor(world).unwrap();
        assert_eq!(metadata.genesis.initial_membership, [identity.peer_id()]);
        assert_eq!((snapshot.snapshot_number, snapshot.sequence, snapshot.epoch), (1, 1, GENESIS_EPOCH));
        assert_eq!(
            (epoch.epoch_number, epoch.fencing_token, epoch.mode),
            (GENESIS_EPOCH, 1, swarm_protocol::EpochMode::Quorum)
        );
        assert_eq!(epoch.base_state_hash, snapshot.state_root);
        assert_eq!(epoch.previous_epoch_hash, None);
        assert_eq!(epoch.authority_peer_id, identity.peer_id());
        assert_eq!(epoch.authority_public_key, identity.public_key());
        assert_eq!((membership.epoch, membership.sequence), (GENESIS_EPOCH, 0));
        assert_eq!(membership.members, descriptor.members);
        assert_eq!(membership.previous_membership_hash, None);
        assert_eq!(config.visibility, visibility);
        assert_eq!(config.compatibility, canonical.compatibility);
        storage.verify_snapshot(&snapshot).unwrap();
        swarm_core::verify_snapshot_signature(&snapshot).unwrap();
        swarm_core::verify_signature(
            epoch.authority_peer_id,
            epoch.authority_public_key,
            &epoch.signing_bytes().unwrap(),
            &epoch.signature,
        )
        .unwrap();
        swarm_core::verify_membership_signature(&membership).unwrap();
        swarm_core::verify_world_config_signature(&config).unwrap();
        assert!(
            establish_initial_epoch(&storage, &identity, &snapshot).is_err(),
            "published epoch must not be recreated"
        );
        assert_eq!(storage.load_epoch_record(world).unwrap(), epoch);
        assert!(
            !dir.0.join("control").join(world.to_hex()).exists(),
            "no runtime or hosting preference may be created"
        );
        assert_eq!(std::fs::read_dir(dir.0.join(".create-staging")).unwrap().count(), 0);
    }

    #[test]
    fn native_create_rejects_unqualified_tuples_before_catalog_or_storage_access() {
        for (minecraft, loader, expected) in [
            ("26.1.5", "0.19.3", "unqualified_runtime"),
            ("26.1.2", "0.20.1", "unqualified_runtime"),
            ("26.2", "0.19.3", "unsupported_runtime_adapter"),
            ("26.1.2", "0.19.2", "unsupported_runtime_adapter"),
        ] {
            let error = create_canonical_world(CreateCanonicalWorldRequest {
                name: "Matrix rejection fixture".into(),
                visibility: "private".into(),
                region: None,
                language: None,
                modpack: CanonicalizeModpackRequest {
                    minecraft_version: minecraft.into(),
                    loader_id: "fabric".into(),
                    loader_version: loader.into(),
                    packages: Vec::new(),
                    datapacks: Vec::new(),
                },
            })
            .unwrap_err();
            assert_eq!(error.code, expected);
        }
    }
}
