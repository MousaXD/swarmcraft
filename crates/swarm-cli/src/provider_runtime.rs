use anyhow::{anyhow, bail, Context, Result};
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde_json::{json, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::DataPaths;
use swarm_protocol::{
    ArtifactSideV1, CanonicalArtifactSourceV1, CanonicalHashAlgorithmV1, CanonicalModpackV1, CanonicalPackageV1,
    CanonicalProviderArtifactV1, CanonicalProviderHashV1, CanonicalProviderV1, CanonicalRetrievalV1,
    RuntimeCompatibilityManifestV1, WorldId,
};

use crate::{
    package_provider::{modrinth::ModrinthClient, ModArtifactLocator, ModDownloadRequest, ProviderId},
    server_mods::{self, InstalledServerMod},
};

const MAX_PROVIDER_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
const CURSEFORGE_API_BASE: &str = "https://api.curseforge.com";
const CURSEFORGE_API_KEY_ENV: &str = "SWARMCRAFT_CURSEFORGE_API_KEY";
const MAX_PROVIDER_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROVIDER_METADATA_HEADERS: usize = 128;
const MAX_PROVIDER_METADATA_HEADER_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_METADATA_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_PROVIDER_METADATA_DEPTH: usize = 32;
const MAX_PROVIDER_METADATA_ARRAY_ITEMS: usize = 2048;
const MAX_PROVIDER_METADATA_OBJECT_ENTRIES: usize = 512;
const MAX_PROVIDER_METADATA_STRING_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_METADATA_NODES: usize = 50_000;

/// Acquire only the exact missing server-side artifacts encoded into the signed
/// compatibility manifest. Provider metadata is a retrieval locator; the final
/// publication gate remains the signed Fabric mod id/version/runtime hash.
pub fn acquire_missing_server_mods(
    paths: &DataPaths,
    world: WorldId,
    manifest: &RuntimeCompatibilityManifestV1,
) -> Result<Vec<InstalledServerMod>> {
    let canonical = CanonicalModpackV1::from_runtime_compatibility(manifest)
        .context("cannot reconstruct exact canonical provider provenance from the signed runtime manifest")?;
    let readiness = server_mods::evaluate_world_mods(paths, world, manifest)?;
    let staging = StagingDir::new(paths, world)?;
    let mut added = Vec::new();

    for package in canonical
        .packages
        .iter()
        .filter(|package| matches!(package.side, ArtifactSideV1::Server | ArtifactSideV1::Both))
    {
        if readiness.installed.iter().any(|candidate| installed_matches(candidate, package)) {
            continue;
        }
        if let Some(conflict) = readiness.installed.iter().find(|candidate| candidate.mod_id == package.artifact_id) {
            bail!(
                "installed mod {} is version {} but this world requires exact version {} with artifact hash {}; remove the incompatible local JAR before retrying runtime preparation",
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
        added.push(server_mods::add_local_mod(paths, world, manifest, &downloaded).with_context(|| {
            format!("downloaded provider artifact {} failed the signed world requirement", artifact.file_name)
        })?);
    }

    let final_readiness = server_mods::evaluate_world_mods(paths, world, manifest)?;
    if !final_readiness.ready {
        let details = final_readiness.issues.iter().map(|issue| issue.message.as_str()).collect::<Vec<_>>().join("; ");
        bail!("server mod preparation is still incomplete after exact provider acquisition: {details}");
    }
    Ok(added)
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
    let project_id =
        artifact.identity.project_id.parse::<u64>().context("canonical CurseForge project ID is not numeric")?;
    let file_id = artifact.identity.version_id.parse::<u64>().context("canonical CurseForge file ID is not numeric")?;
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
    validate_api_key(&api_key)?;

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

    let download_url = if let Some(url) =
        file.get("downloadUrl").and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
    {
        Some(url.to_owned())
    } else {
        curseforge_download_url(project_id, file_id, &api_key)?
    };
    let Some(download_url) = download_url else {
        bail!(
            "CurseForge does not permit automatic download of exact file {project_id}/{file_id}; obtain {} manually and add that exact artifact through the world Mods flow",
            artifact.file_name
        );
    };
    validate_curseforge_artifact_url(&download_url)?;

    let destination = staging.join(&artifact.file_name);
    let status = download_curseforge_artifact(&download_url, &destination)?;
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
    let actual_file_id =
        file.get("id").and_then(Value::as_u64).ok_or_else(|| anyhow!("CurseForge file response omitted id"))?;
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
    safe_filename(file_name)?;
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

fn validate_api_key(api_key: &str) -> Result<()> {
    if api_key.is_empty() || api_key.chars().any(char::is_control) {
        bail!("{CURSEFORGE_API_KEY_ENV} contains invalid control characters");
    }
    Ok(())
}

fn is_curseforge_api_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| host.eq_ignore_ascii_case("api.curseforge.com"))
}

fn is_curseforge_artifact_url(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && (host == "forgecdn.net" || host.ends_with(".forgecdn.net"))
}

fn validate_curseforge_api_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url).context("CurseForge API URL is invalid")?;
    if !is_curseforge_api_url(&parsed) {
        bail!("CurseForge authenticated API URL left the exact api.curseforge.com origin");
    }
    Ok(parsed)
}

fn validate_curseforge_artifact_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url).context("CurseForge artifact URL is invalid")?;
    if !is_curseforge_artifact_url(&parsed) {
        bail!("CurseForge artifact URL left the HTTPS forgecdn.net trust boundary");
    }
    Ok(parsed)
}

fn curseforge_api_client() -> Result<Client> {
    Client::builder()
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many CurseForge API redirects")
            } else if is_curseforge_api_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("authenticated CurseForge API redirect left api.curseforge.com")
            }
        }))
        .user_agent(concat!("SwarmCraft/", env!("CARGO_PKG_VERSION"), " RuntimeCurseForge"))
        .build()
        .context("cannot initialize authenticated CurseForge HTTP client")
}

fn curseforge_artifact_client() -> Result<Client> {
    Client::builder()
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(900))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many CurseForge artifact redirects")
            } else if is_curseforge_artifact_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("CurseForge artifact redirect left the Forge CDN trust boundary")
            }
        }))
        .user_agent(concat!("SwarmCraft/", env!("CARGO_PKG_VERSION"), " RuntimeCurseForge"))
        .build()
        .context("cannot initialize CurseForge artifact HTTP client")
}

fn validate_metadata_headers(headers: &reqwest::header::HeaderMap) -> Result<()> {
    if headers.len() > MAX_PROVIDER_METADATA_HEADERS {
        bail!("response_too_large: CurseForge returned too many metadata headers");
    }
    let mut total = 0usize;
    for (name, value) in headers {
        total = total.saturating_add(name.as_str().len()).saturating_add(value.as_bytes().len());
        if value.as_bytes().len() > MAX_PROVIDER_METADATA_HEADER_VALUE_BYTES
            || total > MAX_PROVIDER_METADATA_HEADER_BYTES
        {
            bail!("response_too_large: CurseForge metadata headers exceeded their byte budget");
        }
    }
    Ok(())
}

fn validate_metadata_value(value: &Value) -> Result<()> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<()> {
        if depth > MAX_PROVIDER_METADATA_DEPTH {
            bail!("response_too_large: CurseForge metadata nesting is too deep");
        }
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_PROVIDER_METADATA_NODES {
            bail!("response_too_large: CurseForge metadata contains too many values");
        }
        match value {
            Value::String(text) if text.len() > MAX_PROVIDER_METADATA_STRING_BYTES => {
                bail!("response_too_large: CurseForge metadata string is too large")
            }
            Value::Array(items) => {
                if items.len() > MAX_PROVIDER_METADATA_ARRAY_ITEMS {
                    bail!("response_too_large: CurseForge metadata array is too large");
                }
                for item in items {
                    visit(item, depth + 1, nodes)?;
                }
            }
            Value::Object(entries) => {
                if entries.len() > MAX_PROVIDER_METADATA_OBJECT_ENTRIES {
                    bail!("response_too_large: CurseForge metadata object has too many fields");
                }
                for (key, item) in entries {
                    if key.len() > 256 {
                        bail!("response_too_large: CurseForge metadata key is too large");
                    }
                    visit(item, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut nodes = 0usize;
    visit(value, 0, &mut nodes)
}

fn read_curseforge_metadata(response: reqwest::blocking::Response) -> Result<Value> {
    validate_metadata_headers(response.headers())?;
    if response.content_length().is_some_and(|length| length > MAX_PROVIDER_METADATA_BYTES as u64) {
        bail!("response_too_large: CurseForge metadata Content-Length exceeded the response bound");
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_PROVIDER_METADATA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("cannot read CurseForge metadata response")?;
    if bytes.len() > MAX_PROVIDER_METADATA_BYTES {
        bail!("response_too_large: CurseForge metadata exceeded the response byte bound");
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let value: Value = serde_json::from_slice(&bytes).context("CurseForge returned malformed JSON")?;
    validate_metadata_value(&value)?;
    Ok(value)
}

fn curseforge_json(method: &str, url: &str, api_key: &str, body: Option<Value>) -> Result<(u16, Value)> {
    validate_api_key(api_key)?;
    let url = validate_curseforge_api_url(url)?;
    let client = curseforge_api_client()?;
    let request = match method {
        "GET" => client.get(url),
        "POST" => {
            let body = serde_json::to_string(&body.unwrap_or(Value::Null))?;
            client.post(url).header("Content-Type", "application/json").body(body)
        }
        _ => bail!("unsupported CurseForge runtime HTTP method: {method}"),
    };
    let response = request
        .header("Accept", "application/json")
        .header("x-api-key", api_key)
        .send()
        .with_context(|| format!("CurseForge {method} request failed"))?;
    let status = response.status().as_u16();
    let value = if (200..300).contains(&status) {
        read_curseforge_metadata(response)?
    } else {
        validate_metadata_headers(response.headers())?;
        Value::Null
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

fn download_curseforge_artifact(url: &str, destination: &Path) -> Result<u16> {
    let url = validate_curseforge_artifact_url(url)?;
    let client = curseforge_artifact_client()?;
    let mut response = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .context("CurseForge artifact download failed")?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Ok(status);
    }
    if response.content_length().is_some_and(|length| length > MAX_PROVIDER_ARTIFACT_BYTES) {
        bail!("CurseForge artifact exceeded the provider download byte bound");
    }
    let result = (|| -> Result<()> {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .with_context(|| format!("cannot create provider artifact {}", destination.display()))?;
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = response.read(&mut buffer).context("CurseForge artifact stream failed")?;
            if read == 0 {
                break;
            }
            total = total.checked_add(read as u64).ok_or_else(|| anyhow!("CurseForge artifact size overflow"))?;
            if total > MAX_PROVIDER_ARTIFACT_BYTES {
                bail!("CurseForge artifact exceeded the provider download byte bound");
            }
            output.write_all(&buffer[..read]).context("cannot write CurseForge provider artifact")?;
        }
        output.sync_all().context("cannot sync CurseForge provider artifact")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result?;
    Ok(status)
}

fn safe_filename(value: &str) -> Result<()> {
    let path = Path::new(value);
    let stem = value.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let windows_reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    let portable = value.len() > 4
        && value == value.trim()
        && value.len() <= 255
        && !path.is_absolute()
        && path.components().count() == 1
        && !value.contains(['/', '\\', ':', '\0'])
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
        && !windows_reserved
        && value.to_ascii_lowercase().ends_with(".jar");
    if !portable {
        bail!("provider artifact filename is not a safe portable JAR basename: {value}");
    }
    Ok(())
}

#[cfg(test)]
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
        let path = paths.root.join("provider-staging").join(format!(
            "runtime-{}-{}-{nonce}",
            world.to_hex(),
            std::process::id()
        ));
        fs::create_dir_all(&path)
            .with_context(|| format!("cannot create provider staging directory {}", path.display()))?;
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
    use swarm_protocol::CanonicalPackageIdentityV1;

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

#[cfg(test)]
mod agent5_runtime_http_security_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn curseforge_runtime_origins_reject_second_origin_and_private_targets() {
        assert!(validate_curseforge_api_url("https://api.curseforge.com/v1/mods/files").is_ok());
        assert!(validate_curseforge_api_url("https://attacker.invalid/steal").is_err());
        assert!(validate_curseforge_artifact_url("https://mediafilez.forgecdn.net/files/example.jar").is_ok());
        assert!(validate_curseforge_artifact_url("https://127.0.0.1/example.jar").is_err());
        assert!(validate_curseforge_artifact_url("https://api.curseforge.com/example.jar").is_err());
    }

    #[test]
    fn runtime_metadata_and_filename_limits_fail_closed() {
        let huge = json!({"text": "x".repeat(MAX_PROVIDER_METADATA_STRING_BYTES + 1)});
        assert!(validate_metadata_value(&huge).unwrap_err().to_string().contains("response_too_large"));
        for invalid in ["../evil.jar", "..\\evil.jar", "C:\\evil.jar", "\\\\server\\evil.jar", "NUL.jar"] {
            assert!(safe_filename(invalid).is_err(), "accepted {invalid}");
        }
        assert!(safe_filename("safe-runtime.jar").is_ok());
    }

    #[test]
    fn runtime_api_key_rejects_header_injection() {
        assert!(validate_api_key("secret").is_ok());
        assert!(validate_api_key("secret\nforwarded: value").is_err());
    }
}
