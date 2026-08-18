use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, str::FromStr};
use swarm_cli::{
    host_readiness, migration,
    runtime_installer::{RuntimeInstallOptions, RuntimeInstaller, RuntimeProgress},
    server_mods,
};
use swarm_core::DataPaths;
use swarm_network::ServerModsReadinessV1;
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
            if status.ready {
                let config = migration::load_runtime_config(&paths, world)?;
                let descriptor = storage.load_world_descriptor(world)?;
                host_readiness::record_runtime_verified(&paths, world, &config, descriptor.compatibility_fingerprint)?;
                let world_config = storage.load_world_config(world)?;
                let mods = server_mods::evaluate_world_mods(&paths, world, &world_config.compatibility)?;
                let state = if mods.ready {
                    ServerModsReadinessV1::Ready
                } else if mods
                    .issues
                    .iter()
                    .any(|issue| matches!(issue.kind, server_mods::ModIssueKind::MissingRequired))
                {
                    ServerModsReadinessV1::Missing
                } else {
                    ServerModsReadinessV1::Incompatible
                };
                host_readiness::record_server_mod_readiness(
                    &paths,
                    world,
                    descriptor.compatibility_fingerprint,
                    state,
                )?;
            }
            print_json(&status)?;
        }
    }
    Ok(())
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
