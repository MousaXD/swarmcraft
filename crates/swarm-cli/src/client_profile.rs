//! Advisory Minecraft client preparation. Never launches a program, reads
//! launcher account/configuration files, installs mods, or grants play/hosting.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{collections::BTreeMap, fs, path::Path};
use swarm_protocol::{
    validate_runtime_selection, ArtifactRequirementV1, ArtifactSideV1, RuntimeCompatibilityManifestV1, WorldId,
};
use swarm_storage::Storage;

use crate::{
    historical_authority::collect_signed_config_history,
    server_mods::{inspect_fabric_mod, ModEnvironment},
};

const MAX_DIRECTORY_ENTRIES: usize = 2048;
const MAX_JARS: usize = 256;
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ClientArtifact {
    pub mod_id: String,
    pub version: String,
    pub artifact_hash: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InventoryState {
    NotChecked,
    ExactMatch,
    NeedsReview,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ClientIssue {
    pub code: &'static str,
    pub mod_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClientProfileReport {
    pub world_id: String,
    pub compatibility_fingerprint: String,
    pub minecraft_version: String,
    pub fabric_loader_version: String,
    pub minimum_java_major: u32,
    pub required_mods: Vec<ClientArtifact>,
    pub inventory: InventoryState,
    pub issues: Vec<ClientIssue>,
}

/// The immutable compatibility is anchored in the world ID and creator-signed
/// config history. It is not live membership, authority or connectivity proof.
pub fn prepare_client_profile(
    storage: &Storage,
    world: WorldId,
    mods_dir: Option<&Path>,
) -> Result<ClientProfileReport> {
    let metadata = storage.load_world(world)?;
    let history = collect_signed_config_history(storage, world)?;
    let config = history.first().context("Missing creator-signed client compatibility")?;
    let manifest = &config.compatibility;
    if manifest.loader_id != "fabric"
        || manifest.minecraft_version != metadata.genesis.minecraft_version
        || manifest.loader_version != metadata.genesis.fabric_loader_version
        || manifest.fingerprint()? != metadata.genesis.compatibility_fingerprint
    {
        bail!("Client compatibility does not match canonical world genesis");
    }
    let support = validate_runtime_selection(&manifest.minecraft_version, &manifest.loader_version, None)?;
    let required_mods = client_requirements(manifest)?;
    let (inventory, issues) = match mods_dir {
        None => (InventoryState::NotChecked, Vec::new()),
        Some(dir) => match inspect_inventory(dir, &required_mods) {
            Ok(issues) if issues.is_empty() => (InventoryState::ExactMatch, issues),
            Ok(issues) => (InventoryState::NeedsReview, issues),
            Err(_) => (InventoryState::NeedsReview, vec![issue("folder_unavailable_or_limit", None)]),
        },
    };
    Ok(ClientProfileReport {
        world_id: world.to_string(),
        compatibility_fingerprint: metadata.genesis.compatibility_fingerprint.to_hex(),
        minecraft_version: manifest.minecraft_version.clone(),
        fabric_loader_version: manifest.loader_version.clone(),
        minimum_java_major: support.minimum_java_major,
        required_mods,
        inventory,
        issues,
    })
}

fn client_requirements(manifest: &RuntimeCompatibilityManifestV1) -> Result<Vec<ClientArtifact>> {
    let mut by_id = BTreeMap::<String, &ArtifactRequirementV1>::new();
    for requirement in &manifest.required_client_mods {
        if requirement.side == ArtifactSideV1::Server {
            bail!("Server-only artifact appears in client requirements");
        }
    }
    if manifest.required_server_mods.iter().any(|r| r.side == ArtifactSideV1::Client) {
        bail!("Client-only artifact appears in server requirements");
    }
    // Imported and CLI-created worlds may encode universal mods only in the
    // server array; canonical modpacks list Both in both arrays.
    for requirement in manifest
        .required_client_mods
        .iter()
        .chain(manifest.required_server_mods.iter().filter(|r| r.side == ArtifactSideV1::Both))
    {
        if requirement.side == ArtifactSideV1::Server
            || requirement.artifact_id == "swarmcraft"
            || requirement.artifact_id == "swarmcraft.legacy-compatibility"
        {
            bail!("Server-only SwarmCraft component cannot be a client requirement");
        }
        if let Some(previous) = by_id.insert(requirement.artifact_id.clone(), requirement) {
            if previous != requirement {
                bail!("Conflicting signed client mod requirements");
            }
        }
    }
    for requirement in &manifest.required_server_mods {
        if requirement.side == ArtifactSideV1::Server && by_id.contains_key(&requirement.artifact_id) {
            bail!("Conflicting client/server artifact sides");
        }
    }
    Ok(by_id
        .into_values()
        .map(|r| ClientArtifact {
            mod_id: r.artifact_id.clone(),
            version: r.version.clone(),
            artifact_hash: r.artifact_hash.to_hex(),
        })
        .collect())
}

fn issue(code: &'static str, mod_id: Option<&str>) -> ClientIssue {
    ClientIssue { code, mod_id: mod_id.map(str::to_owned) }
}

fn inspect_inventory(dir: &Path, required: &[ClientArtifact]) -> Result<Vec<ClientIssue>> {
    // Only the explicitly selected mods directory; never walk the launcher
    // profile or read launcher profiles/accounts/options/saves.
    if !dir.is_absolute() || !fs::symlink_metadata(dir)?.is_dir() {
        bail!("Select an existing absolute client mods folder");
    }
    let mut issues = Vec::new();
    let mut installed = BTreeMap::new();
    let mut total = 0u64;
    let mut jars = 0;
    for (index, entry) in fs::read_dir(dir)?.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            bail!("Client mods directory exceeds inspection limit");
        }
        let entry = entry?;
        let path = entry.path();
        if !path.extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("jar")) {
            continue;
        }
        jars += 1;
        if jars > MAX_JARS {
            bail!("Client JAR inventory exceeds inspection limit");
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() {
            issues.push(issue("invalid_jar", None));
            continue;
        }
        total = total.checked_add(metadata.len()).context("Client inventory size overflow")?;
        if total > MAX_TOTAL_BYTES {
            bail!("Client JAR inventory exceeds inspection limit");
        }
        let Ok(mod_info) = inspect_fabric_mod(&path) else {
            issues.push(issue("invalid_jar", None));
            continue;
        };
        let id = mod_info.mod_id.clone();
        if id == "swarmcraft" || matches!(mod_info.environment, ModEnvironment::Server | ModEnvironment::Unknown) {
            issues.push(issue("server_only_or_unknown", Some(&id)));
        }
        if installed.insert(id.clone(), mod_info).is_some() {
            issues.push(issue("duplicate_mod", Some(&id)));
        }
    }
    for artifact in required {
        match installed.get(&artifact.mod_id) {
            None => issues.push(issue("missing_required", Some(&artifact.mod_id))),
            Some(found) if found.version != artifact.version => {
                issues.push(issue("version_mismatch", Some(&artifact.mod_id)))
            }
            Some(found) if found.artifact_hash != artifact.artifact_hash => {
                issues.push(issue("hash_mismatch", Some(&artifact.mod_id)))
            }
            Some(_) => (),
        }
    }
    for id in installed.keys() {
        if !required.iter().any(|r| &r.mod_id == id) {
            issues.push(issue("additional_mod", Some(id)));
        }
    }
    issues.sort_by(|a, b| (a.code, &a.mod_id).cmp(&(b.code, &b.mod_id)));
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{Hash32, PROTOCOL_VERSION};
    fn artifact(id: &str, side: ArtifactSideV1) -> ArtifactRequirementV1 {
        ArtifactRequirementV1 {
            artifact_id: id.into(),
            version: "1".into(),
            artifact_hash: Hash32([1; 32]),
            side,
            provider_hint: None,
        }
    }
    fn manifest() -> RuntimeCompatibilityManifestV1 {
        RuntimeCompatibilityManifestV1 {
            minecraft_version: "26.1.2".into(),
            loader_id: "fabric".into(),
            loader_version: "0.19.3".into(),
            swarmcraft_protocol_version: PROTOCOL_VERSION,
            fabric_adapter_version: "1".into(),
            required_server_mods: vec![
                artifact("server-only", ArtifactSideV1::Server),
                artifact("universal", ArtifactSideV1::Both),
            ],
            required_client_mods: vec![artifact("client-only", ArtifactSideV1::Client)],
            datapacks: Vec::new(),
        }
    }
    #[test]
    fn includes_client_and_both_without_server_only_or_datapacks() {
        let mut m = manifest();
        m.required_client_mods.push(m.required_server_mods[1].clone());
        m.datapacks.push(artifact("datapack", ArtifactSideV1::Server));
        let r = client_requirements(&m).unwrap();
        assert_eq!(r.iter().map(|a| a.mod_id.as_str()).collect::<Vec<_>>(), ["client-only", "universal"]);
        m.required_client_mods.push(artifact("swarmcraft", ArtifactSideV1::Client));
        assert!(client_requirements(&m).is_err());
    }
    #[test]
    fn rejects_conflicting_signed_requirements() {
        let mut m = manifest();
        let mut other = m.required_server_mods[1].clone();
        other.artifact_hash = Hash32([2; 32]);
        m.required_client_mods.push(other);
        assert!(client_requirements(&m).is_err());
    }
    #[test]
    fn inventory_is_explicit_and_reports_missing_invalid_and_ignores_account_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("launcher_accounts.json"), b"private fixture").unwrap();
        assert!(inspect_inventory(tmp.path(), &[]).unwrap().is_empty());
        fs::write(tmp.path().join("invalid.jar"), b"not a jar").unwrap();
        let issues = inspect_inventory(tmp.path(), &client_requirements(&manifest()).unwrap()).unwrap();
        assert_eq!(issues.iter().filter(|i| i.code == "missing_required").count(), 2);
        assert!(issues.iter().any(|i| i.code == "invalid_jar"));
        assert!(inspect_inventory(Path::new("."), &[]).is_err());
        assert_eq!(fs::read(tmp.path().join("launcher_accounts.json")).unwrap(), b"private fixture");
    }
}
