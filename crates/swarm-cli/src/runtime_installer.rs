use crate::{
    migration::{load_runtime_config, save_runtime_config, RuntimeLaunchConfig},
    runtime_layout::{
        managed_fabric_api, managed_fabric_server, managed_java_root, managed_minecraft_server, managed_runtimes_root,
        managed_swarmcraft_fabric, managed_world_config_dir, managed_world_mods_dir, managed_world_root,
        managed_world_server_dir, runtime_install_lock_path, runtime_lock_path, RUNTIME_LOCK_SCHEMA_VERSION,
    },
    server_mods,
};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::DataPaths;
use swarm_protocol::WorldId;
use swarm_storage::Storage;

const MINECRAFT_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const FABRIC_INSTALLERS: &str = "https://meta.fabricmc.net/v2/versions/installer";
const FABRIC_API_METADATA: &str = "https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api/maven-metadata.xml";
const SWARMCRAFT_RELEASE_API: &str = "https://api.github.com/repos/MousaXD/swarmcraft/releases/tags";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeComponentKind {
    Java,
    MinecraftServer,
    FabricLoader,
    FabricApi,
    SwarmcraftFabric,
    ServerDirectories,
    Eula,
    ServerMods,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeComponentState {
    Ready,
    Missing,
    Incompatible,
    Corrupt,
    Required,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeComponentStatus {
    pub kind: RuntimeComponentKind,
    pub state: RuntimeComponentState,
    pub version: Option<String>,
    pub path: Option<PathBuf>,
    pub managed: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub world_id: String,
    pub minecraft_version: String,
    pub fabric_loader_version: String,
    pub required_java_major: u32,
    pub ready: bool,
    pub eula_accepted: bool,
    pub launch_configured: bool,
    pub manual_configuration: bool,
    pub components: Vec<RuntimeComponentStatus>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePhase {
    Checking,
    DownloadingJava,
    DownloadingServer,
    InstallingFabric,
    InstallingFabricApi,
    InstallingSwarmcraftMod,
    PreparingDirectories,
    WaitingForEula,
    Verifying,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeProgress {
    pub phase: RuntimePhase,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePlanAction {
    pub phase: RuntimePhase,
    pub component: Option<RuntimeComponentKind>,
    pub description: String,
    pub requires_network: bool,
    pub requires_eula_acceptance: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePlan {
    pub world_id: String,
    pub ready: bool,
    pub actions: Vec<RuntimePlanAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeInstallOptions {
    pub accept_eula: bool,
    pub game_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeInstallReport {
    pub status: RuntimeStatus,
    pub completed_phases: Vec<RuntimePhase>,
    pub launch_config_saved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeLock {
    schema_version: u16,
    world_id: String,
    minecraft_version: String,
    fabric_loader_version: String,
    required_java_major: u32,
    fabric_installer_version: String,
    fabric_api_version: String,
    swarmcraft_adapter_version: String,
    java_path: PathBuf,
    java_managed: bool,
    #[serde(default)]
    java_provenance: Option<JavaProvenance>,
    artifacts: BTreeMap<String, ArtifactRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ArtifactRecord {
    version: String,
    path: PathBuf,
    source: String,
    sha1: Option<String>,
    sha256: String,
    #[serde(default)]
    upstream_sha256: Option<String>,
    #[serde(default)]
    verification: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct JavaProvenance {
    root: PathBuf,
    executable: PathBuf,
    source: String,
    archive_sha256: String,
    tree_sha256: String,
}

#[derive(Debug, Clone)]
struct ResolvedArtifact {
    version: String,
    source: ArtifactSource,
    sha1: Option<String>,
    sha256: Option<String>,
}

#[derive(Debug, Clone)]
enum ArtifactSource {
    Download(String),
    Local(PathBuf),
}

#[derive(Debug, Clone)]
struct ResolvedRuntime {
    required_java_major: u32,
    java: Option<ResolvedArtifact>,
    minecraft_server: ResolvedArtifact,
    fabric_server: ResolvedArtifact,
    fabric_installer_version: String,
    fabric_api: ResolvedArtifact,
    swarmcraft_fabric: ResolvedArtifact,
    fabric_api_version: String,
    adapter_version: String,
}

pub struct RuntimeInstaller<'a> {
    paths: &'a DataPaths,
    storage: &'a Storage,
}

impl<'a> RuntimeInstaller<'a> {
    pub fn new(paths: &'a DataPaths, storage: &'a Storage) -> Self {
        Self { paths, storage }
    }

    pub fn inspect(&self, world: WorldId) -> Result<RuntimeStatus> {
        let metadata = self.storage.load_world(world)?;
        let lock = match load_runtime_lock(self.paths, world) {
            Ok(lock) => Some(lock),
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                None
            }
            Err(error) => {
                return Err(error)
                    .context("managed runtime metadata is unreadable; preserve it and review before repairing")
            }
        };
        let manual = load_runtime_config(self.paths, world).ok();
        let required_java_major = lock.as_ref().map_or_else(
            || heuristic_java_major(&metadata.genesis.minecraft_version),
            |value| value.required_java_major,
        );
        let lock_compatible = lock.as_ref().is_some_and(|value| {
            value.schema_version == RUNTIME_LOCK_SCHEMA_VERSION
                && value.world_id == world.to_string()
                && value.minecraft_version == metadata.genesis.minecraft_version
                && value.fabric_loader_version == metadata.genesis.fabric_loader_version
        });

        let manual_only = manual.as_ref().filter(|_| lock.is_none());
        let mut components = vec![
            self.java_status(required_java_major, lock.as_ref(), manual.as_ref(), lock_compatible),
            manual_only.map_or_else(
                || self.artifact_status(
                    RuntimeComponentKind::MinecraftServer,
                    "minecraft_server",
                    lock.as_ref(),
                    lock_compatible,
                    Some(managed_world_server_dir(self.paths, world).join("server.jar")),
                ),
                |config| Self::manual_file_status(
                    RuntimeComponentKind::MinecraftServer,
                    &config.server_jar,
                    "manual Fabric server launcher is configured; live Fabric verification proves the exact Minecraft runtime before host readiness",
                ),
            ),
            manual_only.map_or_else(
                || self.artifact_status(
                    RuntimeComponentKind::FabricLoader,
                    "fabric_loader",
                    lock.as_ref(),
                    lock_compatible,
                    None,
                ),
                |config| Self::manual_file_status(
                    RuntimeComponentKind::FabricLoader,
                    &config.server_jar,
                    "manual Fabric launcher is configured; loader compatibility is proven by the live Fabric handshake",
                ),
            ),
            manual_only.map_or_else(
                || self.artifact_status(
                    RuntimeComponentKind::FabricApi,
                    "fabric_api",
                    lock.as_ref(),
                    lock_compatible,
                    Some(managed_world_mods_dir(self.paths, world).join("fabric-api.jar")),
                ),
                |config| Self::manual_file_status(
                    RuntimeComponentKind::FabricApi,
                    &config.mod_jar,
                    "manual SwarmCraft Fabric integration is configured; release packaging embeds Fabric API and live verification remains required",
                ),
            ),
            manual_only.map_or_else(
                || self.swarmcraft_status(world, lock.as_ref(), lock_compatible),
                |config| self.manual_swarmcraft_status(world, config),
            ),
        ];

        let directories_ready = manual_only.is_some()
            || [
                managed_world_root(self.paths, world),
                managed_world_server_dir(self.paths, world),
                managed_world_mods_dir(self.paths, world),
                managed_world_config_dir(self.paths, world),
            ]
            .iter()
            .all(|path| path.is_dir());
        components.push(RuntimeComponentStatus {
            kind: RuntimeComponentKind::ServerDirectories,
            state: if directories_ready { RuntimeComponentState::Ready } else { RuntimeComponentState::Missing },
            version: None,
            path: Some(managed_world_root(self.paths, world)),
            managed: true,
            detail: None,
        });

        let launch_matches = manual.as_ref().is_some_and(|config| {
            lock.as_ref().is_none_or(|lock| {
                config.java == lock.java_path
                    && lock.artifacts.get("fabric_loader").is_some_and(|record| config.server_jar == record.path)
                    && lock.artifacts.get("swarmcraft_fabric").is_some_and(|record| config.mod_jar == record.path)
            })
        });
        let eula_accepted = launch_matches && manual.as_ref().is_some_and(|config| config.accept_eula);
        components.push(RuntimeComponentStatus {
            kind: RuntimeComponentKind::Eula,
            state: if eula_accepted { RuntimeComponentState::Ready } else { RuntimeComponentState::Required },
            version: None,
            path: None,
            managed: true,
            detail: (!eula_accepted)
                .then(|| "explicit Minecraft server EULA acceptance is required before launch".into()),
        });
        components.push(self.server_mods_status(world));

        let ready = components.iter().all(|component| component.state == RuntimeComponentState::Ready);
        Ok(RuntimeStatus {
            world_id: world.to_string(),
            minecraft_version: metadata.genesis.minecraft_version.clone(),
            fabric_loader_version: metadata.genesis.fabric_loader_version.clone(),
            required_java_major,
            ready,
            eula_accepted,
            launch_configured: launch_matches,
            manual_configuration: manual.is_some() && lock.is_none(),
            components,
        })
    }

    pub fn verify(&self, world: WorldId) -> Result<RuntimeStatus> {
        self.inspect(world)
    }

    pub fn plan(&self, world: WorldId) -> Result<RuntimePlan> {
        let status = self.inspect(world)?;
        if status.ready {
            return Ok(RuntimePlan { world_id: world.to_string(), ready: true, actions: Vec::new() });
        }

        let mut actions = Vec::new();
        for component in &status.components {
            if component.state == RuntimeComponentState::Ready {
                continue;
            }
            let (phase, network, eula) = match component.kind {
                RuntimeComponentKind::Java => (RuntimePhase::DownloadingJava, true, false),
                RuntimeComponentKind::MinecraftServer => (RuntimePhase::DownloadingServer, true, false),
                RuntimeComponentKind::FabricLoader => (RuntimePhase::InstallingFabric, true, false),
                RuntimeComponentKind::FabricApi => (RuntimePhase::InstallingFabricApi, true, false),
                RuntimeComponentKind::SwarmcraftFabric => (RuntimePhase::InstallingSwarmcraftMod, true, false),
                RuntimeComponentKind::ServerDirectories => (RuntimePhase::PreparingDirectories, false, false),
                RuntimeComponentKind::Eula => (RuntimePhase::WaitingForEula, false, true),
                RuntimeComponentKind::ServerMods => (RuntimePhase::Verifying, false, false),
            };
            actions.push(RuntimePlanAction {
                phase,
                component: Some(component.kind),
                description: component.detail.clone().unwrap_or_else(|| format!("prepare {:?}", component.kind)),
                requires_network: network,
                requires_eula_acceptance: eula,
            });
        }

        if actions.iter().any(|action| action.requires_network) {
            self.resolve_runtime(world).context("automatic runtime sources could not be resolved")?;
        }
        Ok(RuntimePlan { world_id: world.to_string(), ready: false, actions })
    }

    pub fn install<F: FnMut(RuntimeProgress)>(
        &self,
        world: WorldId,
        options: RuntimeInstallOptions,
        progress: F,
    ) -> Result<RuntimeInstallReport> {
        self.install_inner(world, options, false, progress)
    }

    pub fn repair<F: FnMut(RuntimeProgress)>(
        &self,
        world: WorldId,
        options: RuntimeInstallOptions,
        progress: F,
    ) -> Result<RuntimeInstallReport> {
        self.install_inner(world, options, true, progress)
    }

    fn install_inner<F: FnMut(RuntimeProgress)>(
        &self,
        world: WorldId,
        options: RuntimeInstallOptions,
        force: bool,
        mut progress: F,
    ) -> Result<RuntimeInstallReport> {
        self.storage.load_world(world)?;
        let _guard = InstallGuard::acquire(&runtime_install_lock_path(self.paths, world))?;
        let _cache_guard = InstallGuard::acquire(&managed_runtimes_root(self.paths).join("publication.lock"))?;
        let _runtime_slot = crate::migration::AuthorityRuntimeGuard::acquire(self.paths, world)?;
        crate::runtime_process_guard::refuse_unaccounted_world_runtime(self.paths, world)?;
        let mut completed = Vec::new();
        emit(&mut progress, &mut completed, RuntimePhase::Checking, "Checking local runtime");

        let initial = self.inspect(world)?;
        if !force && platform_components_ready(&initial) {
            if options.accept_eula && !initial.eula_accepted {
                let lock = load_runtime_lock(self.paths, world)?;
                self.save_launch_config(world, &lock, &options)?;
            }
            emit(&mut progress, &mut completed, RuntimePhase::Verifying, "Verifying prepared runtime");
            let status = self.inspect(world)?;
            if status.ready {
                emit(&mut progress, &mut completed, RuntimePhase::Ready, "Runtime is ready");
            } else if !status.eula_accepted {
                emit(
                    &mut progress,
                    &mut completed,
                    RuntimePhase::WaitingForEula,
                    "Minecraft server EULA acceptance is required",
                );
            }
            return Ok(RuntimeInstallReport {
                launch_config_saved: status.launch_configured,
                status,
                completed_phases: completed,
            });
        }

        validate_runtime_segment(&initial.minecraft_version)?;
        validate_runtime_segment(&initial.fabric_loader_version)?;
        let resolved = self.resolve_runtime(world)?;
        validate_runtime_segment(&resolved.fabric_api_version)?;
        validate_runtime_segment(&resolved.adapter_version)?;
        prepare_world_directories(self.paths, world)?;
        emit(&mut progress, &mut completed, RuntimePhase::PreparingDirectories, "Prepared managed server directories");

        let (java_path, java_provenance) = if let Ok(major) = probe_java_major(Path::new("java")) {
            if major == resolved.required_java_major {
                (PathBuf::from("java"), None)
            } else {
                self.install_java(&resolved, force, &mut progress, &mut completed)?
            }
        } else {
            self.install_java(&resolved, force, &mut progress, &mut completed)?
        };

        emit(&mut progress, &mut completed, RuntimePhase::DownloadingServer, "Preparing Minecraft server");
        let minecraft_path = managed_minecraft_server(self.paths, &initial.minecraft_version);
        let minecraft_record = install_cached_artifact(&resolved.minecraft_server, &minecraft_path, force)?;

        emit(&mut progress, &mut completed, RuntimePhase::InstallingFabric, "Preparing Fabric Loader server launcher");
        let fabric_path = managed_fabric_server(self.paths, &initial.minecraft_version, &initial.fabric_loader_version);
        let fabric_record = install_cached_artifact(&resolved.fabric_server, &fabric_path, force)?;

        emit(&mut progress, &mut completed, RuntimePhase::InstallingFabricApi, "Installing Fabric API");
        let fabric_api_path = managed_fabric_api(self.paths, &resolved.fabric_api_version);
        let fabric_api_record = install_cached_artifact(&resolved.fabric_api, &fabric_api_path, force)?;

        emit(
            &mut progress,
            &mut completed,
            RuntimePhase::InstallingSwarmcraftMod,
            "Installing SwarmCraft Fabric integration",
        );
        let swarmcraft_path = managed_swarmcraft_fabric(self.paths, &resolved.adapter_version);
        let swarmcraft_record = install_cached_artifact(&resolved.swarmcraft_fabric, &swarmcraft_path, force)?;

        // All shared artifacts have been prepared before changing any world-local copy.
        // The authority slot prevents a live runtime from consuming partial local staging.
        crate::host_readiness::invalidate_runtime_verification(self.paths, world)?;
        crate::host_readiness::invalidate_cached_readiness(self.paths, world)?;
        atomic_copy(&minecraft_record.path, &managed_world_server_dir(self.paths, world).join("server.jar"))?;
        atomic_copy(&fabric_api_record.path, &managed_world_mods_dir(self.paths, world).join("fabric-api.jar"))?;
        atomic_copy(&swarmcraft_record.path, &managed_world_mods_dir(self.paths, world).join("swarmcraft-fabric.jar"))?;
        let mut artifacts = BTreeMap::new();
        artifacts.insert("minecraft_server".into(), minecraft_record);
        artifacts.insert("fabric_loader".into(), fabric_record);
        artifacts.insert("fabric_api".into(), fabric_api_record);
        artifacts.insert("swarmcraft_fabric".into(), swarmcraft_record);
        let lock = RuntimeLock {
            schema_version: RUNTIME_LOCK_SCHEMA_VERSION,
            world_id: world.to_string(),
            minecraft_version: initial.minecraft_version,
            fabric_loader_version: initial.fabric_loader_version,
            required_java_major: resolved.required_java_major,
            fabric_installer_version: resolved.fabric_installer_version,
            fabric_api_version: resolved.fabric_api_version,
            swarmcraft_adapter_version: resolved.adapter_version,
            java_managed: java_path != Path::new("java"),
            java_path,
            java_provenance,
            artifacts,
        };
        atomic_json(&runtime_lock_path(self.paths, world), &lock)?;

        if options.accept_eula {
            self.save_launch_config(world, &lock, &options)?;
        } else {
            emit(
                &mut progress,
                &mut completed,
                RuntimePhase::WaitingForEula,
                "Minecraft server EULA acceptance is required",
            );
        }

        emit(&mut progress, &mut completed, RuntimePhase::Verifying, "Verifying installed runtime");
        let status = self.inspect(world)?;
        if status.ready {
            emit(&mut progress, &mut completed, RuntimePhase::Ready, "Runtime is ready");
        }
        Ok(RuntimeInstallReport { launch_config_saved: status.launch_configured, status, completed_phases: completed })
    }

    fn install_java<F: FnMut(RuntimeProgress)>(
        &self,
        resolved: &ResolvedRuntime,
        force: bool,
        progress: &mut F,
        completed: &mut Vec<RuntimePhase>,
    ) -> Result<(PathBuf, Option<JavaProvenance>)> {
        emit(progress, completed, RuntimePhase::DownloadingJava, "Preparing managed Java runtime");
        let artifact = resolved.java.as_ref().context("managed Java source was not resolved")?;
        let base = managed_java_root(self.paths, resolved.required_java_major).join(platform_key());
        let provenance = install_java_generation(artifact, &base, resolved.required_java_major, force)?;
        Ok((provenance.executable.clone(), Some(provenance)))
    }

    fn save_launch_config(&self, world: WorldId, lock: &RuntimeLock, options: &RuntimeInstallOptions) -> Result<()> {
        if !options.accept_eula {
            bail!("explicit Minecraft server EULA acceptance is required before runtime configuration");
        }
        let server = lock.artifacts.get("fabric_loader").context("Fabric launcher is missing from runtime lock")?;
        let bridge = lock
            .artifacts
            .get("swarmcraft_fabric")
            .context("SwarmCraft Fabric integration is missing from runtime lock")?;
        let status = self.server_mods_status(world);
        if status.state != RuntimeComponentState::Ready {
            bail!("runtime cannot be made host-ready until required server mods are satisfied");
        }
        save_runtime_config(
            self.paths,
            world,
            &RuntimeLaunchConfig {
                java: lock.java_path.clone(),
                server_jar: server.path.clone(),
                mod_jar: bridge.path.clone(),
                accept_eula: true,
                game_endpoint: options.game_endpoint.clone(),
            },
        )
    }

    fn resolve_runtime(&self, world: WorldId) -> Result<ResolvedRuntime> {
        let metadata = self.storage.load_world(world)?;
        let adapter_version = self
            .storage
            .load_world_config(world)
            .ok()
            .map(|config| config.compatibility.fabric_adapter_version)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        if adapter_version != env!("CARGO_PKG_VERSION") {
            bail!(
                "world requires SwarmCraft Fabric adapter {}, but this application build provides {}",
                adapter_version,
                env!("CARGO_PKG_VERSION")
            );
        }

        let (required_java_major, minecraft_server) = resolve_minecraft(&metadata.genesis.minecraft_version)?;
        let fabric_installer_version = resolve_fabric_installer()?;
        let fabric_server = ResolvedArtifact {
            version: metadata.genesis.fabric_loader_version.clone(),
            source: ArtifactSource::Download(format!(
                "https://meta.fabricmc.net/v2/versions/loader/{}/{}/{}/server/jar",
                metadata.genesis.minecraft_version, metadata.genesis.fabric_loader_version, fabric_installer_version
            )),
            sha1: None,
            sha256: None,
        };
        let (fabric_api_version, fabric_api) = resolve_fabric_api(&metadata.genesis.minecraft_version)?;
        let swarmcraft_fabric = resolve_swarmcraft_fabric(&adapter_version)?;
        let java = if probe_java_major(Path::new("java")).ok() == Some(required_java_major) {
            None
        } else {
            Some(resolve_managed_java(required_java_major)?)
        };
        Ok(ResolvedRuntime {
            required_java_major,
            java,
            minecraft_server,
            fabric_server,
            fabric_installer_version,
            fabric_api,
            swarmcraft_fabric,
            fabric_api_version,
            adapter_version,
        })
    }

    fn java_status(
        &self,
        required: u32,
        lock: Option<&RuntimeLock>,
        manual: Option<&RuntimeLaunchConfig>,
        lock_compatible: bool,
    ) -> RuntimeComponentStatus {
        let (path, managed) = if let Some(lock) = lock {
            (lock.java_path.clone(), lock.java_managed)
        } else if let Some(config) = manual {
            (config.java.clone(), false)
        } else {
            (PathBuf::from("java"), false)
        };
        let managed_valid = !managed
            || lock
                .and_then(|value| value.java_provenance.as_ref())
                .is_some_and(|record| record.executable == path && verify_java_provenance(record).is_ok());
        let probe = if managed_valid {
            probe_java_major(&path)
        } else {
            Err(anyhow::anyhow!("managed Java provenance is missing or corrupt"))
        };
        let state = match probe {
            Err(_) if !managed_valid => RuntimeComponentState::Corrupt,
            Ok(major) if major == required && (lock.is_none() || lock_compatible) => RuntimeComponentState::Ready,
            Ok(_) => RuntimeComponentState::Incompatible,
            Err(_) => RuntimeComponentState::Missing,
        };
        RuntimeComponentStatus {
            kind: RuntimeComponentKind::Java,
            state,
            version: probe.ok().map(|major| major.to_string()),
            path: Some(path),
            managed,
            detail: Some(if !managed_valid {
                "Managed Java provenance is missing or corrupt; repair before execution".into()
            } else if state != RuntimeComponentState::Ready {
                format!("Minecraft requires Java {required}")
            } else if managed {
                "Official archive SHA-256 and extracted runtime tree verified; live compatibility still required".into()
            } else {
                "External Java: version probe only; binary distribution provenance is not verified".into()
            }),
        }
    }

    fn artifact_status(
        &self,
        kind: RuntimeComponentKind,
        key: &str,
        lock: Option<&RuntimeLock>,
        lock_compatible: bool,
        staged: Option<PathBuf>,
    ) -> RuntimeComponentStatus {
        let Some(record) = lock.and_then(|value| value.artifacts.get(key)) else {
            return RuntimeComponentStatus {
                kind,
                state: RuntimeComponentState::Missing,
                version: None,
                path: None,
                managed: true,
                detail: None,
            };
        };
        if !lock_compatible {
            return RuntimeComponentStatus {
                kind,
                state: RuntimeComponentState::Incompatible,
                version: Some(record.version.clone()),
                path: Some(record.path.clone()),
                managed: true,
                detail: Some("installed artifact belongs to a different world runtime profile".into()),
            };
        }
        let source_ok = verify_record_file(&record.path, record).is_ok();
        let staged_ok = staged.as_ref().is_none_or(|path| verify_record_file(path, record).is_ok());
        RuntimeComponentStatus {
            kind,
            state: if source_ok && staged_ok { RuntimeComponentState::Ready } else { RuntimeComponentState::Corrupt },
            version: Some(record.version.clone()),
            path: Some(record.path.clone()),
            managed: true,
            detail: Some(if source_ok && staged_ok {
                format!(
                    "{}; source: {}",
                    record
                        .verification
                        .as_deref()
                        .unwrap_or("Legacy local SHA-256 only; upstream verification not recorded"),
                    record.source
                )
            } else {
                "artifact is missing or its recorded SHA-256 no longer matches".into()
            }),
        }
    }

    fn swarmcraft_status(
        &self,
        world: WorldId,
        lock: Option<&RuntimeLock>,
        lock_compatible: bool,
    ) -> RuntimeComponentStatus {
        let expected = self
            .storage
            .load_world_config(world)
            .ok()
            .map(|config| config.compatibility.fabric_adapter_version)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        if expected != env!("CARGO_PKG_VERSION") {
            return RuntimeComponentStatus {
                kind: RuntimeComponentKind::SwarmcraftFabric,
                state: RuntimeComponentState::Incompatible,
                version: Some(expected.clone()),
                path: None,
                managed: true,
                detail: Some(format!(
                    "world requires adapter {expected}, application provides {}",
                    env!("CARGO_PKG_VERSION")
                )),
            };
        }
        self.artifact_status(
            RuntimeComponentKind::SwarmcraftFabric,
            "swarmcraft_fabric",
            lock,
            lock_compatible,
            Some(managed_world_mods_dir(self.paths, world).join("swarmcraft-fabric.jar")),
        )
    }

    fn manual_swarmcraft_status(&self, world: WorldId, config: &RuntimeLaunchConfig) -> RuntimeComponentStatus {
        let expected = self
            .storage
            .load_world_config(world)
            .ok()
            .map(|world_config| world_config.compatibility.fabric_adapter_version)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        if expected != env!("CARGO_PKG_VERSION") {
            return RuntimeComponentStatus {
                kind: RuntimeComponentKind::SwarmcraftFabric,
                state: RuntimeComponentState::Incompatible,
                version: Some(expected.clone()),
                path: Some(config.mod_jar.clone()),
                managed: false,
                detail: Some(format!(
                    "world requires SwarmCraft Fabric adapter {expected}, but this application build provides {}",
                    env!("CARGO_PKG_VERSION")
                )),
            };
        }
        Self::manual_file_status(
            RuntimeComponentKind::SwarmcraftFabric,
            &config.mod_jar,
            "manual SwarmCraft Fabric integration is configured; host readiness still requires the live authenticated Fabric handshake",
        )
    }

    fn manual_file_status(kind: RuntimeComponentKind, path: &Path, ready_detail: &str) -> RuntimeComponentStatus {
        let ready = path.is_file();
        RuntimeComponentStatus {
            kind,
            state: if ready { RuntimeComponentState::Ready } else { RuntimeComponentState::Missing },
            version: None,
            path: Some(path.to_path_buf()),
            managed: false,
            detail: Some(if ready {
                ready_detail.to_owned()
            } else {
                format!("manual runtime file is missing: {}", path.display())
            }),
        }
    }

    fn server_mods_status(&self, world: WorldId) -> RuntimeComponentStatus {
        let config = match self.storage.load_world_config(world) {
            Ok(config) => config,
            Err(_) => {
                return RuntimeComponentStatus {
                    kind: RuntimeComponentKind::ServerMods,
                    state: RuntimeComponentState::Unavailable,
                    version: None,
                    path: Some(server_mods::mods_dir(self.paths, world)),
                    managed: false,
                    detail: Some("canonical runtime compatibility manifest is not synchronized yet".into()),
                };
            }
        };
        match server_mods::evaluate_world_mods(self.paths, world, &config.compatibility) {
            Ok(readiness) => RuntimeComponentStatus {
                kind: RuntimeComponentKind::ServerMods,
                state: if readiness.ready { RuntimeComponentState::Ready } else { RuntimeComponentState::Incompatible },
                version: None,
                path: Some(readiness.mods_dir),
                managed: false,
                detail: (!readiness.ready).then(|| {
                    readiness.issues.iter().map(|issue| issue.message.as_str()).collect::<Vec<_>>().join("; ")
                }),
            },
            Err(error) => RuntimeComponentStatus {
                kind: RuntimeComponentKind::ServerMods,
                state: RuntimeComponentState::Incompatible,
                version: None,
                path: Some(server_mods::mods_dir(self.paths, world)),
                managed: false,
                detail: Some(error.to_string()),
            },
        }
    }
}

fn platform_components_ready(status: &RuntimeStatus) -> bool {
    status.components.iter().all(|component| {
        matches!(component.kind, RuntimeComponentKind::Eula) || component.state == RuntimeComponentState::Ready
    })
}

fn emit<F: FnMut(RuntimeProgress)>(
    progress: &mut F,
    completed: &mut Vec<RuntimePhase>,
    phase: RuntimePhase,
    message: impl Into<String>,
) {
    completed.push(phase);
    progress(RuntimeProgress { phase, message: message.into() });
}

fn prepare_world_directories(paths: &DataPaths, world: WorldId) -> Result<()> {
    for path in [
        managed_world_root(paths, world),
        managed_world_server_dir(paths, world),
        managed_world_mods_dir(paths, world),
        managed_world_config_dir(paths, world),
    ] {
        fs::create_dir_all(&path).with_context(|| format!("cannot create {}", path.display()))?;
    }
    Ok(())
}

fn load_runtime_lock(paths: &DataPaths, world: WorldId) -> Result<RuntimeLock> {
    let path = runtime_lock_path(paths, world);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("managed runtime lock must be a regular file");
    }
    let bytes = fs::read(&path).with_context(|| format!("runtime lock is missing at {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_runtime_segment(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || value == "."
        || value == ".."
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
    {
        bail!("unsafe runtime version component");
    }
    Ok(())
}

fn resolve_minecraft(version: &str) -> Result<(u32, ResolvedArtifact)> {
    let manifest: Value = serde_json::from_str(&curl_text(MINECRAFT_MANIFEST, &["piston-meta.mojang.com"])?)?;
    let entry = manifest["versions"]
        .as_array()
        .and_then(|values| values.iter().find(|value| value["id"].as_str() == Some(version)))
        .with_context(|| format!("Minecraft version {version} is not present in Mojang's official manifest"))?;
    let metadata_url = entry["url"].as_str().context("Mojang version entry has no metadata URL")?;
    let metadata: Value = serde_json::from_str(&curl_text(metadata_url, &["piston-meta.mojang.com"])?)?;
    let major = metadata["javaVersion"]["majorVersion"]
        .as_u64()
        .context("Mojang metadata does not declare a Java major version")? as u32;
    let server = &metadata["downloads"]["server"];
    let url = server["url"].as_str().context("Mojang metadata does not provide a server download")?;
    trusted_https(url, &["piston-data.mojang.com", "launcher.mojang.com"])?;
    let sha1 = Some(server["sha1"].as_str().context("Mojang server SHA-1 is missing")?.to_owned());
    Ok((
        major,
        ResolvedArtifact {
            version: version.to_owned(),
            source: ArtifactSource::Download(url.to_owned()),
            sha1,
            sha256: None,
        },
    ))
}

fn resolve_fabric_installer() -> Result<String> {
    let values: Value = serde_json::from_str(&curl_text(FABRIC_INSTALLERS, &["meta.fabricmc.net"])?)?;
    let installers = values.as_array().context("Fabric installer metadata is not an array")?;
    installers
        .iter()
        .find(|value| value["stable"].as_bool() == Some(true))
        .or_else(|| installers.first())
        .and_then(|value| value["version"].as_str())
        .map(ToOwned::to_owned)
        .context("Fabric metadata contains no installer version")
}

fn resolve_fabric_api(minecraft: &str) -> Result<(String, ResolvedArtifact)> {
    let xml = curl_text(FABRIC_API_METADATA, &["maven.fabricmc.net"])?;
    let versions = xml_values(&xml, "version");
    let suffix = format!("+{minecraft}");
    let version = versions
        .iter()
        .rev()
        .find(|version| version.ends_with(&suffix))
        .cloned()
        .with_context(|| format!("Fabric API has no published artifact matching Minecraft {minecraft}"))?;
    let base =
        format!("https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api/{version}/fabric-api-{version}.jar");
    let sha1 = curl_text(&format!("{base}.sha1"), &["maven.fabricmc.net"])?
        .split_whitespace()
        .next()
        .context("Fabric API SHA-1 response is empty")?
        .to_owned();
    Ok((
        version.clone(),
        ResolvedArtifact { version, source: ArtifactSource::Download(base), sha1: Some(sha1), sha256: None },
    ))
}

fn resolve_swarmcraft_fabric(version: &str) -> Result<ResolvedArtifact> {
    if let Some(path) = env::var_os("SWARMCRAFT_FABRIC_MOD_JAR").map(PathBuf::from) {
        if path.is_file() {
            return Ok(ResolvedArtifact {
                version: version.to_owned(),
                sha256: Some(hash_file(&path, HashKind::Sha256)?),
                sha1: None,
                source: ArtifactSource::Local(path),
            });
        }
    }
    let release = load_swarmcraft_release(version)?;
    let assets = release["assets"].as_array().context("SwarmCraft release has no assets")?;
    let jar_name = format!("swarmcraft-fabric-{version}.jar");
    let checksum_name = format!("{jar_name}.sha256");
    let jar_url = release_asset_url(assets, &jar_name)?;
    let checksum_url = release_asset_url(assets, &checksum_name)?;
    let sha256 = curl_text(&checksum_url, &["github.com"])?
        .split_whitespace()
        .next()
        .context("SwarmCraft Fabric checksum file is empty")?
        .to_owned();
    Ok(ResolvedArtifact {
        version: version.to_owned(),
        source: ArtifactSource::Download(jar_url),
        sha1: None,
        sha256: Some(sha256),
    })
}

fn swarmcraft_release_candidates(version: &str, preview_commit: Option<&str>) -> Vec<String> {
    if let Some(commit) = preview_commit.filter(|commit| !commit.is_empty()) {
        return vec![format!("{SWARMCRAFT_RELEASE_API}/main-snapshot-{commit}")];
    }
    vec![format!("{SWARMCRAFT_RELEASE_API}/v{version}"), format!("{SWARMCRAFT_RELEASE_API}/main-latest")]
}

fn load_swarmcraft_release(version: &str) -> Result<Value> {
    let jar_name = format!("swarmcraft-fabric-{version}.jar");
    let checksum_name = format!("{jar_name}.sha256");
    let mut failures = Vec::new();
    for api in swarmcraft_release_candidates(version, option_env!("SWARMCRAFT_PREVIEW_COMMIT")) {
        match curl_text(&api, &["api.github.com"]) {
            Ok(body) => match serde_json::from_str::<Value>(&body) {
                Ok(release) => {
                    let Some(assets) = release["assets"].as_array() else {
                        failures.push(format!("{api}: release has no assets"));
                        continue;
                    };
                    let tag = api.rsplit('/').next().context("release tag missing")?;
                    let matches_tag =
                        release["tag_name"].as_str() == Some(tag) && release["draft"].as_bool() == Some(false);
                    let exact_asset = |name: &str| {
                        release_asset_url(assets, name).is_ok_and(|url| {
                            reqwest::Url::parse(&url).ok().is_some_and(|url| {
                                url.path() == format!("/MousaXD/swarmcraft/releases/download/{tag}/{name}")
                            })
                        })
                    };
                    if matches_tag && exact_asset(&jar_name) && exact_asset(&checksum_name) {
                        return Ok(release);
                    }
                    failures.push(format!("{api}: exact {jar_name} and checksum were not both published"));
                }
                Err(error) => failures.push(format!("{api}: invalid release metadata: {error}")),
            },
            Err(error) => failures.push(format!("{api}: {error}")),
        }
    }
    bail!("no published SwarmCraft Fabric artifacts for version {version}: {}", failures.join("; "))
}

fn release_asset_url(assets: &[Value], name: &str) -> Result<String> {
    let mut matches = assets.iter().filter(|asset| asset["name"].as_str() == Some(name));
    let asset = matches.next().with_context(|| format!("release asset {name} was not published"))?;
    if matches.next().is_some() {
        bail!("release asset {name} is ambiguous");
    }
    let url = asset["browser_download_url"].as_str().context("release asset URL missing")?;
    trusted_https(url, &["github.com"])?;
    let parsed = reqwest::Url::parse(url)?;
    if !parsed.path().starts_with("/MousaXD/swarmcraft/releases/download/")
        || parsed.path_segments().and_then(Iterator::last) != Some(name)
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("release asset is not the exact official SwarmCraft artifact");
    }
    Ok(url.to_owned())
}

fn resolve_managed_java(major: u32) -> Result<ResolvedArtifact> {
    let architecture = adoptium_arch()?;
    let os = adoptium_os();
    for image in ["jre", "jdk"] {
        let url = format!(
            "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={architecture}&image_type={image}&os={os}&vendor=eclipse"
        );
        let values: Value = serde_json::from_str(&curl_text(&url, &["api.adoptium.net"])?)?;
        let Some(asset) = values.as_array().and_then(|values| values.first()) else {
            continue;
        };
        let package = &asset["binary"]["package"];
        let link = package["link"].as_str().context("Adoptium package has no download link")?;
        trusted_https(link, &["github.com", "api.adoptium.net"])?;
        let checksum = package["checksum"].as_str().context("Adoptium package has no SHA-256 checksum")?;
        return Ok(ResolvedArtifact {
            version: major.to_string(),
            source: ArtifactSource::Download(link.to_owned()),
            sha1: None,
            sha256: Some(checksum.to_owned()),
        });
    }
    bail!("Adoptium has no compatible Java {major} runtime for this platform")
}

fn artifact_verification(artifact: &ResolvedArtifact) -> String {
    match (&artifact.source, &artifact.sha256, &artifact.sha1) {
        (ArtifactSource::Local(_), _, _) => {
            "Local operator-supplied artifact; SHA-256 pins supplied bytes, not publisher provenance".into()
        }
        (_, Some(_), _) => "Official-source SHA-256 matched; local SHA-256 retained".into(),
        (_, None, Some(_)) => "Official-source SHA-1 matched; local SHA-256 retained (not a signature)".into(),
        _ => "Official generated launcher; HTTPS and local SHA-256 only, no upstream checksum or signature".into(),
    }
}

fn validate_expected_hashes(artifact: &ResolvedArtifact) -> Result<()> {
    for (value, length) in [(&artifact.sha1, 40), (&artifact.sha256, 64)] {
        if let Some(value) = value {
            if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("invalid expected artifact checksum");
            }
        }
    }
    Ok(())
}

fn verify_record_file(path: &Path, record: &ArtifactRecord) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        bail!("runtime artifact is not a nonempty regular file");
    }
    let sha256 = hash_file(path, HashKind::Sha256)?;
    if !eq_hash(&sha256, &record.sha256)
        || record.upstream_sha256.as_ref().is_some_and(|expected| !eq_hash(expected, &sha256))
    {
        bail!("runtime-lock SHA-256 verification failed");
    }
    if let Some(expected) = &record.sha1 {
        if !eq_hash(expected, &hash_file(path, HashKind::Sha1)?) {
            bail!("runtime-lock SHA-1 verification failed");
        }
    }
    Ok(())
}

/// Shared pre-execution guard used inside the world authority slot. Existing
/// manually configured worlds without a managed lock remain supported.
pub(crate) fn verify_managed_launch(
    paths: &DataPaths,
    world: WorldId,
    minecraft: &str,
    loader: &str,
    java: &Path,
    server: &Path,
    bridge: &Path,
) -> Result<()> {
    let lock = match load_runtime_lock(paths, world) {
        Ok(lock) => lock,
        Err(error)
            if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(())
        }
        Err(error) => return Err(error).context("unreadable managed runtime lock; launch denied"),
    };
    if lock.schema_version != RUNTIME_LOCK_SCHEMA_VERSION
        || lock.world_id != world.to_string()
        || lock.minecraft_version != minecraft
        || lock.fabric_loader_version != loader
        || lock.java_path != java
        || lock.artifacts.get("fabric_loader").is_none_or(|record| record.path != server)
        || lock.artifacts.get("swarmcraft_fabric").is_none_or(|record| record.path != bridge)
    {
        bail!("configured launch does not match managed runtime lock");
    }
    if lock.java_managed {
        let record =
            lock.java_provenance.as_ref().context("managed Java provenance is missing; repair before launch")?;
        if record.executable != java {
            bail!("managed Java executable differs from its provenance");
        }
        verify_java_provenance(record)?;
    }
    for (key, staged) in [
        ("minecraft_server", Some(managed_world_server_dir(paths, world).join("server.jar"))),
        ("fabric_loader", None),
        ("fabric_api", Some(managed_world_mods_dir(paths, world).join("fabric-api.jar"))),
        ("swarmcraft_fabric", Some(managed_world_mods_dir(paths, world).join("swarmcraft-fabric.jar"))),
    ] {
        let record = lock.artifacts.get(key).context("managed runtime artifact receipt missing")?;
        verify_record_file(&record.path, record)?;
        if let Some(staged) = staged {
            verify_record_file(&staged, record)?;
        }
    }
    Ok(())
}

fn verify_artifact_record(artifact: &ResolvedArtifact, record: &ArtifactRecord) -> Result<()> {
    if record.version != artifact.version
        || record.source != artifact_source_label(&artifact.source)
        || record.sha1 != artifact.sha1
        || record.upstream_sha256 != artifact.sha256
        || record.verification.as_deref() != Some(artifact_verification(artifact).as_str())
    {
        bail!("runtime artifact provenance does not match resolved source");
    }
    verify_record_file(&record.path, record)
}

fn read_cache_record<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 128 * 1024 {
                bail!("unsafe runtime cache receipt");
            }
            Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
        }
    }
}

fn check_generation_path(root: &Path, path: &Path, filename: &std::ffi::OsStr) -> Result<()> {
    let generation = path.parent().context("artifact generation missing")?;
    if generation.parent() != Some(root)
        || path.file_name() != Some(filename)
        || !generation.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("generation-"))
        || fs::symlink_metadata(generation)?.file_type().is_symlink()
    {
        bail!("runtime cache receipt points outside its generation");
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Immutable generation cache; caller holds the shared publication lock.
/// Existing world locks always retain their original paths. A successful
/// refresh updates only a small lookup receipt for future installations.
fn install_cached_artifact(artifact: &ResolvedArtifact, legacy_path: &Path, force: bool) -> Result<ArtifactRecord> {
    validate_expected_hashes(artifact)?;
    let filename = legacy_path.file_name().context("runtime artifact filename missing")?;
    let identity = serde_json::to_vec(&(
        &artifact.version,
        artifact_source_label(&artifact.source),
        &artifact.sha1,
        &artifact.sha256,
    ))?;
    let key = hex::encode(Sha256::digest(identity));
    let root = legacy_path.parent().context("runtime artifact parent missing")?.join("verified").join(key);
    swarm_core::create_private_dir_all(&root)?;
    let receipt = root.join("current.json");
    let current = read_cache_record::<ArtifactRecord>(&receipt).and_then(|record| {
        if let Some(record) = record.as_ref() {
            check_generation_path(&root, &record.path, filename)?;
            verify_artifact_record(artifact, record)?;
        }
        Ok(record)
    });
    let current = match current {
        Ok(Some(record)) if !force => return Ok(record),
        Ok(record) => record,
        Err(error) if !force => return Err(error).context("runtime cache is corrupt; explicit repair is required"),
        Err(_) => None, // explicit repair may create a new generation, never remove the old one
    };

    let stage = tempfile::Builder::new().prefix(".download-").tempdir_in(&root)?;
    let mut record = install_artifact(artifact, &stage.path().join(filename), true)?;
    if artifact.sha1.is_none() && artifact.sha256.is_none() {
        // Generated Fabric launchers have no upstream digest. Require a real
        // JAR manifest, not an HTML/JSON error page returned with HTTP 200.
        if fs::metadata(&record.path)?.len() > 32 * 1024 * 1024 {
            bail!("generated runtime launcher exceeds size limit");
        }
        let bytes = fs::read(&record.path)?;
        crate::server_mods::read_zip_entry(&bytes, "META-INF/MANIFEST.MF")
            .context("generated runtime launcher is not a valid JAR")?;
    }
    if let Some(previous) = current {
        if previous.sha256 == record.sha256 {
            return Ok(previous);
        }
    }
    let generation = root.join(format!("generation-{}-{}", record.sha256, unique_suffix()));
    record.path = generation.join(filename);
    sync_directory(stage.path())?;
    fs::rename(stage.path(), &generation)?;
    sync_directory(&root)?;
    // If publication of the lookup receipt fails, the previous receipt and
    // immutable generations survive. The new unreferenced directory is safe.
    atomic_json(&receipt, &record)?;
    Ok(record)
}

fn java_tree_hash(root: &Path) -> Result<String> {
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        bail!("managed Java root is a symlink");
    }
    let canonical = root.canonicalize()?;
    let mut digest = Sha256::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let entry = entry?;
        let path = entry.path();
        if path == root {
            continue;
        }
        let relative = path.strip_prefix(root)?.to_str().context("non-UTF8 Java entry")?;
        let metadata = fs::symlink_metadata(path)?;
        let value = if metadata.file_type().is_symlink() {
            let target = fs::read_link(path)?;
            if target.is_absolute() || !path.canonicalize()?.starts_with(&canonical) {
                bail!("managed Java contains an external or dangling symlink");
            }
            format!("link:{}", target.to_str().context("non-UTF8 Java symlink")?)
        } else if metadata.is_file() {
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o777
            };
            #[cfg(not(unix))]
            let mode = 0u32;
            format!("file:{mode}:{}", hash_file(path, HashKind::Sha256)?)
        } else if metadata.is_dir() {
            "directory".into()
        } else {
            bail!("managed Java contains a non-regular entry");
        };
        digest.update(serde_json::to_vec(&(relative, value))?);
        digest.update(b"\n");
    }
    Ok(hex::encode(digest.finalize()))
}

fn verify_java_provenance(record: &JavaProvenance) -> Result<()> {
    let relative =
        record.executable.strip_prefix(&record.root).context("Java executable outside its recorded runtime")?;
    if relative.components().any(|part| !matches!(part, std::path::Component::Normal(_)))
        || !fs::symlink_metadata(&record.executable)?.is_file()
        || !record.executable.canonicalize()?.starts_with(record.root.canonicalize()?)
        || java_tree_hash(&record.root)? != record.tree_sha256
    {
        bail!("managed Java runtime tree or executable no longer matches recorded provenance");
    }
    Ok(())
}

fn install_java_generation(
    artifact: &ResolvedArtifact,
    base: &Path,
    major: u32,
    force: bool,
) -> Result<JavaProvenance> {
    validate_expected_hashes(artifact)?;
    let expected = artifact.sha256.as_deref().context("managed Java requires an official archive SHA-256")?;
    let root = base.join("verified").join(expected.to_ascii_lowercase());
    swarm_core::create_private_dir_all(&root)?;
    let receipt = root.join("current.json");
    let current = read_cache_record::<JavaProvenance>(&receipt).and_then(|record| {
        if let Some(record) = record.as_ref() {
            if record.root.parent() != Some(root.as_path())
                || record.source != artifact_source_label(&artifact.source)
                || !eq_hash(&record.archive_sha256, expected)
            {
                bail!("managed Java receipt does not match resolved archive");
            }
            verify_java_provenance(record)?;
        }
        Ok(record)
    });
    match current {
        Ok(Some(record)) if !force => {
            if probe_java_major(&record.executable)? != major {
                bail!("managed Java cached runtime has wrong major version");
            }
            return Ok(record);
        }
        Err(error) if !force => {
            return Err(error).context("managed Java cache is corrupt; explicit repair is required")
        }
        _ => {}
    }
    let stage = tempfile::Builder::new().prefix(".java-").tempdir_in(&root)?;
    let archive = stage.path().join("java.archive");
    let archive_record = install_artifact(artifact, &archive, true)?;
    let extraction = stage.path().join("runtime");
    fs::create_dir(&extraction)?;
    let status = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&extraction)
        .status()
        .context("managed Java extraction requires the platform tar utility")?;
    if !status.success() {
        bail!("managed Java archive extraction failed");
    }
    // Validate the entire extracted tree before running the version probe.
    let tree_sha256 = java_tree_hash(&extraction)?;
    let staged_java = find_java_executable(&extraction).context("managed Java archive did not contain bin/java")?;
    let relative = staged_java.strip_prefix(&extraction)?.to_path_buf();
    if !fs::symlink_metadata(&staged_java)?.is_file()
        || !staged_java.canonicalize()?.starts_with(extraction.canonicalize()?)
    {
        bail!("managed Java executable is not an internal regular file");
    }
    if probe_java_major(&staged_java)? != major {
        bail!("downloaded Java runtime is incompatible with the selected Minecraft version");
    }
    if java_tree_hash(&extraction)? != tree_sha256 {
        bail!("Java runtime tree changed during verification");
    }
    // Sync regular files and directories before publishing the generation.
    #[cfg(unix)]
    for entry in walkdir::WalkDir::new(&extraction).follow_links(false).contents_first(true) {
        let entry = entry?;
        if entry.file_type().is_file() || entry.file_type().is_dir() {
            fs::File::open(entry.path())?.sync_all()?;
        }
    }
    let generation = root.join(format!("generation-{}", unique_suffix()));
    let record = JavaProvenance {
        executable: generation.join(relative),
        root: generation.clone(),
        source: artifact_source_label(&artifact.source),
        archive_sha256: archive_record.sha256,
        tree_sha256,
    };
    fs::rename(&extraction, &generation)?;
    sync_directory(&root)?;
    atomic_json(&receipt, &record)?;
    Ok(record)
}

fn install_artifact(artifact: &ResolvedArtifact, destination: &Path, force: bool) -> Result<ArtifactRecord> {
    validate_expected_hashes(artifact)?;
    if destination.is_file() && !force {
        let sha256 = hash_file(destination, HashKind::Sha256)?;
        let sha1_ok = artifact.sha1.as_ref().is_none_or(|expected| {
            hash_file(destination, HashKind::Sha1).is_ok_and(|actual| eq_hash(expected, &actual))
        });
        let sha256_ok = artifact.sha256.as_ref().is_none_or(|expected| eq_hash(expected, &sha256));
        if sha1_ok && sha256_ok {
            return Ok(ArtifactRecord {
                version: artifact.version.clone(),
                path: destination.to_path_buf(),
                source: artifact_source_label(&artifact.source),
                sha1: artifact.sha1.clone(),
                sha256,
                upstream_sha256: artifact.sha256.clone(),
                verification: Some(artifact_verification(artifact)),
            });
        }
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension(format!("part-{}", unique_suffix()));
    let result = match &artifact.source {
        ArtifactSource::Download(url) => curl_download(url, &temporary),
        ArtifactSource::Local(path) => fs::copy(path, &temporary).map(|_| ()).map_err(anyhow::Error::from),
    };
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let verification = (|| -> Result<String> {
        if let Some(expected) = &artifact.sha1 {
            let actual = hash_file(&temporary, HashKind::Sha1)?;
            if !eq_hash(expected, &actual) {
                bail!("downloaded artifact SHA-1 mismatch");
            }
        }
        let sha256 = hash_file(&temporary, HashKind::Sha256)?;
        if let Some(expected) = &artifact.sha256 {
            if !eq_hash(expected, &sha256) {
                bail!("downloaded artifact SHA-256 mismatch");
            }
        }
        if fs::metadata(&temporary)?.len() == 0 {
            bail!("runtime artifact download was empty");
        }
        sync_file(&temporary)?;
        Ok(sha256)
    })();
    let sha256 = match verification {
        Ok(sha256) => sha256,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    publish_replace(&temporary, destination)
        .with_context(|| format!("cannot publish downloaded artifact at {}", destination.display()))?;
    Ok(ArtifactRecord {
        version: artifact.version.clone(),
        path: destination.to_path_buf(),
        source: artifact_source_label(&artifact.source),
        sha1: artifact.sha1.clone(),
        sha256,
        upstream_sha256: artifact.sha256.clone(),
        verification: Some(artifact_verification(artifact)),
    })
}

fn artifact_source_label(source: &ArtifactSource) -> String {
    match source {
        ArtifactSource::Download(url) => url.clone(),
        ArtifactSource::Local(path) => format!("local:{}", path.display()),
    }
}

fn atomic_copy(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination.parent().context("managed artifact destination has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = destination.with_extension(format!("part-{}", unique_suffix()));
    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    sync_file(&temporary)?;
    publish_replace(&temporary, destination)?;
    Ok(())
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let parent = path.parent().context("runtime metadata path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", unique_suffix()));
    let mut file = OpenOptions::new().create_new(true).write(true).open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    publish_replace(&temporary, path)?;
    Ok(())
}

fn publish_replace(temporary: &Path, destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => bail!("unsafe runtime destination"),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    if let Err(error) = crate::authority_permit::replace_permit_atomically(temporary, destination) {
        let _ = fs::remove_file(temporary);
        return Err(error.into());
    }
    #[cfg(unix)]
    fs::File::open(destination.parent().context("runtime destination has no parent")?)?.sync_all()?;
    Ok(())
}

fn sync_file(path: &Path) -> Result<()> {
    OpenOptions::new().write(true).open(path)?.sync_all()?;
    Ok(())
}

struct InstallGuard {
    file: fs::File,
}

impl InstallGuard {
    fn acquire(path: &Path) -> Result<Self> {
        let parent = path.parent().context("runtime install lock has no parent")?;
        swarm_core::create_private_dir_all(parent)?;
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                bail!("unsafe runtime installation lock")
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let mut file = OpenOptions::new().create(true).read(true).write(true).truncate(false).open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                bail!("another runtime installation is already in progress; retry when it finishes")
            }
            Err(error) => return Err(error).context("runtime install lock I/O failed"),
        }
        file.set_len(0)?;
        writeln!(file, "pid={}", std::process::id())?;
        file.sync_all()?;
        Ok(Self { file })
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
        // Retain the lock inode: unlinking after unlock allows a third writer to bypass a second holder.
    }
}

fn curl_text(url: &str, hosts: &[&str]) -> Result<String> {
    trusted_https(url, hosts)?;
    let output = Command::new("curl")
        .args([
            "-q",
            "-fsSL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-redirs",
            "5",
            "--retry",
            "2",
            "--max-filesize",
            "16777216",
            "--retry-max-time",
            "120",
            "--connect-timeout",
            "15",
            "--max-time",
            "120",
            "-H",
        ])
        .arg(format!("User-Agent: SwarmCraft/{}", env!("CARGO_PKG_VERSION")))
        .arg("--write-out")
        .arg("\n%{url_effective}")
        .arg(url)
        .output()
        .context("automatic runtime setup requires the platform curl utility")?;
    if !output.status.success() {
        bail!("official runtime metadata request failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let text = String::from_utf8(output.stdout).context("official runtime metadata was not UTF-8")?;
    let (body, effective) = text.rsplit_once('\n').context("missing effective metadata URL")?;
    validate_effective_url(url, effective)?;
    if body.len() > 16 * 1024 * 1024 {
        bail!("official runtime metadata exceeds size limit");
    }
    Ok(body.to_owned())
}

fn curl_download(url: &str, destination: &Path) -> Result<()> {
    trusted_https(
        url,
        &[
            "piston-data.mojang.com",
            "launcher.mojang.com",
            "meta.fabricmc.net",
            "maven.fabricmc.net",
            "github.com",
            "api.adoptium.net",
        ],
    )?;
    let output = Command::new("curl")
        .args([
            "-q",
            "-fL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-redirs",
            "5",
            "--retry",
            "2",
            "--max-filesize",
            "1073741824",
            "--retry-max-time",
            "900",
            "--connect-timeout",
            "15",
            "--max-time",
            "900",
            "-H",
        ])
        .arg(format!("User-Agent: SwarmCraft/{}", env!("CARGO_PKG_VERSION")))
        .arg("-o")
        .arg(destination)
        .arg("--write-out")
        .arg("%{url_effective}")
        .arg(url)
        .output()
        .context("automatic runtime setup requires the platform curl utility")?;
    if !output.status.success() {
        bail!("official artifact download failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let effective = String::from_utf8(output.stdout).context("invalid effective artifact URL")?;
    validate_effective_url(url, &effective)?;
    Ok(())
}

fn validate_effective_url(initial: &str, effective: &str) -> Result<()> {
    let origin = reqwest::Url::parse(initial)?;
    let host = origin.host_str().context("runtime source host missing")?;
    let hosts = if matches!(host, "github.com" | "api.adoptium.net") {
        vec![
            host,
            "github.com",
            "release-assets.githubusercontent.com",
            "objects.githubusercontent.com",
            "github-releases.githubusercontent.com",
        ]
    } else {
        vec![host]
    };
    trusted_https(effective, &hosts)
}

fn trusted_https(url: &str, hosts: &[&str]) -> Result<()> {
    let parsed = reqwest::Url::parse(url).context("invalid runtime download URL")?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || !parsed.host_str().is_some_and(|host| hosts.iter().any(|allowed| host.eq_ignore_ascii_case(allowed)))
    {
        bail!("runtime download URL is not a trusted HTTPS source");
    }
    Ok(())
}

fn probe_java_major(path: &Path) -> Result<u32> {
    let output = Command::new(path)
        .arg("-version")
        .output()
        .with_context(|| format!("cannot run {} -version", path.display()))?;
    if !output.status.success() {
        bail!("Java version probe failed");
    }
    let text = format!("{}\n{}", String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
    parse_java_major(&text).context("Java version output did not contain a recognizable version")
}

fn parse_java_major(text: &str) -> Option<u32> {
    let quoted = text.split('"').nth(1)?;
    if let Some(rest) = quoted.strip_prefix("1.") {
        return rest.split('.').next()?.parse().ok();
    }
    quoted.split(['.', '-']).next()?.parse().ok()
}

fn heuristic_java_major(minecraft: &str) -> u32 {
    let parts: Vec<u32> = minecraft.split('.').filter_map(|part| part.parse().ok()).collect();
    if parts.first().copied().unwrap_or_default() >= 26 {
        25
    } else if parts.first() == Some(&1)
        && (parts.get(1).copied().unwrap_or_default() >= 21
            || (parts.get(1) == Some(&20) && parts.get(2).copied().unwrap_or_default() >= 5))
    {
        21
    } else if parts.first() == Some(&1) && parts.get(1).copied().unwrap_or_default() >= 18 {
        17
    } else if parts.first() == Some(&1) && parts.get(1) == Some(&17) {
        16
    } else {
        8
    }
}

fn java_executable_under(root: &Path) -> PathBuf {
    root.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" })
}

fn find_java_executable(root: &Path) -> Option<PathBuf> {
    let direct = java_executable_under(root);
    if direct.is_file() {
        return Some(direct);
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().ok()?.is_dir() {
            if let Some(found) = find_java_executable(&entry.path()) {
                return Some(found);
            }
        }
    }
    None
}

fn adoptium_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    }
}

fn adoptium_arch() -> Result<&'static str> {
    match env::consts::ARCH {
        "x86_64" => Ok("x64"),
        "aarch64" => Ok("aarch64"),
        "x86" => Ok("x86"),
        other => bail!("managed Java is not configured for architecture {other}"),
    }
}

fn platform_key() -> String {
    format!("{}-{}", adoptium_os(), env::consts::ARCH)
}

fn xml_values(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut values = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let value_start = start + open.len();
        let tail = &rest[value_start..];
        let Some(end) = tail.find(&close) else {
            break;
        };
        values.push(tail[..end].trim().to_owned());
        rest = &tail[end + close.len()..];
    }
    values
}

#[derive(Clone, Copy)]
enum HashKind {
    Sha1,
    Sha256,
}

fn hash_file(path: &Path, kind: HashKind) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("cannot open {} for hashing", path.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    match kind {
        HashKind::Sha1 => {
            let mut hasher = Sha1::new();
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        HashKind::Sha256 => {
            let mut hasher = Sha256::new();
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
    }
}

fn eq_hash(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_java_versions() {
        assert_eq!(parse_java_major("openjdk version \"25.0.1\" 2026-01-01"), Some(25));
        assert_eq!(parse_java_major("java version \"1.8.0_412\""), Some(8));
        assert_eq!(parse_java_major("openjdk version \"21-ea\""), Some(21));
    }

    #[test]
    fn java_heuristic_covers_current_profiles() {
        assert_eq!(heuristic_java_major("26.1.2"), 25);
        assert_eq!(heuristic_java_major("1.21.1"), 21);
        assert_eq!(heuristic_java_major("1.20.6"), 21);
        assert_eq!(heuristic_java_major("1.20.4"), 17);
        assert_eq!(heuristic_java_major("1.17.1"), 16);
    }

    #[test]
    fn versioned_release_candidates_keep_legacy_fallback() {
        assert_eq!(
            swarmcraft_release_candidates("0.4.0", None),
            [format!("{SWARMCRAFT_RELEASE_API}/v0.4.0"), format!("{SWARMCRAFT_RELEASE_API}/main-latest"),]
        );
    }

    #[test]
    fn preview_resolves_only_its_own_commit_snapshot() {
        let commit = "1234567890abcdef1234567890abcdef12345678";
        assert_eq!(
            swarmcraft_release_candidates("0.5.1", Some(commit)),
            [format!("{SWARMCRAFT_RELEASE_API}/main-snapshot-{commit}")]
        );
        assert_eq!(swarmcraft_release_candidates("0.5.1", Some("")), swarmcraft_release_candidates("0.5.1", None));
    }

    #[test]
    fn extracts_fabric_api_versions_without_xml_dependency() {
        let xml = "<metadata><versioning><versions><version>0.1+1.20.1</version><version>0.2+1.21.1</version></versions></versioning></metadata>";
        assert_eq!(xml_values(xml, "version"), vec!["0.1+1.20.1", "0.2+1.21.1"]);
    }

    #[test]
    fn rejects_untrusted_or_plain_http_sources() {
        assert!(trusted_https("http://meta.fabricmc.net/file", &["meta.fabricmc.net"]).is_err());
        assert!(trusted_https("https://evil.example/file", &["meta.fabricmc.net"]).is_err());
        assert!(trusted_https("https://meta.fabricmc.net/file", &["meta.fabricmc.net"]).is_ok());
    }

    #[test]
    fn install_lock_rejects_concurrent_writer_and_cleans_up() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.lock");
        let first = InstallGuard::acquire(&path).unwrap();
        assert!(InstallGuard::acquire(&path).is_err());
        drop(first);
        assert!(InstallGuard::acquire(&path).is_ok());
    }

    #[test]
    fn local_artifact_hash_mismatch_is_rejected_and_temp_is_cleaned() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.jar");
        let destination = temp.path().join("installed.jar");
        fs::write(&source, b"fixture").unwrap();
        let artifact = ResolvedArtifact {
            version: "test".into(),
            source: ArtifactSource::Local(source),
            sha1: None,
            sha256: Some("00".repeat(32)),
        };
        assert!(install_artifact(&artifact, &destination, false).is_err());
        assert!(!destination.exists());
        assert_eq!(
            fs::read_dir(temp.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().to_string_lossy().contains("part-"))
                .count(),
            0
        );
    }

    #[test]
    fn atomic_local_install_is_retry_safe() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.jar");
        let destination = temp.path().join("installed.jar");
        fs::write(&source, b"fixture").unwrap();
        let hash = hash_file(&source, HashKind::Sha256).unwrap();
        let artifact = ResolvedArtifact {
            version: "test".into(),
            source: ArtifactSource::Local(source),
            sha1: None,
            sha256: Some(hash.clone()),
        };
        let first = install_artifact(&artifact, &destination, false).unwrap();
        let second = install_artifact(&artifact, &destination, false).unwrap();
        assert_eq!(first.sha256, hash);
        assert_eq!(second.sha256, hash);
    }

    fn local_artifact(source: &Path, bytes: &[u8]) -> ResolvedArtifact {
        fs::write(source, bytes).unwrap();
        ResolvedArtifact {
            version: "fixture".into(),
            source: ArtifactSource::Local(source.to_path_buf()),
            sha1: None,
            sha256: Some(hex::encode(Sha256::digest(bytes))),
        }
    }

    #[test]
    fn cache_outage_failed_refresh_and_new_generation_preserve_old_paths() {
        let temp = tempfile::tempdir().unwrap();
        let lock = temp.path().join("publication.lock");
        let _guard = InstallGuard::acquire(&lock).unwrap();
        let legacy = temp.path().join("shared/server.jar");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, b"legacy referenced by an older world").unwrap();
        let source = temp.path().join("source.jar");
        let artifact = local_artifact(&source, b"known good");
        let first = install_cached_artifact(&artifact, &legacy, false).unwrap();
        fs::remove_file(&source).unwrap();
        let second_world = install_cached_artifact(&artifact, &legacy, false).unwrap();
        assert_eq!(first, second_world);
        assert!(install_cached_artifact(&artifact, &legacy, true).is_err());
        assert_eq!(fs::read(&first.path).unwrap(), b"known good");
        assert_eq!(fs::read(&legacy).unwrap(), b"legacy referenced by an older world");
        fs::write(&source, b"corrupted bytes during refresh").unwrap();
        assert!(install_cached_artifact(&artifact, &legacy, true).is_err());
        assert_eq!(install_cached_artifact(&artifact, &legacy, false).unwrap(), first);
        let changed = local_artifact(&source, b"new verified version of same runtime");
        let next = install_cached_artifact(&changed, &legacy, true).unwrap();
        assert_ne!(next.path, first.path);
        assert_eq!(fs::read(&first.path).unwrap(), b"known good");
        assert_eq!(fs::read(&next.path).unwrap(), b"new verified version of same runtime");
    }

    #[test]
    fn cache_corruption_refuses_reuse_and_repair_keeps_old_generation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.jar");
        let artifact = local_artifact(&source, b"known good");
        let legacy = temp.path().join("shared/server.jar");
        let old = install_cached_artifact(&artifact, &legacy, false).unwrap();
        fs::write(&old.path, b"tampered").unwrap();
        assert!(install_cached_artifact(&artifact, &legacy, false).is_err());
        let new = install_cached_artifact(&artifact, &legacy, true).unwrap();
        assert_ne!(old.path, new.path);
        assert_eq!(fs::read(&old.path).unwrap(), b"tampered");
        assert_eq!(fs::read(&new.path).unwrap(), b"known good");
        assert_eq!(install_cached_artifact(&artifact, &legacy, false).unwrap(), new);
    }

    #[test]
    fn unpinned_generated_launcher_never_adopts_unreceipted_cache_or_error_page() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("shared/fabric-server-launch.jar");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, b"unreceipted old bytes").unwrap();
        let source = temp.path().join("response.jar");
        fs::write(&source, b"<html>504 upstream outage</html>").unwrap();
        let artifact = ResolvedArtifact {
            version: "fixture".into(),
            source: ArtifactSource::Local(source),
            sha1: None,
            sha256: None,
        };
        assert!(install_cached_artifact(&artifact, &legacy, false).is_err());
        assert_eq!(fs::read(&legacy).unwrap(), b"unreceipted old bytes");
    }

    #[test]
    fn shared_publication_lock_retains_same_inode_across_worlds() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("runtimes/publication.lock");
        let first_world = InstallGuard::acquire(&temp.path().join("world-a/install.lock")).unwrap();
        let second_world = InstallGuard::acquire(&temp.path().join("world-b/install.lock")).unwrap();
        let first = InstallGuard::acquire(&cache).unwrap();
        assert!(InstallGuard::acquire(&cache).is_err());
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&cache).unwrap().ino()
        };
        drop(first);
        assert!(cache.is_file(), "never unlink an OS-backed lock");
        let second = InstallGuard::acquire(&cache).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(&cache).unwrap().ino(), inode);
        }
        assert!(InstallGuard::acquire(&cache).is_err());
        drop((first_world, second_world, second));
    }

    #[test]
    fn source_validation_rejects_unexpected_hosts_credentials_and_release_substitution() {
        assert!(trusted_https("https://meta.fabricmc.net:8443/x", &["meta.fabricmc.net"]).is_err());
        assert!(trusted_https("https://user@meta.fabricmc.net/x", &["meta.fabricmc.net"]).is_err());
        assert!(validate_effective_url("https://meta.fabricmc.net/source", "https://evil.example/file").is_err());
        assert!(validate_effective_url(
            "https://github.com/file",
            "https://release-assets.githubusercontent.com/file?sig=test"
        )
        .is_ok());
        assert!(validate_effective_url("https://github.com/file", "http://release-assets.githubusercontent.com/file")
            .is_err());
        let name = "swarmcraft-fabric-0.5.1.jar";
        let wrong = serde_json::json!({"name": name, "browser_download_url": format!("https://github.com/other/repo/releases/download/v0.5.1/{name}")});
        assert!(release_asset_url(&[wrong], name).is_err());
        let right = serde_json::json!({"name": name, "browser_download_url": format!("https://github.com/MousaXD/swarmcraft/releases/download/v0.5.1/{name}")});
        assert!(release_asset_url(&[right.clone(), right], name).is_err());
        assert!(validate_runtime_segment("../escape").is_err());
        assert!(validate_runtime_segment("0.155.2+26.1.2").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn java_generation_hashes_complete_tree_and_preserves_previous_runtime() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture");
        fs::create_dir_all(fixture.join("jdk/bin")).unwrap();
        fs::create_dir_all(fixture.join("jdk/lib")).unwrap();
        let executable = fixture.join("jdk/bin/java");
        fs::write(&executable, b"#!/bin/sh\nprintf 'openjdk version \"25.0.1\"\n' >&2\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(fixture.join("jdk/lib/library"), b"known native library").unwrap();
        symlink("../lib/library", fixture.join("jdk/bin/internal-link")).unwrap();
        let archive = temp.path().join("fixture.tar");
        assert!(Command::new("tar")
            .args(["-cf"])
            .arg(&archive)
            .arg("-C")
            .arg(&fixture)
            .arg("jdk")
            .status()
            .unwrap()
            .success());
        let bytes = fs::read(&archive).unwrap();
        let artifact = local_artifact(&archive, &bytes);
        let base = temp.path().join("java");
        let first = install_java_generation(&artifact, &base, 25, false).unwrap();
        assert!(verify_java_provenance(&first).is_ok());
        fs::remove_file(&archive).unwrap();
        assert_eq!(install_java_generation(&artifact, &base, 25, false).unwrap(), first);
        assert!(install_java_generation(&artifact, &base, 25, true).is_err());
        assert!(verify_java_provenance(&first).is_ok());
        fs::write(first.root.join("jdk/lib/library"), b"changed library").unwrap();
        assert!(verify_java_provenance(&first).is_err());
        assert!(install_java_generation(&artifact, &base, 25, false).is_err());
        fs::write(&archive, bytes).unwrap();
        let repaired = install_java_generation(&artifact, &base, 25, true).unwrap();
        assert_ne!(repaired.root, first.root);
        assert!(verify_java_provenance(&repaired).is_ok());
        assert!(verify_java_provenance(&first).is_err());
        symlink(temp.path(), repaired.root.join("outside")).unwrap();
        assert!(verify_java_provenance(&repaired).is_err());
    }
}
