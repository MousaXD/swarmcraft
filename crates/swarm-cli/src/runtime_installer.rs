use crate::{
    migration::{load_runtime_config, save_runtime_config, RuntimeLaunchConfig},
    runtime_layout::{
        managed_fabric_api, managed_fabric_server, managed_java_root, managed_minecraft_server,
        managed_swarmcraft_fabric, managed_world_config_dir, managed_world_mods_dir,
        managed_world_root, managed_world_server_dir, runtime_install_lock_path, runtime_lock_path,
        RUNTIME_LOCK_SCHEMA_VERSION,
    },
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::DataPaths;
use swarm_protocol::WorldId;
use swarm_storage::Storage;

const MINECRAFT_MANIFEST: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const FABRIC_INSTALLERS: &str = "https://meta.fabricmc.net/v2/versions/installer";
const FABRIC_API_METADATA: &str =
    "https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api/maven-metadata.xml";
const SWARMCRAFT_RELEASE_API: &str =
    "https://api.github.com/repos/MousaXD/swarmcraft/releases/tags";

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
    artifacts: BTreeMap<String, ArtifactRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ArtifactRecord {
    version: String,
    path: PathBuf,
    source: String,
    sha1: Option<String>,
    sha256: String,
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
        let lock = load_runtime_lock(self.paths, world).ok();
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

        let mut components = vec![
            self.java_status(
                required_java_major,
                lock.as_ref(),
                manual.as_ref(),
                lock_compatible,
            ),
            self.artifact_status(
                RuntimeComponentKind::MinecraftServer,
                "minecraft_server",
                lock.as_ref(),
                lock_compatible,
                Some(managed_world_server_dir(self.paths, world).join("server.jar")),
            ),
            self.artifact_status(
                RuntimeComponentKind::FabricLoader,
                "fabric_loader",
                lock.as_ref(),
                lock_compatible,
                None,
            ),
            self.artifact_status(
                RuntimeComponentKind::FabricApi,
                "fabric_api",
                lock.as_ref(),
                lock_compatible,
                Some(managed_world_mods_dir(self.paths, world).join("fabric-api.jar")),
            ),
            self.swarmcraft_status(world, lock.as_ref(), lock_compatible),
        ];

        let directories_ready = [
            managed_world_root(self.paths, world),
            managed_world_server_dir(self.paths, world),
            managed_world_mods_dir(self.paths, world),
            managed_world_config_dir(self.paths, world),
        ]
        .iter()
        .all(|path| path.is_dir());
        components.push(RuntimeComponentStatus {
            kind: RuntimeComponentKind::ServerDirectories,
            state: if directories_ready {
                RuntimeComponentState::Ready
            } else {
                RuntimeComponentState::Missing
            },
            version: None,
            path: Some(managed_world_root(self.paths, world)),
            managed: true,
            detail: None,
        });

        let eula_accepted = manual.as_ref().is_some_and(|config| config.accept_eula);
        components.push(RuntimeComponentStatus {
            kind: RuntimeComponentKind::Eula,
            state: if eula_accepted {
                RuntimeComponentState::Ready
            } else {
                RuntimeComponentState::Required
            },
            version: None,
            path: None,
            managed: true,
            detail: (!eula_accepted)
                .then(|| "explicit Minecraft server EULA acceptance is required before launch".into()),
        });
        components.push(self.server_mods_status(world));

        let ready = components
            .iter()
            .all(|component| component.state == RuntimeComponentState::Ready);
        Ok(RuntimeStatus {
            world_id: world.to_string(),
            minecraft_version: metadata.genesis.minecraft_version.clone(),
            fabric_loader_version: metadata.genesis.fabric_loader_version.clone(),
            required_java_major,
            ready,
            eula_accepted,
            launch_configured: manual.is_some(),
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
            return Ok(RuntimePlan {
                world_id: world.to_string(),
                ready: true,
                actions: Vec::new(),
            });
        }

        let mut actions = Vec::new();
        for component in &status.components {
            if component.state == RuntimeComponentState::Ready {
                continue;
            }
            let (phase, network, eula) = match component.kind {
                RuntimeComponentKind::Java => (RuntimePhase::DownloadingJava, true, false),
                RuntimeComponentKind::MinecraftServer => {
                    (RuntimePhase::DownloadingServer, true, false)
                }
                RuntimeComponentKind::FabricLoader => {
                    (RuntimePhase::InstallingFabric, true, false)
                }
                RuntimeComponentKind::FabricApi => {
                    (RuntimePhase::InstallingFabricApi, true, false)
                }
                RuntimeComponentKind::SwarmcraftFabric => {
                    (RuntimePhase::InstallingSwarmcraftMod, true, false)
                }
                RuntimeComponentKind::ServerDirectories => {
                    (RuntimePhase::PreparingDirectories, false, false)
                }
                RuntimeComponentKind::Eula => (RuntimePhase::WaitingForEula, false, true),
                RuntimeComponentKind::ServerMods => (RuntimePhase::Verifying, false, false),
            };
            actions.push(RuntimePlanAction {
                phase,
                component: Some(component.kind),
                description: component
                    .detail
                    .clone()
                    .unwrap_or_else(|| format!("prepare {:?}", component.kind)),
                requires_network: network,
                requires_eula_acceptance: eula,
            });
        }

        if actions.iter().any(|action| action.requires_network) {
            self.resolve_runtime(world)
                .context("automatic runtime sources could not be resolved")?;
        }
        Ok(RuntimePlan {
            world_id: world.to_string(),
            ready: false,
            actions,
        })
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
        let mut completed = Vec::new();
        emit(
            &mut progress,
            &mut completed,
            RuntimePhase::Checking,
            "Checking local runtime",
        );

        let initial = self.inspect(world)?;
        if !force && platform_components_ready(&initial) {
            if options.accept_eula && !initial.eula_accepted {
                let lock = load_runtime_lock(self.paths, world)?;
                self.save_launch_config(world, &lock, &options)?;
            }
            emit(
                &mut progress,
                &mut completed,
                RuntimePhase::Verifying,
                "Verifying prepared runtime",
            );
            let status = self.inspect(world)?;
            if status.ready {
                emit(
                    &mut progress,
                    &mut completed,
                    RuntimePhase::Ready,
                    "Runtime is ready",
                );
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

        let resolved = self.resolve_runtime(world)?;
        prepare_world_directories(self.paths, world)?;
        emit(
            &mut progress,
            &mut completed,
            RuntimePhase::PreparingDirectories,
            "Prepared managed server directories",
        );

        let java_path = if let Ok(major) = probe_java_major(Path::new("java")) {
            if major == resolved.required_java_major {
                PathBuf::from("java")
            } else {
                self.install_java(&resolved, force, &mut progress, &mut completed)?
            }
        } else {
            self.install_java(&resolved, force, &mut progress, &mut completed)?
        };

        emit(
            &mut progress,
            &mut completed,
            RuntimePhase::DownloadingServer,
            "Preparing Minecraft server",
        );
        let minecraft_path =
            managed_minecraft_server(self.paths, &initial.minecraft_version);
        let minecraft_record =
            install_artifact(&resolved.minecraft_server, &minecraft_path, force)?;
        atomic_copy(
            &minecraft_path,
            &managed_world_server_dir(self.paths, world).join("server.jar"),
        )?;

        emit(
            &mut progress,
            &mut completed,
            RuntimePhase::InstallingFabric,
            "Preparing Fabric Loader server launcher",
        );
        let fabric_path = managed_fabric_server(
            self.paths,
            &initial.minecraft_version,
            &initial.fabric_loader_version,
        );
        let fabric_record = install_artifact(&resolved.fabric_server, &fabric_path, force)?;

        emit(
            &mut progress,
            &mut completed,
            RuntimePhase::InstallingFabricApi,
            "Installing Fabric API",
        );
        let fabric_api_path = managed_fabric_api(self.paths, &resolved.fabric_api_version);
        let fabric_api_record = install_artifact(&resolved.fabric_api, &fabric_api_path, force)?;
        atomic_copy(
            &fabric_api_path,
            &managed_world_mods_dir(self.paths, world).join("fabric-api.jar"),
        )?;

        emit(
            &mut progress,
            &mut completed,
            RuntimePhase::InstallingSwarmcraftMod,
            "Installing SwarmCraft Fabric integration",
        );
        let swarmcraft_path = managed_swarmcraft_fabric(self.paths, &resolved.adapter_version);
        let swarmcraft_record =
            install_artifact(&resolved.swarmcraft_fabric, &swarmcraft_path, force)?;
        atomic_copy(
            &swarmcraft_path,
            &managed_world_mods_dir(self.paths, world).join("swarmcraft-fabric.jar"),
        )?;

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

        emit(
            &mut progress,
            &mut completed,
            RuntimePhase::Verifying,
            "Verifying installed runtime",
        );
        let status = self.inspect(world)?;
        if status.ready {
            emit(
                &mut progress,
                &mut completed,
                RuntimePhase::Ready,
                "Runtime is ready",
            );
        }
        Ok(RuntimeInstallReport {
            launch_config_saved: status.launch_configured,
            status,
            completed_phases: completed,
        })
    }

    fn install_java<F: FnMut(RuntimeProgress)>(
        &self,
        resolved: &ResolvedRuntime,
        force: bool,
        progress: &mut F,
        completed: &mut Vec<RuntimePhase>,
    ) -> Result<PathBuf> {
        emit(
            progress,
            completed,
            RuntimePhase::DownloadingJava,
            "Preparing managed Java runtime",
        );
        let artifact = resolved
            .java
            .as_ref()
            .context("managed Java source was not resolved")?;
        let target_root =
            managed_java_root(self.paths, resolved.required_java_major).join(platform_key());
        let java = java_executable_under(&target_root);
        if java.exists()
            && !force
            && probe_java_major(&java).ok() == Some(resolved.required_java_major)
        {
            return Ok(java);
        }
        if target_root.exists() {
            fs::remove_dir_all(&target_root)
                .with_context(|| format!("cannot replace {}", target_root.display()))?;
        }
        let parent = target_root
            .parent()
            .context("managed Java directory has no parent")?;
        fs::create_dir_all(parent)?;
        let archive = parent.join(format!("java-{}.archive", unique_suffix()));
        let archive_record = install_artifact(artifact, &archive, true)?;
        if let Some(expected) = &artifact.sha256 {
            if !eq_hash(expected, &archive_record.sha256) {
                bail!("managed Java archive hash mismatch");
            }
        }
        let staging = parent.join(format!("extract-{}", unique_suffix()));
        fs::create_dir_all(&staging)?;
        let status = Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&staging)
            .status()
            .context("managed Java extraction requires the platform tar utility")?;
        let _ = fs::remove_file(&archive);
        if !status.success() {
            let _ = fs::remove_dir_all(&staging);
            bail!("managed Java archive extraction failed");
        }
        let staged_java = find_java_executable(&staging)
            .context("managed Java archive did not contain bin/java")?;
        let relative = staged_java.strip_prefix(&staging)?.to_path_buf();
        if probe_java_major(&staged_java)? != resolved.required_java_major {
            let _ = fs::remove_dir_all(&staging);
            bail!("downloaded Java runtime is incompatible with the selected Minecraft version");
        }
        fs::rename(&staging, &target_root).with_context(|| {
            format!(
                "cannot atomically publish managed Java at {}",
                target_root.display()
            )
        })?;
        Ok(target_root.join(relative))
    }

    fn save_launch_config(
        &self,
        world: WorldId,
        lock: &RuntimeLock,
        options: &RuntimeInstallOptions,
    ) -> Result<()> {
        if !options.accept_eula {
            bail!("explicit Minecraft server EULA acceptance is required before runtime configuration");
        }
        let server = lock
            .artifacts
            .get("fabric_loader")
            .context("Fabric launcher is missing from runtime lock")?;
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

        let (required_java_major, minecraft_server) =
            resolve_minecraft(&metadata.genesis.minecraft_version)?;
        let fabric_installer_version = resolve_fabric_installer()?;
        let fabric_server = ResolvedArtifact {
            version: metadata.genesis.fabric_loader_version.clone(),
            source: ArtifactSource::Download(format!(
                "https://meta.fabricmc.net/v2/versions/loader/{}/{}/{}/server/jar",
                metadata.genesis.minecraft_version,
                metadata.genesis.fabric_loader_version,
                fabric_installer_version
            )),
            sha1: None,
            sha256: None,
        };
        let (fabric_api_version, fabric_api) =
            resolve_fabric_api(&metadata.genesis.minecraft_version)?;
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
        let probe = probe_java_major(&path);
        let state = match probe {
            Ok(major) if major == required && (lock.is_none() || lock_compatible) => {
                RuntimeComponentState::Ready
            }
            Ok(_) => RuntimeComponentState::Incompatible,
            Err(_) => RuntimeComponentState::Missing,
        };
        RuntimeComponentStatus {
            kind: RuntimeComponentKind::Java,
            state,
            version: probe.ok().map(|major| major.to_string()),
            path: Some(path),
            managed,
            detail: (state != RuntimeComponentState::Ready)
                .then(|| format!("Minecraft requires Java {required}")),
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
                detail: Some(
                    "installed artifact belongs to a different world runtime profile".into(),
                ),
            };
        }
        let source_ok = record.path.is_file()
            && hash_file(&record.path, HashKind::Sha256)
                .is_ok_and(|hash| eq_hash(&hash, &record.sha256));
        let staged_ok = staged.as_ref().is_none_or(|path| {
            path.is_file()
                && hash_file(path, HashKind::Sha256)
                    .is_ok_and(|hash| eq_hash(&hash, &record.sha256))
        });
        RuntimeComponentStatus {
            kind,
            state: if source_ok && staged_ok {
                RuntimeComponentState::Ready
            } else {
                RuntimeComponentState::Corrupt
            },
            version: Some(record.version.clone()),
            path: Some(record.path.clone()),
            managed: true,
            detail: (!(source_ok && staged_ok))
                .then(|| "artifact is missing or its recorded SHA-256 no longer matches".into()),
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

    fn server_mods_status(&self, world: WorldId) -> RuntimeComponentStatus {
        let config = match self.storage.load_world_config(world) {
            Ok(config) => config,
            Err(_) => {
                return RuntimeComponentStatus {
                    kind: RuntimeComponentKind::ServerMods,
                    state: RuntimeComponentState::Unavailable,
                    version: None,
                    path: Some(managed_world_mods_dir(self.paths, world)),
                    managed: false,
                    detail: Some(
                        "canonical runtime compatibility manifest is not synchronized yet".into(),
                    ),
                };
            }
        };
        let external: Vec<_> = config
            .compatibility
            .required_server_mods
            .iter()
            .filter(|requirement| {
                !matches!(
                    requirement.artifact_id.as_str(),
                    "swarmcraft.legacy-compatibility"
                        | "fabric-api"
                        | "fabric_api"
                        | "swarmcraft"
                        | "swarmcraft-fabric"
                )
            })
            .map(|requirement| format!("{} {}", requirement.artifact_id, requirement.version))
            .collect();
        RuntimeComponentStatus {
            kind: RuntimeComponentKind::ServerMods,
            state: if external.is_empty() {
                RuntimeComponentState::Ready
            } else {
                RuntimeComponentState::Unavailable
            },
            version: None,
            path: Some(managed_world_mods_dir(self.paths, world)),
            managed: false,
            detail: (!external.is_empty()).then(|| {
                format!(
                    "required user server mods need the server-mod manager and are not auto-downloaded: {}",
                    external.join(", ")
                )
            }),
        }
    }
}

fn platform_components_ready(status: &RuntimeStatus) -> bool {
    status.components.iter().all(|component| {
        matches!(component.kind, RuntimeComponentKind::Eula)
            || component.state == RuntimeComponentState::Ready
    })
}

fn emit<F: FnMut(RuntimeProgress)>(
    progress: &mut F,
    completed: &mut Vec<RuntimePhase>,
    phase: RuntimePhase,
    message: impl Into<String>,
) {
    completed.push(phase);
    progress(RuntimeProgress {
        phase,
        message: message.into(),
    });
}

fn prepare_world_directories(paths: &DataPaths, world: WorldId) -> Result<()> {
    for path in [
        managed_world_root(paths, world),
        managed_world_server_dir(paths, world),
        managed_world_mods_dir(paths, world),
        managed_world_config_dir(paths, world),
    ] {
        fs::create_dir_all(&path)
            .with_context(|| format!("cannot create {}", path.display()))?;
    }
    Ok(())
}

fn load_runtime_lock(paths: &DataPaths, world: WorldId) -> Result<RuntimeLock> {
    let path = runtime_lock_path(paths, world);
    let bytes = fs::read(&path)
        .with_context(|| format!("runtime lock is missing at {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn resolve_minecraft(version: &str) -> Result<(u32, ResolvedArtifact)> {
    let manifest: Value = serde_json::from_str(&curl_text(
        MINECRAFT_MANIFEST,
        &["piston-meta.mojang.com"],
    )?)?;
    let entry = manifest["versions"]
        .as_array()
        .and_then(|values| {
            values
                .iter()
                .find(|value| value["id"].as_str() == Some(version))
        })
        .with_context(|| {
            format!("Minecraft version {version} is not present in Mojang's official manifest")
        })?;
    let metadata_url = entry["url"]
        .as_str()
        .context("Mojang version entry has no metadata URL")?;
    let metadata: Value = serde_json::from_str(&curl_text(
        metadata_url,
        &["piston-meta.mojang.com"],
    )?)?;
    let major = metadata["javaVersion"]["majorVersion"]
        .as_u64()
        .context("Mojang metadata does not declare a Java major version")?
        as u32;
    let server = &metadata["downloads"]["server"];
    let url = server["url"]
        .as_str()
        .context("Mojang metadata does not provide a server download")?;
    trusted_https(url, &["piston-data.mojang.com", "launcher.mojang.com"])?;
    let sha1 = server["sha1"].as_str().map(ToOwned::to_owned);
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
    let values: Value = serde_json::from_str(&curl_text(
        FABRIC_INSTALLERS,
        &["meta.fabricmc.net"],
    )?)?;
    let installers = values
        .as_array()
        .context("Fabric installer metadata is not an array")?;
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
        .with_context(|| {
            format!("Fabric API has no published artifact matching Minecraft {minecraft}")
        })?;
    let base = format!(
        "https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api/{version}/fabric-api-{version}.jar"
    );
    let sha1 = curl_text(&format!("{base}.sha1"), &["maven.fabricmc.net"])?
        .split_whitespace()
        .next()
        .context("Fabric API SHA-1 response is empty")?
        .to_owned();
    Ok((
        version.clone(),
        ResolvedArtifact {
            version,
            source: ArtifactSource::Download(base),
            sha1: Some(sha1),
            sha256: None,
        },
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
    let api = format!("{SWARMCRAFT_RELEASE_API}/v{version}");
    let release: Value = serde_json::from_str(&curl_text(&api, &["api.github.com"])?)?;
    let assets = release["assets"]
        .as_array()
        .context("SwarmCraft release has no assets")?;
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

fn release_asset_url(assets: &[Value], name: &str) -> Result<String> {
    let url = assets
        .iter()
        .find(|asset| asset["name"].as_str() == Some(name))
        .and_then(|asset| asset["browser_download_url"].as_str())
        .with_context(|| format!("release asset {name} was not published"))?;
    trusted_https(url, &["github.com"])?;
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
        let link = package["link"]
            .as_str()
            .context("Adoptium package has no download link")?;
        trusted_https(link, &["github.com", "api.adoptium.net"])?;
        let checksum = package["checksum"]
            .as_str()
            .context("Adoptium package has no SHA-256 checksum")?;
        return Ok(ResolvedArtifact {
            version: major.to_string(),
            source: ArtifactSource::Download(link.to_owned()),
            sha1: None,
            sha256: Some(checksum.to_owned()),
        });
    }
    bail!("Adoptium has no compatible Java {major} runtime for this platform")
}

fn install_artifact(
    artifact: &ResolvedArtifact,
    destination: &Path,
    force: bool,
) -> Result<ArtifactRecord> {
    if destination.is_file() && !force {
        let sha256 = hash_file(destination, HashKind::Sha256)?;
        let sha1_ok = artifact.sha1.as_ref().is_none_or(|expected| {
            hash_file(destination, HashKind::Sha1)
                .is_ok_and(|actual| eq_hash(expected, &actual))
        });
        let sha256_ok = artifact
            .sha256
            .as_ref()
            .is_none_or(|expected| eq_hash(expected, &sha256));
        if sha1_ok && sha256_ok {
            return Ok(ArtifactRecord {
                version: artifact.version.clone(),
                path: destination.to_path_buf(),
                source: artifact_source_label(&artifact.source),
                sha1: artifact.sha1.clone(),
                sha256,
            });
        }
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension(format!("part-{}", unique_suffix()));
    let result = match &artifact.source {
        ArtifactSource::Download(url) => curl_download(url, &temporary),
        ArtifactSource::Local(path) => fs::copy(path, &temporary)
            .map(|_| ())
            .map_err(anyhow::Error::from),
    };
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Some(expected) = &artifact.sha1 {
        let actual = hash_file(&temporary, HashKind::Sha1)?;
        if !eq_hash(expected, &actual) {
            let _ = fs::remove_file(&temporary);
            bail!("downloaded artifact SHA-1 mismatch");
        }
    }
    let sha256 = hash_file(&temporary, HashKind::Sha256)?;
    if let Some(expected) = &artifact.sha256 {
        if !eq_hash(expected, &sha256) {
            let _ = fs::remove_file(&temporary);
            bail!("downloaded artifact SHA-256 mismatch");
        }
    }
    sync_file(&temporary)?;
    if destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("cannot replace {}", destination.display()))?;
    }
    fs::rename(&temporary, destination).with_context(|| {
        format!(
            "cannot publish downloaded artifact at {}",
            destination.display()
        )
    })?;
    Ok(ArtifactRecord {
        version: artifact.version.clone(),
        path: destination.to_path_buf(),
        source: artifact_source_label(&artifact.source),
        sha1: artifact.sha1.clone(),
        sha256,
    })
}

fn artifact_source_label(source: &ArtifactSource) -> String {
    match source {
        ArtifactSource::Download(url) => url.clone(),
        ArtifactSource::Local(path) => format!("local:{}", path.display()),
    }
}

fn atomic_copy(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("managed artifact destination has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = destination.with_extension(format!("part-{}", unique_suffix()));
    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    sync_file(&temporary)?;
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(&temporary, destination)?;
    Ok(())
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let parent = path
        .parent()
        .context("runtime metadata path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", unique_suffix()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

fn sync_file(path: &Path) -> Result<()> {
    OpenOptions::new().read(true).open(path)?.sync_all()?;
    Ok(())
}

struct InstallGuard {
    path: PathBuf,
}

impl InstallGuard {
    fn acquire(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .context("runtime install lock has no parent")?;
        fs::create_dir_all(parent)?;
        match OpenOptions::new().create_new(true).write(true).open(path) {
            Ok(mut file) => {
                writeln!(file, "pid={}", std::process::id())?;
                file.sync_all()?;
                Ok(Self {
                    path: path.to_path_buf(),
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                bail!("another runtime installation is already in progress for this world")
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn curl_text(url: &str, hosts: &[&str]) -> Result<String> {
    trusted_https(url, hosts)?;
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--retry",
            "2",
            "--connect-timeout",
            "15",
            "--max-time",
            "120",
            "-H",
        ])
        .arg(format!(
            "User-Agent: SwarmCraft/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .arg(url)
        .output()
        .context("automatic runtime setup requires the platform curl utility")?;
    if !output.status.success() {
        bail!(
            "official runtime metadata request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("official runtime metadata was not UTF-8")
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
            "-fL",
            "--retry",
            "2",
            "--connect-timeout",
            "15",
            "--max-time",
            "900",
            "-H",
        ])
        .arg(format!(
            "User-Agent: SwarmCraft/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .arg("-o")
        .arg(destination)
        .arg(url)
        .output()
        .context("automatic runtime setup requires the platform curl utility")?;
    if !output.status.success() {
        bail!(
            "official artifact download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn trusted_https(url: &str, hosts: &[&str]) -> Result<()> {
    let rest = url
        .strip_prefix("https://")
        .context("runtime downloads must use HTTPS")?;
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    if hosts
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
    {
        Ok(())
    } else {
        bail!("runtime download host is not trusted: {host}")
    }
}

fn probe_java_major(path: &Path) -> Result<u32> {
    let output = Command::new(path)
        .arg("-version")
        .output()
        .with_context(|| format!("cannot run {} -version", path.display()))?;
    if !output.status.success() {
        bail!("Java version probe failed");
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
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
    let parts: Vec<u32> = minecraft
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect();
    if parts.first().copied().unwrap_or_default() >= 26 {
        25
    } else if parts.first() == Some(&1)
        && (parts.get(1).copied().unwrap_or_default() >= 21
            || (parts.get(1) == Some(&20)
                && parts.get(2).copied().unwrap_or_default() >= 5))
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
    root.join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" })
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
    #[cfg(windows)]
    {
        let algorithm = match kind {
            HashKind::Sha1 => "SHA1",
            HashKind::Sha256 => "SHA256",
        };
        let output = Command::new("certutil")
            .arg("-hashfile")
            .arg(path)
            .arg(algorithm)
            .output()?;
        if !output.status.success() {
            bail!("certutil failed to hash {}", path.display());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        parse_hash_output(&text).context("certutil returned no digest")
    }
    #[cfg(not(windows))]
    {
        let bits = match kind {
            HashKind::Sha1 => "1",
            HashKind::Sha256 => "256",
        };
        let shasum = Command::new("shasum")
            .arg("-a")
            .arg(bits)
            .arg(path)
            .output();
        if let Ok(output) = shasum {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .next()
                    .map(ToOwned::to_owned)
                    .context("shasum returned no digest");
            }
        }
        let program = match kind {
            HashKind::Sha1 => "sha1sum",
            HashKind::Sha256 => "sha256sum",
        };
        let output = Command::new(program)
            .arg(path)
            .output()
            .with_context(|| {
                format!(
                    "no SHA utility is available to verify {}",
                    path.display()
                )
            })?;
        if !output.status.success() {
            bail!("{program} failed to hash {}", path.display());
        }
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .map(ToOwned::to_owned)
            .context("hash utility returned no digest")
    }
}

#[cfg(windows)]
fn parse_hash_output(text: &str) -> Option<String> {
    text.lines()
        .map(|line| {
            line.chars()
                .filter(|character| character.is_ascii_hexdigit())
                .collect::<String>()
        })
        .find(|line| line.len() == 40 || line.len() == 64)
}

fn eq_hash(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_java_versions() {
        assert_eq!(
            parse_java_major("openjdk version \"25.0.1\" 2026-01-01"),
            Some(25)
        );
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
    fn extracts_fabric_api_versions_without_xml_dependency() {
        let xml = "<metadata><versioning><versions><version>0.1+1.20.1</version><version>0.2+1.21.1</version></versions></versioning></metadata>";
        assert_eq!(
            xml_values(xml, "version"),
            vec!["0.1+1.20.1", "0.2+1.21.1"]
        );
    }

    #[test]
    fn rejects_untrusted_or_plain_http_sources() {
        assert!(trusted_https("http://meta.fabricmc.net/file", &["meta.fabricmc.net"]).is_err());
        assert!(trusted_https("https://evil.example/file", &["meta.fabricmc.net"]).is_err());
        assert!(trusted_https(
            "https://meta.fabricmc.net/file",
            &["meta.fabricmc.net"]
        )
        .is_ok());
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
}
