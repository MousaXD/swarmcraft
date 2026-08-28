use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::{fs, path::Path};
use swarm_protocol::{
    runtime_artifact_hash, ArtifactRequirementV1, ArtifactSideV1, CanonicalArtifactSourceV1, CanonicalDependencyKindV1,
    CanonicalDependencyV1, CanonicalHashAlgorithmV1, CanonicalLoaderV1, CanonicalModpackV1, CanonicalPackageIdentityV1,
    CanonicalPackageV1, CanonicalProviderArtifactV1, CanonicalProviderHashV1, CanonicalProviderV1,
    CanonicalRetrievalV1, RuntimeCompatibilityManifestV1, CANONICAL_MODPACK_SCHEMA_VERSION,
};

const MAX_CANONICAL_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalizeModpackRequest {
    pub minecraft_version: String,
    #[serde(default = "default_loader_id")]
    pub loader_id: String,
    pub loader_version: String,
    #[serde(default)]
    pub packages: Vec<CanonicalizePackageRequest>,
    #[serde(default)]
    pub datapacks: Vec<CanonicalizeDatapackRequest>,
}

fn default_loader_id() -> String {
    "fabric".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalizePackageRequest {
    pub artifact_id: String,
    pub version: String,
    pub side: String,
    pub artifact_path: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
    #[serde(default)]
    pub provider_hashes: Vec<ProviderHashRequest>,
    #[serde(default)]
    pub retrieval: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<DependencyRequest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalizeDatapackRequest {
    pub artifact_id: String,
    pub version: String,
    pub artifact_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHashRequest {
    pub algorithm: String,
    pub digest: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyRequest {
    pub kind: String,
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalModpackResponse {
    pub manifest: CanonicalModpackV1,
    pub compatibility: RuntimeCompatibilityManifestV1,
    pub compatibility_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalizationFailure {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
}

impl CanonicalizationFailure {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), artifact_id: None }
    }

    fn for_artifact(code: &str, artifact_id: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), artifact_id: Some(artifact_id.into()) }
    }
}

#[tauri::command]
pub fn canonicalize_modpack(
    request: CanonicalizeModpackRequest,
) -> Result<CanonicalModpackResponse, CanonicalizationFailure> {
    let mut packages = Vec::with_capacity(request.packages.len());
    for package in request.packages {
        packages.push(canonicalize_package(package)?);
    }

    let mut datapacks = Vec::with_capacity(request.datapacks.len());
    for datapack in request.datapacks {
        let bytes = read_artifact(&datapack.artifact_path, &datapack.artifact_id)?;
        datapacks.push(ArtifactRequirementV1 {
            artifact_id: datapack.artifact_id,
            version: datapack.version,
            artifact_hash: runtime_artifact_hash(&bytes),
            side: ArtifactSideV1::Server,
            provider_hint: None,
        });
    }

    let mut manifest = CanonicalModpackV1 {
        schema_version: CANONICAL_MODPACK_SCHEMA_VERSION,
        minecraft_version: request.minecraft_version,
        loader: CanonicalLoaderV1 { id: request.loader_id, version: request.loader_version },
        packages,
        datapacks,
    };
    manifest.normalize();
    manifest
        .validate()
        .map_err(|error| CanonicalizationFailure::new("canonical_validation_failed", error.to_string()))?;
    let compatibility = manifest
        .to_runtime_compatibility(env!("CARGO_PKG_VERSION"))
        .map_err(|error| CanonicalizationFailure::new("canonical_validation_failed", error.to_string()))?;
    let fingerprint = compatibility
        .fingerprint()
        .map_err(|error| CanonicalizationFailure::new("canonical_fingerprint_failed", error.to_string()))?;

    Ok(CanonicalModpackResponse { manifest, compatibility, compatibility_fingerprint: fingerprint.to_string() })
}

fn canonicalize_package(request: CanonicalizePackageRequest) -> Result<CanonicalPackageV1, CanonicalizationFailure> {
    let bytes = read_artifact(&request.artifact_path, &request.artifact_id)?;
    if let Some(expected) = request.file_size {
        if expected != bytes.len() as u64 {
            return Err(CanonicalizationFailure::for_artifact(
                "artifact_size_mismatch",
                &request.artifact_id,
                format!("provider declared {expected} bytes but exact artifact contains {} bytes", bytes.len()),
            ));
        }
    }

    let side = parse_side(&request.side).map_err(|message| {
        CanonicalizationFailure::for_artifact("invalid_environment", &request.artifact_id, message)
    })?;
    let source = match request.provider.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("local") => CanonicalArtifactSourceV1::Local {
            file_name: request.file_name.or_else(|| file_name(&request.artifact_path)),
        },
        Some(provider) => {
            let provider = parse_provider(provider).map_err(|message| {
                CanonicalizationFailure::for_artifact("unsupported_provider", &request.artifact_id, message)
            })?;
            let project_id = require_field(request.project_id, "project ID", &request.artifact_id)?;
            let version_id = require_field(request.version_id, "version/file ID", &request.artifact_id)?;
            let file_name = require_field(
                request.file_name.or_else(|| file_name(&request.artifact_path)),
                "artifact filename",
                &request.artifact_id,
            )?;
            let hashes = verify_provider_hashes(&bytes, &request.provider_hashes, &request.artifact_id)?;
            let retrieval =
                parse_retrieval(request.retrieval.as_deref().unwrap_or("provider_download")).map_err(|message| {
                    CanonicalizationFailure::for_artifact("invalid_retrieval_state", &request.artifact_id, message)
                })?;
            let dependencies =
                request.dependencies.into_iter().map(canonicalize_dependency).collect::<Result<Vec<_>, _>>()?;
            CanonicalArtifactSourceV1::Provider {
                artifact: CanonicalProviderArtifactV1 {
                    identity: CanonicalPackageIdentityV1 { provider, project_id, version_id },
                    file_name,
                    file_size: Some(bytes.len() as u64),
                    hashes,
                    retrieval,
                    dependencies,
                },
            }
        }
    };

    Ok(CanonicalPackageV1 {
        artifact_id: request.artifact_id,
        version: request.version,
        artifact_hash: runtime_artifact_hash(&bytes),
        side,
        source,
    })
}

fn canonicalize_dependency(request: DependencyRequest) -> Result<CanonicalDependencyV1, CanonicalizationFailure> {
    let provider = parse_provider(&request.provider)
        .map_err(|message| CanonicalizationFailure::new("invalid_dependency_provider", message))?;
    let kind = match request.kind.trim().to_ascii_lowercase().as_str() {
        "required" => CanonicalDependencyKindV1::Required,
        "optional" => CanonicalDependencyKindV1::Optional,
        "incompatible" => CanonicalDependencyKindV1::Incompatible,
        "embedded" => CanonicalDependencyKindV1::Embedded,
        other => {
            return Err(CanonicalizationFailure::new(
                "invalid_dependency_kind",
                format!("unsupported dependency kind {other}"),
            ))
        }
    };
    Ok(CanonicalDependencyV1 {
        kind,
        target: CanonicalPackageIdentityV1 { provider, project_id: request.project_id, version_id: request.version_id },
    })
}

fn verify_provider_hashes(
    bytes: &[u8],
    requested: &[ProviderHashRequest],
    artifact_id: &str,
) -> Result<Vec<CanonicalProviderHashV1>, CanonicalizationFailure> {
    if requested.is_empty() {
        return Err(CanonicalizationFailure::for_artifact(
            "provider_hash_missing",
            artifact_id,
            "provider-backed artifacts require provider verification hashes",
        ));
    }
    let mut result = Vec::with_capacity(requested.len());
    for requested_hash in requested {
        let algorithm = parse_hash_algorithm(&requested_hash.algorithm).map_err(|message| {
            CanonicalizationFailure::for_artifact("unsupported_hash_algorithm", artifact_id, message)
        })?;
        let expected = requested_hash.digest.trim().to_ascii_lowercase();
        let actual = match algorithm {
            CanonicalHashAlgorithmV1::Sha512 => hex::encode(Sha512::digest(bytes)),
            CanonicalHashAlgorithmV1::Sha256 => hex::encode(Sha256::digest(bytes)),
            CanonicalHashAlgorithmV1::Sha1 => hex::encode(Sha1::digest(bytes)),
            CanonicalHashAlgorithmV1::Md5 => hex::encode(Md5::digest(bytes)),
        };
        if actual != expected {
            return Err(CanonicalizationFailure::for_artifact(
                "provider_hash_mismatch",
                artifact_id,
                format!("{:?} verification failed for exact provider artifact", algorithm),
            ));
        }
        result.push(CanonicalProviderHashV1 { algorithm, digest_hex: actual });
    }
    result.sort();
    result.dedup();
    Ok(result)
}

fn read_artifact(path: &str, artifact_id: &str) -> Result<Vec<u8>, CanonicalizationFailure> {
    let metadata = fs::metadata(path).map_err(|error| {
        CanonicalizationFailure::for_artifact(
            "artifact_unavailable",
            artifact_id,
            format!("cannot inspect exact artifact: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(CanonicalizationFailure::for_artifact(
            "artifact_unavailable",
            artifact_id,
            "exact artifact path is not a regular file",
        ));
    }
    if metadata.len() == 0 || metadata.len() > MAX_CANONICAL_ARTIFACT_BYTES {
        return Err(CanonicalizationFailure::for_artifact(
            "artifact_size_invalid",
            artifact_id,
            format!("exact artifact size {} is outside the supported range", metadata.len()),
        ));
    }
    fs::read(path).map_err(|error| {
        CanonicalizationFailure::for_artifact(
            "artifact_unavailable",
            artifact_id,
            format!("cannot read exact artifact: {error}"),
        )
    })
}

fn file_name(path: &str) -> Option<String> {
    Path::new(path).file_name().and_then(|value| value.to_str()).map(str::to_owned)
}

fn require_field(value: Option<String>, label: &str, artifact_id: &str) -> Result<String, CanonicalizationFailure> {
    value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()).ok_or_else(|| {
        CanonicalizationFailure::for_artifact(
            "unresolved_provider_identity",
            artifact_id,
            format!("exact {label} is required"),
        )
    })
}

fn parse_provider(value: &str) -> Result<CanonicalProviderV1, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "modrinth" => Ok(CanonicalProviderV1::Modrinth),
        "curseforge" | "curse_forge" => Ok(CanonicalProviderV1::CurseForge),
        other => Err(format!("unsupported provider {other}")),
    }
}

fn parse_side(value: &str) -> Result<ArtifactSideV1, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "server" | "server_only" => Ok(ArtifactSideV1::Server),
        "client" | "client_only" => Ok(ArtifactSideV1::Client),
        "both" | "client_and_server" | "universal" => Ok(ArtifactSideV1::Both),
        other => Err(format!("unsupported package environment {other}")),
    }
}

fn parse_retrieval(value: &str) -> Result<CanonicalRetrievalV1, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "provider_download" | "provider" | "downloaded" => Ok(CanonicalRetrievalV1::ProviderDownload),
        "manual_required" | "manual_artifact_required" | "manual" => Ok(CanonicalRetrievalV1::ManualRequired),
        other => Err(format!("unsupported retrieval state {other}")),
    }
}

fn parse_hash_algorithm(value: &str) -> Result<CanonicalHashAlgorithmV1, String> {
    match value.trim().to_ascii_lowercase().replace('-', "").as_str() {
        "sha512" => Ok(CanonicalHashAlgorithmV1::Sha512),
        "sha256" => Ok(CanonicalHashAlgorithmV1::Sha256),
        "sha1" => Ok(CanonicalHashAlgorithmV1::Sha1),
        "md5" => Ok(CanonicalHashAlgorithmV1::Md5),
        other => Err(format!("unsupported provider hash algorithm {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn artifact_file(contents: &[u8]) -> String {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("swarmcraft-agent4-{nonce}.jar"));
        fs::write(&path, contents).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn provider_hash_mismatch_fails_closed() {
        let path = artifact_file(b"exact-bytes");
        let request = CanonicalizePackageRequest {
            artifact_id: "example".into(),
            version: "1.0.0".into(),
            side: "server".into(),
            artifact_path: path.clone(),
            provider: Some("modrinth".into()),
            project_id: Some("project".into()),
            version_id: Some("version".into()),
            file_name: Some("example.jar".into()),
            file_size: Some(11),
            provider_hashes: vec![ProviderHashRequest { algorithm: "sha256".into(), digest: "00".repeat(32) }],
            retrieval: None,
            dependencies: Vec::new(),
        };
        let error = canonicalize_package(request).unwrap_err();
        assert_eq!(error.code, "provider_hash_mismatch");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn exact_local_artifact_produces_runtime_hash() {
        let path = artifact_file(b"exact-local-bytes");
        let request = CanonicalizePackageRequest {
            artifact_id: "local".into(),
            version: "1.0.0".into(),
            side: "both".into(),
            artifact_path: path.clone(),
            provider: None,
            project_id: None,
            version_id: None,
            file_name: None,
            file_size: None,
            provider_hashes: Vec::new(),
            retrieval: None,
            dependencies: Vec::new(),
        };
        let package = canonicalize_package(request).unwrap();
        assert_eq!(package.artifact_hash, runtime_artifact_hash(b"exact-local-bytes"));
        let _ = fs::remove_file(path);
    }
}
