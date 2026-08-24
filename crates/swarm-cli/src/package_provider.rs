use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, path::PathBuf};

pub mod modrinth;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Modrinth,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageEnvironment {
    Client,
    Server,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModEnvironment {
    ClientAndServer,
    ClientOnly,
    ClientOnlyServerOptional,
    SingleplayerOnly,
    ServerOnly,
    ServerOnlyClientOptional,
    DedicatedServerOnly,
    ClientOrServer,
    ClientOrServerPrefersBoth,
    Unknown,
}

impl ModEnvironment {
    pub fn supports(self, environment: PackageEnvironment) -> bool {
        match environment {
            PackageEnvironment::Server => matches!(
                self,
                Self::ClientAndServer
                    | Self::ServerOnly
                    | Self::ServerOnlyClientOptional
                    | Self::DedicatedServerOnly
                    | Self::ClientOrServer
                    | Self::ClientOrServerPrefersBoth
            ),
            PackageEnvironment::Client => matches!(
                self,
                Self::ClientAndServer
                    | Self::ClientOnly
                    | Self::ClientOnlyServerOptional
                    | Self::ServerOnlyClientOptional
                    | Self::ClientOrServer
                    | Self::ClientOrServerPrefersBoth
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseType {
    Release,
    Beta,
    Alpha,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModSearchQuery {
    pub query: String,
    pub minecraft_version: String,
    pub loader: String,
    pub environment: PackageEnvironment,
    pub release_type: Option<ReleaseType>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModProjectSummary {
    pub provider: ProviderId,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModLicense {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModProjectDetails {
    pub summary: ModProjectSummary,
    pub status: String,
    pub project_type: String,
    pub environments: Vec<ModEnvironment>,
    pub minecraft_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub license: ModLicense,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModDependency {
    pub kind: DependencyKind,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ArtifactHashes {
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModArtifactLocator {
    pub provider: ProviderId,
    pub project_id: String,
    pub version_id: String,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArtifactRetrieval {
    ProviderDownload,
    ManualRequired { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModArtifact {
    pub filename: String,
    pub url: Option<String>,
    pub locator: ModArtifactLocator,
    pub primary: bool,
    pub size: u64,
    pub hashes: ArtifactHashes,
    pub file_type: Option<String>,
    pub retrieval: ArtifactRetrieval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModVersion {
    pub provider: ProviderId,
    pub project_id: String,
    pub version_id: String,
    pub display_name: String,
    pub version_number: String,
    pub minecraft_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub environment: ModEnvironment,
    pub release_type: ReleaseType,
    pub published_at: String,
    pub dependencies: Vec<ModDependency>,
    pub files: Vec<ModArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModVersionFilter {
    pub minecraft_version: String,
    pub loader: String,
    pub environment: PackageEnvironment,
    pub release_type: Option<ReleaseType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateLimitSnapshot {
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub reset_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModSearchResult {
    pub items: Vec<ModProjectSummary>,
    pub offset: u32,
    pub limit: u32,
    pub total_hits: u64,
    pub rate_limit: Option<RateLimitSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModVersionList {
    pub items: Vec<ModVersion>,
    pub rate_limit: Option<RateLimitSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModResolveRequest {
    pub root_version_id: String,
    pub minecraft_version: String,
    pub loader: String,
    pub environment: PackageEnvironment,
    #[serde(default)]
    pub allowed_release_types: Vec<ReleaseType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedModGraph {
    pub provider: ProviderId,
    pub root_version_id: String,
    pub versions: Vec<ModVersion>,
    pub optional_dependencies: Vec<ModDependency>,
    pub incompatibilities: Vec<ModDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModDownloadRequest {
    pub locator: ModArtifactLocator,
    pub destination_dir: PathBuf,
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadedArtifact {
    pub provider: ProviderId,
    pub project_id: String,
    pub version_id: String,
    pub filename: String,
    pub path: PathBuf,
    pub size: u64,
    pub source_url: String,
    pub hashes: ArtifactHashes,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFailureKind {
    InvalidRequest,
    RateLimited,
    Unavailable,
    NotFound,
    MalformedResponse,
    Incompatible,
    DependencyCycle,
    UnresolvedDependency,
    HashMismatch,
    DownloadInterrupted,
    RetrievalRestricted,
    Io,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderFailure {
    pub provider: ProviderId,
    pub kind: ProviderFailureKind,
    pub message: String,
    pub retry_after_seconds: Option<u64>,
    pub remediation: Option<String>,
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

impl ProviderFailure {
    pub fn new(kind: ProviderFailureKind, message: impl Into<String>) -> Self {
        Self {
            provider: ProviderId::Modrinth,
            kind,
            message: message.into(),
            retry_after_seconds: None,
            remediation: None,
            details: BTreeMap::new(),
        }
    }

    pub fn with_retry_after(mut self, seconds: Option<u64>) -> Self {
        self.retry_after_seconds = seconds;
        self
    }

    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProviderFailure {}

pub type ProviderResult<T> = Result<T, ProviderFailure>;
