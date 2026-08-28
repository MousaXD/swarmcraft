use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

use super::{ArtifactRequirementV1, ArtifactSideV1, Hash32, RuntimeCompatibilityManifestV1, PROTOCOL_VERSION};

pub const CANONICAL_MODPACK_SCHEMA_VERSION: u16 = 1;
const CANONICAL_SOURCE_HINT_PREFIX: &str = "swarmcraft-canonical-source-v1:";
const RUNTIME_ARTIFACT_HASH_DOMAIN: &[u8] = b"swarmcraft/runtime-artifact/v1\0";

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum CanonicalModpackError {
    #[error("unsupported canonical manifest version: {0}")]
    UnsupportedManifestVersion(u16),
    #[error("{0} must be an exact, non-empty identifier")]
    AmbiguousVersion(String),
    #[error("invalid canonical package: {0}")]
    InvalidPackage(String),
    #[error("required dependency is unavailable: {0}")]
    MissingRequiredDependency(String),
    #[error("incompatible dependency is selected: {0}")]
    IncompatibleDependency(String),
    #[error("conflicting duplicate package: {0}")]
    DuplicatePackage(String),
    #[error("invalid provider hash: {0}")]
    InvalidProviderHash(String),
    #[error("canonical provider provenance is malformed: {0}")]
    MalformedProviderHint(String),
    #[error("canonical encoding failed: {0}")]
    Encode(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalProviderV1 {
    Modrinth,
    CurseForge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalDependencyKindV1 {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalHashAlgorithmV1 {
    Sha512,
    Sha256,
    Sha1,
    Md5,
}

impl CanonicalHashAlgorithmV1 {
    fn expected_hex_len(self) -> usize {
        match self {
            Self::Sha512 => 128,
            Self::Sha256 => 64,
            Self::Sha1 => 40,
            Self::Md5 => 32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalProviderHashV1 {
    pub algorithm: CanonicalHashAlgorithmV1,
    pub digest_hex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalRetrievalV1 {
    ProviderDownload,
    ManualRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalPackageIdentityV1 {
    pub provider: CanonicalProviderV1,
    pub project_id: String,
    /// Modrinth version ID or CurseForge exact file ID.
    pub version_id: String,
}

impl CanonicalPackageIdentityV1 {
    pub fn display_key(&self) -> String {
        format!("{:?}:{}/{}", self.provider, self.project_id, self.version_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalDependencyV1 {
    pub kind: CanonicalDependencyKindV1,
    pub target: CanonicalPackageIdentityV1,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalProviderArtifactV1 {
    pub identity: CanonicalPackageIdentityV1,
    pub file_name: String,
    pub file_size: Option<u64>,
    /// Provider-supplied verification metadata. Mutable URLs are intentionally excluded.
    pub hashes: Vec<CanonicalProviderHashV1>,
    pub retrieval: CanonicalRetrievalV1,
    /// Exact selected dependency targets only. Required dependency closure must be complete.
    pub dependencies: Vec<CanonicalDependencyV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanonicalArtifactSourceV1 {
    Provider {
        artifact: CanonicalProviderArtifactV1,
    },
    /// Exact local bytes, used by explicit local/imported JAR workflows where provider identity is unavailable.
    Local {
        file_name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalPackageV1 {
    /// Fabric mod ID or other stable runtime artifact ID.
    pub artifact_id: String,
    pub version: String,
    /// SwarmCraft domain-separated hash of the exact artifact bytes.
    pub artifact_hash: Hash32,
    pub side: ArtifactSideV1,
    pub source: CanonicalArtifactSourceV1,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalLoaderV1 {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalModpackV1 {
    pub schema_version: u16,
    pub minecraft_version: String,
    pub loader: CanonicalLoaderV1,
    pub packages: Vec<CanonicalPackageV1>,
    pub datapacks: Vec<ArtifactRequirementV1>,
}

impl CanonicalModpackV1 {
    pub fn normalize(&mut self) {
        self.minecraft_version = self.minecraft_version.trim().to_owned();
        self.loader.id = self.loader.id.trim().to_ascii_lowercase();
        self.loader.version = self.loader.version.trim().to_owned();
        for package in &mut self.packages {
            package.artifact_id = package.artifact_id.trim().to_owned();
            package.version = package.version.trim().to_owned();
            normalize_source(&mut package.source);
        }
        self.packages.sort();
        self.packages.dedup();
        normalize_artifact_requirements(&mut self.datapacks);
    }

    pub fn validate(&self) -> Result<(), CanonicalModpackError> {
        if self.schema_version != CANONICAL_MODPACK_SCHEMA_VERSION {
            return Err(CanonicalModpackError::UnsupportedManifestVersion(self.schema_version));
        }
        validate_exact_version("Minecraft version", &self.minecraft_version)?;
        validate_exact_identifier("loader ID", &self.loader.id)?;
        validate_exact_version("loader version", &self.loader.version)?;

        let mut selected_provider_identities = BTreeMap::<CanonicalPackageIdentityV1, &CanonicalPackageV1>::new();
        let mut runtime_keys = BTreeMap::<(String, String, Hash32, ArtifactSideV1), &CanonicalArtifactSourceV1>::new();

        for package in &self.packages {
            validate_exact_identifier("artifact ID", &package.artifact_id)?;
            validate_exact_version("artifact version", &package.version)?;
            let runtime_key =
                (package.artifact_id.clone(), package.version.clone(), package.artifact_hash, package.side);
            if let Some(existing) = runtime_keys.insert(runtime_key, &package.source) {
                if existing != &package.source {
                    return Err(CanonicalModpackError::DuplicatePackage(package.artifact_id.clone()));
                }
            }
            if let CanonicalArtifactSourceV1::Provider { artifact } = &package.source {
                validate_provider_artifact(artifact)?;
                if let Some(existing) = selected_provider_identities.insert(artifact.identity.clone(), package) {
                    if existing != package {
                        return Err(CanonicalModpackError::DuplicatePackage(artifact.identity.display_key()));
                    }
                }
            }
        }

        let selected: BTreeSet<_> = selected_provider_identities.keys().cloned().collect();
        for package in &self.packages {
            let CanonicalArtifactSourceV1::Provider { artifact } = &package.source else {
                continue;
            };
            for dependency in &artifact.dependencies {
                match dependency.kind {
                    CanonicalDependencyKindV1::Required => {
                        if !selected.contains(&dependency.target) {
                            return Err(CanonicalModpackError::MissingRequiredDependency(
                                dependency.target.display_key(),
                            ));
                        }
                    }
                    CanonicalDependencyKindV1::Incompatible => {
                        if selected.contains(&dependency.target) {
                            return Err(CanonicalModpackError::IncompatibleDependency(dependency.target.display_key()));
                        }
                    }
                    CanonicalDependencyKindV1::Optional | CanonicalDependencyKindV1::Embedded => {}
                }
            }
        }

        for datapack in &self.datapacks {
            validate_exact_identifier("datapack ID", &datapack.artifact_id)?;
            validate_exact_version("datapack version", &datapack.version)?;
        }
        Ok(())
    }

    pub fn to_runtime_compatibility(
        &self,
        fabric_adapter_version: impl Into<String>,
    ) -> Result<RuntimeCompatibilityManifestV1, CanonicalModpackError> {
        let mut canonical = self.clone();
        canonical.normalize();
        canonical.validate()?;

        let mut required_server_mods = Vec::new();
        let mut required_client_mods = Vec::new();
        for package in &canonical.packages {
            let requirement = ArtifactRequirementV1 {
                artifact_id: package.artifact_id.clone(),
                version: package.version.clone(),
                artifact_hash: package.artifact_hash,
                side: package.side,
                provider_hint: Some(encode_canonical_source(&package.source)?),
            };
            match package.side {
                ArtifactSideV1::Server => required_server_mods.push(requirement),
                ArtifactSideV1::Client => required_client_mods.push(requirement),
                ArtifactSideV1::Both => {
                    required_server_mods.push(requirement.clone());
                    required_client_mods.push(requirement);
                }
            }
        }

        let mut manifest = RuntimeCompatibilityManifestV1 {
            minecraft_version: canonical.minecraft_version,
            loader_id: canonical.loader.id,
            loader_version: canonical.loader.version,
            swarmcraft_protocol_version: PROTOCOL_VERSION,
            fabric_adapter_version: fabric_adapter_version.into(),
            required_server_mods,
            required_client_mods,
            datapacks: canonical.datapacks,
        };
        manifest.normalize();
        Ok(manifest)
    }

    /// Uses the existing authoritative compatibility fingerprint mechanism.
    pub fn compatibility_fingerprint(
        &self,
        fabric_adapter_version: impl Into<String>,
    ) -> Result<Hash32, CanonicalModpackError> {
        self.to_runtime_compatibility(fabric_adapter_version)?
            .fingerprint()
            .map_err(|error| CanonicalModpackError::Encode(error.to_string()))
    }

    pub fn from_runtime_compatibility(
        manifest: &RuntimeCompatibilityManifestV1,
    ) -> Result<Self, CanonicalModpackError> {
        let mut packages = BTreeMap::<(String, String, Hash32), CanonicalPackageV1>::new();
        for (required_side, values) in [
            (ArtifactSideV1::Server, &manifest.required_server_mods),
            (ArtifactSideV1::Client, &manifest.required_client_mods),
        ] {
            for requirement in values {
                let source = match requirement.provider_hint.as_deref() {
                    Some(hint) if hint.starts_with(CANONICAL_SOURCE_HINT_PREFIX) => decode_canonical_source(hint)?,
                    _ => CanonicalArtifactSourceV1::Local { file_name: None },
                };
                let key = (requirement.artifact_id.clone(), requirement.version.clone(), requirement.artifact_hash);
                let observed_side = match requirement.side {
                    ArtifactSideV1::Both => ArtifactSideV1::Both,
                    _ => required_side,
                };
                match packages.get_mut(&key) {
                    Some(existing) => {
                        if existing.source != source {
                            return Err(CanonicalModpackError::DuplicatePackage(requirement.artifact_id.clone()));
                        }
                        if existing.side != observed_side {
                            existing.side = ArtifactSideV1::Both;
                        }
                    }
                    None => {
                        packages.insert(
                            key,
                            CanonicalPackageV1 {
                                artifact_id: requirement.artifact_id.clone(),
                                version: requirement.version.clone(),
                                artifact_hash: requirement.artifact_hash,
                                side: observed_side,
                                source,
                            },
                        );
                    }
                }
            }
        }

        let mut result = Self {
            schema_version: CANONICAL_MODPACK_SCHEMA_VERSION,
            minecraft_version: manifest.minecraft_version.clone(),
            loader: CanonicalLoaderV1 { id: manifest.loader_id.clone(), version: manifest.loader_version.clone() },
            packages: packages.into_values().collect(),
            datapacks: manifest.datapacks.clone(),
        };
        result.normalize();
        result.validate()?;
        Ok(result)
    }
}

pub fn runtime_artifact_hash(bytes: &[u8]) -> Hash32 {
    Hash32::from_domain_bytes(RUNTIME_ARTIFACT_HASH_DOMAIN, bytes)
}

pub fn encode_canonical_source(source: &CanonicalArtifactSourceV1) -> Result<String, CanonicalModpackError> {
    let bytes = postcard::to_allocvec(source).map_err(|error| CanonicalModpackError::Encode(error.to_string()))?;
    Ok(format!("{CANONICAL_SOURCE_HINT_PREFIX}{}", hex::encode(bytes)))
}

pub fn decode_canonical_source(hint: &str) -> Result<CanonicalArtifactSourceV1, CanonicalModpackError> {
    let encoded = hint
        .strip_prefix(CANONICAL_SOURCE_HINT_PREFIX)
        .ok_or_else(|| CanonicalModpackError::MalformedProviderHint("unsupported prefix".into()))?;
    let bytes = hex::decode(encoded).map_err(|_| CanonicalModpackError::MalformedProviderHint("invalid hex".into()))?;
    postcard::from_bytes(&bytes).map_err(|error| CanonicalModpackError::MalformedProviderHint(error.to_string()))
}

fn normalize_source(source: &mut CanonicalArtifactSourceV1) {
    match source {
        CanonicalArtifactSourceV1::Provider { artifact } => {
            artifact.identity.project_id = artifact.identity.project_id.trim().to_owned();
            artifact.identity.version_id = artifact.identity.version_id.trim().to_owned();
            artifact.file_name = artifact.file_name.trim().to_owned();
            for hash in &mut artifact.hashes {
                hash.digest_hex = hash.digest_hex.trim().to_ascii_lowercase();
            }
            artifact.hashes.sort();
            artifact.hashes.dedup();
            for dependency in &mut artifact.dependencies {
                dependency.target.project_id = dependency.target.project_id.trim().to_owned();
                dependency.target.version_id = dependency.target.version_id.trim().to_owned();
            }
            artifact.dependencies.sort();
            artifact.dependencies.dedup();
        }
        CanonicalArtifactSourceV1::Local { file_name } => {
            if let Some(value) = file_name {
                *value = value.trim().to_owned();
                if value.is_empty() {
                    *file_name = None;
                }
            }
        }
    }
}

fn validate_provider_artifact(artifact: &CanonicalProviderArtifactV1) -> Result<(), CanonicalModpackError> {
    validate_exact_identifier("provider project ID", &artifact.identity.project_id)?;
    validate_exact_identifier("provider version/file ID", &artifact.identity.version_id)?;
    validate_exact_identifier("artifact filename", &artifact.file_name)?;
    if artifact.file_size == Some(0) {
        return Err(CanonicalModpackError::InvalidPackage("provider artifact size cannot be zero".into()));
    }
    if artifact.hashes.is_empty() {
        return Err(CanonicalModpackError::InvalidProviderHash(format!(
            "{} has no provider verification hashes",
            artifact.identity.display_key()
        )));
    }
    let mut algorithms = BTreeMap::new();
    for hash in &artifact.hashes {
        let digest = hash.digest_hex.trim();
        if digest.len() != hash.algorithm.expected_hex_len() || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CanonicalModpackError::InvalidProviderHash(format!(
                "{:?} digest for {}",
                hash.algorithm,
                artifact.identity.display_key()
            )));
        }
        if let Some(previous) = algorithms.insert(hash.algorithm, digest.to_ascii_lowercase()) {
            if previous != digest.to_ascii_lowercase() {
                return Err(CanonicalModpackError::InvalidProviderHash(format!(
                    "conflicting {:?} digests for {}",
                    hash.algorithm,
                    artifact.identity.display_key()
                )));
            }
        }
    }
    for dependency in &artifact.dependencies {
        validate_exact_identifier("dependency project ID", &dependency.target.project_id)?;
        validate_exact_identifier("dependency version/file ID", &dependency.target.version_id)?;
    }
    Ok(())
}

fn validate_exact_identifier(label: &str, value: &str) -> Result<(), CanonicalModpackError> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return Err(CanonicalModpackError::InvalidPackage(format!("{label} is empty or contains whitespace")));
    }
    Ok(())
}

fn validate_exact_version(label: &str, value: &str) -> Result<(), CanonicalModpackError> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || lower == "latest"
        || lower == "unknown"
        || value.chars().any(char::is_whitespace)
        || value.chars().any(|ch| matches!(ch, '*' | '^' | '~' | '<' | '>' | '=' | ',' | '|'))
    {
        return Err(CanonicalModpackError::AmbiguousVersion(label.to_owned()));
    }
    Ok(())
}

fn normalize_artifact_requirements(values: &mut Vec<ArtifactRequirementV1>) {
    values.sort_by(|a, b| {
        a.artifact_id
            .cmp(&b.artifact_id)
            .then(a.version.cmp(&b.version))
            .then(a.artifact_hash.cmp(&b.artifact_hash))
            .then(a.side.cmp(&b.side))
            .then(a.provider_hint.cmp(&b.provider_hint))
    });
    values.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(algorithm: CanonicalHashAlgorithmV1, byte: char) -> CanonicalProviderHashV1 {
        CanonicalProviderHashV1 {
            algorithm,
            digest_hex: std::iter::repeat_n(byte, algorithm.expected_hex_len()).collect(),
        }
    }

    fn identity(provider: CanonicalProviderV1, project: &str, version: &str) -> CanonicalPackageIdentityV1 {
        CanonicalPackageIdentityV1 { provider, project_id: project.into(), version_id: version.into() }
    }

    fn provider_package(
        artifact_id: &str,
        provider: CanonicalProviderV1,
        project: &str,
        version_id: &str,
        byte: u8,
        dependencies: Vec<CanonicalDependencyV1>,
    ) -> CanonicalPackageV1 {
        CanonicalPackageV1 {
            artifact_id: artifact_id.into(),
            version: format!("1.0.{byte}"),
            artifact_hash: Hash32([byte; 32]),
            side: ArtifactSideV1::Server,
            source: CanonicalArtifactSourceV1::Provider {
                artifact: CanonicalProviderArtifactV1 {
                    identity: identity(provider, project, version_id),
                    file_name: format!("{artifact_id}.jar"),
                    file_size: Some(10),
                    hashes: vec![hash(CanonicalHashAlgorithmV1::Sha256, 'a')],
                    retrieval: CanonicalRetrievalV1::ProviderDownload,
                    dependencies,
                },
            },
        }
    }

    fn pack(packages: Vec<CanonicalPackageV1>) -> CanonicalModpackV1 {
        CanonicalModpackV1 {
            schema_version: CANONICAL_MODPACK_SCHEMA_VERSION,
            minecraft_version: "1.21.8".into(),
            loader: CanonicalLoaderV1 { id: "fabric".into(), version: "0.17.2".into() },
            packages,
            datapacks: Vec::new(),
        }
    }

    #[test]
    fn canonical_order_and_fingerprint_ignore_provider_response_order() {
        let a = provider_package("alpha", CanonicalProviderV1::Modrinth, "a", "v1", 1, Vec::new());
        let b = provider_package("beta", CanonicalProviderV1::CurseForge, "2", "20", 2, Vec::new());
        let first = pack(vec![b.clone(), a.clone()]);
        let second = pack(vec![a, b]);
        assert_eq!(
            first.compatibility_fingerprint("0.4.0").unwrap(),
            second.compatibility_fingerprint("0.4.0").unwrap()
        );
        assert_eq!(first.to_runtime_compatibility("0.4.0").unwrap(), second.to_runtime_compatibility("0.4.0").unwrap());
    }

    #[test]
    fn modrinth_and_curseforge_provenance_round_trip_without_collapsing() {
        let modrinth = provider_package("same", CanonicalProviderV1::Modrinth, "same", "mr-v1", 1, Vec::new());
        let curseforge = provider_package("same", CanonicalProviderV1::CurseForge, "42", "420", 2, Vec::new());
        let original = pack(vec![modrinth, curseforge]);
        let runtime = original.to_runtime_compatibility("0.4.0").unwrap();
        let rebuilt = CanonicalModpackV1::from_runtime_compatibility(&runtime).unwrap();
        assert_eq!(rebuilt.packages.len(), 2);
        assert!(rebuilt.packages.iter().any(|entry| matches!(entry.source, CanonicalArtifactSourceV1::Provider { ref artifact } if artifact.identity.provider == CanonicalProviderV1::Modrinth)));
        assert!(rebuilt.packages.iter().any(|entry| matches!(entry.source, CanonicalArtifactSourceV1::Provider { ref artifact } if artifact.identity.provider == CanonicalProviderV1::CurseForge)));
    }

    #[test]
    fn direct_transitive_duplicate_and_cycle_safe_required_dependencies_validate() {
        let a_id = identity(CanonicalProviderV1::Modrinth, "a", "1");
        let b_id = identity(CanonicalProviderV1::Modrinth, "b", "1");
        let c_id = identity(CanonicalProviderV1::Modrinth, "c", "1");
        let req = |target| CanonicalDependencyV1 { kind: CanonicalDependencyKindV1::Required, target };
        let a = provider_package("a", CanonicalProviderV1::Modrinth, "a", "1", 1, vec![req(b_id.clone()), req(b_id)]);
        let b = provider_package("b", CanonicalProviderV1::Modrinth, "b", "1", 2, vec![req(c_id)]);
        let c = provider_package("c", CanonicalProviderV1::Modrinth, "c", "1", 3, vec![req(a_id)]);
        let mut canonical = pack(vec![c, a, b]);
        canonical.normalize();
        assert!(canonical.validate().is_ok());
    }

    #[test]
    fn missing_and_incompatible_required_dependencies_fail_closed() {
        let missing = identity(CanonicalProviderV1::Modrinth, "missing", "1");
        let required = CanonicalDependencyV1 { kind: CanonicalDependencyKindV1::Required, target: missing };
        let root = provider_package("root", CanonicalProviderV1::Modrinth, "root", "1", 1, vec![required]);
        assert!(matches!(pack(vec![root]).validate(), Err(CanonicalModpackError::MissingRequiredDependency(_))));

        let other_id = identity(CanonicalProviderV1::Modrinth, "other", "1");
        let incompatible = CanonicalDependencyV1 { kind: CanonicalDependencyKindV1::Incompatible, target: other_id };
        let root = provider_package("root", CanonicalProviderV1::Modrinth, "root", "1", 1, vec![incompatible]);
        let other = provider_package("other", CanonicalProviderV1::Modrinth, "other", "1", 2, Vec::new());
        assert!(matches!(pack(vec![root, other]).validate(), Err(CanonicalModpackError::IncompatibleDependency(_))));
    }

    #[test]
    fn artifact_change_changes_existing_authoritative_fingerprint() {
        let a = pack(vec![provider_package("a", CanonicalProviderV1::Modrinth, "a", "v1", 1, Vec::new())]);
        let b = pack(vec![provider_package("a", CanonicalProviderV1::Modrinth, "a", "v1", 2, Vec::new())]);
        assert_ne!(a.compatibility_fingerprint("0.4.0").unwrap(), b.compatibility_fingerprint("0.4.0").unwrap());
    }

    #[test]
    fn ambiguous_runtime_versions_are_rejected() {
        let mut canonical = pack(Vec::new());
        canonical.minecraft_version = "latest".into();
        assert!(matches!(canonical.validate(), Err(CanonicalModpackError::AmbiguousVersion(_))));
        canonical.minecraft_version = "1.21.8".into();
        canonical.loader.version = ">=0.17".into();
        assert!(matches!(canonical.validate(), Err(CanonicalModpackError::AmbiguousVersion(_))));
    }

    #[test]
    fn exact_manual_artifact_is_canonical_after_bytes_are_known() {
        let mut package = provider_package("cf", CanonicalProviderV1::CurseForge, "9", "99", 4, Vec::new());
        let CanonicalArtifactSourceV1::Provider { artifact } = &mut package.source else { unreachable!() };
        artifact.retrieval = CanonicalRetrievalV1::ManualRequired;
        assert!(pack(vec![package]).validate().is_ok());
    }

    #[test]
    fn runtime_hash_matches_existing_server_mod_domain_contract() {
        assert_eq!(
            runtime_artifact_hash(b"jar-bytes"),
            Hash32::from_domain_bytes(b"swarmcraft/runtime-artifact/v1\0", b"jar-bytes")
        );
    }
}
