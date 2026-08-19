use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, str::FromStr};
use swarm_cli::{
    host_readiness, launch_guard, migration,
    runtime_installer::{
        RuntimeComponentKind, RuntimeComponentState, RuntimeInstallOptions, RuntimeInstaller, RuntimeProgress,
        RuntimeStatus,
    },
    server_mods,
};
use swarm_core::DataPaths;
use swarm_network::{HostRuntimeReadinessV1, ServerModsReadinessV1};
use swarm_protocol::WorldId;
use swarm_storage::Storage;

#[derive(Debug, Parser)]
#[command(
    name = "swarmcraft-runtime",
    version,
    about = "Prepare and verify managed Minecraft/Fabric runtimes for SwarmCraft"
)]
struct Args {
    #[arg(long, global = true, env = "SWARMCRAFT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: RuntimeCommand,
}

#[derive(Debug, Subcommand)]
enum RuntimeCommand {
    /// Inspect local runtime state without changing it.
    Status { world: String },
    /// Resolve official sources and describe the work required to make the runtime ready.
    Plan { world: String },
    /// Install missing managed runtime components. EULA acceptance is never implicit.
    Install {
        world: String,
        #[arg(long)]
        accept_eula: bool,
        #[arg(long)]
        game_endpoint: Option<String>,
    },
    /// Re-download/rebuild managed components and verify them again.
    Repair {
        world: String,
        #[arg(long)]
        accept_eula: bool,
        #[arg(long)]
        game_endpoint: Option<String>,
    },
    /// Re-hash and re-check the installed runtime without downloading anything.
    Verify { world: String },
    /// Launch the persisted managed runtime through the shared Rust authority/migration path.
    Launch { world: String },
}

fn main() -> Result<()> {
    let args = Args::parse();
    let paths = match args.data_dir {
        Some(root) => DataPaths::from_root(root),
        None => DataPaths::discover()?,
    };
    let storage = Storage::open(paths.root.clone())?;
    let installer = RuntimeInstaller::new(&paths, &storage);

    match args.command {
        RuntimeCommand::Status { world } => {
            print_json(&installer.inspect(parse_world(&world)?)?)?;
        }
        RuntimeCommand::Plan { world } => {
            print_json(&installer.plan(parse_world(&world)?)?)?;
        }
        RuntimeCommand::Install { world, accept_eula, game_endpoint } => {
            let report = installer.install(
                parse_world(&world)?,
                RuntimeInstallOptions { accept_eula, game_endpoint },
                print_progress,
            )?;
            print_json(&report)?;
        }
        RuntimeCommand::Repair { world, accept_eula, game_endpoint } => {
            let report = installer.repair(
                parse_world(&world)?,
                RuntimeInstallOptions { accept_eula, game_endpoint },
                print_progress,
            )?;
            print_json(&report)?;
        }
        RuntimeCommand::Verify { world } => {
            let world = parse_world(&world)?;
            let status = installer.verify(world)?;
            let descriptor = storage.load_world_descriptor(world)?;
            let world_config = storage.load_world_config(world)?;

            // Server-mod verification is an independent proof boundary. Record
            // the exact current result even when the platform runtime itself is
            // healthy but a required user mod is missing or incompatible.
            let mods = server_mods::evaluate_world_mods(&paths, world, &world_config.compatibility)?;
            let mod_state = if mods.ready {
                ServerModsReadinessV1::Ready
            } else if mods.issues.iter().any(|issue| matches!(issue.kind, server_mods::ModIssueKind::MissingRequired)) {
                ServerModsReadinessV1::Missing
            } else {
                ServerModsReadinessV1::Incompatible
            };
            host_readiness::record_server_mod_readiness(
                &paths,
                world,
                descriptor.compatibility_fingerprint,
                mod_state,
            )?;

            // Runtime proof excludes third-party server-mod readiness. This is
            // what lets Host Readiness distinguish "runtime verified, mod
            // missing" from a genuinely unverified runtime.
            if runtime_platform_ready(&status) {
                let config = migration::load_runtime_config(&paths, world)?;
                if status.manual_configuration {
                    let live =
                        host_readiness::local_runtime_readiness(&paths, world, descriptor.compatibility_fingerprint)?;
                    if live != HostRuntimeReadinessV1::Ready {
                        anyhow::bail!(
                            "manual Advanced runtime is configured but not authoritatively verified; launch it through the shared SwarmCraft runtime once so the authenticated Fabric compatibility handshake can prove this exact configuration"
                        );
                    }
                } else {
                    host_readiness::record_runtime_verified(
                        &paths,
                        world,
                        &config,
                        descriptor.compatibility_fingerprint,
                    )?;
                }
            }
            print_json(&status)?;
        }
        RuntimeCommand::Launch { world } => {
            let world = parse_world(&world)?;
            let status = installer.verify(world)?;
            if !status.ready {
                anyhow::bail!("managed runtime launch was requested before runtime verification reported Ready");
            }
            launch_guard::ensure_direct_launch_safe(&storage, world)?;
            let config = migration::load_runtime_config(&paths, world)?;
            let options = migration::HostOptions {
                world,
                java: config.java,
                server_jar: config.server_jar,
                mod_jar: config.mod_jar,
                accept_eula: config.accept_eula,
            };
            tokio::runtime::Runtime::new()?.block_on(migration::run_authority_runtime(&paths, &storage, options))?;
        }
    }
    Ok(())
}

fn runtime_platform_ready(status: &RuntimeStatus) -> bool {
    status.components.iter().all(|component| {
        component.kind == RuntimeComponentKind::ServerMods || component.state == RuntimeComponentState::Ready
    })
}

fn parse_world(value: &str) -> Result<WorldId> {
    WorldId::from_str(value).with_context(|| format!("invalid world ID: {value}"))
}

fn print_progress(progress: RuntimeProgress) {
    if let Ok(json) = serde_json::to_string(&progress) {
        eprintln!("{json}");
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_cli::runtime_installer::RuntimeComponentStatus;

    #[test]
    fn server_mod_failure_does_not_erase_runtime_verification_boundary() {
        let status = RuntimeStatus {
            world_id: "scworld:test".into(),
            minecraft_version: "1.21.1".into(),
            fabric_loader_version: "0.16.0".into(),
            required_java_major: 21,
            ready: false,
            eula_accepted: true,
            launch_configured: true,
            manual_configuration: false,
            components: vec![
                RuntimeComponentStatus {
                    kind: RuntimeComponentKind::Java,
                    state: RuntimeComponentState::Ready,
                    version: Some("21".into()),
                    path: None,
                    managed: false,
                    detail: None,
                },
                RuntimeComponentStatus {
                    kind: RuntimeComponentKind::Eula,
                    state: RuntimeComponentState::Ready,
                    version: None,
                    path: None,
                    managed: true,
                    detail: None,
                },
                RuntimeComponentStatus {
                    kind: RuntimeComponentKind::ServerMods,
                    state: RuntimeComponentState::Incompatible,
                    version: None,
                    path: None,
                    managed: false,
                    detail: Some("required mod missing".into()),
                },
            ],
        };
        assert!(runtime_platform_ready(&status));
        assert!(!status.ready);
    }
}
