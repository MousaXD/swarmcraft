use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_consensus::has_quorum;
use swarm_core::{protocol_v2::verify_world_config_signature, verify_snapshot_signature, DataPaths};
use swarm_network::{HostCapabilityV1, HostRuntimeReadinessV1, ServerModsReadinessV1};
use swarm_protocol::{
    EpochMode, EpochRecordV1, Hash32, PeerId, SnapshotManifestV1, WorldDescriptorV1, WorldId, WorldStatusV1,
};
use swarm_storage::Storage;

use crate::migration::{load_runtime_config, RuntimeLaunchConfig};

pub const HOST_READINESS_MAX_AGE_MS: u64 = 6_000;
const RUNTIME_VERIFICATION_SCHEMA: u16 = 1;
const MOD_VERIFICATION_SCHEMA: u16 = 1;
const LEGACY_COMPAT_ARTIFACT: &str = "swarmcraft.legacy-compatibility";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostReadinessState {
    Safe,
    Sleeping,
    WorldWillStop,
    Syncing,
    BlockedByRuntime,
    BlockedByMods,
    BlockedByQuorum,
    DegradedSafety,
    Conflict,
    NotCurrentHost,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostPeerReadiness {
    pub peer_id: String,
    pub reachable: bool,
    pub current_state: bool,
    pub authority_eligible: bool,
    pub capability_fresh: bool,
    pub runtime: HostRuntimeReadinessV1,
    pub server_mods: ServerModsReadinessV1,
    pub conflict_free: bool,
    pub recovery_quorum_without_authority: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostReadinessReport {
    pub world_id: String,
    pub generated_unix_ms: u64,
    pub local_peer_id: String,
    pub current_host_peer_id: Option<String>,
    pub state: HostReadinessState,
    pub safe_to_shutdown: bool,
    pub successor_peer_id: Option<String>,
    pub handoff_candidate_peer_id: Option<String>,
    pub world_data_replicated: bool,
    pub detail: String,
    pub peers: Vec<HostPeerReadiness>,
}

#[derive(Debug, Clone)]
pub struct PeerReadinessObservation {
    pub peer_id: PeerId,
    pub reachable: bool,
    pub status: Option<WorldStatusV1>,
    pub capability: Option<HostCapabilityV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeVerificationRecord {
    schema_version: u16,
    world_id: WorldId,
    compatibility_fingerprint: Hash32,
    java: PathBuf,
    server_jar: PathBuf,
    mod_jar: PathBuf,
    server_jar_hash: String,
    mod_jar_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ServerModsVerificationRecord {
    schema_version: u16,
    world_id: WorldId,
    compatibility_fingerprint: Hash32,
    state: ServerModsReadinessV1,
}

pub fn invalidate_runtime_verification(paths: &DataPaths, world: WorldId) -> Result<()> {
    remove_if_present(&runtime_verification_path(paths, world))
}

pub fn record_runtime_verified(
    paths: &DataPaths,
    world: WorldId,
    config: &RuntimeLaunchConfig,
    compatibility_fingerprint: Hash32,
) -> Result<()> {
    if !java_available(&config.java) {
        anyhow::bail!("verified runtime Java executable is no longer available: {}", config.java.display());
    }
    if !config.server_jar.is_file() {
        anyhow::bail!("verified runtime server jar is missing: {}", config.server_jar.display());
    }
    if !config.mod_jar.is_file() {
        anyhow::bail!("verified SwarmCraft Fabric mod is missing: {}", config.mod_jar.display());
    }
    let record = RuntimeVerificationRecord {
        schema_version: RUNTIME_VERIFICATION_SCHEMA,
        world_id: world,
        compatibility_fingerprint,
        java: config.java.clone(),
        server_jar: config.server_jar.clone(),
        mod_jar: config.mod_jar.clone(),
        server_jar_hash: file_hash(&config.server_jar)?,
        mod_jar_hash: file_hash(&config.mod_jar)?,
    };
    atomic_json(&runtime_verification_path(paths, world), &record)
}

/// Machine-local producer contract for the server-mod manager.
///
/// `Ready` must only be recorded after the exact server-mod requirements for
/// `compatibility_fingerprint` have been checked on this machine. A changed
/// world compatibility fingerprint automatically invalidates this assertion.
pub fn record_server_mod_readiness(
    paths: &DataPaths,
    world: WorldId,
    compatibility_fingerprint: Hash32,
    state: ServerModsReadinessV1,
) -> Result<()> {
    let record = ServerModsVerificationRecord {
        schema_version: MOD_VERIFICATION_SCHEMA,
        world_id: world,
        compatibility_fingerprint,
        state,
    };
    atomic_json(&server_mod_verification_path(paths, world), &record)
}

pub fn invalidate_server_mod_readiness(paths: &DataPaths, world: WorldId) -> Result<()> {
    remove_if_present(&server_mod_verification_path(paths, world))
}

pub fn local_host_capability(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    recovery_quorum_without_authority: bool,
) -> Result<Option<HostCapabilityV1>> {
    let Ok(metadata) = storage.load_world(world) else {
        return Ok(None);
    };
    let Ok(descriptor) = storage.load_world_descriptor(world) else {
        return Ok(None);
    };
    let compatibility_fingerprint = descriptor.compatibility_fingerprint;
    let mut runtime = local_runtime_readiness(paths, world, compatibility_fingerprint)?;
    let mut server_mods = local_server_mod_readiness(paths, storage, world, compatibility_fingerprint)?;
    if metadata.genesis.compatibility_fingerprint != compatibility_fingerprint {
        runtime = HostRuntimeReadinessV1::Incompatible;
        server_mods = ServerModsReadinessV1::Incompatible;
    }
    Ok(Some(HostCapabilityV1 {
        world_id: world,
        compatibility_fingerprint,
        runtime,
        server_mods,
        conflict_free: storage.list_solo_conflicts(world)?.is_empty(),
        recovery_quorum_without_authority,
    }))
}

pub fn local_runtime_readiness(
    paths: &DataPaths,
    world: WorldId,
    compatibility_fingerprint: Hash32,
) -> Result<HostRuntimeReadinessV1> {
    let config = match load_runtime_config(paths, world) {
        Ok(config) => config,
        Err(_) => return Ok(HostRuntimeReadinessV1::MissingConfiguration),
    };
    if !config.accept_eula {
        return Ok(HostRuntimeReadinessV1::EulaRequired);
    }
    if !java_available(&config.java) {
        return Ok(HostRuntimeReadinessV1::MissingJava);
    }
    if !config.server_jar.is_file() {
        return Ok(HostRuntimeReadinessV1::MissingServerJar);
    }
    if !config.mod_jar.is_file() {
        return Ok(HostRuntimeReadinessV1::MissingSwarmCraftMod);
    }

    let record: RuntimeVerificationRecord = match read_json(&runtime_verification_path(paths, world)) {
        Ok(record) => record,
        Err(_) => return Ok(HostRuntimeReadinessV1::Unverified),
    };
    if record.schema_version != RUNTIME_VERIFICATION_SCHEMA
        || record.world_id != world
        || record.compatibility_fingerprint != compatibility_fingerprint
    {
        return Ok(HostRuntimeReadinessV1::Incompatible);
    }
    if record.java != config.java || record.server_jar != config.server_jar || record.mod_jar != config.mod_jar {
        return Ok(HostRuntimeReadinessV1::Unverified);
    }
    if record.server_jar_hash != file_hash(&config.server_jar)? || record.mod_jar_hash != file_hash(&config.mod_jar)? {
        return Ok(HostRuntimeReadinessV1::Incompatible);
    }
    Ok(HostRuntimeReadinessV1::Ready)
}

pub fn local_server_mod_readiness(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    compatibility_fingerprint: Hash32,
) -> Result<ServerModsReadinessV1> {
    let config = match storage.load_world_config(world) {
        Ok(config) => config,
        Err(_) => return Ok(ServerModsReadinessV1::Unverified),
    };
    if verify_world_config_signature(&config).is_err() {
        return Ok(ServerModsReadinessV1::Incompatible);
    }
    if config.compatibility_fingerprint()? != compatibility_fingerprint {
        return Ok(ServerModsReadinessV1::Incompatible);
    }
    let has_user_server_mods =
        config.compatibility.required_server_mods.iter().any(|artifact| artifact.artifact_id != LEGACY_COMPAT_ARTIFACT);
    if !has_user_server_mods {
        return Ok(ServerModsReadinessV1::Ready);
    }
    let record: ServerModsVerificationRecord = match read_json(&server_mod_verification_path(paths, world)) {
        Ok(record) => record,
        Err(_) => return Ok(ServerModsReadinessV1::Unverified),
    };
    if record.schema_version != MOD_VERIFICATION_SCHEMA
        || record.world_id != world
        || record.compatibility_fingerprint != compatibility_fingerprint
    {
        return Ok(ServerModsReadinessV1::Incompatible);
    }
    Ok(record.state)
}

pub fn surviving_recovery_quorum(
    descriptor: &WorldDescriptorV1,
    epoch: &EpochRecordV1,
    latest: &SnapshotManifestV1,
    local_peer: PeerId,
    observations: &[PeerReadinessObservation],
) -> Result<bool> {
    if local_peer == epoch.authority_peer_id {
        return Ok(false);
    }
    let Some(local) = descriptor.member(local_peer) else {
        return Ok(false);
    };
    if local.banned || !local.authority_eligible {
        return Ok(false);
    }
    let latest_hash = latest.manifest_hash()?;
    let mut survivors = HashSet::from([local_peer]);
    for observation in observations {
        if !observation.reachable || observation.peer_id == epoch.authority_peer_id || observation.peer_id == local_peer
        {
            continue;
        }
        let Some(member) = descriptor.member(observation.peer_id) else {
            continue;
        };
        if member.banned {
            continue;
        }
        let Some(status) = observation.status.as_ref() else {
            continue;
        };
        if status_matches_canonical(status, descriptor, epoch, latest, latest_hash) {
            survivors.insert(observation.peer_id);
        }
    }
    let member_count = descriptor.members.iter().filter(|member| !member.banned).count();
    Ok(has_quorum(member_count, survivors.len()))
}

pub fn evaluate_from_storage(
    storage: &Storage,
    local_peer: PeerId,
    world: WorldId,
    observations: &[PeerReadinessObservation],
) -> Result<HostReadinessReport> {
    let descriptor = match storage.load_world_descriptor(world) {
        Ok(value) => value,
        Err(error) => {
            return Ok(unknown_report(world, local_peer, None, format!("World membership is unavailable: {error}")))
        }
    };
    let epoch = match storage.load_epoch_record(world) {
        Ok(value) => value,
        Err(error) => {
            return Ok(unknown_report(world, local_peer, None, format!("Authority state is unavailable: {error}")))
        }
    };
    if storage.load_sleep_record(world).is_ok() {
        return Ok(HostReadinessReport {
            world_id: world.to_string(),
            generated_unix_ms: unix_millis()?,
            local_peer_id: local_peer.to_string(),
            current_host_peer_id: Some(epoch.authority_peer_id.to_string()),
            state: HostReadinessState::Sleeping,
            safe_to_shutdown: true,
            successor_peer_id: None,
            handoff_candidate_peer_id: None,
            world_data_replicated: observations.iter().any(|peer| peer.status.is_some()),
            detail: "This world is already durably sleeping. Powering off this device will not interrupt a running Minecraft server.".into(),
            peers: Vec::new(),
        });
    }
    let latest = match storage.latest_snapshot(world)? {
        Some(value) => value,
        None => {
            return Ok(unknown_report(
                world,
                local_peer,
                Some(epoch.authority_peer_id),
                "No canonical snapshot is available.".into(),
            ))
        }
    };
    if let Err(error) = storage.verify_snapshot(&latest) {
        return Ok(unknown_report(
            world,
            local_peer,
            Some(epoch.authority_peer_id),
            format!("The latest snapshot could not be verified: {error}"),
        ));
    }
    if let Err(error) = verify_snapshot_signature(&latest) {
        return Ok(unknown_report(
            world,
            local_peer,
            Some(epoch.authority_peer_id),
            format!("The latest snapshot signature could not be verified: {error}"),
        ));
    }
    let has_conflicts = !storage.list_solo_conflicts(world)?.is_empty();
    evaluate_host_readiness(local_peer, &descriptor, &epoch, &latest, has_conflicts, observations)
}

pub fn evaluate_host_readiness(
    local_peer: PeerId,
    descriptor: &WorldDescriptorV1,
    epoch: &EpochRecordV1,
    latest: &SnapshotManifestV1,
    has_conflicts: bool,
    observations: &[PeerReadinessObservation],
) -> Result<HostReadinessReport> {
    let world = descriptor.world_id;
    let latest_hash = latest.manifest_hash()?;
    let mut peers = Vec::new();
    for member in descriptor.members.iter().filter(|member| member.peer_id != local_peer && !member.banned) {
        let observation = observations.iter().find(|value| value.peer_id == member.peer_id);
        let reachable = observation.is_some_and(|value| value.reachable);
        let current_state = observation
            .and_then(|value| value.status.as_ref())
            .is_some_and(|status| status_matches_canonical(status, descriptor, epoch, latest, latest_hash));
        let status_eligible =
            observation.and_then(|value| value.status.as_ref()).is_some_and(|status| status.authority_eligible);
        let capability = observation.and_then(|value| value.capability.as_ref());
        let capability_fresh = capability.is_some();
        let capability_matches = capability.is_some_and(|value| {
            value.world_id == world && value.compatibility_fingerprint == descriptor.compatibility_fingerprint
        });
        let runtime = match capability {
            Some(value) if capability_matches => value.runtime,
            Some(_) => HostRuntimeReadinessV1::Incompatible,
            None => HostRuntimeReadinessV1::Unverified,
        };
        let server_mods = match capability {
            Some(value) if capability_matches => value.server_mods,
            Some(_) => ServerModsReadinessV1::Incompatible,
            None => ServerModsReadinessV1::Unverified,
        };
        peers.push(HostPeerReadiness {
            peer_id: member.peer_id.to_string(),
            reachable,
            current_state,
            authority_eligible: member.authority_eligible && status_eligible,
            capability_fresh,
            runtime,
            server_mods,
            conflict_free: capability.is_some_and(|value| capability_matches && value.conflict_free),
            recovery_quorum_without_authority: capability
                .is_some_and(|value| capability_matches && value.recovery_quorum_without_authority),
        });
    }
    peers.sort_by(|left, right| left.peer_id.cmp(&right.peer_id));
    let world_data_replicated = peers.iter().any(|peer| peer.current_state);
    let generated_unix_ms = unix_millis()?;

    let base = |state: HostReadinessState,
                safe_to_shutdown: bool,
                successor_peer_id: Option<String>,
                handoff: Option<String>,
                detail: String| HostReadinessReport {
        world_id: world.to_string(),
        generated_unix_ms,
        local_peer_id: local_peer.to_string(),
        current_host_peer_id: Some(epoch.authority_peer_id.to_string()),
        state,
        safe_to_shutdown,
        successor_peer_id,
        handoff_candidate_peer_id: handoff,
        world_data_replicated,
        detail,
        peers: peers.clone(),
    };

    if local_peer != epoch.authority_peer_id {
        return Ok(base(
            HostReadinessState::NotCurrentHost,
            false,
            None,
            None,
            "This device is not the current host. SwarmCraft has not proven that removing this quorum member is safe, so shutdown readiness is intentionally fail-closed.".into(),
        ));
    }
    if has_conflicts {
        return Ok(base(
            HostReadinessState::Conflict,
            false,
            None,
            None,
            "Conflicting world history is preserved on this device. Resolve the conflict before relying on automatic host takeover.".into(),
        ));
    }
    if epoch.mode == EpochMode::Solo {
        return Ok(base(
            HostReadinessState::DegradedSafety,
            false,
            None,
            None,
            "This world is running with solo authority history. Wait for quorum reconciliation before treating another replica as a safe automatic successor.".into(),
        ));
    }

    let eligible_current = |peer: &&HostPeerReadiness| peer.reachable && peer.current_state && peer.authority_eligible;
    if let Some(successor) = peers.iter().filter(eligible_current).find(|peer| {
        peer.runtime == HostRuntimeReadinessV1::Ready
            && peer.server_mods == ServerModsReadinessV1::Ready
            && peer.conflict_free
            && peer.recovery_quorum_without_authority
    }) {
        return Ok(base(
            HostReadinessState::Safe,
            true,
            Some(successor.peer_id.clone()),
            Some(successor.peer_id.clone()),
            format!("Safe to shut down. {} has the current world state, a verified compatible runtime and mods, and can still form the recovery quorum after this host disappears.", successor.peer_id),
        ));
    }

    let handoff = peers.iter().filter(eligible_current).find(|peer| {
        peer.runtime == HostRuntimeReadinessV1::Ready
            && peer.server_mods == ServerModsReadinessV1::Ready
            && peer.conflict_free
    });
    if let Some(candidate) = handoff {
        return Ok(base(
            HostReadinessState::BlockedByQuorum,
            false,
            None,
            Some(candidate.peer_id.clone()),
            format!("{} is ready to host, but automatic recovery would not retain quorum after this computer disappears. Transfer hosting before shutdown.", candidate.peer_id),
        ));
    }

    let canonical_eligible = peers.iter().filter(eligible_current).collect::<Vec<_>>();
    if canonical_eligible.iter().any(|peer| peer.runtime != HostRuntimeReadinessV1::Ready) {
        return Ok(base(
            HostReadinessState::BlockedByRuntime,
            false,
            None,
            None,
            "Another current replica is authority-eligible, but no successor has a verified compatible Minecraft/Fabric runtime on that machine.".into(),
        ));
    }
    if canonical_eligible.iter().any(|peer| peer.server_mods != ServerModsReadinessV1::Ready) {
        return Ok(base(
            HostReadinessState::BlockedByMods,
            false,
            None,
            None,
            "Another current replica can host in principle, but its required server-mod set is missing, incompatible, or not yet verified.".into(),
        ));
    }
    if canonical_eligible.iter().any(|peer| !peer.conflict_free) {
        return Ok(base(
            HostReadinessState::DegradedSafety,
            false,
            None,
            None,
            "A possible successor reports unresolved history safety state and is not eligible for a green shutdown decision.".into(),
        ));
    }
    if peers.iter().any(|peer| peer.reachable && !peer.current_state) {
        return Ok(base(
            HostReadinessState::Syncing,
            false,
            None,
            None,
            "Wait before shutting down. At least one reachable peer has not yet proven it holds the exact current canonical snapshot.".into(),
        ));
    }

    Ok(base(
        HostReadinessState::WorldWillStop,
        false,
        None,
        None,
        if world_data_replicated {
            "No other reachable, authority-eligible device is ready to keep hosting. A current replica exists, but storage replication alone does not make it a host.".into()
        } else {
            "No other reachable device currently proves it can keep this world online. Shutting down this computer will make the world unavailable.".into()
        },
    ))
}

pub fn publish_report(paths: &DataPaths, world: WorldId, report: &HostReadinessReport) -> Result<()> {
    atomic_json(&host_readiness_path(paths, world), report)
}

pub fn load_host_readiness_report(paths: &DataPaths, world: WorldId) -> Result<HostReadinessReport> {
    let mut report: HostReadinessReport = read_json(&host_readiness_path(paths, world))?;
    if report.world_id != world.to_string() {
        anyhow::bail!("host-readiness report references a different world");
    }
    let now = unix_millis()?;
    if now.saturating_sub(report.generated_unix_ms) > HOST_READINESS_MAX_AGE_MS {
        report.state = HostReadinessState::Unknown;
        report.safe_to_shutdown = false;
        report.successor_peer_id = None;
        report.handoff_candidate_peer_id = None;
        report.detail = "Host-readiness data is stale. Keep SwarmCraft running and reconnect peers before deciding whether to shut down.".into();
        for peer in &mut report.peers {
            peer.reachable = false;
            peer.current_state = false;
            peer.capability_fresh = false;
            peer.recovery_quorum_without_authority = false;
        }
    }
    Ok(report)
}

pub fn unknown_report(
    world: WorldId,
    local_peer: PeerId,
    current_host: Option<PeerId>,
    detail: String,
) -> HostReadinessReport {
    HostReadinessReport {
        world_id: world.to_string(),
        generated_unix_ms: unix_millis().unwrap_or(0),
        local_peer_id: local_peer.to_string(),
        current_host_peer_id: current_host.map(|peer| peer.to_string()),
        state: HostReadinessState::Unknown,
        safe_to_shutdown: false,
        successor_peer_id: None,
        handoff_candidate_peer_id: None,
        world_data_replicated: false,
        detail,
        peers: Vec::new(),
    }
}

fn status_matches_canonical(
    status: &WorldStatusV1,
    descriptor: &WorldDescriptorV1,
    epoch: &EpochRecordV1,
    latest: &SnapshotManifestV1,
    latest_hash: Hash32,
) -> bool {
    status.world_id == descriptor.world_id
        && status.epoch == epoch.epoch_number
        && status.sequence == latest.sequence
        && status.latest_snapshot == Some(latest_hash)
        && status.state_hash == Some(latest.state_root)
        && status.compatibility_fingerprint == descriptor.compatibility_fingerprint
}

fn java_available(java: &Path) -> bool {
    if java.is_absolute() || java.components().count() > 1 {
        return java.is_file();
    }
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    let names = java_command_names(java);
    env::split_paths(&path).any(|directory| names.iter().any(|name| directory.join(name).is_file()))
}

fn java_command_names(java: &Path) -> Vec<PathBuf> {
    #[cfg(not(windows))]
    {
        vec![java.to_path_buf()]
    }
    #[cfg(windows)]
    {
        let mut names = vec![java.to_path_buf()];
        if java.extension().is_none() {
            let extensions = env::var_os("PATHEXT")
                .map(|value| value.to_string_lossy().split(';').map(str::to_owned).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![".EXE".into(), ".CMD".into(), ".BAT".into()]);
            for extension in extensions {
                names.push(PathBuf::from(format!("{}{}", java.to_string_lossy(), extension.to_ascii_lowercase())));
                names.push(PathBuf::from(format!("{}{}", java.to_string_lossy(), extension.to_ascii_uppercase())));
            }
        }
        names
    }
}

fn file_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("cannot hash {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn runtime_verification_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("runtime-verified.json")
}

fn server_mod_verification_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("server-mods-verified.json")
}

fn host_readiness_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("host-readiness.json")
}

fn control_dir(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("control").join(world.to_hex())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("cannot decode {}", path.display()))
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let parent = path.parent().context("control file has no parent directory")?;
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

fn unix_millis() -> Result<u64> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).context("system clock is before Unix epoch")?;
    Ok(duration.as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{WorldMemberV1, PROTOCOL_VERSION};

    fn member(id: u8, eligible: bool) -> WorldMemberV1 {
        WorldMemberV1 { peer_id: PeerId([id; 32]), public_key: [id; 32], authority_eligible: eligible, banned: false }
    }

    fn fixture(member_count: usize) -> (WorldDescriptorV1, EpochRecordV1, SnapshotManifestV1) {
        let world = WorldId([9; 32]);
        let members = (1..=member_count).map(|id| member(id as u8, true)).collect::<Vec<_>>();
        let descriptor = WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: Hash32([7; 32]),
            members,
            preferred_replication_factor: 3,
        };
        let latest = SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            snapshot_number: 4,
            epoch: 5,
            sequence: 12,
            previous_snapshot_hash: None,
            entries: Vec::new(),
            state_root: Hash32([8; 32]),
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            signature: vec![1],
        };
        let epoch = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: 5,
            previous_epoch_hash: None,
            base_state_hash: latest.state_root,
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            mode: EpochMode::Quorum,
            fencing_token: 6,
            reason: "test".into(),
            signature: vec![1],
        };
        (descriptor, epoch, latest)
    }

    fn observation(
        peer: u8,
        descriptor: &WorldDescriptorV1,
        epoch: &EpochRecordV1,
        latest: &SnapshotManifestV1,
        runtime: HostRuntimeReadinessV1,
        mods: ServerModsReadinessV1,
        recovery_quorum: bool,
    ) -> PeerReadinessObservation {
        PeerReadinessObservation {
            peer_id: PeerId([peer; 32]),
            reachable: true,
            status: Some(WorldStatusV1 {
                world_id: descriptor.world_id,
                epoch: epoch.epoch_number,
                sequence: latest.sequence,
                latest_snapshot: Some(latest.manifest_hash().unwrap()),
                state_hash: Some(latest.state_root),
                compatibility_fingerprint: descriptor.compatibility_fingerprint,
                authority_eligible: true,
            }),
            capability: Some(HostCapabilityV1 {
                world_id: descriptor.world_id,
                compatibility_fingerprint: descriptor.compatibility_fingerprint,
                runtime,
                server_mods: mods,
                conflict_free: true,
                recovery_quorum_without_authority: recovery_quorum,
            }),
        }
    }

    #[test]
    fn healthy_successor_is_safe_only_with_surviving_quorum() {
        let (descriptor, epoch, latest) = fixture(3);
        let peers = vec![observation(
            2,
            &descriptor,
            &epoch,
            &latest,
            HostRuntimeReadinessV1::Ready,
            ServerModsReadinessV1::Ready,
            true,
        )];
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, false, &peers).unwrap();
        assert_eq!(report.state, HostReadinessState::Safe);
        assert!(report.safe_to_shutdown);
        assert_eq!(report.successor_peer_id.as_deref(), Some(PeerId([2; 32]).to_string().as_str()));
    }

    #[test]
    fn stale_replica_is_syncing() {
        let (descriptor, epoch, latest) = fixture(3);
        let mut bob = observation(
            2,
            &descriptor,
            &epoch,
            &latest,
            HostRuntimeReadinessV1::Ready,
            ServerModsReadinessV1::Ready,
            true,
        );
        bob.status.as_mut().unwrap().sequence -= 1;
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, false, &[bob]).unwrap();
        assert_eq!(report.state, HostReadinessState::Syncing);
        assert!(!report.safe_to_shutdown);
    }

    #[test]
    fn incompatible_runtime_blocks_shutdown() {
        let (descriptor, epoch, latest) = fixture(3);
        let peers = vec![observation(
            2,
            &descriptor,
            &epoch,
            &latest,
            HostRuntimeReadinessV1::Incompatible,
            ServerModsReadinessV1::Ready,
            true,
        )];
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, false, &peers).unwrap();
        assert_eq!(report.state, HostReadinessState::BlockedByRuntime);
    }

    #[test]
    fn missing_mods_block_shutdown() {
        let (descriptor, epoch, latest) = fixture(3);
        let peers = vec![observation(
            2,
            &descriptor,
            &epoch,
            &latest,
            HostRuntimeReadinessV1::Ready,
            ServerModsReadinessV1::Missing,
            true,
        )];
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, false, &peers).unwrap();
        assert_eq!(report.state, HostReadinessState::BlockedByMods);
    }

    #[test]
    fn offline_successor_means_world_will_stop() {
        let (descriptor, epoch, latest) = fixture(3);
        let mut bob = observation(
            2,
            &descriptor,
            &epoch,
            &latest,
            HostRuntimeReadinessV1::Ready,
            ServerModsReadinessV1::Ready,
            true,
        );
        bob.reachable = false;
        bob.status = None;
        bob.capability = None;
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, false, &[bob]).unwrap();
        assert_eq!(report.state, HostReadinessState::WorldWillStop);
    }

    #[test]
    fn two_member_successor_requires_explicit_handoff() {
        let (descriptor, epoch, latest) = fixture(2);
        let peers = vec![observation(
            2,
            &descriptor,
            &epoch,
            &latest,
            HostRuntimeReadinessV1::Ready,
            ServerModsReadinessV1::Ready,
            false,
        )];
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, false, &peers).unwrap();
        assert_eq!(report.state, HostReadinessState::BlockedByQuorum);
        assert!(!report.safe_to_shutdown);
        assert!(report.handoff_candidate_peer_id.is_some());
    }

    #[test]
    fn solo_authority_is_degraded_not_green() {
        let (descriptor, mut epoch, latest) = fixture(3);
        epoch.mode = EpochMode::Solo;
        let peers = vec![observation(
            2,
            &descriptor,
            &epoch,
            &latest,
            HostRuntimeReadinessV1::Ready,
            ServerModsReadinessV1::Ready,
            true,
        )];
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, false, &peers).unwrap();
        assert_eq!(report.state, HostReadinessState::DegradedSafety);
    }

    #[test]
    fn conflicting_history_is_never_green() {
        let (descriptor, epoch, latest) = fixture(3);
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, true, &[]).unwrap();
        assert_eq!(report.state, HostReadinessState::Conflict);
        assert!(!report.safe_to_shutdown);
    }

    #[test]
    fn only_member_means_world_will_stop() {
        let (descriptor, epoch, latest) = fixture(1);
        let report = evaluate_host_readiness(PeerId([1; 32]), &descriptor, &epoch, &latest, false, &[]).unwrap();
        assert_eq!(report.state, HostReadinessState::WorldWillStop);
        assert!(!report.safe_to_shutdown);
    }

    #[test]
    fn recovery_quorum_is_measured_without_the_current_authority() {
        let (descriptor, epoch, latest) = fixture(3);
        let sarah = observation(
            3,
            &descriptor,
            &epoch,
            &latest,
            HostRuntimeReadinessV1::Unverified,
            ServerModsReadinessV1::Unverified,
            false,
        );
        assert!(surviving_recovery_quorum(&descriptor, &epoch, &latest, PeerId([2; 32]), &[sarah]).unwrap());

        let (descriptor, epoch, latest) = fixture(2);
        assert!(!surviving_recovery_quorum(&descriptor, &epoch, &latest, PeerId([2; 32]), &[]).unwrap());
    }

    #[test]
    fn runtime_proof_detects_changed_artifacts_and_reconfiguration() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let world = WorldId([6; 32]);
        let fingerprint = Hash32([4; 32]);
        let java = temp.path().join("java");
        let server = temp.path().join("server.jar");
        let fabric = temp.path().join("swarmcraft.jar");
        fs::write(&java, b"java").unwrap();
        fs::write(&server, b"server-v1").unwrap();
        fs::write(&fabric, b"fabric-v1").unwrap();
        let config = RuntimeLaunchConfig {
            java,
            server_jar: server.clone(),
            mod_jar: fabric,
            accept_eula: true,
            game_endpoint: None,
        };
        crate::migration::save_runtime_config(&paths, world, &config).unwrap();
        assert_eq!(local_runtime_readiness(&paths, world, fingerprint).unwrap(), HostRuntimeReadinessV1::Unverified);
        record_runtime_verified(&paths, world, &config, fingerprint).unwrap();
        assert_eq!(local_runtime_readiness(&paths, world, fingerprint).unwrap(), HostRuntimeReadinessV1::Ready);

        fs::write(&server, b"server-v2").unwrap();
        assert_eq!(local_runtime_readiness(&paths, world, fingerprint).unwrap(), HostRuntimeReadinessV1::Incompatible);

        fs::write(&server, b"server-v1").unwrap();
        record_runtime_verified(&paths, world, &config, fingerprint).unwrap();
        crate::migration::save_runtime_config(&paths, world, &config).unwrap();
        assert_eq!(local_runtime_readiness(&paths, world, fingerprint).unwrap(), HostRuntimeReadinessV1::Unverified);
    }
}
