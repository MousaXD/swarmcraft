use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use swarm_core::DataPaths;
use swarm_protocol::{ArtifactRequirementV1, ArtifactSideV1, Hash32, RuntimeCompatibilityManifestV1, WorldId};

const ARTIFACT_HASH_DOMAIN: &[u8] = b"swarmcraft/runtime-artifact/v1\0";
const MAX_JAR_BYTES: usize = 256 * 1024 * 1024;
const MAX_METADATA_COMPRESSED_BYTES: usize = 4 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const FABRIC_METADATA_PATH: &str = "fabric.mod.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModEnvironment {
    Server,
    Universal,
    Client,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModComponentKind {
    ManagedRuntime,
    UserServerMod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricModMetadata {
    pub mod_id: String,
    pub version: String,
    pub name: Option<String>,
    pub environment: ModEnvironment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledServerMod {
    pub file_name: String,
    pub path: PathBuf,
    pub mod_id: String,
    pub version: String,
    pub artifact_hash: String,
    pub environment: ModEnvironment,
    pub component_kind: ModComponentKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequiredServerMod {
    pub mod_id: String,
    pub version: String,
    pub artifact_hash: String,
    pub side: ArtifactSideV1,
    pub component_kind: ModComponentKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ModIssueKind {
    InvalidJar,
    DuplicateModId,
    ConflictingVersion,
    MissingRequired,
    HashMismatch,
    VersionMismatch,
    UnexpectedMod,
    ClientOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModIssue {
    pub kind: ModIssueKind,
    pub mod_id: Option<String>,
    pub file_name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerModReadiness {
    pub world_id: String,
    pub mods_dir: PathBuf,
    pub ready: bool,
    pub required: Vec<RequiredServerMod>,
    pub installed: Vec<InstalledServerMod>,
    pub issues: Vec<ModIssue>,
}

#[derive(Debug, Clone)]
struct JarInspection {
    metadata: FabricModMetadata,
    artifact_hash: Hash32,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ZipEntry {
    compression: u16,
    crc32: u32,
    compressed_size: usize,
    uncompressed_size: usize,
    local_header_offset: usize,
}

pub fn mods_dir(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.worlds_dir().join(world.to_hex()).join("runtime-profile").join("mods")
}

pub fn inspect_fabric_mod(path: &Path) -> Result<InstalledServerMod> {
    let inspected = inspect_jar(path)?;
    Ok(installed_from_inspection(path, &inspected))
}

pub fn requirements_from_jars(paths: &[PathBuf]) -> Result<Vec<ArtifactRequirementV1>> {
    let mut requirements = Vec::with_capacity(paths.len());
    let mut seen = BTreeSet::new();
    for path in paths {
        let requirement = requirement_from_jar(path)?;
        if !seen.insert(requirement.artifact_id.clone()) {
            bail!("duplicate Fabric mod id {} in selected server mods", requirement.artifact_id);
        }
        requirements.push(requirement);
    }
    requirements.sort_by(|a, b| {
        a.artifact_id
            .cmp(&b.artifact_id)
            .then(a.version.cmp(&b.version))
            .then(a.artifact_hash.0.cmp(&b.artifact_hash.0))
    });
    Ok(requirements)
}

pub fn requirement_from_jar(path: &Path) -> Result<ArtifactRequirementV1> {
    let inspected = inspect_jar(path)?;
    if inspected.metadata.environment == ModEnvironment::Client {
        bail!("Fabric mod {} is client-only and cannot be required as a server mod", inspected.metadata.mod_id);
    }
    if is_managed_component(&inspected.metadata.mod_id) {
        bail!("{} is a SwarmCraft-managed runtime component, not a user server mod", inspected.metadata.mod_id);
    }
    Ok(ArtifactRequirementV1 {
        artifact_id: inspected.metadata.mod_id,
        version: inspected.metadata.version,
        artifact_hash: inspected.artifact_hash,
        side: match inspected.metadata.environment {
            ModEnvironment::Universal => ArtifactSideV1::Both,
            ModEnvironment::Server | ModEnvironment::Unknown => ArtifactSideV1::Server,
            ModEnvironment::Client => unreachable!(),
        },
        provider_hint: None,
    })
}

pub fn add_local_mod(
    paths: &DataPaths,
    world: WorldId,
    manifest: &RuntimeCompatibilityManifestV1,
    source: &Path,
) -> Result<InstalledServerMod> {
    let inspected = inspect_jar(source)?;
    if inspected.metadata.environment == ModEnvironment::Client {
        bail!("Fabric mod {} is client-only", inspected.metadata.mod_id);
    }
    if is_managed_component(&inspected.metadata.mod_id) {
        bail!(
            "{} is managed by the runtime installer and cannot be added as a user server mod",
            inspected.metadata.mod_id
        );
    }
    let expected = canonical_user_requirements(manifest)
        .into_iter()
        .find(|required| required.artifact_id == inspected.metadata.mod_id)
        .ok_or_else(|| {
            anyhow!(
                "{} is not part of this world's canonical server-mod profile; protocol v1 profiles cannot be changed in place",
                inspected.metadata.mod_id
            )
        })?;
    if expected.version != inspected.metadata.version {
        bail!(
            "{} version mismatch: world requires {}, selected JAR is {}",
            expected.artifact_id,
            expected.version,
            inspected.metadata.version
        );
    }
    if expected.artifact_hash != inspected.artifact_hash {
        bail!("{} artifact hash does not match the world's canonical requirement", expected.artifact_id);
    }

    let dir = mods_dir(paths, world);
    fs::create_dir_all(&dir).with_context(|| format!("cannot create server mods directory {}", dir.display()))?;
    let destination =
        dir.join(canonical_file_name(&inspected.metadata.mod_id, &inspected.metadata.version, inspected.artifact_hash));
    atomic_write(&destination, &inspected.bytes)?;
    Ok(installed_from_inspection(&destination, &inspected))
}

pub fn remove_local_mod(paths: &DataPaths, world: WorldId, mod_id: &str) -> Result<Vec<PathBuf>> {
    let dir = mods_dir(paths, world);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    for path in jar_paths(&dir)? {
        let Ok(inspected) = inspect_jar(&path) else {
            continue;
        };
        if inspected.metadata.mod_id == mod_id {
            matches.push(path);
        }
    }
    if matches.len() > 1 {
        bail!("multiple JARs claim mod id {mod_id}; remove the conflicting files explicitly from {}", dir.display());
    }
    for path in &matches {
        fs::remove_file(path).with_context(|| format!("cannot remove {}", path.display()))?;
    }
    if !matches.is_empty() {
        sync_parent(&dir)?;
    }
    Ok(matches)
}

pub fn evaluate_world_mods(
    paths: &DataPaths,
    world: WorldId,
    manifest: &RuntimeCompatibilityManifestV1,
) -> Result<ServerModReadiness> {
    evaluate_mod_directory(world, mods_dir(paths, world), manifest)
}

pub fn install_verified_user_mods(
    paths: &DataPaths,
    world: WorldId,
    manifest: &RuntimeCompatibilityManifestV1,
    destination: &Path,
) -> Result<ServerModReadiness> {
    let readiness = evaluate_world_mods(paths, world, manifest)?;
    if !readiness.ready {
        let summary = readiness.issues.iter().map(|issue| issue.message.as_str()).collect::<Vec<_>>().join("; ");
        bail!("server mod runtime profile is not ready: {summary}");
    }
    fs::create_dir_all(destination)
        .with_context(|| format!("cannot create runtime mods directory {}", destination.display()))?;
    let required_ids: BTreeSet<&str> = readiness
        .required
        .iter()
        .filter(|required| required.component_kind == ModComponentKind::UserServerMod)
        .map(|required| required.mod_id.as_str())
        .collect();
    for installed in &readiness.installed {
        if installed.component_kind != ModComponentKind::UserServerMod
            || !required_ids.contains(installed.mod_id.as_str())
        {
            continue;
        }
        let target = destination.join(&installed.file_name);
        fs::copy(&installed.path, &target).with_context(|| {
            format!("cannot stage server mod {} into {}", installed.path.display(), target.display())
        })?;
    }
    Ok(readiness)
}

pub fn compare_runtime_profile(
    manifest: &RuntimeCompatibilityManifestV1,
    installed: &[InstalledServerMod],
) -> Vec<ModIssue> {
    evaluate_inventory(manifest, installed).1
}

fn evaluate_mod_directory(
    world: WorldId,
    dir: PathBuf,
    manifest: &RuntimeCompatibilityManifestV1,
) -> Result<ServerModReadiness> {
    let mut installed = Vec::new();
    let mut issues = Vec::new();
    if dir.exists() {
        for path in jar_paths(&dir)? {
            match inspect_jar(&path) {
                Ok(inspected) => installed.push(installed_from_inspection(&path, &inspected)),
                Err(error) => issues.push(ModIssue {
                    kind: ModIssueKind::InvalidJar,
                    mod_id: None,
                    file_name: path.file_name().map(|name| name.to_string_lossy().into_owned()),
                    message: format!("{} is not a valid Fabric mod JAR: {error:#}", path.display()),
                }),
            }
        }
    }
    installed
        .sort_by(|a, b| a.mod_id.cmp(&b.mod_id).then(a.version.cmp(&b.version)).then(a.file_name.cmp(&b.file_name)));
    let (required, mut inventory_issues) = evaluate_inventory(manifest, &installed);
    issues.append(&mut inventory_issues);
    issues.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.mod_id.cmp(&b.mod_id)).then(a.file_name.cmp(&b.file_name)));
    Ok(ServerModReadiness {
        world_id: world.to_string(),
        mods_dir: dir,
        ready: issues.is_empty(),
        required,
        installed,
        issues,
    })
}

fn evaluate_inventory(
    manifest: &RuntimeCompatibilityManifestV1,
    installed: &[InstalledServerMod],
) -> (Vec<RequiredServerMod>, Vec<ModIssue>) {
    let required_values = canonical_physical_requirements(manifest);
    let required = required_values
        .iter()
        .map(|value| RequiredServerMod {
            mod_id: value.artifact_id.clone(),
            version: value.version.clone(),
            artifact_hash: value.artifact_hash.to_hex(),
            side: value.side,
            component_kind: component_kind(&value.artifact_id),
        })
        .collect::<Vec<_>>();
    let mut issues = Vec::new();
    let mut by_id: BTreeMap<&str, Vec<&InstalledServerMod>> = BTreeMap::new();
    for item in installed {
        by_id.entry(&item.mod_id).or_default().push(item);
        if item.environment == ModEnvironment::Client {
            issues.push(ModIssue {
                kind: ModIssueKind::ClientOnly,
                mod_id: Some(item.mod_id.clone()),
                file_name: Some(item.file_name.clone()),
                message: format!("{} is client-only and cannot be loaded by the server profile", item.mod_id),
            });
        }
    }
    for (mod_id, values) in &by_id {
        if values.len() > 1 {
            issues.push(ModIssue {
                kind: ModIssueKind::DuplicateModId,
                mod_id: Some((*mod_id).to_owned()),
                file_name: None,
                message: format!("multiple JARs claim Fabric mod id {mod_id}"),
            });
            if values.iter().map(|value| value.version.as_str()).collect::<BTreeSet<_>>().len() > 1 {
                issues.push(ModIssue {
                    kind: ModIssueKind::ConflictingVersion,
                    mod_id: Some((*mod_id).to_owned()),
                    file_name: None,
                    message: format!("multiple versions of {mod_id} are installed"),
                });
            }
        }
    }

    let required_user_ids: BTreeSet<&str> = required_values
        .iter()
        .filter(|value| component_kind(&value.artifact_id) == ModComponentKind::UserServerMod)
        .map(|value| value.artifact_id.as_str())
        .collect();
    for required_value in
        required_values.iter().filter(|value| component_kind(&value.artifact_id) == ModComponentKind::UserServerMod)
    {
        let Some(values) = by_id.get(required_value.artifact_id.as_str()) else {
            issues.push(ModIssue {
                kind: ModIssueKind::MissingRequired,
                mod_id: Some(required_value.artifact_id.clone()),
                file_name: None,
                message: format!(
                    "required server mod {} {} is missing",
                    required_value.artifact_id, required_value.version
                ),
            });
            continue;
        };
        let exact = values.iter().find(|value| value.artifact_hash == required_value.artifact_hash.to_hex());
        if exact.is_none() {
            if values.iter().any(|value| value.version != required_value.version) {
                issues.push(ModIssue {
                    kind: ModIssueKind::VersionMismatch,
                    mod_id: Some(required_value.artifact_id.clone()),
                    file_name: None,
                    message: format!(
                        "{} is installed at a different version; world requires {}",
                        required_value.artifact_id, required_value.version
                    ),
                });
            } else {
                issues.push(ModIssue {
                    kind: ModIssueKind::HashMismatch,
                    mod_id: Some(required_value.artifact_id.clone()),
                    file_name: None,
                    message: format!("{} bytes do not match the canonical artifact hash", required_value.artifact_id),
                });
            }
        }
    }
    for item in installed.iter().filter(|item| item.component_kind == ModComponentKind::UserServerMod) {
        if !required_user_ids.contains(item.mod_id.as_str()) {
            issues.push(ModIssue {
                kind: ModIssueKind::UnexpectedMod,
                mod_id: Some(item.mod_id.clone()),
                file_name: Some(item.file_name.clone()),
                message: format!(
                    "{} is not part of this world's canonical server-mod profile and will not be launched",
                    item.mod_id
                ),
            });
        }
    }
    (required, issues)
}

fn canonical_physical_requirements(manifest: &RuntimeCompatibilityManifestV1) -> Vec<ArtifactRequirementV1> {
    let mut values = manifest
        .required_server_mods
        .iter()
        .filter(|value| value.artifact_id != "swarmcraft.legacy-compatibility")
        .cloned()
        .collect::<Vec<_>>();
    values.sort_by(|a, b| {
        a.artifact_id
            .cmp(&b.artifact_id)
            .then(a.version.cmp(&b.version))
            .then(a.artifact_hash.0.cmp(&b.artifact_hash.0))
    });
    values
}

fn canonical_user_requirements(manifest: &RuntimeCompatibilityManifestV1) -> Vec<ArtifactRequirementV1> {
    canonical_physical_requirements(manifest)
        .into_iter()
        .filter(|value| component_kind(&value.artifact_id) == ModComponentKind::UserServerMod)
        .collect()
}

fn component_kind(mod_id: &str) -> ModComponentKind {
    if is_managed_component(mod_id) {
        ModComponentKind::ManagedRuntime
    } else {
        ModComponentKind::UserServerMod
    }
}

fn is_managed_component(mod_id: &str) -> bool {
    matches!(mod_id, "fabric-api" | "swarmcraft")
}

fn jar_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot enumerate {}", dir.display()))? {
        let entry = entry.with_context(|| format!("cannot enumerate {}", dir.display()))?;
        let path = entry.path();
        if entry.file_type().with_context(|| format!("cannot inspect {}", path.display()))?.is_file()
            && path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("jar"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn installed_from_inspection(path: &Path, inspected: &JarInspection) -> InstalledServerMod {
    InstalledServerMod {
        file_name: path.file_name().map_or_else(|| "mod.jar".into(), |name| name.to_string_lossy().into_owned()),
        path: path.to_path_buf(),
        mod_id: inspected.metadata.mod_id.clone(),
        version: inspected.metadata.version.clone(),
        artifact_hash: inspected.artifact_hash.to_hex(),
        environment: inspected.metadata.environment,
        component_kind: component_kind(&inspected.metadata.mod_id),
    }
}

fn inspect_jar(path: &Path) -> Result<JarInspection> {
    let metadata = fs::symlink_metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    if metadata.len() > MAX_JAR_BYTES as u64 {
        bail!("JAR exceeds the {} MiB safety limit", MAX_JAR_BYTES / (1024 * 1024));
    }
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let artifact_hash = Hash32::from_domain_bytes(ARTIFACT_HASH_DOMAIN, &bytes);
    let metadata_bytes = read_zip_entry(&bytes, FABRIC_METADATA_PATH)?;
    let metadata = parse_fabric_metadata(&metadata_bytes)?;
    Ok(JarInspection { metadata, artifact_hash, bytes })
}

fn parse_fabric_metadata(bytes: &[u8]) -> Result<FabricModMetadata> {
    let value: Value = serde_json::from_slice(bytes).context("fabric.mod.json is not valid JSON")?;
    let schema =
        value.get("schemaVersion").and_then(Value::as_u64).context("fabric.mod.json has no numeric schemaVersion")?;
    if schema != 1 {
        bail!("unsupported Fabric metadata schemaVersion {schema}");
    }
    let mod_id = value.get("id").and_then(Value::as_str).context("fabric.mod.json has no string id")?;
    let version = value.get("version").and_then(Value::as_str).context("fabric.mod.json has no string version")?;
    validate_mod_id(mod_id)?;
    if version.trim().is_empty() {
        bail!("Fabric mod version is empty");
    }
    let environment = match value.get("environment").and_then(Value::as_str) {
        None | Some("*") => ModEnvironment::Universal,
        Some("server") => ModEnvironment::Server,
        Some("client") => ModEnvironment::Client,
        Some(_) => ModEnvironment::Unknown,
    };
    Ok(FabricModMetadata {
        mod_id: mod_id.to_owned(),
        version: version.to_owned(),
        name: value.get("name").and_then(Value::as_str).map(str::to_owned),
        environment,
    })
}

fn validate_mod_id(value: &str) -> Result<()> {
    if value.len() < 2
        || value.len() > 64
        || !value.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-')
    {
        bail!("invalid Fabric mod id {value:?}");
    }
    Ok(())
}

fn canonical_file_name(mod_id: &str, version: &str, hash: Hash32) -> String {
    let safe_version = version
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') { ch } else { '_' })
        .collect::<String>();
    let hex = hash.to_hex();
    format!("{mod_id}-{safe_version}-{}.jar", &hex[..12])
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("mod destination has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    if path.exists() {
        let existing = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        if existing == bytes {
            return Ok(());
        }
        bail!("refusing to replace a different artifact at {}", path.display());
    }
    let tmp = path.with_extension("jar.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .with_context(|| format!("cannot create {}", tmp.display()))?;
    file.write_all(bytes).with_context(|| format!("cannot write {}", tmp.display()))?;
    file.sync_all().with_context(|| format!("cannot sync {}", tmp.display()))?;
    drop(file);
    fs::rename(&tmp, path).with_context(|| format!("cannot publish {}", path.display()))?;
    sync_parent(parent)
}

fn sync_parent(parent: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .with_context(|| format!("cannot sync {}", parent.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
}

fn read_zip_entry(bytes: &[u8], wanted: &str) -> Result<Vec<u8>> {
    let eocd = find_eocd(bytes)?;
    if read_u16(bytes, eocd + 4)? != 0 || read_u16(bytes, eocd + 6)? != 0 {
        bail!("multi-disk ZIP/JAR files are unsupported");
    }
    let entries_on_disk = read_u16(bytes, eocd + 8)?;
    let total_entries = read_u16(bytes, eocd + 10)?;
    if entries_on_disk != total_entries || total_entries == u16::MAX {
        bail!("ZIP64 or inconsistent central directory is unsupported");
    }
    let central_size = read_u32(bytes, eocd + 12)? as usize;
    let central_offset = read_u32(bytes, eocd + 16)? as usize;
    let central_end = central_offset.checked_add(central_size).context("ZIP central directory overflows")?;
    if central_end > bytes.len() || central_end > eocd {
        bail!("ZIP central directory points outside the JAR");
    }

    let mut cursor = central_offset;
    let mut found = None;
    for _ in 0..total_entries {
        if read_u32(bytes, cursor)? != 0x0201_4b50 {
            bail!("invalid ZIP central-directory entry");
        }
        let flags = read_u16(bytes, cursor + 8)?;
        if flags & 1 != 0 {
            bail!("encrypted ZIP/JAR entries are unsupported");
        }
        let compression = read_u16(bytes, cursor + 10)?;
        let crc32 = read_u32(bytes, cursor + 16)?;
        let compressed_size_u32 = read_u32(bytes, cursor + 20)?;
        let uncompressed_size_u32 = read_u32(bytes, cursor + 24)?;
        let name_len = read_u16(bytes, cursor + 28)? as usize;
        let extra_len = read_u16(bytes, cursor + 30)? as usize;
        let comment_len = read_u16(bytes, cursor + 32)? as usize;
        let disk_start = read_u16(bytes, cursor + 34)?;
        let local_offset_u32 = read_u32(bytes, cursor + 42)?;
        if compressed_size_u32 == u32::MAX
            || uncompressed_size_u32 == u32::MAX
            || local_offset_u32 == u32::MAX
            || disk_start != 0
        {
            bail!("ZIP64 entries are unsupported");
        }
        let name_start = cursor + 46;
        let name_end = name_start.checked_add(name_len).context("ZIP file name overflows")?;
        let name_bytes = bytes.get(name_start..name_end).context("truncated ZIP file name")?;
        if name_bytes == wanted.as_bytes() {
            if found.is_some() {
                bail!("JAR contains duplicate {wanted} entries");
            }
            found = Some(ZipEntry {
                compression,
                crc32,
                compressed_size: compressed_size_u32 as usize,
                uncompressed_size: uncompressed_size_u32 as usize,
                local_header_offset: local_offset_u32 as usize,
            });
        }
        cursor = name_end
            .checked_add(extra_len)
            .and_then(|value| value.checked_add(comment_len))
            .context("ZIP central directory entry overflows")?;
        if cursor > central_end {
            bail!("ZIP central directory entry exceeds declared size");
        }
    }
    let entry = found.ok_or_else(|| anyhow!("JAR does not contain root {wanted}"))?;
    if entry.compressed_size > MAX_METADATA_COMPRESSED_BYTES || entry.uncompressed_size > MAX_METADATA_BYTES {
        bail!("fabric.mod.json exceeds safety limits");
    }
    extract_zip_entry(bytes, &entry)
}

fn find_eocd(bytes: &[u8]) -> Result<usize> {
    if bytes.len() < 22 {
        bail!("file is too small to be a ZIP/JAR");
    }
    let start = bytes.len().saturating_sub(22 + u16::MAX as usize);
    for offset in (start..=bytes.len() - 22).rev() {
        if bytes.get(offset..offset + 4) == Some(&[0x50, 0x4b, 0x05, 0x06]) {
            let comment_len = read_u16(bytes, offset + 20)? as usize;
            if offset + 22 + comment_len == bytes.len() {
                return Ok(offset);
            }
        }
    }
    bail!("ZIP end-of-central-directory record not found")
}

fn extract_zip_entry(bytes: &[u8], entry: &ZipEntry) -> Result<Vec<u8>> {
    let local = entry.local_header_offset;
    if read_u32(bytes, local)? != 0x0403_4b50 {
        bail!("invalid ZIP local header");
    }
    let flags = read_u16(bytes, local + 6)?;
    let method = read_u16(bytes, local + 8)?;
    if flags & 1 != 0 || method != entry.compression {
        bail!("ZIP local header conflicts with central directory");
    }
    let name_len = read_u16(bytes, local + 26)? as usize;
    let extra_len = read_u16(bytes, local + 28)? as usize;
    let data_start = local
        .checked_add(30)
        .and_then(|value| value.checked_add(name_len))
        .and_then(|value| value.checked_add(extra_len))
        .context("ZIP local header overflows")?;
    let data_end = data_start.checked_add(entry.compressed_size).context("ZIP entry overflows")?;
    let compressed = bytes.get(data_start..data_end).context("truncated ZIP entry")?;
    let output = match entry.compression {
        0 => compressed.to_vec(),
        8 => inflate_raw(compressed, entry.uncompressed_size, MAX_METADATA_BYTES)?,
        other => bail!("unsupported ZIP compression method {other}"),
    };
    if output.len() != entry.uncompressed_size {
        bail!("ZIP entry expanded to {}, expected {}", output.len(), entry.uncompressed_size);
    }
    if crc32(&output) != entry.crc32 {
        bail!("ZIP entry CRC32 mismatch");
    }
    Ok(output)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let raw = bytes.get(offset..offset + 2).context("truncated ZIP integer")?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let raw = bytes.get(offset..offset + 4).context("truncated ZIP integer")?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit: 0 }
    }

    fn read_bits(&mut self, count: usize) -> Result<u32> {
        if count > 24 {
            bail!("invalid DEFLATE bit request");
        }
        let mut value = 0u32;
        for shift in 0..count {
            let byte = *self.bytes.get(self.bit / 8).context("truncated DEFLATE stream")?;
            value |= (((byte >> (self.bit % 8)) & 1) as u32) << shift;
            self.bit += 1;
        }
        Ok(value)
    }

    fn align_byte(&mut self) {
        self.bit = (self.bit + 7) & !7;
    }

    fn byte_position(&self) -> usize {
        self.bit / 8
    }

    fn set_byte_position(&mut self, position: usize) {
        self.bit = position * 8;
    }
}

#[derive(Debug)]
struct Huffman {
    codes: Vec<Vec<(u32, usize)>>,
    max_len: usize,
}

impl Huffman {
    fn from_lengths(lengths: &[u8]) -> Result<Self> {
        let max_len = lengths.iter().copied().max().unwrap_or(0) as usize;
        if max_len > 15 {
            bail!("DEFLATE Huffman code exceeds 15 bits");
        }
        let mut counts = vec![0u32; max_len + 1];
        for &length in lengths {
            if length != 0 {
                counts[length as usize] += 1;
            }
        }
        let mut next = vec![0u32; max_len + 1];
        let mut code = 0u32;
        for bits in 1..=max_len {
            code = (code + counts[bits - 1]) << 1;
            next[bits] = code;
        }
        let mut codes = vec![Vec::new(); max_len + 1];
        for (symbol, &length) in lengths.iter().enumerate() {
            let bits = length as usize;
            if bits == 0 {
                continue;
            }
            let canonical = next[bits];
            next[bits] += 1;
            if canonical >= (1u32 << bits) {
                bail!("oversubscribed DEFLATE Huffman tree");
            }
            codes[bits].push((reverse_low_bits(canonical, bits), symbol));
        }
        Ok(Self { codes, max_len })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<usize> {
        if self.max_len == 0 {
            bail!("DEFLATE Huffman tree has no symbols");
        }
        let mut code = 0u32;
        for length in 1..=self.max_len {
            code |= reader.read_bits(1)? << (length - 1);
            if let Some((_, symbol)) = self.codes[length].iter().find(|(candidate, _)| *candidate == code) {
                return Ok(*symbol);
            }
        }
        bail!("invalid DEFLATE Huffman symbol")
    }
}

fn reverse_low_bits(mut value: u32, count: usize) -> u32 {
    let mut reversed = 0u32;
    for _ in 0..count {
        reversed = (reversed << 1) | (value & 1);
        value >>= 1;
    }
    reversed
}

fn inflate_raw(input: &[u8], expected_size: usize, max_output: usize) -> Result<Vec<u8>> {
    if expected_size > max_output {
        bail!("DEFLATE output exceeds safety limit");
    }
    let mut reader = BitReader::new(input);
    let mut output = Vec::with_capacity(expected_size.min(max_output));
    loop {
        let final_block = reader.read_bits(1)? != 0;
        match reader.read_bits(2)? {
            0 => inflate_stored_block(&mut reader, &mut output, max_output)?,
            1 => {
                let (literal, distance) = fixed_huffman()?;
                inflate_huffman_block(&mut reader, &mut output, max_output, &literal, &distance)?;
            }
            2 => {
                let (literal, distance) = dynamic_huffman(&mut reader)?;
                inflate_huffman_block(&mut reader, &mut output, max_output, &literal, &distance)?;
            }
            _ => bail!("reserved DEFLATE block type"),
        }
        if final_block {
            break;
        }
    }
    if output.len() != expected_size {
        bail!("DEFLATE output length mismatch");
    }
    Ok(output)
}

fn inflate_stored_block(reader: &mut BitReader<'_>, output: &mut Vec<u8>, max_output: usize) -> Result<()> {
    reader.align_byte();
    let position = reader.byte_position();
    let len = read_u16(reader.bytes, position)? as usize;
    let nlen = read_u16(reader.bytes, position + 2)?;
    if (len as u16) != !nlen {
        bail!("invalid uncompressed DEFLATE block length");
    }
    let start = position + 4;
    let end = start.checked_add(len).context("DEFLATE stored block overflows")?;
    let bytes = reader.bytes.get(start..end).context("truncated DEFLATE stored block")?;
    if output.len().saturating_add(bytes.len()) > max_output {
        bail!("DEFLATE output exceeds safety limit");
    }
    output.extend_from_slice(bytes);
    reader.set_byte_position(end);
    Ok(())
}

fn fixed_huffman() -> Result<(Huffman, Huffman)> {
    let mut literal_lengths = vec![0u8; 288];
    literal_lengths[0..=143].fill(8);
    literal_lengths[144..=255].fill(9);
    literal_lengths[256..=279].fill(7);
    literal_lengths[280..=287].fill(8);
    let distance_lengths = vec![5u8; 32];
    Ok((Huffman::from_lengths(&literal_lengths)?, Huffman::from_lengths(&distance_lengths)?))
}

fn dynamic_huffman(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman)> {
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 32 {
        bail!("invalid DEFLATE dynamic Huffman dimensions");
    }
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut code_lengths = [0u8; 19];
    for &symbol in ORDER.iter().take(hclen) {
        code_lengths[symbol] = reader.read_bits(3)? as u8;
    }
    let code_tree = Huffman::from_lengths(&code_lengths)?;
    let total = hlit + hdist;
    let mut lengths = Vec::with_capacity(total);
    while lengths.len() < total {
        let symbol = code_tree.decode(reader)?;
        match symbol {
            0..=15 => lengths.push(symbol as u8),
            16 => {
                let previous = *lengths.last().context("DEFLATE repeat code has no previous length")?;
                let count = reader.read_bits(2)? as usize + 3;
                extend_lengths(&mut lengths, total, previous, count)?;
            }
            17 => {
                let count = reader.read_bits(3)? as usize + 3;
                extend_lengths(&mut lengths, total, 0, count)?;
            }
            18 => {
                let count = reader.read_bits(7)? as usize + 11;
                extend_lengths(&mut lengths, total, 0, count)?;
            }
            symbol => bail!("invalid DEFLATE code-length symbol {symbol}"),
        }
    }
    let literal = Huffman::from_lengths(&lengths[..hlit])?;
    let distance = Huffman::from_lengths(&lengths[hlit..])?;
    Ok((literal, distance))
}

fn extend_lengths(lengths: &mut Vec<u8>, total: usize, value: u8, count: usize) -> Result<()> {
    if lengths.len().saturating_add(count) > total {
        bail!("DEFLATE code-length repeat exceeds declared tree size");
    }
    lengths.extend(std::iter::repeat_n(value, count));
    Ok(())
}

fn inflate_huffman_block(
    reader: &mut BitReader<'_>,
    output: &mut Vec<u8>,
    max_output: usize,
    literal: &Huffman,
    distance: &Huffman,
) -> Result<()> {
    loop {
        match literal.decode(reader)? {
            symbol @ 0..=255 => push_output(output, symbol as u8, max_output)?,
            256 => return Ok(()),
            symbol @ 257..=285 => {
                let length = decode_length(reader, symbol)?;
                let distance_symbol = distance.decode(reader)?;
                let distance = decode_distance(reader, distance_symbol)?;
                if distance == 0 || distance > output.len() {
                    bail!("invalid DEFLATE back-reference distance");
                }
                if output.len().saturating_add(length) > max_output {
                    bail!("DEFLATE output exceeds safety limit");
                }
                for _ in 0..length {
                    let byte = output[output.len() - distance];
                    output.push(byte);
                }
            }
            symbol => bail!("invalid DEFLATE literal/length symbol {symbol}"),
        }
    }
}

fn push_output(output: &mut Vec<u8>, byte: u8, max_output: usize) -> Result<()> {
    if output.len() >= max_output {
        bail!("DEFLATE output exceeds safety limit");
    }
    output.push(byte);
    Ok(())
}

fn decode_length(reader: &mut BitReader<'_>, symbol: usize) -> Result<usize> {
    const BASE: [usize; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227,
        258,
    ];
    const EXTRA: [usize; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
    let index = symbol.checked_sub(257).filter(|index| *index < BASE.len()).context("invalid DEFLATE length symbol")?;
    Ok(BASE[index] + reader.read_bits(EXTRA[index])? as usize)
}

fn decode_distance(reader: &mut BitReader<'_>, symbol: usize) -> Result<usize> {
    const BASE: [usize; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097,
        6145, 8193, 12289, 16385, 24577,
    ];
    const EXTRA: [usize; 30] =
        [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
    let base = *BASE.get(symbol).context("invalid DEFLATE distance symbol")?;
    Ok(base + reader.read_bits(EXTRA[symbol])? as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::PROTOCOL_VERSION;

    fn requirement(id: &str, version: &str, hash_byte: u8) -> ArtifactRequirementV1 {
        ArtifactRequirementV1 {
            artifact_id: id.into(),
            version: version.into(),
            artifact_hash: Hash32([hash_byte; 32]),
            side: ArtifactSideV1::Server,
            provider_hint: None,
        }
    }

    fn manifest(required_server_mods: Vec<ArtifactRequirementV1>) -> RuntimeCompatibilityManifestV1 {
        RuntimeCompatibilityManifestV1 {
            minecraft_version: "1.21.8".into(),
            loader_id: "fabric".into(),
            loader_version: "0.17.2".into(),
            swarmcraft_protocol_version: PROTOCOL_VERSION,
            fabric_adapter_version: "0.3.0".into(),
            required_server_mods,
            required_client_mods: Vec::new(),
            datapacks: Vec::new(),
        }
    }

    fn installed(id: &str, version: &str, hash_byte: u8) -> InstalledServerMod {
        InstalledServerMod {
            file_name: format!("{id}.jar"),
            path: PathBuf::from(format!("/mods/{id}.jar")),
            mod_id: id.into(),
            version: version.into(),
            artifact_hash: Hash32([hash_byte; 32]).to_hex(),
            environment: ModEnvironment::Server,
            component_kind: component_kind(id),
        }
    }

    #[test]
    fn parses_deflated_fabric_metadata() {
        let compressed = hex_bytes("ab562a4ece48cd4d0c4b2d2acecccf53b232d451ca4c51b252cac92cc9c82ccd55d2512a83492919e8199aea190085f2127353817c1fb89ad4bcb2cca2fcbcdcd4bc12a078716a115093522d00");
        let bytes = inflate_raw(&compressed, 93, 1024).unwrap();
        let parsed = parse_fabric_metadata(&bytes).unwrap();
        assert_eq!(parsed.mod_id, "lithium");
        assert_eq!(parsed.version, "0.15.0");
        assert_eq!(parsed.environment, ModEnvironment::Server);
    }

    #[test]
    fn valid_fabric_mod_jar_is_identified_and_hashed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lithium.jar");
        let metadata = br#"{"schemaVersion":1,"id":"lithium","version":"0.15.0","environment":"server"}"#;
        fs::write(&path, stored_jar(FABRIC_METADATA_PATH, metadata)).unwrap();
        let inspected = inspect_fabric_mod(&path).unwrap();
        assert_eq!(inspected.mod_id, "lithium");
        assert_eq!(inspected.version, "0.15.0");
        assert_eq!(inspected.environment, ModEnvironment::Server);
        assert_eq!(inspected.component_kind, ModComponentKind::UserServerMod);
        assert_eq!(inspected.artifact_hash.len(), 64);
    }

    #[test]
    fn rejects_invalid_jar() {
        assert!(read_zip_entry(b"not a jar", FABRIC_METADATA_PATH).is_err());
    }

    #[test]
    fn duplicate_mod_id_and_conflicting_versions_are_reported() {
        let profile = manifest(vec![requirement("lithium", "1", 1)]);
        let values = vec![installed("lithium", "1", 1), installed("lithium", "2", 2)];
        let issues = compare_runtime_profile(&profile, &values);
        assert!(issues.iter().any(|issue| issue.kind == ModIssueKind::DuplicateModId));
        assert!(issues.iter().any(|issue| issue.kind == ModIssueKind::ConflictingVersion));
    }

    #[test]
    fn single_wrong_version_is_reported() {
        let profile = manifest(vec![requirement("lithium", "1", 1)]);
        let issues = compare_runtime_profile(&profile, &[installed("lithium", "2", 2)]);
        assert!(issues.iter().any(|issue| issue.kind == ModIssueKind::VersionMismatch));
    }

    #[test]
    fn hash_mismatch_is_reported() {
        let profile = manifest(vec![requirement("lithium", "1", 1)]);
        let issues = compare_runtime_profile(&profile, &[installed("lithium", "1", 2)]);
        assert!(issues.iter().any(|issue| issue.kind == ModIssueKind::HashMismatch));
    }

    #[test]
    fn missing_required_mod_blocks_peer_runtime_compatibility() {
        let profile = manifest(vec![requirement("lithium", "1", 1), requirement("ferritecore", "1", 2)]);
        let bob = vec![installed("lithium", "1", 1)];
        let issues = compare_runtime_profile(&profile, &bob);
        assert!(issues.iter().any(|issue| {
            issue.kind == ModIssueKind::MissingRequired && issue.mod_id.as_deref() == Some("ferritecore")
        }));
    }

    #[test]
    fn exact_peer_inventory_is_compatible() {
        let profile = manifest(vec![requirement("lithium", "1", 1), requirement("ferritecore", "1", 2)]);
        let bob = vec![installed("lithium", "1", 1), installed("ferritecore", "1", 2)];
        assert!(compare_runtime_profile(&profile, &bob).is_empty());
    }

    #[test]
    fn server_only_and_unknown_environments_are_classified() {
        let server =
            parse_fabric_metadata(br#"{"schemaVersion":1,"id":"servermod","version":"1","environment":"server"}"#)
                .unwrap();
        let unknown = parse_fabric_metadata(
            br#"{"schemaVersion":1,"id":"oddmod","version":"1","environment":"dedicated_server"}"#,
        )
        .unwrap();
        let universal = parse_fabric_metadata(br#"{"schemaVersion":1,"id":"bothmod","version":"1"}"#).unwrap();
        assert_eq!(server.environment, ModEnvironment::Server);
        assert_eq!(unknown.environment, ModEnvironment::Unknown);
        assert_eq!(universal.environment, ModEnvironment::Universal);
    }

    #[test]
    fn managed_runtime_components_do_not_require_user_mod_files() {
        let profile = manifest(vec![requirement("fabric-api", "1", 1), requirement("swarmcraft", "1", 2)]);
        assert!(compare_runtime_profile(&profile, &[]).is_empty());
    }

    fn stored_jar(name: &str, contents: &[u8]) -> Vec<u8> {
        let name = name.as_bytes();
        let crc = crc32(contents);
        let mut out = Vec::new();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(contents);

        let central_offset = out.len() as u32;
        out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name);
        let central_size = out.len() as u32 - central_offset;

        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&central_size.to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn hex_bytes(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let high = (pair[0] as char).to_digit(16).unwrap();
                let low = (pair[1] as char).to_digit(16).unwrap();
                ((high << 4) | low) as u8
            })
            .collect()
    }
}
