use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::{
    env,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::DataPaths;
use swarm_protocol::{
    ArtifactSideV1, CanonicalArtifactSourceV1, CanonicalHashAlgorithmV1, CanonicalModpackV1,
    CanonicalPackageV1, CanonicalProviderArtifactV1, CanonicalProviderHashV1, CanonicalProviderV1,
    CanonicalRetrievalV1, RuntimeCompatibilityManifestV1, WorldId,
};

use crate::{
    package_provider::{modrinth::ModrinthClient, ModArtifactLocator, ModDownloadRequest, ProviderId},
    server_mods::{self, InstalledServerMod},
};

const MAX_PROVIDER_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
const CURSEFORGE_API_BASE: &str = "https://api.curseforge.com";
const CURSEFORGE_API_KEY_ENV: &str = "SWARMCRAFT_CURSEFORGE_API_KEY";

pub fn acquire_missing_server_mods(
    paths: &DataPaths,
    world: WorldId,
    manifest: &RuntimeCompatibilityManifestV1,
) -> Result<Vec<InstalledServerMod>> {
    let canonical = CanonicalModpackV1::from_runtime_compatibility(manifest)
        .context("cannot reconstruct exact canonical provider provenance from the signed runtime manifest")?;
    let readiness = server_mods::evaluate_world_mods(paths, world, manifest)?;
    let staging = StagingDir::new(paths, world)?;
    let mut installed = Vec::new();

    for package in canonical.packages.iter().filter(|package| {
        matches!(package.side, ArtifactSideV1::Server | ArtifactSideV1::Both)
    }) {
        if readiness.installed.iter().any(|candidate| installed_matches(candidate, package)) {
            continue;
        }
        if let Some(conflict) = readiness.installed.iter().find(|candidate| candidate.mod_id == package.artifact_id) {
            bail!(
                "installed mod {} is {}, but this world requires exact version {} with artifact hash {}; remove the incompatible local JAR before retrying runtime preparation",
                package.artifact_id,
                conflict.version,
                package.version,
                package.artifact_hash
            );
        }

        let CanonicalArtifactSourceV1::Provider { artifact } = &package.source else {
            bail!(
                "required mod {} {} is an exact local/imported artifact and is not installed; supply the original canonical JAR through the world Mods flow before runtime preparation",
                package.artifact_id,
                package.version
            );
        };
        if artifact.retrieval == CanonicalRetrievalV1::ManualRequired {
            bail!(
                "automatic download is not permitted for exact {:?} artifact {}/{} ({}); obtain that exact provider file and add it through the world Mods flow",
                artifact.identity.provider,
                artifact.identity.project_id,
                artifact.identity.version_id,
                artifact.file_name
            );
        }

        let downloaded = match artifact.identity.provider {
            CanonicalProviderV1::Modrinth => acquire_modrinth(artifact, staging.path())?,
            CanonicalProviderV1::CurseForge => acquire_curseforge(artifact, staging.path())?,
        };
        verify_canonical_hashes(&downloaded, &artifact.hashes)?;
        if let Some(expected) = artifact.file_size {
            let actual = fs::metadata(&downloaded)
                .with_context(|| format!("cannot inspect downloaded provider artifact {}", downloaded.display()))?
                .len();
            if actual != expected {
                bail!(
                    "provider artifact size mismatch for {}: canonical source requires {expected} bytes, downloaded {actual}",
                    artifact.file_name
                );
            }
        }
        installed.push(
            server_mods::add_local_mod(paths, world, manifest, &downloaded)
                .with_context(|| format!("downloaded provider artifact {} failed the signed world requirement", artifact.file_name))?,
        );
    }

    let final_readiness = server_mods::evaluate_world_mods(paths, world, manifest)?;
    if !final_readiness.ready {
        let details = final_readiness
            .issues
            .iter()
            .map(|issue| issue.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        bail!("server mod preparation is still incomplete after exact provider acquisition: {details}");
    }
    Ok(installed)
}

fn installed_matches(candidate: &InstalledServerMod, package: &CanonicalPackageV1) -> bool {
    candidate.mod_id == package.artifact_id
        && candidate.version == package.version
        && candidate.artifact_hash == package.artifact_hash.to_string()
}

fn acquire_modrinth(artifact: &CanonicalProviderArtifactV1, staging: &Path) -> Result<PathBuf> {
    let sha1 = canonical_hash(&artifact.hashes, CanonicalHashAlgorithmV1::Sha1);
    let sha512 = canonical_hash(&artifact.hashes, CanonicalHashAlgorithmV1::Sha512);
    if sha1.is_none() && sha512.is_none() {
        bail!(
            "exact Modrinth artifact {}/{} has no canonical SHA-1 or SHA-512 provider identity",
            artifact.identity.project_id,
            artifact.identity.version_id
        );
    }
    let client = ModrinthClient::production().context("cannot initialize Modrinth provider for runtime preparation")?;
    let downloaded = client
        .download(&ModDownloadRequest {
            locator: ModArtifactLocator {
                provider: ProviderId::Modrinth,
                project_id: artifact.identity.project_id.clone(),
                version_id: artifact.identity.version_id.clone(),
                sha1,
                sha512,
            },
            destination_dir: staging.to_path_buf(),
            max_bytes: Some(MAX_PROVIDER_ARTIFACT_BYTES),
        })
        .context("Modrinth could not provide the exact canonical artifact")?;
    if downloaded.filename != artifact.file_name {
        bail!(
            "Modrinth exact artifact filename changed for {}/{}: canonical source requires {}, provider returned {}",
            artifact.identity.project_id,
            artifact.identity.version_id,
            artifact.file_name,
            downloaded.filename
        );
    }
    Ok(downloaded.path)
}

fn acquire_curseforge(artifact: &CanonicalProviderArtifactV1, staging: &Path) -> Result<PathBuf> {
    let project_id = artifact
        .identity
        .project_id
        .parse::<u64>()
        .context("canonical CurseForge project ID is not numeric")?;
    let file_id = artifact
        .identity
        .version_id
        .parse::<u64>()
        .context("canonical CurseForge file ID is not numeric")?;
    safe_filename(&artifact.file_name)?;
    let api_key = env::var(CURSEFORGE_API_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "CurseForge runtime acquisition requires the machine-local {CURSEFORGE_API_KEY_ENV} environment variable"
            )
        })?;

    let (status, value) = curseforge_json(
        "POST",
        &format!("{CURSEFORGE_API_BASE}/v1/mods/files"),
        &api_key,
        Some(json!({ "fileIds": [file_id] })),
    )?;
    ensure_curseforge_status(status, "exact file metadata")?;
    let file = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .filter(|value| value.is_object())
        .ok_or_else(|| anyhow!("CurseForge exact file {file_id} is unavailable or removed"))?;
    validate_curseforge_file(file, project_id, file_id, artifact)?;

    let download_url = file
        .get("downloadUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| curseforge_download_url(project_id, file_id, &api_key).transpose().ok().flatten());
    let Some(download_url) = download_url else {
        bail!(
            "CurseForge does not permit automatic download of exact file {project_id}/{file_id}; obtain {} manually and add that exact artifact through the world Mods flow",
            artifact.file_name
        );
    };
    if !download_url.starts_with("https://") || download_url.chars().any(char::is_whitespace) {
        bail!("CurseForge returned an untrusted non-HTTPS artifact URL");
    }

    let destination = staging.join(&artifact.file_name);
    let status = curl_download(&download_url, &destination)?;
    if matches!(status, 403 | 404) {
        let _ = fs::remove_file(&destination);
        bail!(
            "CurseForge does not permit automatic download of exact file {project_id}/{file_id}; obtain {} manually and add that exact artifact through the world Mods flow",
            artifact.file_name
        );
    }
    if !(200..300).contains(&status) {
        let _ = fs::remove_file(&destination);
        bail!("CurseForge exact artifact download returned HTTP {status}");
    }
    Ok(destination)
}

fn curseforge_download_url(project_id: u64, file_id: u64, api_key: &str) -> Result<Option<String>> {
    let (status, value) = curseforge_json(
        "GET",
        &format!("{CURSEFORGE_API_BASE}/v1/mods/{project_id}/files/{file_id}/download-url"),
        api_key,
        None,
    )?;
    if matches!(status, 403 | 404) {
        return Ok(None);
    }
    ensure_curseforge_status(status, "download URL")?;
    Ok(value
        .get("data")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned))
}

fn validate_curseforge_file(
    file: &Value,
    project_id: u64,
    file_id: u64,
    artifact: &CanonicalProviderArtifactV1,
) -> Result<()> {
    let actual_file_id = file.get("id").and_then(Value::as_u64).ok_or_else(|| anyhow!("CurseForge file response omitted id"))?;
    let actual_project_id =
        file.get("modId").and_then(Value::as_u64).ok_or_else(|| anyhow!("CurseForge file response omitted modId"))?;
    if actual_file_id != file_id || actual_project_id != project_id {
        bail!("CurseForge returned a different project/file identity than the signed canonical requirement");
    }
    let file_name = file
        .get("fileName")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("CurseForge file response omitted fileName"))?;
    if file_name != artifact.file_name {
        bail!(
            "CurseForge exact artifact filename changed: canonical source requires {}, provider returned {file_name}",
            artifact.file_name
        );
    }
    if let Some(expected) = artifact.file_size {
        let actual = file
            .get("fileLength")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("CurseForge file response omitted fileLength"))?;
        if actual != expected {
            bail!("CurseForge provider metadata size changed for exact file {project_id}/{file_id}");
        }
    }

    for expected in &artifact.hashes {
        let algo = match expected.algorithm {
            CanonicalHashAlgorithmV1::Sha1 => Some(1),
            CanonicalHashAlgorithmV1::Md5 => Some(2),
            CanonicalHashAlgorithmV1::Sha256 | CanonicalHashAlgorithmV1::Sha512 => None,
        };
        let Some(algo) = algo else { continue };
        let provider_value = file
            .get("hashes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| entry.get("algo").and_then(Value::as_u64) == Some(algo))
            .and_then(|entry| entry.get("value"))
            .and_then(Value::as_str);
        if let Some(provider_value) = provider_value {
            if !provider_value.eq_ignore_ascii_case(&expected.digest_hex) {
                bail!("CurseForge provider hash metadata changed for exact file {project_id}/{file_id}");
            }
        }
    }
    Ok(())
}

fn canonical_hash(hashes: &[CanonicalProviderHashV1], algorithm: CanonicalHashAlgorithmV1) -> Option<String> {
    hashes.iter().find(|hash| hash.algorithm == algorithm).map(|hash| hash.digest_hex.clone())
}

fn verify_canonical_hashes(path: &Path, hashes: &[CanonicalProviderHashV1]) -> Result<()> {
    if hashes.is_empty() {
        bail!("provider-backed runtime artifact has no canonical provider hashes");
    }
    let mut file = File::open(path).with_context(|| format!("cannot read provider artifact {}", path.display()))?;
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        sha1.update(&buffer[..read]);
        sha256.update(&buffer[..read]);
        sha512.update(&buffer[..read]);
    }
    let actual_sha1 = hex::encode(sha1.finalize());
    let actual_sha256 = hex::encode(sha256.finalize());
    let actual_sha512 = hex::encode(sha512.finalize());
    let mut verified = 0usize;
    for hash in hashes {
        let actual = match hash.algorithm {
            CanonicalHashAlgorithmV1::Sha1 => Some(actual_sha1.as_str()),
            CanonicalHashAlgorithmV1::Sha256 => Some(actual_sha256.as_str()),
            CanonicalHashAlgorithmV1::Sha512 => Some(actual_sha512.as_str()),
            CanonicalHashAlgorithmV1::Md5 => None,
        };
        let Some(actual) = actual else { continue };
        verified += 1;
        if !actual.eq_ignore_ascii_case(&hash.digest_hex) {
            bail!("downloaded provider artifact failed canonical {:?} verification", hash.algorithm);
        }
    }
    if verified == 0 {
        bail!(
            "automatic runtime acquisition requires a canonical SHA-1, SHA-256, or SHA-512 provider hash; MD5-only artifacts must be supplied manually"
        );
    }
    Ok(())
}

fn curseforge_json(method: &str, url: &str, api_key: &str, body: Option<Value>) -> Result<(u16, Value)> {
    let output_path = temporary_path("curseforge-json");
    let mut command = Command::new("curl");
    command.args([
        "-sS",
        "-L",
        "--proto",
        "=https",
        "--connect-timeout",
        "15",
        "--max-time",
        "120",
        "-H",
        "Accept: application/json",
    ]);
    command.arg("-H").arg(format!("x-api-key: {api_key}"));
    if method == "POST" {
        command.args(["-X", "POST", "-H", "Content-Type: application/json"]);
        command.arg("--data").arg(serde_json::to_string(&body.unwrap_or(Value::Null))?);
    }
    let output = command
        .arg("-o")
        .arg(&output_path)
        .arg("--write-out")
        .arg("%{http_code}")
        .arg(url)
        .output()
        .with_context(|| format!("cannot start curl for CurseForge {method} request"))?;
    let body_bytes = fs::read(&output_path).unwrap_or_default();
    let _ = fs::remove_file(&output_path);
    if !output.status.success() {
        bail!("CurseForge request failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let status = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .context("curl returned an invalid CurseForge HTTP status")?;
    let value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).context("CurseForge returned malformed JSON")?
    };
    Ok((status, value))
}

fn ensure_curseforge_status(status: u16, operation: &str) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => bail!("CurseForge rejected {CURSEFORGE_API_KEY_ENV} while requesting {operation}"),
        404 => bail!("CurseForge {operation} is unavailable or removed"),
        429 => bail!("CurseForge rate limited runtime preparation while requesting {operation}; retry later"),
        500..=599 => bail!("CurseForge is unavailable while requesting {operation} (HTTP {status})"),
        _ => bail!("CurseForge request for {operation} failed with HTTP {status}"),
    }
}

fn curl_download(url: &str, destination: &Path) -> Result<u16> {
    let output = Command::new("curl")
        .args([
            "-sS",
            "-L",
            "--proto",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "900",
            "--max-filesize",
        ])
        .arg(MAX_PROVIDER_ARTIFACT_BYTES.to_string())
        .arg("-o")
        .arg(destination)
        .arg("--write-out")
        .arg("%{http_code}")
        .arg(url)
        .output()
        .context("cannot start curl for CurseForge artifact download")?;
    if !output.status.success() {
        let _ = fs::remove_file(destination);
        bail!("CurseForge artifact download failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .context("curl returned an invalid CurseForge artifact HTTP status")
}

fn safe_filename(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().count() != 1
        || !value.to_ascii_lowercase().ends_with(".jar")
    {
        bail!("provider artifact filename is not a safe JAR basename: {value}");
    }
    Ok(())
}

fn temporary_path(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    env::temp_dir().join(format!("swarmcraft-{prefix}-{}-{nonce}", std::process::id()))
}

struct StagingDir {
    path: PathBuf,
}

impl StagingDir {
    fn new(paths: &DataPaths, world: WorldId) -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let path = paths
            .root
            .join("provider-staging")
            .join(format!("runtime-{}-{}-{nonce}", world.to_hex(), std::process::id()));
        fs::create_dir_all(&path).with_context(|| format!("cannot create provider staging directory {}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{
        CanonicalPackageIdentityV1, CanonicalProviderHashV1, CanonicalRetrievalV1,
    };

    fn curseforge_artifact() -> CanonicalProviderArtifactV1 {
        CanonicalProviderArtifactV1 {
            identity: CanonicalPackageIdentityV1 {
                provider: CanonicalProviderV1::CurseForge,
                project_id: "123".into(),
                version_id: "456".into(),
            },
            file_name: "example.jar".into(),
            file_size: Some(3),
            hashes: vec![
                CanonicalProviderHashV1 {
                    algorithm: CanonicalHashAlgorithmV1::Sha1,
                    digest_hex: "a9993e364706816aba3e25717850c26c9cd0d89d".into(),
                },
                CanonicalProviderHashV1 {
                    algorithm: CanonicalHashAlgorithmV1::Md5,
                    digest_hex: "900150983cd24fb0d6963f7d28e17f72".into(),
                },
            ],
            retrieval: CanonicalRetrievalV1::ProviderDownload,
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn curseforge_exact_identity_and_provider_metadata_must_match() {
        let file = json!({
            "id": 456,
            "modId": 123,
            "fileName": "example.jar",
            "fileLength": 3,
            "hashes": [
                {"algo": 1, "value": "a9993e364706816aba3e25717850c26c9cd0d89d"},
                {"algo": 2, "value": "900150983cd24fb0d6963f7d28e17f72"}
            ]
        });
        validate_curseforge_file(&file, 123, 456, &curseforge_artifact()).unwrap();
        assert!(validate_curseforge_file(&file, 123, 457, &curseforge_artifact()).is_err());
    }

    #[test]
    fn strong_canonical_hash_verification_rejects_wrong_bytes() {
        let path = temporary_path("provider-runtime-test");
        fs::write(&path, b"abc").unwrap();
        let hashes = vec![CanonicalProviderHashV1 {
            algorithm: CanonicalHashAlgorithmV1::Sha1,
            digest_hex: "0000000000000000000000000000000000000000".into(),
        }];
        assert!(verify_canonical_hashes(&path, &hashes).is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn md5_only_provider_identity_requires_manual_runtime_remediation() {
        let path = temporary_path("provider-runtime-md5-test");
        fs::write(&path, b"abc").unwrap();
        let hashes = vec![CanonicalProviderHashV1 {
            algorithm: CanonicalHashAlgorithmV1::Md5,
            digest_hex: "900150983cd24fb0d6963f7d28e17f72".into(),
        }];
        let error = verify_canonical_hashes(&path, &hashes).unwrap_err().to_string();
        assert!(error.contains("MD5-only"));
        let _ = fs::remove_file(path);
    }
}
