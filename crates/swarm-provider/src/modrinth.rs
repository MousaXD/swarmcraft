use crate::{
    ArtifactHashes, ArtifactRetrieval, DependencyKind, DownloadedArtifact, ModArtifact, ModArtifactLocator,
    ModDependency, ModDownloadRequest, ModEnvironment, ModLicense, ModProjectDetails, ModProjectSummary,
    ModResolveRequest, ModSearchQuery, ModSearchResult, ModVersion, ModVersionFilter, ModVersionList,
    PackageEnvironment, ProviderFailure, ProviderFailureKind, ProviderId, ProviderResult, RateLimitSnapshot,
    ReleaseType, ResolvedModGraph,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub const MODRINTH_API_BASE: &str = "https://api.modrinth.com/v2";
pub const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
pub const ABSOLUTE_MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub trait ModrinthTransport {
    fn get(&self, url: &str) -> ProviderResult<HttpResponse>;
    fn download(&self, url: &str, destination: &Path, max_bytes: u64) -> ProviderResult<()>;
}

#[derive(Debug, Clone)]
pub struct CurlTransport {
    user_agent: String,
}

impl CurlTransport {
    pub fn new(user_agent: impl Into<String>) -> ProviderResult<Self> {
        let user_agent = user_agent.into();
        if user_agent.trim().is_empty() {
            return Err(failure(ProviderFailureKind::InvalidRequest, "Modrinth User-Agent must identify SwarmCraft"));
        }
        Ok(Self { user_agent })
    }
}

impl ModrinthTransport for CurlTransport {
    fn get(&self, url: &str) -> ProviderResult<HttpResponse> {
        trusted_https(url, &["api.modrinth.com"])?;
        let headers_path = temporary_path("modrinth-headers");
        let body_path = temporary_path("modrinth-body");
        let output = Command::new("curl")
            .args([
                "-sS",
                "-L",
                "--proto",
                "=https",
                "--connect-timeout",
                "15",
                "--max-time",
                "60",
                "-A",
            ])
            .arg(&self.user_agent)
            .arg("-D")
            .arg(&headers_path)
            .arg("-o")
            .arg(&body_path)
            .arg("--write-out")
            .arg("%{http_code}")
            .arg(url)
            .output()
            .map_err(|error| failure(ProviderFailureKind::Unavailable, format!("cannot start curl for Modrinth: {error}")))?;

        let status_text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() {
            let _ = fs::remove_file(&headers_path);
            let _ = fs::remove_file(&body_path);
            return Err(failure(
                ProviderFailureKind::Unavailable,
                format!("Modrinth request failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            ));
        }
        let status = status_text.parse::<u16>().map_err(|_| {
            failure(
                ProviderFailureKind::MalformedResponse,
                format!("curl returned an invalid HTTP status for Modrinth: {status_text}"),
            )
        })?;
        let headers = fs::read_to_string(&headers_path)
            .map(|text| parse_headers(&text))
            .unwrap_or_default();
        let body = fs::read(&body_path).map_err(|error| {
            failure(ProviderFailureKind::Io, format!("cannot read Modrinth response body: {error}"))
        })?;
        let _ = fs::remove_file(&headers_path);
        let _ = fs::remove_file(&body_path);
        Ok(HttpResponse { status, headers, body })
    }

    fn download(&self, url: &str, destination: &Path, max_bytes: u64) -> ProviderResult<()> {
        trusted_https(url, &["cdn.modrinth.com"])?;
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
            .arg(max_bytes.to_string())
            .arg("-A")
            .arg(&self.user_agent)
            .arg("-o")
            .arg(destination)
            .arg("--write-out")
            .arg("%{http_code}")
            .arg(url)
            .output()
            .map_err(|error| failure(ProviderFailureKind::DownloadInterrupted, format!("cannot start Modrinth download: {error}")))?;

        if !output.status.success() {
            let _ = fs::remove_file(destination);
            return Err(failure(
                ProviderFailureKind::DownloadInterrupted,
                format!("Modrinth artifact download failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            ));
        }
        let status = String::from_utf8_lossy(&output.stdout).trim().parse::<u16>().map_err(|_| {
            failure(ProviderFailureKind::MalformedResponse, "Modrinth artifact download returned an invalid HTTP status")
        })?;
        if !(200..300).contains(&status) {
            let _ = fs::remove_file(destination);
            return Err(http_failure(status, &BTreeMap::new(), "Modrinth artifact"));
        }
        let size = fs::metadata(destination)
            .map_err(|error| failure(ProviderFailureKind::Io, format!("cannot inspect downloaded Modrinth artifact: {error}")))?
            .len();
        if size > max_bytes {
            let _ = fs::remove_file(destination);
            return Err(failure(
                ProviderFailureKind::DownloadInterrupted,
                format!("Modrinth artifact exceeded the {max_bytes}-byte download bound"),
            ));
        }
        Ok(())
    }
}

pub struct ModrinthClient<T = CurlTransport> {
    transport: T,
    base_url: String,
}

impl ModrinthClient<CurlTransport> {
    pub fn production() -> ProviderResult<Self> {
        let user_agent = format!(
            "MousaXD/swarmcraft/{} (https://github.com/MousaXD/swarmcraft)",
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self { transport: CurlTransport::new(user_agent)?, base_url: MODRINTH_API_BASE.to_owned() })
    }
}

impl<T: ModrinthTransport> ModrinthClient<T> {
    pub fn with_transport(base_url: impl Into<String>, transport: T) -> ProviderResult<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        trusted_https(&base_url, &["api.modrinth.com", "fixtures.invalid"])?;
        Ok(Self { transport, base_url })
    }

    pub fn search(&self, query: &ModSearchQuery) -> ProviderResult<ModSearchResult> {
        validate_search_query(query)?;
        let facets = search_facets(query.environment, &query.minecraft_version, &query.loader);
        let facets = serde_json::to_string(&facets)
            .map_err(|error| failure(ProviderFailureKind::InvalidRequest, format!("cannot encode Modrinth search facets: {error}")))?;
        let url = self.url(
            "/search",
            &[
                ("query", query.query.as_str()),
                ("facets", facets.as_str()),
                ("offset", &query.offset.to_string()),
                ("limit", &query.limit.to_string()),
            ],
        );
        let (raw, headers): (RawSearchResponse, _) = self.request_json(&url, "Modrinth search")?;
        let items = raw
            .hits
            .into_iter()
            .filter(|hit| hit.project_type == "mod")
            .map(|hit| ModProjectSummary {
                provider: ProviderId::Modrinth,
                project_id: hit.project_id,
                slug: hit.slug,
                title: hit.title,
                description: hit.description,
                icon_url: hit.icon_url,
                categories: hit.categories,
            })
            .collect();
        Ok(ModSearchResult {
            items,
            offset: raw.offset,
            limit: raw.limit,
            total_hits: raw.total_hits,
            rate_limit: rate_limit(&headers),
        })
    }

    pub fn project(&self, project_id_or_slug: &str) -> ProviderResult<ModProjectDetails> {
        let id = require_nonempty(project_id_or_slug, "Modrinth project ID or slug")?;
        let url = format!("{}/project/{}", self.base_url, encode_path_segment(id));
        let (raw, _): (RawProject, _) = self.request_json(&url, "Modrinth project")?;
        if raw.project_type != "mod" {
            return Err(failure(
                ProviderFailureKind::Incompatible,
                format!("Modrinth project {} is {}, not a mod", raw.id, raw.project_type),
            ));
        }
        let mut categories = raw.categories;
        categories.extend(raw.additional_categories);
        categories.sort();
        categories.dedup();
        Ok(ModProjectDetails {
            summary: ModProjectSummary {
                provider: ProviderId::Modrinth,
                project_id: raw.id,
                slug: raw.slug.unwrap_or_else(|| id.to_owned()),
                title: raw.title,
                description: raw.description,
                icon_url: raw.icon_url,
                categories,
            },
            status: raw.status,
            project_type: raw.project_type,
            environments: raw.environment.into_iter().map(parse_environment).collect(),
            minecraft_versions: raw.game_versions,
            loaders: raw.loaders,
            license: ModLicense { id: raw.license.id, name: raw.license.name, url: raw.license.url },
        })
    }

    pub fn versions(&self, project_id: &str, filter: &ModVersionFilter) -> ProviderResult<ModVersionList> {
        validate_filter(filter)?;
        let project_id = require_nonempty(project_id, "Modrinth project ID")?;
        let loaders = serde_json::to_string(&[filter.loader.as_str()]).map_err(|error| {
            failure(ProviderFailureKind::InvalidRequest, format!("cannot encode Modrinth loader filter: {error}"))
        })?;
        let game_versions = serde_json::to_string(&[filter.minecraft_version.as_str()]).map_err(|error| {
            failure(ProviderFailureKind::InvalidRequest, format!("cannot encode Modrinth Minecraft filter: {error}"))
        })?;
        let path = format!("/project/{}/version", encode_path_segment(project_id));
        let url = self.url(
            &path,
            &[
                ("loaders", loaders.as_str()),
                ("game_versions", game_versions.as_str()),
                ("include_changelog", "false"),
            ],
        );
        let (raw, headers): (Vec<RawVersion>, _) = self.request_json(&url, "Modrinth project versions")?;
        let mut items = Vec::new();
        for raw_version in raw {
            let version = convert_version(raw_version)?;
            if version_compatible(&version, filter) {
                items.push(version);
            }
        }
        items.sort_by(|a, b| b.published_at.cmp(&a.published_at).then_with(|| a.version_id.cmp(&b.version_id)));
        Ok(ModVersionList { items, rate_limit: rate_limit(&headers) })
    }

    pub fn version(&self, version_id: &str) -> ProviderResult<ModVersion> {
        let version_id = require_nonempty(version_id, "Modrinth version ID")?;
        let url = format!("{}/version/{}", self.base_url, encode_path_segment(version_id));
        let (raw, _): (RawVersion, _) = self.request_json(&url, "Modrinth version")?;
        convert_version(raw)
    }

    pub fn resolve(&self, request: &ModResolveRequest) -> ProviderResult<ResolvedModGraph> {
        validate_resolve_request(request)?;
        let root = self.version(&request.root_version_id)?;
        let mut state = ResolutionState::default();
        self.resolve_version(root, request, &mut state)?;
        validate_incompatibilities(&state)?;
        let mut versions: Vec<_> = state.versions.into_values().collect();
        versions.sort_by(|a, b| a.project_id.cmp(&b.project_id).then_with(|| a.version_id.cmp(&b.version_id)));
        Ok(ResolvedModGraph {
            provider: ProviderId::Modrinth,
            root_version_id: request.root_version_id.clone(),
            versions,
            optional_dependencies: state.optional_dependencies,
            incompatibilities: state.incompatibilities,
        })
    }

    pub fn download(&self, request: &ModDownloadRequest) -> ProviderResult<DownloadedArtifact> {
        if request.locator.provider != ProviderId::Modrinth {
            return Err(failure(ProviderFailureKind::InvalidRequest, "artifact locator does not belong to Modrinth"));
        }
        if request.locator.sha1.is_none() && request.locator.sha512.is_none() {
            return Err(failure(
                ProviderFailureKind::InvalidRequest,
                "Modrinth artifact locator must include a provider SHA-1 or SHA-512 identity",
            ));
        }
        let version = self.version(&request.locator.version_id)?;
        if version.project_id != request.locator.project_id {
            return Err(failure(
                ProviderFailureKind::Incompatible,
                "Modrinth version does not belong to the requested project",
            ));
        }
        let artifact = version
            .files
            .iter()
            .find(|file| locator_matches(&request.locator, file))
            .cloned()
            .ok_or_else(|| failure(ProviderFailureKind::NotFound, "exact Modrinth file hash is no longer present on the selected version"))?;
        let source_url = match &artifact.retrieval {
            ArtifactRetrieval::ProviderDownload => artifact.url.clone().ok_or_else(|| {
                failure(ProviderFailureKind::RetrievalRestricted, "Modrinth file has no provider download URL")
            })?,
            ArtifactRetrieval::ManualRequired { reason } => {
                return Err(failure(ProviderFailureKind::RetrievalRestricted, reason.clone()).with_remediation(
                    "Obtain the exact artifact from its authorized source and import it through the canonical modpack/local-artifact flow.",
                ));
            }
        };
        trusted_https(&source_url, &["cdn.modrinth.com"])?;

        let max_bytes = request.max_bytes.unwrap_or(DEFAULT_MAX_ARTIFACT_BYTES);
        if max_bytes == 0 || max_bytes > ABSOLUTE_MAX_ARTIFACT_BYTES {
            return Err(failure(
                ProviderFailureKind::InvalidRequest,
                format!("artifact download bound must be between 1 and {ABSOLUTE_MAX_ARTIFACT_BYTES} bytes"),
            ));
        }
        if artifact.size > max_bytes {
            return Err(failure(
                ProviderFailureKind::RetrievalRestricted,
                format!("Modrinth artifact is {} bytes, exceeding the configured {max_bytes}-byte bound", artifact.size),
            ));
        }
        safe_filename(&artifact.filename)?;
        fs::create_dir_all(&request.destination_dir).map_err(|error| {
            failure(ProviderFailureKind::Io, format!("cannot create Modrinth artifact directory: {error}"))
        })?;
        let destination = request.destination_dir.join(&artifact.filename);
        let temporary = request
            .destination_dir
            .join(format!(".{}.part-{}", artifact.filename, unique_suffix()));
        if let Err(error) = self.transport.download(&source_url, &temporary, max_bytes) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }

        let verification = (|| -> ProviderResult<ArtifactHashes> {
            let metadata = fs::metadata(&temporary).map_err(|error| {
                failure(ProviderFailureKind::Io, format!("cannot inspect temporary Modrinth artifact: {error}"))
            })?;
            if metadata.len() != artifact.size {
                return Err(failure(
                    ProviderFailureKind::DownloadInterrupted,
                    format!("Modrinth artifact size mismatch: expected {}, received {}", artifact.size, metadata.len()),
                ));
            }
            let hashes = hash_file(&temporary)?;
            verify_provider_hashes(&artifact.hashes, &hashes)?;
            OpenOptions::new()
                .write(true)
                .open(&temporary)
                .and_then(|file| file.sync_all())
                .map_err(|error| failure(ProviderFailureKind::Io, format!("cannot fsync Modrinth artifact: {error}")))?;
            Ok(hashes)
        })();

        let hashes = match verification {
            Ok(hashes) => hashes,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
        };
        publish_replace(&temporary, &destination)?;
        Ok(DownloadedArtifact {
            provider: ProviderId::Modrinth,
            project_id: version.project_id,
            version_id: version.version_id,
            filename: artifact.filename,
            path: destination,
            size: artifact.size,
            source_url,
            hashes,
        })
    }

    fn resolve_version(
        &self,
        version: ModVersion,
        request: &ModResolveRequest,
        state: &mut ResolutionState,
    ) -> ProviderResult<()> {
        if state.visiting.iter().any(|id| id == &version.version_id) {
            let mut cycle = state.visiting.clone();
            cycle.push(version.version_id.clone());
            return Err(failure(ProviderFailureKind::DependencyCycle, "Modrinth dependency cycle detected")
                .with_detail("cycle", cycle.join(" -> ")));
        }
        if state.versions.contains_key(&version.version_id) {
            return Ok(());
        }
        validate_resolved_version(&version, request)?;
        if let Some(existing) = state.selected_by_project.get(&version.project_id) {
            if existing != &version.version_id {
                return Err(failure(
                    ProviderFailureKind::Incompatible,
                    format!(
                        "required dependencies selected two versions of Modrinth project {}: {} and {}",
                        version.project_id, existing, version.version_id
                    ),
                ));
            }
        } else {
            state.selected_by_project.insert(version.project_id.clone(), version.version_id.clone());
        }

        state.visiting.push(version.version_id.clone());
        for dependency in version.dependencies.clone() {
            match dependency.kind {
                DependencyKind::Required => {
                    let required = self.resolve_required_dependency(&dependency, request)?;
                    self.resolve_version(required, request, state)?;
                }
                DependencyKind::Optional => state.optional_dependencies.push(dependency),
                DependencyKind::Incompatible => state.incompatibilities.push(dependency),
                DependencyKind::Embedded => {}
            }
        }
        state.visiting.pop();
        state.versions.insert(version.version_id.clone(), version);
        Ok(())
    }

    fn resolve_required_dependency(
        &self,
        dependency: &ModDependency,
        request: &ModResolveRequest,
    ) -> ProviderResult<ModVersion> {
        if let Some(version_id) = dependency.version_id.as_deref() {
            let version = self.version(version_id).map_err(|error| {
                if error.kind == ProviderFailureKind::NotFound {
                    failure(
                        ProviderFailureKind::UnresolvedDependency,
                        format!("required Modrinth dependency version {version_id} was removed or is unavailable"),
                    )
                } else {
                    error
                }
            })?;
            if let Some(project_id) = dependency.project_id.as_deref() {
                if version.project_id != project_id {
                    return Err(failure(
                        ProviderFailureKind::UnresolvedDependency,
                        format!("required dependency version {version_id} belongs to a different Modrinth project"),
                    ));
                }
            }
            return Ok(version);
        }
        let project_id = dependency.project_id.as_deref().ok_or_else(|| {
            failure(
                ProviderFailureKind::UnresolvedDependency,
                "required Modrinth dependency has neither a project ID nor a version ID",
            )
        })?;
        let filter = ModVersionFilter {
            minecraft_version: request.minecraft_version.clone(),
            loader: request.loader.clone(),
            environment: request.environment,
            release_type: None,
        };
        let mut candidates = self.versions(project_id, &filter)?.items;
        if !request.allowed_release_types.is_empty() {
            candidates.retain(|version| request.allowed_release_types.contains(&version.release_type));
        }
        candidates.into_iter().next().ok_or_else(|| {
            failure(
                ProviderFailureKind::UnresolvedDependency,
                format!(
                    "required Modrinth dependency {project_id} has no Fabric version compatible with Minecraft {} and the requested environment",
                    request.minecraft_version
                ),
            )
        })
    }

    fn request_json<R: DeserializeOwned>(&self, url: &str, label: &str) -> ProviderResult<(R, BTreeMap<String, String>)> {
        let response = self.transport.get(url)?;
        if !(200..300).contains(&response.status) {
            return Err(http_failure(response.status, &response.headers, label));
        }
        let parsed = serde_json::from_slice(&response.body).map_err(|error| {
            failure(
                ProviderFailureKind::MalformedResponse,
                format!("{label} returned malformed JSON: {error}"),
            )
        })?;
        Ok((parsed, response.headers))
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> String {
        let mut url = format!("{}{}", self.base_url, path);
        if !query.is_empty() {
            url.push('?');
            for (index, (key, value)) in query.iter().enumerate() {
                if index > 0 {
                    url.push('&');
                }
                url.push_str(&percent_encode(key));
                url.push('=');
                url.push_str(&percent_encode(value));
            }
        }
        url
    }
}

#[derive(Default)]
struct ResolutionState {
    visiting: Vec<String>,
    versions: BTreeMap<String, ModVersion>,
    selected_by_project: BTreeMap<String, String>,
    optional_dependencies: Vec<ModDependency>,
    incompatibilities: Vec<ModDependency>,
}

#[derive(Deserialize)]
struct RawSearchResponse {
    hits: Vec<RawSearchHit>,
    offset: u32,
    limit: u32,
    total_hits: u64,
}

#[derive(Deserialize)]
struct RawSearchHit {
    project_id: String,
    project_type: String,
    slug: String,
    title: String,
    description: String,
    #[serde(default)]
    categories: Vec<String>,
    icon_url: Option<String>,
}

#[derive(Deserialize)]
struct RawProject {
    id: String,
    title: String,
    description: String,
    status: String,
    project_type: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    additional_categories: Vec<String>,
    environment: Vec<String>,
    game_versions: Vec<String>,
    loaders: Vec<String>,
    license: RawLicense,
    slug: Option<String>,
    icon_url: Option<String>,
}

#[derive(Deserialize)]
struct RawLicense {
    id: String,
    name: String,
    url: Option<String>,
}

#[derive(Deserialize)]
struct RawVersion {
    name: String,
    version_number: String,
    dependencies: Vec<RawDependency>,
    game_versions: Vec<String>,
    version_type: String,
    loaders: Vec<String>,
    id: String,
    project_id: String,
    date_published: String,
    environment: String,
    files: Vec<RawFile>,
}

#[derive(Deserialize)]
struct RawDependency {
    version_id: Option<String>,
    project_id: Option<String>,
    file_name: Option<String>,
    dependency_type: String,
}

#[derive(Deserialize)]
struct RawFile {
    hashes: BTreeMap<String, String>,
    url: String,
    filename: String,
    primary: bool,
    size: u64,
    file_type: Option<String>,
}

fn convert_version(raw: RawVersion) -> ProviderResult<ModVersion> {
    let release_type = match raw.version_type.as_str() {
        "release" => ReleaseType::Release,
        "beta" => ReleaseType::Beta,
        "alpha" => ReleaseType::Alpha,
        other => {
            return Err(failure(
                ProviderFailureKind::MalformedResponse,
                format!("Modrinth version {} has unknown release type {other}", raw.id),
            ));
        }
    };
    let dependencies = raw
        .dependencies
        .into_iter()
        .map(|dependency| {
            let kind = match dependency.dependency_type.as_str() {
                "required" => DependencyKind::Required,
                "optional" => DependencyKind::Optional,
                "incompatible" => DependencyKind::Incompatible,
                "embedded" => DependencyKind::Embedded,
                other => {
                    return Err(failure(
                        ProviderFailureKind::MalformedResponse,
                        format!("Modrinth version {} has unknown dependency type {other}", raw.id),
                    ));
                }
            };
            Ok(ModDependency {
                kind,
                project_id: dependency.project_id,
                version_id: dependency.version_id,
                file_name: dependency.file_name,
            })
        })
        .collect::<ProviderResult<Vec<_>>>()?;

    let has_primary = raw.files.iter().any(|file| file.primary);
    let mut files = Vec::with_capacity(raw.files.len());
    for (index, file) in raw.files.into_iter().enumerate() {
        files.push(convert_file(&raw.project_id, &raw.id, file, !has_primary && index == 0));
    }
    Ok(ModVersion {
        provider: ProviderId::Modrinth,
        project_id: raw.project_id,
        version_id: raw.id,
        display_name: raw.name,
        version_number: raw.version_number,
        minecraft_versions: raw.game_versions,
        loaders: raw.loaders,
        environment: parse_environment(raw.environment),
        release_type,
        published_at: raw.date_published,
        dependencies,
        files,
    })
}

fn convert_file(project_id: &str, version_id: &str, raw: RawFile, infer_primary: bool) -> ModArtifact {
    let sha1 = raw.hashes.get("sha1").cloned();
    let sha512 = raw.hashes.get("sha512").cloned();
    let hashes = ArtifactHashes { sha1: sha1.clone(), sha512: sha512.clone(), sha256: None };
    let file_type_installable = raw.file_type.as_deref().is_none_or(|kind| kind == "unknown");
    let trusted_url = trusted_https(&raw.url, &["cdn.modrinth.com"]).is_ok();
    let has_provider_hash = sha1.is_some() || sha512.is_some();
    let is_jar = raw.filename.to_ascii_lowercase().ends_with(".jar");
    let retrieval = if file_type_installable && trusted_url && has_provider_hash && is_jar {
        ArtifactRetrieval::ProviderDownload
    } else {
        let reason = if !is_jar {
            "selected Modrinth file is not an installable JAR"
        } else if !file_type_installable {
            "selected Modrinth file is an auxiliary artifact rather than the installable mod JAR"
        } else if !trusted_url {
            "Modrinth file URL is outside the authorized Modrinth CDN boundary"
        } else {
            "Modrinth file has no provider-published SHA-1 or SHA-512 hash"
        };
        ArtifactRetrieval::ManualRequired { reason: reason.to_owned() }
    };
    ModArtifact {
        filename: raw.filename,
        url: trusted_url.then_some(raw.url),
        locator: ModArtifactLocator {
            provider: ProviderId::Modrinth,
            project_id: project_id.to_owned(),
            version_id: version_id.to_owned(),
            sha1,
            sha512,
        },
        primary: raw.primary || infer_primary,
        size: raw.size,
        hashes,
        file_type: raw.file_type,
        retrieval,
    }
}

fn parse_environment(value: impl AsRef<str>) -> ModEnvironment {
    match value.as_ref() {
        "client_and_server" => ModEnvironment::ClientAndServer,
        "client_only" => ModEnvironment::ClientOnly,
        "client_only_server_optional" => ModEnvironment::ClientOnlyServerOptional,
        "singleplayer_only" => ModEnvironment::SingleplayerOnly,
        "server_only" => ModEnvironment::ServerOnly,
        "server_only_client_optional" => ModEnvironment::ServerOnlyClientOptional,
        "dedicated_server_only" => ModEnvironment::DedicatedServerOnly,
        "client_or_server" => ModEnvironment::ClientOrServer,
        "client_or_server_prefers_both" => ModEnvironment::ClientOrServerPrefersBoth,
        _ => ModEnvironment::Unknown,
    }
}

fn validate_search_query(query: &ModSearchQuery) -> ProviderResult<()> {
    require_nonempty(&query.minecraft_version, "Minecraft version")?;
    validate_loader(&query.loader)?;
    if query.limit == 0 || query.limit > 100 {
        return Err(failure(ProviderFailureKind::InvalidRequest, "Modrinth search limit must be between 1 and 100"));
    }
    Ok(())
}

fn validate_filter(filter: &ModVersionFilter) -> ProviderResult<()> {
    require_nonempty(&filter.minecraft_version, "Minecraft version")?;
    validate_loader(&filter.loader)?;
    Ok(())
}

fn validate_resolve_request(request: &ModResolveRequest) -> ProviderResult<()> {
    require_nonempty(&request.root_version_id, "root Modrinth version ID")?;
    require_nonempty(&request.minecraft_version, "Minecraft version")?;
    validate_loader(&request.loader)?;
    let unique: BTreeSet<_> = request.allowed_release_types.iter().copied().collect();
    if unique.len() != request.allowed_release_types.len() {
        return Err(failure(ProviderFailureKind::InvalidRequest, "allowed release types contain duplicates"));
    }
    Ok(())
}

fn validate_loader(loader: &str) -> ProviderResult<()> {
    let loader = require_nonempty(loader, "mod loader")?;
    if !loader.eq_ignore_ascii_case("fabric") {
        return Err(failure(
            ProviderFailureKind::Incompatible,
            format!("Modrinth provider currently prepares Fabric mods, not {loader}"),
        ));
    }
    Ok(())
}

fn validate_resolved_version(version: &ModVersion, request: &ModResolveRequest) -> ProviderResult<()> {
    let filter = ModVersionFilter {
        minecraft_version: request.minecraft_version.clone(),
        loader: request.loader.clone(),
        environment: request.environment,
        release_type: None,
    };
    if !version_compatible(version, &filter) {
        return Err(failure(
            ProviderFailureKind::Incompatible,
            format!(
                "Modrinth version {} is not compatible with Minecraft {}, Fabric, and the requested environment",
                version.version_id, request.minecraft_version
            ),
        ));
    }
    if !request.allowed_release_types.is_empty() && !request.allowed_release_types.contains(&version.release_type) {
        return Err(failure(
            ProviderFailureKind::Incompatible,
            format!("Modrinth version {} has a disallowed release type", version.version_id),
        ));
    }
    Ok(())
}

fn version_compatible(version: &ModVersion, filter: &ModVersionFilter) -> bool {
    version.minecraft_versions.iter().any(|value| value == &filter.minecraft_version)
        && version.loaders.iter().any(|value| value.eq_ignore_ascii_case(&filter.loader))
        && version.environment.supports(filter.environment)
        && filter.release_type.is_none_or(|release_type| version.release_type == release_type)
}

fn validate_incompatibilities(state: &ResolutionState) -> ProviderResult<()> {
    for dependency in &state.incompatibilities {
        if let Some(version_id) = dependency.version_id.as_deref() {
            if state.versions.contains_key(version_id) {
                return Err(failure(
                    ProviderFailureKind::Incompatible,
                    format!("resolved dependency graph includes Modrinth version {version_id}, which another selected version marks incompatible"),
                ));
            }
        } else if let Some(project_id) = dependency.project_id.as_deref() {
            if let Some(version_id) = state.selected_by_project.get(project_id) {
                return Err(failure(
                    ProviderFailureKind::Incompatible,
                    format!(
                        "resolved dependency graph includes Modrinth project {project_id} version {version_id}, which another selected version marks incompatible"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn locator_matches(locator: &ModArtifactLocator, artifact: &ModArtifact) -> bool {
    locator.project_id == artifact.locator.project_id
        && locator.version_id == artifact.locator.version_id
        && locator.sha1.as_ref().is_none_or(|expected| artifact.hashes.sha1.as_ref().is_some_and(|actual| eq_hash(expected, actual)))
        && locator.sha512.as_ref().is_none_or(|expected| artifact.hashes.sha512.as_ref().is_some_and(|actual| eq_hash(expected, actual)))
}

fn verify_provider_hashes(expected: &ArtifactHashes, actual: &ArtifactHashes) -> ProviderResult<()> {
    if expected.sha1.is_none() && expected.sha512.is_none() {
        return Err(failure(
            ProviderFailureKind::RetrievalRestricted,
            "Modrinth artifact lacks a provider-published cryptographic hash",
        ));
    }
    if let Some(expected_sha1) = expected.sha1.as_deref() {
        let actual_sha1 = actual.sha1.as_deref().unwrap_or_default();
        if !eq_hash(expected_sha1, actual_sha1) {
            return Err(failure(ProviderFailureKind::HashMismatch, "downloaded Modrinth artifact SHA-1 mismatch"));
        }
    }
    if let Some(expected_sha512) = expected.sha512.as_deref() {
        let actual_sha512 = actual.sha512.as_deref().unwrap_or_default();
        if !eq_hash(expected_sha512, actual_sha512) {
            return Err(failure(ProviderFailureKind::HashMismatch, "downloaded Modrinth artifact SHA-512 mismatch"));
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> ProviderResult<ArtifactHashes> {
    let mut file = fs::File::open(path)
        .map_err(|error| failure(ProviderFailureKind::Io, format!("cannot open Modrinth artifact for hashing: {error}")))?;
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| failure(ProviderFailureKind::Io, format!("cannot read Modrinth artifact for hashing: {error}")))?;
        if read == 0 {
            break;
        }
        sha1.update(&buffer[..read]);
        sha256.update(&buffer[..read]);
        sha512.update(&buffer[..read]);
    }
    Ok(ArtifactHashes {
        sha1: Some(lower_hex(&sha1.finalize())),
        sha512: Some(lower_hex(&sha512.finalize())),
        sha256: Some(lower_hex(&sha256.finalize())),
    })
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn search_facets(environment: PackageEnvironment, minecraft_version: &str, loader: &str) -> Vec<Vec<String>> {
    vec![
        vec!["project_type:mod".to_owned()],
        vec![format!("categories:{}", loader.to_ascii_lowercase())],
        vec![format!("versions:{minecraft_version}")],
        environment_facet_values(environment)
            .into_iter()
            .map(|value| format!("environment:{value}"))
            .collect(),
    ]
}

fn environment_facet_values(environment: PackageEnvironment) -> Vec<&'static str> {
    match environment {
        PackageEnvironment::Server => vec![
            "client_and_server",
            "server_only",
            "server_only_client_optional",
            "dedicated_server_only",
            "client_or_server",
            "client_or_server_prefers_both",
        ],
        PackageEnvironment::Client => vec![
            "client_and_server",
            "client_only",
            "client_only_server_optional",
            "server_only_client_optional",
            "client_or_server",
            "client_or_server_prefers_both",
        ],
    }
}

fn rate_limit(headers: &BTreeMap<String, String>) -> Option<RateLimitSnapshot> {
    let snapshot = RateLimitSnapshot {
        limit: header_u64(headers, "x-ratelimit-limit"),
        remaining: header_u64(headers, "x-ratelimit-remaining"),
        reset_seconds: header_u64(headers, "x-ratelimit-reset"),
    };
    (snapshot.limit.is_some() || snapshot.remaining.is_some() || snapshot.reset_seconds.is_some()).then_some(snapshot)
}

fn http_failure(status: u16, headers: &BTreeMap<String, String>, label: &str) -> ProviderFailure {
    match status {
        404 => failure(ProviderFailureKind::NotFound, format!("{label} was not found or was removed")),
        410 => failure(
            ProviderFailureKind::Unavailable,
            "Modrinth API v2 is no longer available; SwarmCraft must update its provider implementation",
        )
        .with_remediation("Update SwarmCraft to a version that supports Modrinth's current API."),
        429 => failure(ProviderFailureKind::RateLimited, "Modrinth rate limit exceeded")
            .with_retry_after(header_u64(headers, "retry-after").or_else(|| header_u64(headers, "x-ratelimit-reset"))),
        500..=599 => failure(ProviderFailureKind::Unavailable, format!("{label} is temporarily unavailable (HTTP {status})")),
        _ => failure(ProviderFailureKind::Unavailable, format!("{label} failed with HTTP {status}")),
    }
}

fn parse_headers(text: &str) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for line in text.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    headers
}

fn header_u64(headers: &BTreeMap<String, String>, name: &str) -> Option<u64> {
    headers.get(name).and_then(|value| value.trim().parse().ok())
}

fn safe_filename(filename: &str) -> ProviderResult<()> {
    let path = Path::new(filename);
    let file_name = path.file_name().and_then(|value| value.to_str());
    if file_name != Some(filename) || filename.is_empty() {
        return Err(failure(ProviderFailureKind::MalformedResponse, "Modrinth returned an unsafe artifact filename"));
    }
    Ok(())
}

fn publish_replace(temporary: &Path, destination: &Path) -> ProviderResult<()> {
    if !destination.exists() {
        return fs::rename(temporary, destination)
            .map_err(|error| failure(ProviderFailureKind::Io, format!("cannot publish Modrinth artifact: {error}")));
    }
    let backup = destination.with_extension(format!("backup-{}", unique_suffix()));
    fs::rename(destination, &backup)
        .map_err(|error| failure(ProviderFailureKind::Io, format!("cannot preserve existing artifact before replacement: {error}")))?;
    match fs::rename(temporary, destination) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            let restore = fs::rename(&backup, destination);
            let _ = fs::remove_file(temporary);
            if let Err(restore_error) = restore {
                return Err(failure(
                    ProviderFailureKind::Io,
                    format!("artifact publication failed ({error}) and rollback failed ({restore_error}); backup remains at {}", backup.display()),
                ));
            }
            Err(failure(ProviderFailureKind::Io, format!("cannot publish Modrinth artifact: {error}")))
        }
    }
}

fn require_nonempty<'a>(value: &'a str, label: &str) -> ProviderResult<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        Err(failure(ProviderFailureKind::InvalidRequest, format!("{label} is required")))
    } else {
        Ok(value)
    }
}

fn trusted_https(url: &str, hosts: &[&str]) -> ProviderResult<()> {
    let rest = url.strip_prefix("https://").ok_or_else(|| {
        failure(ProviderFailureKind::RetrievalRestricted, "Modrinth provider requests and downloads must use HTTPS")
    })?;
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    if hosts.iter().any(|allowed| host.eq_ignore_ascii_case(allowed)) {
        Ok(())
    } else {
        Err(failure(
            ProviderFailureKind::RetrievalRestricted,
            format!("Modrinth URL host is outside the provider trust boundary: {host}"),
        ))
    }
}

fn encode_path_segment(value: &str) -> String {
    percent_encode(value)
}

fn percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push(char::from_digit((byte >> 4) as u32, 16).unwrap().to_ascii_uppercase());
            output.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap().to_ascii_uppercase());
        }
    }
    output
}

fn eq_hash(expected: &str, actual: &str) -> bool {
    expected.trim().eq_ignore_ascii_case(actual.trim())
}

fn temporary_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", unique_suffix()))
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |duration| duration.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

fn failure(kind: ProviderFailureKind, message: impl Into<String>) -> ProviderFailure {
    ProviderFailure::new(kind, message)
}
