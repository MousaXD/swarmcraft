use swarm_protocol::{ArtifactRequirementV1, ArtifactSideV1, Hash32, RuntimeCompatibilityManifestV1, PROTOCOL_VERSION};

fn artifact(id: &str, hash_byte: u8, provider_hint: Option<&str>) -> ArtifactRequirementV1 {
    ArtifactRequirementV1 {
        artifact_id: id.into(),
        version: "1.0.0".into(),
        artifact_hash: Hash32([hash_byte; 32]),
        side: ArtifactSideV1::Server,
        provider_hint: provider_hint.map(str::to_owned),
    }
}

fn compatibility(required_server_mods: Vec<ArtifactRequirementV1>) -> RuntimeCompatibilityManifestV1 {
    RuntimeCompatibilityManifestV1 {
        minecraft_version: "1.21.8".into(),
        loader_id: "fabric".into(),
        loader_version: "0.17.2".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: "0.2.0".into(),
        required_server_mods,
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    }
}

#[test]
fn canonical_runtime_permutations_have_one_fingerprint() {
    let a =
        compatibility(vec![artifact("zeta", 9, Some("modrinth:zeta")), artifact("alpha", 1, Some("modrinth:alpha"))]);
    let b =
        compatibility(vec![artifact("alpha", 1, Some("modrinth:alpha")), artifact("zeta", 9, Some("modrinth:zeta"))]);
    assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
}

#[test]
fn provider_hint_is_canonical_signed_compatibility_material() {
    let a = compatibility(vec![artifact("example", 1, Some("modrinth:first"))]);
    let b = compatibility(vec![artifact("example", 1, Some("modrinth:second"))]);
    assert_ne!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
}

#[test]
fn duplicate_runtime_identity_is_rejected_even_when_provider_hint_differs() {
    let duplicate = compatibility(vec![
        artifact("example", 1, Some("modrinth:first")),
        artifact("example", 1, Some("modrinth:second")),
    ]);
    assert!(duplicate.validate_semantics().is_err());
    assert!(duplicate.fingerprint().is_err());
}

#[test]
fn exact_duplicate_runtime_identity_is_rejected_instead_of_silently_deduplicated() {
    let item = artifact("example", 1, Some("modrinth:example"));
    let duplicate = compatibility(vec![item.clone(), item]);
    assert!(duplicate.validate_semantics().is_err());
    assert!(duplicate.fingerprint().is_err());
}
