use md5::Md5;
use reqwest::{redirect::Policy, StatusCode};
use serde_json::{json, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncWriteExt;

use super::launcher_commands::resolve_provider_staging_session;

const API_BASE: &str = "https://api.curseforge.com";
const API_KEY_ENV: &str = "SWARMCRAFT_CURSEFORGE_API_KEY";
const MINECRAFT_GAME_ID: u64 = 432;
const MINECRAFT_MODS_CLASS_ID: u64 = 6;
const FABRIC_MOD_LOADER_TYPE: u64 = 4;
const MAX_PAGE_SIZE: u64 = 50;
const MAX_SEARCH_INDEX: u64 = 10_000;
const MAX_DEPENDENCY_PACKAGES: usize = 128;
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_METADATA_HEADERS: usize = 128;
const MAX_METADATA_HEADER_BYTES: usize = 64 * 1024;
const MAX_METADATA_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_METADATA_DEPTH: usize = 32;
const MAX_METADATA_ARRAY_ITEMS: usize = 2048;
const MAX_METADATA_OBJECT_ENTRIES: usize = 512;
const MAX_METADATA_STRING_BYTES: usize = 64 * 1024;
const MAX_METADATA_NODES: usize = 50_000;

#[derive(Debug, Clone)]
struct ProviderError {
    status: &'static str,
    code: &'static str,
    message: String,
    retry_after_seconds: Option<u64>,
}

impl ProviderError {
    fn new(status: &'static str, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), retry_after_seconds: None }
    }

    fn retry_after(mut self, seconds: Option<u64>) -> Self {
        self.retry_after_seconds = seconds;
        self
    }

    fn into_response(self) -> Value {
        json!({
            "status": self.status,
            "provider": "curseforge",
            "error": {
                "code": self.code,
                "message": self.message,
                "retry_after_seconds": self.retry_after_seconds,
            }
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum MissingResource {
    Project,
    File,
    Generic,
}

#[derive(Debug, Clone)]
struct Target {
    minecraft: String,
    loader: String,
    environment: String,
}

impl Target {
    fn parse(minecraft: String, loader: String, environment: String) -> Result<Self, ProviderError> {
        let minecraft = minecraft.trim().to_owned();
        if minecraft.is_empty() {
            return Err(ProviderError::new(
                "invalid_request",
                "minecraft_version_required",
                "Minecraft version is required",
            ));
        }
        let loader = loader.trim().to_ascii_lowercase();
        if loader != "fabric" {
            return Err(ProviderError::new(
                "incompatible",
                "unsupported_loader",
                "The CurseForge provider currently resolves Fabric mods only",
            ));
        }
        let environment = environment.trim().to_ascii_lowercase();
        if !matches!(environment.as_str(), "server" | "client" | "both") {
            return Err(ProviderError::new(
                "invalid_request",
                "invalid_environment",
                "Environment must be server, client, or both",
            ));
        }
        Ok(Self { minecraft, loader, environment })
    }
}

struct CurseForgeClient {
    api_http: reqwest::Client,
    artifact_http: reqwest::Client,
    api_key: Option<String>,
}

impl CurseForgeClient {
    fn from_environment() -> Result<Self, ProviderError> {
        let common = || {
            reqwest::Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .user_agent(concat!("SwarmCraft/", env!("CARGO_PKG_VERSION"), " CurseForgeProvider"))
        };
        let api_http = common()
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many CurseForge API redirects")
                } else if is_curseforge_api_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("authenticated CurseForge API redirect left api.curseforge.com")
                }
            }))
            .build()
            .map_err(|error| {
                ProviderError::new(
                    "unavailable",
                    "provider_initialization_failed",
                    format!("Could not initialize authenticated CurseForge HTTP client: {error}"),
                )
            })?;
        let artifact_http = common()
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many CurseForge artifact redirects")
                } else if is_curseforge_artifact_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("CurseForge artifact redirect left the Forge CDN trust boundary")
                }
            }))
            .build()
            .map_err(|error| {
                ProviderError::new(
                    "unavailable",
                    "provider_initialization_failed",
                    format!("Could not initialize CurseForge artifact HTTP client: {error}"),
                )
            })?;
        Ok(Self { api_http, artifact_http, api_key: normalize_api_key(env::var(API_KEY_ENV).ok()) })
    }

    fn require_api_key(&self) -> Result<&str, ProviderError> {
        self.api_key.as_deref().ok_or_else(|| {
            ProviderError::new(
                "configuration_required",
                "missing_api_credential",
                format!("CurseForge browsing requires the machine-local {API_KEY_ENV} environment variable"),
            )
        })
    }

    async fn get_json(
        &self,
        path: &str,
        query: &[(&str, String)],
        missing: MissingResource,
    ) -> Result<Value, ProviderError> {
        let key = self.require_api_key()?;
        let response = self
            .api_http
            .get(format!("{API_BASE}{path}"))
            .header("Accept", "application/json")
            .header("x-api-key", key)
            .query(query)
            .send()
            .await
            .map_err(map_request_error)?;
        parse_json_response(response, missing).await
    }

    async fn post_json(&self, path: &str, body: Value, missing: MissingResource) -> Result<Value, ProviderError> {
        let key = self.require_api_key()?;
        let response = self
            .api_http
            .post(format!("{API_BASE}{path}"))
            .header("Accept", "application/json")
            .header("x-api-key", key)
            .json(&body)
            .send()
            .await
            .map_err(map_request_error)?;
        parse_json_response(response, missing).await
    }

    async fn fetch_project(&self, project_id: u64) -> Result<Value, ProviderError> {
        let value = self.get_json(&format!("/v1/mods/{project_id}"), &[], MissingResource::Project).await?;
        value.get("data").filter(|value| value.is_object()).cloned().ok_or_else(malformed_response)
    }

    async fn fetch_file(&self, file_id: u64) -> Result<Value, ProviderError> {
        let value = self.post_json("/v1/mods/files", json!({ "fileIds": [file_id] }), MissingResource::File).await?;
        value
            .get("data")
            .and_then(Value::as_array)
            .and_then(|files| files.first())
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| {
                ProviderError::new(
                    "not_found",
                    "removed_file",
                    format!("CurseForge file {file_id} is unavailable or has been removed"),
                )
            })
    }

    async fn files_for_project(
        &self,
        project_id: u64,
        minecraft: &str,
        fabric_only: bool,
    ) -> Result<Vec<Value>, ProviderError> {
        let mut query = vec![
            ("gameVersion", minecraft.to_owned()),
            ("pageSize", MAX_PAGE_SIZE.to_string()),
            ("index", "0".to_owned()),
        ];
        if fabric_only {
            query.push(("modLoaderType", FABRIC_MOD_LOADER_TYPE.to_string()));
        }
        let value = self.get_json(&format!("/v1/mods/{project_id}/files"), &query, MissingResource::Project).await?;
        let files = value.get("data").and_then(Value::as_array).ok_or_else(malformed_response)?;
        Ok(files.clone())
    }

    async fn compatible_files(&self, project_id: u64, target: &Target) -> Result<Vec<Value>, ProviderError> {
        let exact = self.files_for_project(project_id, &target.minecraft, true).await?;
        if !exact.is_empty() {
            return Ok(exact);
        }
        let game_version = self.files_for_project(project_id, &target.minecraft, false).await?;
        Err(classify_version_gap(project_id, &target.minecraft, &game_version))
    }

    async fn download_url(&self, file: &Value) -> Result<Option<String>, ProviderError> {
        if let Some(url) = nonempty_string(file.get("downloadUrl")) {
            return validate_download_url(url).map(Some);
        }
        let project_id = required_u64(file, "modId")?;
        let file_id = required_u64(file, "id")?;
        let key = self.require_api_key()?;
        let response = self
            .api_http
            .get(format!("{API_BASE}/v1/mods/{project_id}/files/{file_id}/download-url"))
            .header("Accept", "application/json")
            .header("x-api-key", key)
            .send()
            .await
            .map_err(map_request_error)?;
        if matches!(response.status(), StatusCode::FORBIDDEN | StatusCode::NOT_FOUND) {
            return Ok(None);
        }
        let value = parse_json_response(response, MissingResource::File).await?;
        match value.get("data").and_then(Value::as_str) {
            Some(url) if !url.trim().is_empty() => validate_download_url(url).map(Some),
            _ => Ok(None),
        }
    }
}

fn normalize_api_key(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
}

fn is_curseforge_api_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| host.eq_ignore_ascii_case("api.curseforge.com"))
}

fn is_curseforge_artifact_url(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && (host == "forgecdn.net" || host.ends_with(".forgecdn.net"))
}

fn map_request_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::new("unavailable", "timeout", "The CurseForge API request timed out")
    } else {
        ProviderError::new("unavailable", "provider_unavailable", format!("CurseForge could not be reached: {error}"))
    }
}

fn map_download_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::new("unavailable", "timeout", "The CurseForge artifact download timed out")
    } else {
        ProviderError::new(
            "download_failed",
            "interrupted_download",
            format!("CurseForge artifact download was interrupted: {error}"),
        )
    }
}

fn metadata_limit_error(message: impl Into<String>) -> ProviderError {
    ProviderError::new("error", "response_too_large", message)
}

fn validate_metadata_headers(headers: &reqwest::header::HeaderMap) -> Result<(), ProviderError> {
    if headers.len() > MAX_METADATA_HEADERS {
        return Err(metadata_limit_error("CurseForge returned too many metadata response headers"));
    }
    let mut total = 0usize;
    for (name, value) in headers {
        total = total.saturating_add(name.as_str().len()).saturating_add(value.as_bytes().len());
        if value.as_bytes().len() > MAX_METADATA_HEADER_VALUE_BYTES || total > MAX_METADATA_HEADER_BYTES {
            return Err(metadata_limit_error("CurseForge metadata response headers exceeded their byte budget"));
        }
    }
    Ok(())
}

fn validate_metadata_value(value: &Value) -> Result<(), ProviderError> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), ProviderError> {
        if depth > MAX_METADATA_DEPTH {
            return Err(metadata_limit_error("CurseForge metadata nesting is too deep"));
        }
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_METADATA_NODES {
            return Err(metadata_limit_error("CurseForge metadata contains too many values"));
        }
        match value {
            Value::String(text) if text.len() > MAX_METADATA_STRING_BYTES => {
                Err(metadata_limit_error("CurseForge metadata string is too large"))
            }
            Value::Array(items) => {
                if items.len() > MAX_METADATA_ARRAY_ITEMS {
                    return Err(metadata_limit_error("CurseForge metadata array is too large"));
                }
                for item in items {
                    visit(item, depth + 1, nodes)?;
                }
                Ok(())
            }
            Value::Object(entries) => {
                if entries.len() > MAX_METADATA_OBJECT_ENTRIES {
                    return Err(metadata_limit_error("CurseForge metadata object has too many fields"));
                }
                for (key, item) in entries {
                    if key.len() > 256 {
                        return Err(metadata_limit_error("CurseForge metadata key is too large"));
                    }
                    visit(item, depth + 1, nodes)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    let mut nodes = 0usize;
    visit(value, 0, &mut nodes)
}

fn parse_metadata_bytes(bytes: &[u8]) -> Result<Value, ProviderError> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(metadata_limit_error(format!(
            "CurseForge metadata exceeded the {MAX_METADATA_BYTES}-byte response bound"
        )));
    }
    let value = serde_json::from_slice::<Value>(bytes).map_err(|_| malformed_response())?;
    validate_metadata_value(&value)?;
    Ok(value)
}

async fn parse_json_response(
    mut response: reqwest::Response,
    missing: MissingResource,
) -> Result<Value, ProviderError> {
    validate_metadata_headers(response.headers())?;
    if !response.status().is_success() {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        return Err(map_http_status(response.status().as_u16(), missing, retry_after));
    }
    if response.content_length().is_some_and(|length| length > MAX_METADATA_BYTES as u64) {
        return Err(metadata_limit_error("CurseForge metadata Content-Length exceeded the response bound"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(map_request_error)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_METADATA_BYTES {
            return Err(metadata_limit_error(format!(
                "CurseForge metadata exceeded the {MAX_METADATA_BYTES}-byte response bound"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_metadata_bytes(&bytes)
}

fn map_http_status(status: u16, missing: MissingResource, retry_after: Option<u64>) -> ProviderError {
    match status {
        401 | 403 => ProviderError::new(
            "configuration_required",
            "invalid_api_credential",
            format!("CurseForge rejected {API_KEY_ENV}"),
        ),
        404 => match missing {
            MissingResource::Project => ProviderError::new(
                "not_found",
                "removed_project",
                "The CurseForge project is unavailable or has been removed",
            ),
            MissingResource::File => ProviderError::new(
                "not_found",
                "removed_file",
                "The CurseForge file is unavailable or has been removed",
            ),
            MissingResource::Generic => ProviderError::new(
                "not_found",
                "provider_resource_not_found",
                "The requested CurseForge resource was not found",
            ),
        },
        429 => ProviderError::new("rate_limited", "rate_limited", "CurseForge rate limited this client")
            .retry_after(retry_after),
        500..=599 => {
            ProviderError::new("unavailable", "provider_unavailable", format!("CurseForge returned HTTP {status}"))
        }
        300..=399 => ProviderError::new(
            "error",
            "redirect_rejected",
            "CurseForge redirect was rejected by the provider origin policy",
        ),
        _ => ProviderError::new("error", "provider_request_failed", format!("CurseForge returned HTTP {status}")),
    }
}

fn malformed_response() -> ProviderError {
    ProviderError::new("error", "malformed_response", "CurseForge returned a malformed or incomplete response")
}

fn ok(data: Value) -> Value {
    json!({
        "status": "ok",
        "provider": "curseforge",
        "data": data,
    })
}

fn required_u64(value: &Value, key: &str) -> Result<u64, ProviderError> {
    value.get(key).and_then(Value::as_u64).ok_or_else(malformed_response)
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ProviderError> {
    value.get(key).and_then(Value::as_str).filter(|value| !value.trim().is_empty()).ok_or_else(malformed_response)
}

fn nonempty_string(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
}

fn release_type(value: u64) -> &'static str {
    match value {
        1 => "release",
        2 => "beta",
        3 => "alpha",
        _ => "unknown",
    }
}

fn dependency_kind(value: u64) -> &'static str {
    match value {
        1 => "embedded_library",
        2 => "optional",
        3 => "required",
        4 => "tool",
        5 => "incompatible",
        6 => "include",
        _ => "unknown",
    }
}

fn provider_hashes(file: &Value) -> Vec<Value> {
    file.get("hashes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hash| {
            let value = nonempty_string(hash.get("value"))?;
            let algo = hash.get("algo").and_then(Value::as_u64).unwrap_or_default();
            let algorithm = match algo {
                1 => "sha1".to_owned(),
                2 => "md5".to_owned(),
                other => format!("curseforge_algo_{other}"),
            };
            Some(json!({ "algorithm": algorithm, "value": value.to_ascii_lowercase() }))
        })
        .collect()
}

fn game_versions(file: &Value) -> Vec<String> {
    file.get("gameVersions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn is_loader_tag(version: &str) -> bool {
    ["Fabric", "Forge", "NeoForge", "Quilt", "LiteLoader", "Cauldron"]
        .iter()
        .any(|loader| version.eq_ignore_ascii_case(loader))
}

fn minecraft_versions(file: &Value) -> Vec<String> {
    game_versions(file).into_iter().filter(|version| !is_loader_tag(version)).collect()
}

fn loader_tags(file: &Value) -> Vec<String> {
    game_versions(file).into_iter().filter(|version| is_loader_tag(version)).collect()
}

fn mapped_dependencies(file: &Value) -> Vec<Value> {
    file.get("dependencies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|dependency| {
            let project_id = dependency.get("modId")?.as_u64()?;
            let relation_type = dependency.get("relationType").and_then(Value::as_u64).unwrap_or_default();
            Some(json!({
                "project_id": project_id.to_string(),
                "kind": dependency_kind(relation_type),
                "required": relation_type == 3,
                "optional": relation_type == 2,
            }))
        })
        .collect()
}

fn map_file(file: &Value, environment: &str) -> Result<Value, ProviderError> {
    let id = required_u64(file, "id")?;
    let project_id = required_u64(file, "modId")?;
    let display_name = required_string(file, "displayName")?;
    let file_name = required_string(file, "fileName")?;
    safe_jar_filename(file_name)?;
    let release = release_type(file.get("releaseType").and_then(Value::as_u64).unwrap_or_default());
    let file_size = file.get("fileLength").and_then(Value::as_u64).unwrap_or_default();
    let minecraft_versions = minecraft_versions(file);
    let loaders = loader_tags(file);
    let direct_url = nonempty_string(file.get("downloadUrl")).is_some();
    Ok(json!({
        "provider": "curseforge",
        "project_id": project_id.to_string(),
        "file_id": id.to_string(),
        "version_id": id.to_string(),
        "name": display_name,
        "file_name": file_name,
        "minecraft_versions": minecraft_versions,
        "loaders": loaders,
        "release_type": release,
        "environment": {
            "requested": environment,
            "applicability": "unknown",
            "source": "CurseForge Core API does not expose reliable client/server side applicability for ordinary Minecraft files"
        },
        "dependencies": mapped_dependencies(file),
        "download_availability": if direct_url { "direct_url" } else { "provider_lookup_required" },
        "hashes": provider_hashes(file),
        "file_size": file_size,
        "is_available": file.get("isAvailable").and_then(Value::as_bool).unwrap_or(false),
        "file_date": file.get("fileDate").cloned().unwrap_or(Value::Null),
    }))
}

fn map_project(project: &Value, environment: &str) -> Result<Value, ProviderError> {
    let id = required_u64(project, "id")?;
    let name = required_string(project, "name")?;
    let slug = required_string(project, "slug")?;
    let website_url = project.get("links").and_then(|links| links.get("websiteUrl")).cloned().unwrap_or(Value::Null);
    let authors = project
        .get("authors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|author| author.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    Ok(json!({
        "provider": "curseforge",
        "project_id": id.to_string(),
        "name": name,
        "slug": slug,
        "summary": project.get("summary").cloned().unwrap_or(Value::Null),
        "website_url": website_url,
        "authors": authors,
        "download_count": project.get("downloadCount").cloned().unwrap_or(Value::Null),
        "allow_mod_distribution": project.get("allowModDistribution").cloned().unwrap_or(Value::Null),
        "is_available": project.get("isAvailable").and_then(Value::as_bool).unwrap_or(false),
        "environment": {
            "requested": environment,
            "applicability": "unknown"
        }
    }))
}

fn validate_selected_file(file: &Value, target: &Target) -> Result<(), ProviderError> {
    let versions = minecraft_versions(file);
    if !versions.iter().any(|version| version == &target.minecraft) {
        return Err(ProviderError::new(
            "incompatible",
            "no_compatible_minecraft_version",
            format!("The selected CurseForge file is not tagged for Minecraft {}", target.minecraft),
        ));
    }
    let loaders = loader_tags(file);
    if !loaders.iter().any(|loader| loader.eq_ignore_ascii_case("fabric")) {
        return Err(ProviderError::new(
            "incompatible",
            "no_fabric_build",
            "The selected CurseForge file is not a Fabric build",
        ));
    }
    Ok(())
}

fn classify_version_gap(project_id: u64, minecraft: &str, game_version_files: &[Value]) -> ProviderError {
    if game_version_files.is_empty() {
        ProviderError::new(
            "incompatible",
            "no_compatible_minecraft_version",
            format!("CurseForge project {project_id} has no file for Minecraft {minecraft}"),
        )
    } else {
        ProviderError::new(
            "incompatible",
            "no_fabric_build",
            format!("CurseForge project {project_id} has files for Minecraft {minecraft}, but no Fabric build"),
        )
    }
}

fn select_best_file(mut files: Vec<Value>) -> Result<Value, ProviderError> {
    files.retain(|file| file.get("isAvailable").and_then(Value::as_bool).unwrap_or(false));
    files.sort_by(|left, right| {
        let left_date = left.get("fileDate").and_then(Value::as_str).unwrap_or_default();
        let right_date = right.get("fileDate").and_then(Value::as_str).unwrap_or_default();
        right_date.cmp(left_date).then_with(|| {
            right
                .get("id")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                .cmp(&left.get("id").and_then(Value::as_u64).unwrap_or_default())
        })
    });
    files.into_iter().next().ok_or_else(|| {
        ProviderError::new(
            "incompatible",
            "impossible_dependency_selection",
            "A required CurseForge dependency has no available compatible file",
        )
    })
}

fn relation_entries(file: &Value) -> Vec<(u64, u64)> {
    file.get("dependencies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|dependency| Some((dependency.get("modId")?.as_u64()?, dependency.get("relationType")?.as_u64()?)))
        .collect()
}

fn should_select_required_dependency(selected: &BTreeMap<u64, Value>, dependency_id: u64) -> bool {
    !selected.contains_key(&dependency_id)
}

fn detect_impossible_relations(selected: &BTreeMap<u64, Value>) -> Result<(), ProviderError> {
    for (project_id, file) in selected {
        for (dependency_id, relation_type) in relation_entries(file) {
            if relation_type == 5 && selected.contains_key(&dependency_id) {
                return Err(ProviderError::new(
                    "incompatible",
                    "impossible_dependency_selection",
                    format!("CurseForge project {project_id} declares selected project {dependency_id} incompatible"),
                ));
            }
        }
    }
    Ok(())
}

async fn resolve_dependency_graph(
    client: &CurseForgeClient,
    root: Value,
    target: &Target,
) -> Result<Value, ProviderError> {
    validate_selected_file(&root, target)?;
    let root_project = required_u64(&root, "modId")?;
    let root_file = required_u64(&root, "id")?;
    let mut selected = BTreeMap::new();
    selected.insert(root_project, root.clone());
    let mut queue = VecDeque::from([root]);
    let mut required_edges = Vec::new();
    let mut optional_edges = Vec::new();
    let mut incompatible_edges = Vec::new();
    let mut expanded = BTreeSet::new();

    while let Some(file) = queue.pop_front() {
        let from_project = required_u64(&file, "modId")?;
        let from_file = required_u64(&file, "id")?;
        if !expanded.insert(from_file) {
            continue;
        }
        for (dependency_id, relation_type) in relation_entries(&file) {
            match relation_type {
                3 => {
                    required_edges.push(json!({
                        "from_project_id": from_project.to_string(),
                        "from_file_id": from_file.to_string(),
                        "project_id": dependency_id.to_string(),
                        "kind": "required",
                    }));
                    if !should_select_required_dependency(&selected, dependency_id) {
                        continue;
                    }
                    if selected.len() >= MAX_DEPENDENCY_PACKAGES {
                        return Err(ProviderError::new(
                            "incompatible",
                            "dependency_graph_too_large",
                            format!("CurseForge dependency resolution exceeded {MAX_DEPENDENCY_PACKAGES} packages"),
                        ));
                    }
                    let candidates = client
                        .compatible_files(dependency_id, target)
                        .await
                        .map_err(|error| {
                            if matches!(error.code, "no_compatible_minecraft_version" | "no_fabric_build") {
                                ProviderError::new(
                                    "incompatible",
                                    "impossible_dependency_selection",
                                    format!(
                                        "Required CurseForge dependency {dependency_id} has no compatible Fabric file for Minecraft {}: {}",
                                        target.minecraft, error.message
                                    ),
                                )
                            } else {
                                error
                            }
                        })?;
                    let selected_file = select_best_file(candidates)?;
                    validate_selected_file(&selected_file, target)?;
                    selected.insert(dependency_id, selected_file.clone());
                    queue.push_back(selected_file);
                }
                2 => optional_edges.push(json!({
                    "from_project_id": from_project.to_string(),
                    "from_file_id": from_file.to_string(),
                    "project_id": dependency_id.to_string(),
                    "kind": "optional",
                    "automatically_selected": false,
                })),
                5 => incompatible_edges.push(json!({
                    "from_project_id": from_project.to_string(),
                    "from_file_id": from_file.to_string(),
                    "project_id": dependency_id.to_string(),
                    "kind": "incompatible",
                })),
                _ => {}
            }
        }
        detect_impossible_relations(&selected)?;
    }

    detect_impossible_relations(&selected)?;
    let packages = selected.values().map(|file| map_file(file, &target.environment)).collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "provider": "curseforge",
        "root_project_id": root_project.to_string(),
        "root_file_id": root_file.to_string(),
        "minecraft": target.minecraft,
        "loader": target.loader,
        "environment": target.environment,
        "packages": packages,
        "required_dependencies": required_edges,
        "optional_dependencies": optional_edges,
        "incompatible_dependencies": incompatible_edges,
        "optional_dependencies_automatically_installed": false,
        "resolution": "deterministic_newest_compatible_file_date_then_file_id",
    }))
}

fn safe_jar_filename(value: &str) -> Result<(), ProviderError> {
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
    if portable {
        Ok(())
    } else {
        Err(ProviderError::new(
            "download_failed",
            "unsafe_artifact_filename",
            format!("CurseForge returned an unsafe JAR filename: {value}"),
        ))
    }
}

fn validate_download_url(url: &str) -> Result<String, ProviderError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        ProviderError::new("download_failed", "untrusted_download_url", "CurseForge returned an invalid artifact URL")
    })?;
    if !is_curseforge_artifact_url(&parsed) {
        return Err(ProviderError::new(
            "download_failed",
            "untrusted_download_url",
            "CurseForge automatic downloads must stay on the HTTPS forgecdn.net artifact boundary",
        ));
    }
    Ok(parsed.to_string())
}

fn manual_artifact_required(file: &Value, project: &Value) -> Result<Value, ProviderError> {
    let project_id = required_u64(file, "modId")?;
    let file_id = required_u64(file, "id")?;
    let file_name = required_string(file, "fileName")?;
    safe_jar_filename(file_name)?;
    let display_name = required_string(file, "displayName")?;
    let website_url = project.get("links").and_then(|links| links.get("websiteUrl")).cloned().unwrap_or(Value::Null);
    let project_name = nonempty_string(project.get("name")).map(ToOwned::to_owned);
    let project_slug = nonempty_string(project.get("slug")).map(ToOwned::to_owned);
    let minecraft_versions = minecraft_versions(file);
    let loaders = loader_tags(file);
    let fabric_compatible = loaders.iter().any(|loader| loader.eq_ignore_ascii_case("fabric"));
    let reason = "CurseForge did not provide an automatic download URL for this exact file. Desktop must ask the player to obtain this exact artifact and verify it before use.";
    Ok(json!({
        "status": "manual_artifact_required",
        "provider": "curseforge",
        "data": {
            "provider": "curseforge",
            "project_id": project_id.to_string(),
            "file_id": file_id.to_string(),
            "version_id": file_id.to_string(),
            "name": display_name,
            "file_name": file_name,
            "file_size": file.get("fileLength").cloned().unwrap_or(Value::Null),
            "hashes": provider_hashes(file),
            "minecraft_versions": minecraft_versions,
            "loaders": loaders,
            "fabric_compatible": fabric_compatible,
            "dependencies": mapped_dependencies(file),
            "project_url": website_url.clone(),
            "project": {
                "project_id": project_id.to_string(),
                "name": project_name,
                "slug": project_slug,
                "website_url": website_url,
                "allow_mod_distribution": project
                    .get("allowModDistribution")
                    .cloned()
                    .unwrap_or(Value::Null),
            },
            "remediation": {
                "kind": "manual_artifact_required",
                "provider": "curseforge",
                "automatic_retrieval_available": false,
                "reason_code": "provider_download_unavailable",
                "supply_exact_project_id": project_id.to_string(),
                "supply_exact_file_id": file_id.to_string(),
                "reason": reason,
            },
            "reason": reason,
        }
    }))
}

struct TempArtifact {
    path: PathBuf,
    armed: bool,
}

impl TempArtifact {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempArtifact {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

async fn create_temp_artifact(
    destination: &Path,
    file_id: u64,
) -> Result<(tokio::fs::File, TempArtifact), ProviderError> {
    let parent = destination.parent().ok_or_else(|| {
        ProviderError::new(
            "download_failed",
            "invalid_destination",
            "Artifact destination must include a parent directory",
        )
    })?;
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        ProviderError::new(
            "download_failed",
            "destination_unavailable",
            format!("Could not create artifact destination directory: {error}"),
        )
    })?;
    let base = destination.file_name().and_then(|name| name.to_str()).unwrap_or("artifact.jar");
    for attempt in 0..8u64 {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let temp = parent.join(format!(".{base}.swarmcraft-part-{}-{file_id}-{nonce}-{attempt}", std::process::id()));
        match tokio::fs::OpenOptions::new().create_new(true).write(true).open(&temp).await {
            Ok(file) => return Ok((file, TempArtifact::new(temp))),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(ProviderError::new(
                    "download_failed",
                    "destination_unavailable",
                    format!("Could not create temporary artifact file: {error}"),
                ));
            }
        }
    }
    Err(ProviderError::new(
        "download_failed",
        "destination_unavailable",
        "Could not reserve a unique temporary artifact file",
    ))
}

fn validate_declared_artifact_size(expected_size: u64) -> Result<(), ProviderError> {
    if expected_size == 0 || expected_size > MAX_ARTIFACT_BYTES {
        Err(ProviderError::new(
            "download_failed",
            "artifact_size_out_of_bounds",
            format!(
                "CurseForge declared artifact size {expected_size}; automatic downloads are limited to {MAX_ARTIFACT_BYTES} bytes"
            ),
        ))
    } else {
        Ok(())
    }
}

fn verify_download_size(expected: u64, actual: u64) -> Result<(), ProviderError> {
    if actual == expected {
        Ok(())
    } else if actual < expected {
        Err(ProviderError::new(
            "download_failed",
            "interrupted_download",
            format!("CurseForge artifact ended after {actual} bytes; expected {expected}"),
        ))
    } else {
        Err(ProviderError::new(
            "download_failed",
            "artifact_size_mismatch",
            format!("CurseForge artifact exceeded its declared size of {expected} bytes"),
        ))
    }
}

fn verify_provider_hashes(hashes: &[Value], sha1_hex: &str, md5_hex: &str) -> Result<Vec<String>, ProviderError> {
    let mut verified = Vec::new();
    let mut recognized = 0usize;
    for hash in hashes {
        let algorithm = hash.get("algorithm").and_then(Value::as_str).unwrap_or_default();
        let expected = hash.get("value").and_then(Value::as_str).unwrap_or_default();
        let actual = match algorithm {
            "sha1" => {
                recognized += 1;
                sha1_hex
            }
            "md5" => {
                recognized += 1;
                md5_hex
            }
            _ => continue,
        };
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(ProviderError::new(
                "download_failed",
                "hash_mismatch",
                format!("CurseForge {algorithm} hash did not match the downloaded artifact"),
            ));
        }
        verified.push(algorithm.to_owned());
    }
    if !hashes.is_empty() && recognized == 0 {
        return Err(ProviderError::new(
            "download_failed",
            "unsupported_provider_hash",
            "CurseForge supplied hashes, but none used a supported SHA-1 or MD5 algorithm",
        ));
    }
    Ok(verified)
}

fn publish_verified_artifact(temp: &mut TempArtifact, destination: &Path) -> Result<(), ProviderError> {
    std::fs::rename(&temp.path, destination).map_err(|error| {
        ProviderError::new(
            "download_failed",
            "atomic_publication_failed",
            format!("Could not atomically publish verified artifact: {error}"),
        )
    })?;
    temp.disarm();
    Ok(())
}

async fn download_artifact(
    client: &CurseForgeClient,
    file: &Value,
    project: &Value,
    destination: PathBuf,
) -> Result<Value, ProviderError> {
    let file_id = required_u64(file, "id")?;
    let file_name = required_string(file, "fileName")?;
    safe_jar_filename(file_name)?;
    if tokio::fs::try_exists(&destination).await.unwrap_or(false) {
        return Err(ProviderError::new(
            "download_failed",
            "destination_exists",
            "Refusing to replace an existing artifact destination",
        ));
    }
    let expected_size = file.get("fileLength").and_then(Value::as_u64).unwrap_or_default();
    validate_declared_artifact_size(expected_size)?;
    let Some(url) = client.download_url(file).await? else {
        return manual_artifact_required(file, project);
    };
    let mut response = client
        .artifact_http
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(map_download_error)?;
    if matches!(response.status(), StatusCode::FORBIDDEN | StatusCode::NOT_FOUND) {
        return manual_artifact_required(file, project);
    }
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        return Err(map_http_status(429, MissingResource::Generic, retry_after));
    }
    if !response.status().is_success() {
        return Err(ProviderError::new(
            "download_failed",
            "provider_download_failed",
            format!("CurseForge artifact endpoint returned HTTP {}", response.status()),
        ));
    }
    if let Some(content_length) = response.content_length() {
        if content_length > expected_size || content_length > MAX_ARTIFACT_BYTES {
            return Err(ProviderError::new(
                "download_failed",
                "artifact_size_mismatch",
                "CurseForge artifact response exceeds the provider-declared size bound",
            ));
        }
    }

    let (mut output, mut temp) = create_temp_artifact(&destination, file_id).await?;
    let mut sha1 = Sha1::new();
    let mut md5 = Md5::new();
    let mut sha256 = Sha256::new();
    let mut total = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(map_download_error)? {
        total = total.checked_add(chunk.len() as u64).ok_or_else(|| {
            ProviderError::new(
                "download_failed",
                "artifact_size_out_of_bounds",
                "CurseForge artifact size overflowed the download counter",
            )
        })?;
        if total > expected_size || total > MAX_ARTIFACT_BYTES {
            return Err(ProviderError::new(
                "download_failed",
                "artifact_size_mismatch",
                "CurseForge artifact exceeded the provider-declared size bound",
            ));
        }
        sha1.update(&chunk);
        md5.update(&chunk);
        sha256.update(&chunk);
        output.write_all(&chunk).await.map_err(|error| {
            ProviderError::new(
                "download_failed",
                "destination_unavailable",
                format!("Could not write temporary artifact file: {error}"),
            )
        })?;
    }
    verify_download_size(expected_size, total)?;
    output.flush().await.map_err(|error| {
        ProviderError::new(
            "download_failed",
            "destination_unavailable",
            format!("Could not flush temporary artifact file: {error}"),
        )
    })?;
    output.sync_all().await.map_err(|error| {
        ProviderError::new(
            "download_failed",
            "destination_unavailable",
            format!("Could not sync temporary artifact file: {error}"),
        )
    })?;
    drop(output);

    let sha1_hex = hex::encode(sha1.finalize());
    let md5_hex = hex::encode(md5.finalize());
    let sha256_hex = hex::encode(sha256.finalize());
    let hashes = provider_hashes(file);
    let verified = verify_provider_hashes(&hashes, &sha1_hex, &md5_hex)?;

    publish_verified_artifact(&mut temp, &destination)?;
    Ok(json!({
        "status": "downloaded",
        "provider": "curseforge",
        "data": {
            "package": map_file(file, "unknown")?,
            "destination": destination,
            "bytes": total,
            "local_sha256": sha256_hex,
            "provider_hashes_verified": verified,
            "temporary_file_cleanup": "armed_until_atomic_publication",
        }
    }))
}

#[tauri::command]
pub fn curseforge_provider_status() -> Value {
    let configured = normalize_api_key(env::var(API_KEY_ENV).ok()).is_some();
    if configured {
        json!({
            "status": "available",
            "provider": "curseforge",
            "configuration": {
                "credential_source": API_KEY_ENV,
                "credential_present": true,
                "api_base": API_BASE,
            }
        })
    } else {
        ProviderError::new(
            "configuration_required",
            "missing_api_credential",
            format!("Set the machine-local {API_KEY_ENV} environment variable to enable CurseForge browsing"),
        )
        .into_response()
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn curseforge_search(
    query: String,
    minecraft: String,
    loader: String,
    environment: String,
    index: u64,
    page_size: u64,
) -> Value {
    match curseforge_search_inner(query, minecraft, loader, environment, index, page_size).await {
        Ok(value) => ok(value),
        Err(error) => error.into_response(),
    }
}

async fn curseforge_search_inner(
    query: String,
    minecraft: String,
    loader: String,
    environment: String,
    index: u64,
    page_size: u64,
) -> Result<Value, ProviderError> {
    let target = Target::parse(minecraft, loader, environment)?;
    let query = query.trim().to_owned();
    if query.len() > 200 {
        return Err(ProviderError::new(
            "invalid_request",
            "search_query_too_long",
            "CurseForge search queries are limited to 200 characters",
        ));
    }
    let page_size = page_size.clamp(1, MAX_PAGE_SIZE);
    if index >= MAX_SEARCH_INDEX || index.saturating_add(page_size) > MAX_SEARCH_INDEX {
        return Err(ProviderError::new(
            "invalid_request",
            "pagination_out_of_range",
            "CurseForge pagination requires index + pageSize <= 10000",
        ));
    }
    let client = CurseForgeClient::from_environment()?;
    let mut params = vec![
        ("gameId", MINECRAFT_GAME_ID.to_string()),
        ("classId", MINECRAFT_MODS_CLASS_ID.to_string()),
        ("gameVersion", target.minecraft.clone()),
        ("modLoaderType", FABRIC_MOD_LOADER_TYPE.to_string()),
        ("sortField", "2".to_owned()),
        ("sortOrder", "desc".to_owned()),
        ("index", index.to_string()),
        ("pageSize", page_size.to_string()),
    ];
    if !query.is_empty() {
        params.push(("searchFilter", query));
    }
    let value = client.get_json("/v1/mods/search", &params, MissingResource::Generic).await?;
    let projects = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(malformed_response)?
        .iter()
        .map(|project| map_project(project, &target.environment))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "query": {
            "minecraft": target.minecraft,
            "loader": target.loader,
            "environment": target.environment,
            "search": params.iter().find(|(key, _)| *key == "searchFilter").map(|(_, value)| value).cloned().unwrap_or_default(),
        },
        "projects": projects,
        "pagination": value.get("pagination").cloned().unwrap_or(Value::Null),
    }))
}

#[tauri::command]
pub async fn curseforge_project(project_id: u64) -> Value {
    match async {
        let client = CurseForgeClient::from_environment()?;
        let project = client.fetch_project(project_id).await?;
        map_project(&project, "unknown")
    }
    .await
    {
        Ok(value) => ok(value),
        Err(error) => error.into_response(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn curseforge_versions(project_id: u64, minecraft: String, loader: String) -> Value {
    match async {
        let target = Target::parse(minecraft, loader, "both".to_owned())?;
        let client = CurseForgeClient::from_environment()?;
        let files = client.compatible_files(project_id, &target).await?;
        let versions = files.iter().map(|file| map_file(file, "both")).collect::<Result<Vec<_>, _>>()?;
        Ok::<Value, ProviderError>(json!({
            "project_id": project_id.to_string(),
            "minecraft": target.minecraft,
            "loader": target.loader,
            "versions": versions,
        }))
    }
    .await
    {
        Ok(value) => ok(value),
        Err(error) => error.into_response(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn curseforge_resolve(file_id: u64, minecraft: String, loader: String, environment: String) -> Value {
    match async {
        let target = Target::parse(minecraft, loader, environment)?;
        let client = CurseForgeClient::from_environment()?;
        let file = client.fetch_file(file_id).await?;
        resolve_dependency_graph(&client, file, &target).await
    }
    .await
    {
        Ok(value) => ok(value),
        Err(error) => error.into_response(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn curseforge_download(file_id: u64, staging_session: String) -> Value {
    match async {
        let staging = resolve_provider_staging_session(&staging_session)
            .map_err(|message| ProviderError::new("invalid_request", "invalid_staging_session", message))?;
        let client = CurseForgeClient::from_environment()?;
        let file = client.fetch_file(file_id).await?;
        let project_id = required_u64(&file, "modId")?;
        let file_name = required_string(&file, "fileName")?;
        safe_jar_filename(file_name)?;
        let project = client.fetch_project(project_id).await?;
        let destination =
            staging.join("curseforge").join(project_id.to_string()).join(file_id.to_string()).join(file_name);
        download_artifact(&client, &file, &project, destination).await
    }
    .await
    {
        Ok(value) => value,
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_file() -> Value {
        json!({
            "id": 456,
            "modId": 123,
            "isAvailable": true,
            "displayName": "Example 1.2.3",
            "fileName": "example-1.2.3.jar",
            "releaseType": 1,
            "fileStatus": 4,
            "hashes": [
                { "algo": 1, "value": "a9993e364706816aba3e25717850c26c9cd0d89d" },
                { "algo": 2, "value": "900150983cd24fb0d6963f7d28e17f72" }
            ],
            "fileLength": 3,
            "downloadUrl": null,
            "gameVersions": ["1.21.1", "Fabric"],
            "dependencies": [
                { "modId": 700, "relationType": 3 },
                { "modId": 701, "relationType": 2 }
            ],
            "fileDate": "2026-08-20T10:00:00Z"
        })
    }

    #[test]
    fn missing_api_key_is_configuration_required() {
        assert_eq!(normalize_api_key(None), None);
        assert_eq!(normalize_api_key(Some("   ".into())), None);
        assert_eq!(normalize_api_key(Some(" secret ".into())), Some("secret".into()));
    }

    #[test]
    fn invalid_credential_rate_limit_and_removed_resources_are_structured() {
        assert_eq!(map_http_status(401, MissingResource::Generic, None).code, "invalid_api_credential");
        let rate = map_http_status(429, MissingResource::Generic, Some(37));
        assert_eq!(rate.code, "rate_limited");
        assert_eq!(rate.retry_after_seconds, Some(37));
        assert_eq!(map_http_status(404, MissingResource::Project, None).code, "removed_project");
        assert_eq!(map_http_status(404, MissingResource::File, None).code, "removed_file");
        assert_eq!(map_http_status(503, MissingResource::Generic, None).code, "provider_unavailable");
    }

    #[test]
    fn malformed_response_fails_closed() {
        let malformed = json!({ "id": "not-an-id" });
        assert_eq!(map_file(&malformed, "server").unwrap_err().code, "malformed_response");
    }

    #[test]
    fn required_and_optional_dependencies_stay_distinct() {
        let dependencies = mapped_dependencies(&fixture_file());
        assert_eq!(dependencies[0]["kind"], "required");
        assert_eq!(dependencies[0]["required"], true);
        assert_eq!(dependencies[1]["kind"], "optional");
        assert_eq!(dependencies[1]["optional"], true);
    }

    #[test]
    fn no_minecraft_version_and_no_fabric_build_are_distinct() {
        assert_eq!(classify_version_gap(1, "1.21.1", &[]).code, "no_compatible_minecraft_version");
        assert_eq!(classify_version_gap(1, "1.21.1", &[fixture_file()]).code, "no_fabric_build");
    }

    #[test]
    fn selected_file_must_match_minecraft_and_fabric() {
        let target = Target::parse("1.21.1".into(), "fabric".into(), "server".into()).unwrap();
        assert!(validate_selected_file(&fixture_file(), &target).is_ok());
        let wrong_mc = Target::parse("1.20.1".into(), "fabric".into(), "server".into()).unwrap();
        assert_eq!(
            validate_selected_file(&fixture_file(), &wrong_mc).unwrap_err().code,
            "no_compatible_minecraft_version"
        );
        let mut no_fabric = fixture_file();
        no_fabric["gameVersions"] = json!(["1.21.1", "Forge"]);
        assert_eq!(validate_selected_file(&no_fabric, &target).unwrap_err().code, "no_fabric_build");
    }

    #[test]
    fn impossible_dependency_selection_is_detected() {
        let mut root = fixture_file();
        root["dependencies"] = json!([{ "modId": 999, "relationType": 5 }]);
        let mut selected = BTreeMap::new();
        selected.insert(123, root);
        selected.insert(999, fixture_file());
        assert_eq!(detect_impossible_relations(&selected).unwrap_err().code, "impossible_dependency_selection");
    }

    #[test]
    fn absent_automatic_url_produces_exact_manual_remediation() {
        let project = json!({
            "id": 123,
            "name": "Example",
            "slug": "example",
            "allowModDistribution": false,
            "links": { "websiteUrl": "https://www.curseforge.com/minecraft/mc-mods/example" }
        });
        let response = manual_artifact_required(&fixture_file(), &project).unwrap();
        assert_eq!(response["status"], "manual_artifact_required");
        assert_eq!(response["data"]["project_id"], "123");
        assert_eq!(response["data"]["file_id"], "456");
        assert_eq!(response["data"]["version_id"], "456");
        assert_eq!(response["data"]["file_name"], "example-1.2.3.jar");
        assert_eq!(response["data"]["file_size"], 3);
        assert_eq!(response["data"]["minecraft_versions"], json!(["1.21.1"]));
        assert_eq!(response["data"]["loaders"], json!(["Fabric"]));
        assert_eq!(response["data"]["fabric_compatible"], true);
        assert_eq!(response["data"]["hashes"].as_array().unwrap().len(), 2);
        assert_eq!(response["data"]["dependencies"][0]["project_id"], "700");
        assert_eq!(response["data"]["project"]["name"], "Example");
        assert_eq!(response["data"]["project"]["allow_mod_distribution"], false);
        assert_eq!(response["data"]["remediation"]["reason_code"], "provider_download_unavailable");
        assert_eq!(response["data"]["remediation"]["supply_exact_file_id"], "456");
    }

    #[test]
    fn only_https_provider_download_urls_are_accepted() {
        assert!(validate_download_url("https://edge.forgecdn.net/files/example.jar").is_ok());
        assert_eq!(
            validate_download_url("http://edge.forgecdn.net/files/example.jar").unwrap_err().code,
            "untrusted_download_url"
        );
    }

    #[test]
    fn provider_hash_mismatch_is_rejected_and_sha1_md5_can_both_verify() {
        let bytes = b"abc";
        let sha1_hex = hex::encode(Sha1::digest(bytes));
        let md5_hex = hex::encode(Md5::digest(bytes));
        let hashes = provider_hashes(&fixture_file());
        let verified = verify_provider_hashes(&hashes, &sha1_hex, &md5_hex).unwrap();
        assert_eq!(verified, vec!["sha1", "md5"]);
        assert_eq!(verify_provider_hashes(&hashes, "deadbeef", &md5_hex).unwrap_err().code, "hash_mismatch");
    }

    #[test]
    fn interrupted_and_oversized_downloads_fail_closed() {
        assert_eq!(verify_download_size(100, 99).unwrap_err().code, "interrupted_download");
        assert_eq!(verify_download_size(100, 101).unwrap_err().code, "artifact_size_mismatch");
        assert!(verify_download_size(100, 100).is_ok());
    }

    #[test]
    fn provider_metadata_separates_minecraft_versions_from_loader_tags() {
        let mapped = map_file(&fixture_file(), "server").unwrap();
        assert_eq!(mapped["minecraft_versions"], json!(["1.21.1"]));
        assert_eq!(mapped["loaders"], json!(["Fabric"]));
    }

    #[test]
    fn required_dependency_cycle_back_to_selected_project_terminates() {
        let mut selected = BTreeMap::new();
        selected.insert(123, fixture_file());
        assert!(!should_select_required_dependency(&selected, 123));
        assert!(should_select_required_dependency(&selected, 700));
    }

    #[test]
    fn local_sha256_is_exact_and_changes_with_artifact_bytes() {
        let expected = hex::encode(Sha256::digest(b"abc"));
        let different = hex::encode(Sha256::digest(b"abd"));
        assert_eq!(expected, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_ne!(expected, different);
    }

    #[test]
    fn declared_download_size_is_bounded() {
        assert!(validate_declared_artifact_size(1).is_ok());
        assert!(validate_declared_artifact_size(MAX_ARTIFACT_BYTES).is_ok());
        assert_eq!(validate_declared_artifact_size(0).unwrap_err().code, "artifact_size_out_of_bounds");
        assert_eq!(
            validate_declared_artifact_size(MAX_ARTIFACT_BYTES + 1).unwrap_err().code,
            "artifact_size_out_of_bounds"
        );
    }

    #[test]
    fn partial_file_guard_cleans_up_interrupted_artifact() {
        let directory = std::env::temp_dir().join(format!(
            "swarmcraft-curseforge-partial-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let temporary = directory.join(".artifact.jar.part");
        std::fs::write(&temporary, b"partial").unwrap();
        let guard = TempArtifact::new(temporary.clone());
        drop(guard);
        assert!(!temporary.exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn successful_verified_artifact_publication_is_atomic_rename() {
        let directory = std::env::temp_dir().join(format!(
            "swarmcraft-curseforge-publish-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let temporary = directory.join(".artifact.jar.part");
        let destination = directory.join("artifact.jar");
        std::fs::write(&temporary, b"abc").unwrap();
        let mut guard = TempArtifact::new(temporary.clone());
        publish_verified_artifact(&mut guard, &destination).unwrap();
        assert!(!temporary.exists());
        assert_eq!(std::fs::read(&destination).unwrap(), b"abc");
        drop(guard);
        assert!(destination.exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn deterministic_file_selection_prefers_latest_then_highest_id() {
        let mut older = fixture_file();
        older["id"] = json!(10);
        older["fileDate"] = json!("2026-08-19T00:00:00Z");
        let mut latest_low = fixture_file();
        latest_low["id"] = json!(20);
        latest_low["fileDate"] = json!("2026-08-20T00:00:00Z");
        let mut latest_high = latest_low.clone();
        latest_high["id"] = json!(21);
        let selected = select_best_file(vec![latest_low, older, latest_high]).unwrap();
        assert_eq!(selected["id"], 21);
    }
}

#[cfg(test)]
mod agent5_security_tests {
    use super::*;

    #[test]
    fn provider_filename_traversal_matrix_fails_closed() {
        for invalid in [
            "../evil.jar",
            "..\\evil.jar",
            "/tmp/evil.jar",
            "C:\\tmp\\evil.jar",
            "\\\\server\\share\\evil.jar",
            ".jar",
            "CON.jar",
            "NUL.jar",
            "evil.jar.",
            "evil.jar ",
            "dir/evil.jar",
            "dir\\evil.jar",
        ] {
            assert!(safe_jar_filename(invalid).is_err(), "accepted {invalid}");
        }
        assert!(safe_jar_filename("safe-mod_1.2.3.jar").is_ok());
    }
}

#[cfg(test)]
mod agent5_http_security_tests {
    use super::*;

    #[test]
    fn authenticated_api_and_artifact_origins_are_disjoint() {
        let api = reqwest::Url::parse("https://api.curseforge.com/v1/mods/1").unwrap();
        let second_origin = reqwest::Url::parse("https://attacker.invalid/steal").unwrap();
        let forge = reqwest::Url::parse("https://edge.forgecdn.net/files/example.jar").unwrap();
        let private = reqwest::Url::parse("https://127.0.0.1/example.jar").unwrap();
        assert!(is_curseforge_api_url(&api));
        assert!(!is_curseforge_api_url(&second_origin));
        assert!(is_curseforge_artifact_url(&forge));
        assert!(!is_curseforge_artifact_url(&second_origin));
        assert!(!is_curseforge_artifact_url(&private));
    }

    #[test]
    fn provider_metadata_bytes_and_shape_are_bounded() {
        assert_eq!(parse_metadata_bytes(&vec![b' '; MAX_METADATA_BYTES + 1]).unwrap_err().code, "response_too_large");
        let huge = json!({"text": "x".repeat(MAX_METADATA_STRING_BYTES + 1)});
        assert_eq!(validate_metadata_value(&huge).unwrap_err().code, "response_too_large");
    }

    #[test]
    fn api_key_control_characters_are_rejected() {
        assert_eq!(normalize_api_key(Some("secret\nheader".into())), None);
        assert_eq!(normalize_api_key(Some(" secret ".into())), Some("secret".into()));
    }
}
