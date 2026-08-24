use directories::ProjectDirs;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const MOJANG_VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const FABRIC_META_BASE_URL: &str = "https://meta.fabricmc.net/";
pub const DEFAULT_CACHE_TTL_SECONDS: u64 = 30 * 60;
const MOJANG_MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const FABRIC_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_PROVIDER_ENTRIES: usize = 10_000;
const MAX_TOKEN_BYTES: usize = 256;
const MAX_RELEASE_TIME_BYTES: usize = 96;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogProvider {
    Mojang,
    Fabric,
}

impl CatalogProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mojang => "mojang",
            Self::Fabric => "fabric",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogOrigin {
    Network,
    FreshCache,
    StaleCache,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MinecraftVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub version_type: String,
    pub release_time: String,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricLoaderVersion {
    pub version: String,
    pub stable: bool,
    pub minecraft_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogResponse<T> {
    pub provider: CatalogProvider,
    pub source_url: String,
    pub fetched_at_unix_seconds: u64,
    pub cache_expires_at_unix_seconds: u64,
    pub origin: CatalogOrigin,
    pub warning: Option<String>,
    pub versions: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogErrorPayload {
    pub code: String,
    pub provider: String,
    pub message: String,
}

impl CatalogErrorPayload {
    pub fn from_error(provider: CatalogProvider, error: &CatalogError) -> Self {
        Self {
            code: error.code().to_owned(),
            provider: provider.as_str().to_owned(),
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for CatalogErrorPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("official provider request failed: {0}")]
    ProviderUnavailable(String),
    #[error("official provider response exceeded the {limit_bytes} byte limit")]
    ResponseTooLarge { limit_bytes: usize },
    #[error("official provider response was malformed: {0}")]
    MalformedResponse(String),
    #[error("{0} returned no selectable versions")]
    EmptyCatalog(&'static str),
    #[error("invalid catalog input: {0}")]
    InvalidInput(String),
    #[error("Fabric Loader {loader} is not compatible with Minecraft {minecraft}")]
    IncompatibleFabricSelection { minecraft: String, loader: String },
    #[error("catalog cache is unavailable: {0}")]
    CacheUnavailable(String),
}

impl CatalogError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProviderUnavailable(_) => "provider_unavailable",
            Self::ResponseTooLarge { .. } => "response_too_large",
            Self::MalformedResponse(_) => "malformed_provider_response",
            Self::EmptyCatalog(_) => "empty_catalog",
            Self::InvalidInput(_) => "invalid_input",
            Self::IncompatibleFabricSelection { .. } => "incompatible_fabric_selection",
            Self::CacheUnavailable(_) => "cache_unavailable",
        }
    }
}

pub trait CatalogTransport: Send + Sync {
    fn get(&self, url: &Url, max_response_bytes: usize) -> Result<Vec<u8>, CatalogError>;
}

#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: Client,
}

impl HttpTransport {
    pub fn new() -> Result<Self, CatalogError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .redirect(Policy::none())
            .https_only(true)
            .user_agent(concat!("SwarmCraft/", env!("CARGO_PKG_VERSION"), " catalog"))
            .build()
            .map_err(|error| CatalogError::ProviderUnavailable(error.to_string()))?;
        Ok(Self { client })
    }
}

impl CatalogTransport for HttpTransport {
    fn get(&self, url: &Url, max_response_bytes: usize) -> Result<Vec<u8>, CatalogError> {
        if url.scheme() != "https" {
            return Err(CatalogError::InvalidInput(
                "catalog providers must use HTTPS".into(),
            ));
        }
        let response = self
            .client
            .get(url.clone())
            .send()
            .map_err(|error| CatalogError::ProviderUnavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(CatalogError::ProviderUnavailable(format!(
                "HTTP {} from {}",
                response.status(),
                url
            )));
        }

        let mut body = Vec::new();
        response
            .take(max_response_bytes as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|error| CatalogError::ProviderUnavailable(error.to_string()))?;
        if body.len() > max_response_bytes {
            return Err(CatalogError::ResponseTooLarge {
                limit_bytes: max_response_bytes,
            });
        }
        Ok(body)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheRecord {
    fetched_at_unix_seconds: u64,
    source_url: String,
    body: String,
}

#[derive(Debug)]
struct Resolved<T> {
    value: T,
    fetched_at_unix_seconds: u64,
    origin: CatalogOrigin,
    warning: Option<String>,
}

#[derive(Debug)]
pub struct CatalogService<T = HttpTransport> {
    transport: T,
    cache_dir: PathBuf,
    cache_ttl: Duration,
}

impl CatalogService<HttpTransport> {
    pub fn discover() -> Result<Self, CatalogError> {
        Self::http(default_cache_dir()?)
    }

    pub fn http(cache_dir: PathBuf) -> Result<Self, CatalogError> {
        Ok(Self::new(HttpTransport::new()?, cache_dir))
    }
}

impl<T: CatalogTransport> CatalogService<T> {
    pub fn new(transport: T, cache_dir: PathBuf) -> Self {
        Self {
            transport,
            cache_dir,
            cache_ttl: Duration::from_secs(DEFAULT_CACHE_TTL_SECONDS),
        }
    }

    pub fn with_cache_ttl(mut self, cache_ttl: Duration) -> Self {
        self.cache_ttl = cache_ttl;
        self
    }

    pub fn minecraft_versions(
        &self,
        include_snapshots: bool,
        refresh: bool,
    ) -> Result<CatalogResponse<MinecraftVersion>, CatalogError> {
        let url = Url::parse(MOJANG_VERSION_MANIFEST_URL)
            .map_err(|error| CatalogError::InvalidInput(error.to_string()))?;
        let resolved = self.resolve(
            &url,
            MOJANG_MAX_RESPONSE_BYTES,
            refresh,
            parse_minecraft_catalog,
        )?;
        let versions = filter_minecraft_versions(&resolved.value, include_snapshots)?;
        Ok(CatalogResponse {
            provider: CatalogProvider::Mojang,
            source_url: url.to_string(),
            fetched_at_unix_seconds: resolved.fetched_at_unix_seconds,
            cache_expires_at_unix_seconds: resolved
                .fetched_at_unix_seconds
                .saturating_add(self.cache_ttl.as_secs()),
            origin: resolved.origin,
            warning: resolved.warning,
            versions,
        })
    }

    pub fn fabric_loader_versions(
        &self,
        minecraft_version: &str,
        refresh: bool,
    ) -> Result<CatalogResponse<FabricLoaderVersion>, CatalogError> {
        validate_token("Minecraft version", minecraft_version)?;
        let url = fabric_loader_url(minecraft_version)?;
        let minecraft = minecraft_version.to_owned();
        let resolved = self.resolve(&url, FABRIC_MAX_RESPONSE_BYTES, refresh, |body| {
            parse_fabric_loader_catalog(&minecraft, body)
        })?;
        Ok(CatalogResponse {
            provider: CatalogProvider::Fabric,
            source_url: url.to_string(),
            fetched_at_unix_seconds: resolved.fetched_at_unix_seconds,
            cache_expires_at_unix_seconds: resolved
                .fetched_at_unix_seconds
                .saturating_add(self.cache_ttl.as_secs()),
            origin: resolved.origin,
            warning: resolved.warning,
            versions: resolved.value,
        })
    }

    pub fn validate_fabric_selection(
        &self,
        minecraft_version: &str,
        fabric_loader_version: &str,
        refresh: bool,
    ) -> Result<FabricLoaderVersion, CatalogError> {
        validate_token("Fabric Loader version", fabric_loader_version)?;
        let response = self.fabric_loader_versions(minecraft_version, refresh)?;
        validate_fabric_loader_selection(
            minecraft_version,
            fabric_loader_version,
            &response.versions,
        )
    }

    fn resolve<R, F>(
        &self,
        url: &Url,
        max_response_bytes: usize,
        refresh: bool,
        parse: F,
    ) -> Result<Resolved<R>, CatalogError>
    where
        F: Fn(&[u8]) -> Result<R, CatalogError>,
    {
        let now = unix_seconds();
        let cached = self.read_cache(url, max_response_bytes).ok().flatten();

        if !refresh {
            if let Some(record) = cached.as_ref() {
                if now.saturating_sub(record.fetched_at_unix_seconds) <= self.cache_ttl.as_secs() {
                    if let Ok(value) = parse(record.body.as_bytes()) {
                        return Ok(Resolved {
                            value,
                            fetched_at_unix_seconds: record.fetched_at_unix_seconds,
                            origin: CatalogOrigin::FreshCache,
                            warning: None,
                        });
                    }
                }
            }
        }

        match self.transport.get(url, max_response_bytes) {
            Ok(body) => match parse(&body) {
                Ok(value) => {
                    let body = String::from_utf8(body).map_err(|error| {
                        CatalogError::MalformedResponse(format!(
                            "provider JSON was not UTF-8: {error}"
                        ))
                    })?;
                    let record = CacheRecord {
                        fetched_at_unix_seconds: now,
                        source_url: url.to_string(),
                        body,
                    };
                    let warning = self
                        .write_cache(url, &record)
                        .err()
                        .map(|error| format!("Catalog cache could not be updated: {error}"));
                    Ok(Resolved {
                        value,
                        fetched_at_unix_seconds: now,
                        origin: CatalogOrigin::Network,
                        warning,
                    })
                }
                Err(network_parse_error) => self.stale_or_error(
                    cached.as_ref(),
                    &parse,
                    network_parse_error,
                    "Provider refresh returned malformed data",
                ),
            },
            Err(network_error) => self.stale_or_error(
                cached.as_ref(),
                &parse,
                network_error,
                "Provider refresh is unavailable",
            ),
        }
    }

    fn stale_or_error<R, F>(
        &self,
        cached: Option<&CacheRecord>,
        parse: &F,
        error: CatalogError,
        warning_prefix: &str,
    ) -> Result<Resolved<R>, CatalogError>
    where
        F: Fn(&[u8]) -> Result<R, CatalogError>,
    {
        if let Some(record) = cached {
            if let Ok(value) = parse(record.body.as_bytes()) {
                return Ok(Resolved {
                    value,
                    fetched_at_unix_seconds: record.fetched_at_unix_seconds,
                    origin: CatalogOrigin::StaleCache,
                    warning: Some(format!("{warning_prefix}; using cached official data: {error}")),
                });
            }
        }
        Err(error)
    }

    fn cache_path(&self, url: &Url) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update(url.as_str().as_bytes());
        self.cache_dir
            .join(format!("{}.json", hex::encode(digest.finalize())))
    }

    fn read_cache(
        &self,
        url: &Url,
        max_response_bytes: usize,
    ) -> Result<Option<CacheRecord>, CatalogError> {
        let path = self.cache_path(url);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CatalogError::CacheUnavailable(error.to_string())),
        };
        let max_cache_bytes = max_response_bytes.saturating_mul(3).saturating_add(65_536);
        let mut bytes = Vec::new();
        file.take(max_cache_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| CatalogError::CacheUnavailable(error.to_string()))?;
        if bytes.len() > max_cache_bytes {
            return Ok(None);
        }
        let record: CacheRecord = match serde_json::from_slice(&bytes) {
            Ok(record) => record,
            Err(_) => return Ok(None),
        };
        if record.source_url != url.as_str() || record.body.len() > max_response_bytes {
            return Ok(None);
        }
        Ok(Some(record))
    }

    fn write_cache(&self, url: &Url, record: &CacheRecord) -> Result<(), CatalogError> {
        std::fs::create_dir_all(&self.cache_dir)
            .map_err(|error| CatalogError::CacheUnavailable(error.to_string()))?;
        let bytes = serde_json::to_vec(record)
            .map_err(|error| CatalogError::CacheUnavailable(error.to_string()))?;
        std::fs::write(self.cache_path(url), bytes)
            .map_err(|error| CatalogError::CacheUnavailable(error.to_string()))
    }
}

pub fn default_cache_dir() -> Result<PathBuf, CatalogError> {
    let project = ProjectDirs::from("dev", "SwarmCraft", "SwarmCraft").ok_or_else(|| {
        CatalogError::CacheUnavailable("could not determine the operating-system cache directory".into())
    })?;
    Ok(project.cache_dir().join("catalogs"))
}

pub fn parse_minecraft_catalog(body: &[u8]) -> Result<Vec<MinecraftVersion>, CatalogError> {
    #[derive(Deserialize)]
    struct Manifest {
        versions: Vec<Entry>,
    }

    #[derive(Deserialize)]
    struct Entry {
        id: String,
        #[serde(rename = "type")]
        version_type: String,
        #[serde(rename = "releaseTime")]
        release_time: String,
    }

    let manifest: Manifest = serde_json::from_slice(body)
        .map_err(|error| CatalogError::MalformedResponse(error.to_string()))?;
    if manifest.versions.is_empty() {
        return Err(CatalogError::EmptyCatalog("Mojang"));
    }
    if manifest.versions.len() > MAX_PROVIDER_ENTRIES {
        return Err(CatalogError::MalformedResponse(format!(
            "Mojang returned more than {MAX_PROVIDER_ENTRIES} versions"
        )));
    }

    let mut seen = std::collections::HashSet::new();
    let mut versions = Vec::with_capacity(manifest.versions.len());
    for entry in manifest.versions {
        validate_provider_string("Minecraft version id", &entry.id, MAX_TOKEN_BYTES)?;
        validate_provider_string("Minecraft version type", &entry.version_type, 32)?;
        validate_provider_string(
            "Minecraft release time",
            &entry.release_time,
            MAX_RELEASE_TIME_BYTES,
        )?;
        if !entry.release_time.contains('T') {
            return Err(CatalogError::MalformedResponse(format!(
                "Minecraft {} has an invalid releaseTime",
                entry.id
            )));
        }
        if !seen.insert(entry.id.clone()) {
            return Err(CatalogError::MalformedResponse(format!(
                "duplicate Minecraft version {}",
                entry.id
            )));
        }
        let supported = matches!(entry.version_type.as_str(), "release" | "snapshot");
        versions.push(MinecraftVersion {
            id: entry.id,
            version_type: entry.version_type,
            release_time: entry.release_time,
            supported,
        });
    }
    Ok(versions)
}

pub fn filter_minecraft_versions(
    versions: &[MinecraftVersion],
    include_snapshots: bool,
) -> Result<Vec<MinecraftVersion>, CatalogError> {
    let filtered: Vec<_> = versions
        .iter()
        .filter(|version| {
            version.supported
                && (version.version_type == "release"
                    || (include_snapshots && version.version_type == "snapshot"))
        })
        .cloned()
        .collect();
    if filtered.is_empty() {
        return Err(CatalogError::EmptyCatalog(if include_snapshots {
            "Mojang release/snapshot catalog"
        } else {
            "Mojang release catalog"
        }));
    }
    Ok(filtered)
}

pub fn parse_fabric_loader_catalog(
    minecraft_version: &str,
    body: &[u8],
) -> Result<Vec<FabricLoaderVersion>, CatalogError> {
    validate_token("Minecraft version", minecraft_version)?;

    #[derive(Deserialize)]
    struct FabricEntry {
        loader: Loader,
    }

    #[derive(Deserialize)]
    struct Loader {
        version: String,
        stable: bool,
    }

    let entries: Vec<FabricEntry> = serde_json::from_slice(body)
        .map_err(|error| CatalogError::MalformedResponse(error.to_string()))?;
    if entries.len() > MAX_PROVIDER_ENTRIES {
        return Err(CatalogError::MalformedResponse(format!(
            "Fabric returned more than {MAX_PROVIDER_ENTRIES} loader versions"
        )));
    }

    let mut seen = std::collections::HashSet::new();
    let mut versions = Vec::with_capacity(entries.len());
    for entry in entries {
        validate_provider_string("Fabric Loader version", &entry.loader.version, MAX_TOKEN_BYTES)?;
        if !seen.insert(entry.loader.version.clone()) {
            return Err(CatalogError::MalformedResponse(format!(
                "duplicate Fabric Loader version {}",
                entry.loader.version
            )));
        }
        versions.push(FabricLoaderVersion {
            version: entry.loader.version,
            stable: entry.loader.stable,
            minecraft_version: minecraft_version.to_owned(),
        });
    }
    Ok(versions)
}

pub fn validate_fabric_loader_selection(
    minecraft_version: &str,
    fabric_loader_version: &str,
    versions: &[FabricLoaderVersion],
) -> Result<FabricLoaderVersion, CatalogError> {
    validate_token("Minecraft version", minecraft_version)?;
    validate_token("Fabric Loader version", fabric_loader_version)?;
    versions
        .iter()
        .find(|candidate| {
            candidate.minecraft_version == minecraft_version
                && candidate.version == fabric_loader_version
        })
        .cloned()
        .ok_or_else(|| CatalogError::IncompatibleFabricSelection {
            minecraft: minecraft_version.to_owned(),
            loader: fabric_loader_version.to_owned(),
        })
}

fn fabric_loader_url(minecraft_version: &str) -> Result<Url, CatalogError> {
    let mut url = Url::parse(FABRIC_META_BASE_URL)
        .map_err(|error| CatalogError::InvalidInput(error.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| CatalogError::InvalidInput("Fabric Meta URL cannot be a base URL".into()))?
        .extend(["v2", "versions", "loader", minecraft_version]);
    Ok(url)
}

fn validate_token(label: &str, value: &str) -> Result<(), CatalogError> {
    if value.trim() != value || value.is_empty() || value.len() > MAX_TOKEN_BYTES {
        return Err(CatalogError::InvalidInput(format!(
            "{label} must be non-empty, trimmed, and at most {MAX_TOKEN_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(CatalogError::InvalidInput(format!(
            "{label} contains control characters"
        )));
    }
    Ok(())
}

fn validate_provider_string(label: &str, value: &str, max: usize) -> Result<(), CatalogError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(CatalogError::MalformedResponse(format!(
            "{label} is empty, oversized, or contains control characters"
        )));
    }
    Ok(())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    const MOJANG_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/mojang_manifest.json");
    const FABRIC_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/fabric_loaders_26_2.json");

    #[derive(Clone)]
    struct MockTransport {
        calls: Arc<AtomicUsize>,
        responses: Arc<Mutex<VecDeque<Result<Vec<u8>, CatalogError>>>>,
    }

    impl MockTransport {
        fn new(responses: Vec<Result<Vec<u8>, CatalogError>>) -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                responses: Arc::new(Mutex::new(responses.into())),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl CatalogTransport for MockTransport {
        fn get(&self, _url: &Url, _max_response_bytes: usize) -> Result<Vec<u8>, CatalogError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.responses
                .lock()
                .expect("mock response lock")
                .pop_front()
                .unwrap_or_else(|| Err(CatalogError::ProviderUnavailable("no mock response".into())))
        }
    }

    #[test]
    fn parses_mojang_catalog_and_marks_supported_types() {
        let versions = parse_minecraft_catalog(MOJANG_FIXTURE).expect("Mojang fixture should parse");
        assert_eq!(versions[0].id, "26.3-snapshot-1");
        assert_eq!(versions[0].version_type, "snapshot");
        assert!(versions[0].supported);
        assert_eq!(versions[1].id, "26.2");
        assert!(versions[1].supported);
        assert_eq!(versions.last().expect("old beta").version_type, "old_beta");
        assert!(!versions.last().expect("old beta").supported);
    }

    #[test]
    fn stable_minecraft_filter_excludes_snapshots() {
        let versions = parse_minecraft_catalog(MOJANG_FIXTURE).expect("fixture");
        let stable = filter_minecraft_versions(&versions, false).expect("stable releases");
        assert_eq!(stable.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), vec!["26.2", "26.1.5"]);
        let with_snapshots = filter_minecraft_versions(&versions, true).expect("release and snapshots");
        assert_eq!(with_snapshots.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), vec!["26.3-snapshot-1", "26.2", "26.1.5"]);
    }

    #[test]
    fn parses_fabric_catalog_for_exact_minecraft_version() {
        let versions = parse_fabric_loader_catalog("26.2", FABRIC_FIXTURE).expect("Fabric fixture");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, "0.19.3");
        assert!(versions[0].stable);
        assert_eq!(versions[0].minecraft_version, "26.2");
        assert!(!versions[1].stable);
    }

    #[test]
    fn rejects_malformed_provider_payloads() {
        let malformed_mojang = br#"{"versions":[{"id":"26.2","type":"release"}]}"#;
        assert!(matches!(
            parse_minecraft_catalog(malformed_mojang),
            Err(CatalogError::MalformedResponse(_))
        ));
        let malformed_fabric = br#"[{"loader":{"version":"0.19.3"}}]"#;
        assert!(matches!(
            parse_fabric_loader_catalog("26.2", malformed_fabric),
            Err(CatalogError::MalformedResponse(_))
        ));
    }

    #[test]
    fn empty_catalogs_have_explicit_semantics() {
        assert!(matches!(
            parse_minecraft_catalog(br#"{"versions":[]}"#),
            Err(CatalogError::EmptyCatalog("Mojang"))
        ));
        let fabric = parse_fabric_loader_catalog("26.2", b"[]").expect("empty Fabric list is a valid no-compatible-loader result");
        assert!(fabric.is_empty());
    }

    #[test]
    fn incompatible_fabric_selection_is_rejected() {
        let versions = parse_fabric_loader_catalog("26.2", FABRIC_FIXTURE).expect("fixture");
        assert!(matches!(
            validate_fabric_loader_selection("26.2", "9.9.9", &versions),
            Err(CatalogError::IncompatibleFabricSelection { .. })
        ));
        assert!(matches!(
            validate_fabric_loader_selection("26.1.5", "0.19.3", &versions),
            Err(CatalogError::IncompatibleFabricSelection { .. })
        ));
    }

    #[test]
    fn provider_unavailable_without_cache_fails_closed() {
        let directory = tempdir().expect("tempdir");
        let transport = MockTransport::new(vec![Err(CatalogError::ProviderUnavailable("offline".into()))]);
        let service = CatalogService::new(transport, directory.path().to_owned());
        assert!(matches!(
            service.minecraft_versions(false, false),
            Err(CatalogError::ProviderUnavailable(_))
        ));
    }

    #[test]
    fn fresh_cache_avoids_a_second_provider_request() {
        let directory = tempdir().expect("tempdir");
        let transport = MockTransport::new(vec![Ok(MOJANG_FIXTURE.to_vec())]);
        let calls = transport.clone();
        let service = CatalogService::new(transport, directory.path().to_owned());

        let first = service.minecraft_versions(false, false).expect("network catalog");
        assert_eq!(first.origin, CatalogOrigin::Network);
        let second = service.minecraft_versions(false, false).expect("cached catalog");
        assert_eq!(second.origin, CatalogOrigin::FreshCache);
        assert_eq!(calls.calls(), 1);
    }

    #[test]
    fn refresh_failure_uses_source_backed_stale_cache_with_warning() {
        let directory = tempdir().expect("tempdir");
        let transport = MockTransport::new(vec![
            Ok(MOJANG_FIXTURE.to_vec()),
            Err(CatalogError::ProviderUnavailable("offline".into())),
        ]);
        let service = CatalogService::new(transport, directory.path().to_owned());
        service.minecraft_versions(false, false).expect("seed cache");
        let stale = service.minecraft_versions(false, true).expect("stale fallback");
        assert_eq!(stale.origin, CatalogOrigin::StaleCache);
        assert!(stale.warning.as_deref().unwrap_or_default().contains("offline"));
    }

    #[test]
    fn fabric_mapping_and_cache_are_scoped_to_minecraft_version() {
        let directory = tempdir().expect("tempdir");
        let transport = MockTransport::new(vec![
            Ok(FABRIC_FIXTURE.to_vec()),
            Ok(b"[]".to_vec()),
        ]);
        let service = CatalogService::new(transport, directory.path().to_owned());
        let compatible = service.fabric_loader_versions("26.2", false).expect("26.2 loaders");
        assert_eq!(compatible.versions[0].minecraft_version, "26.2");
        let other = service.fabric_loader_versions("26.1.5", false).expect("other MC version");
        assert!(other.versions.is_empty());
    }
}
