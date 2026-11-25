use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, str::FromStr};
use swarm_cli::{
    client_profile,
    discovery::{self, DiscoverySearchInputV1},
    host_readiness, launch_guard, migration, provider_runtime,
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
    /// Print the build's pinned new-world baseline and declared adapter range.
    CompatibilityMatrix,
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
    /// Inspect one exact Fabric mod JAR using the shared Rust authority parser.
    InspectMod { path: PathBuf },
    /// Acquire permitted exact server mods without accepting EULA or starting a server.
    AcquireMods { world: String },
    /// Describe the signed client profile and optionally inspect one selected mods folder.
    ClientProfile {
        world: String,
        #[arg(long)]
        mods_dir: Option<PathBuf>,
    },
    /// Publish signed public/unlisted discovery hints from this member's local world store.
    /// Bind to a reachable local/VPN address; private worlds are never published.
    DiscoveryServe {
        #[arg(long, default_value = "/ip4/127.0.0.1/tcp/0")]
        listen: String,
        /// Publish only explicitly consented contact presence, no world metadata or votes.
        #[arg(long)]
        presence_only: bool,
        #[arg(long = "bootstrap")]
        bootstrap: Vec<String>,
    },
    /// Search authenticated public world announcements. Discovery never grants membership.
    DiscoverySearch {
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        region: Option<String>,
        #[arg(long)]
        language: Option<String>,
        /// Maximum verified listings in this bounded snapshot (1 through 64).
        #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u16).range(1..=64))]
        limit: u16,
        #[arg(long = "bootstrap")]
        bootstrap: Vec<String>,
    },
    /// Resolve one authenticated public/unlisted world announcement by exact world ID.
    DiscoveryResolve {
        world: String,
        #[arg(long = "bootstrap")]
        bootstrap: Vec<String>,
    },
    /// Local browse preferences, directory suppression and untrusted report records.
    Moderation {
        #[command(subcommand)]
        command: ModerationCommand,
    },
    /// Launch the persisted managed runtime through the shared Rust authority/migration path.
    Launch { world: String },
}

#[derive(Debug, Subcommand)]
enum ModerationCommand {
    /// Read local state without opening or changing canonical world storage.
    State,
    /// Hide/restore a world only in this device's public browse results.
    Hide {
        world: String,
        #[arg(long, action = clap::ArgAction::Set)]
        enabled: bool,
    },
    /// Suppress browse results on this device's directory publisher.
    Suppress {
        world: String,
        #[arg(long)]
        reason: String,
    },
    /// Restore browse eligibility on this device's directory publisher.
    Restore { world: String },
    /// Reverify a public listing and save a minimal local report; never sends it.
    Report {
        world: String,
        #[arg(long)]
        config: String,
        #[arg(long)]
        reason: String,
        #[arg(long = "bootstrap")]
        bootstrap: Vec<String>,
    },
    /// Print a minimal transferable report envelope, never sends it.
    ExportReport { report: String },
    /// Independently check an explicitly supplied untrusted report against live public discovery.
    ReviewReport {
        #[arg(long)]
        report_file: PathBuf,
        #[arg(long = "bootstrap")]
        bootstrap: Vec<String>,
    },
    /// Delete a local report record without changing any policy.
    Dismiss { report: String },
}

fn main() -> Result<()> {
    let args = Args::parse();
    if matches!(args.command, RuntimeCommand::CompatibilityMatrix) {
        return print_json(&swarm_protocol::runtime_compatibility_matrix());
    }
    let paths = match args.data_dir {
        Some(root) => DataPaths::from_root(root),
        None => DataPaths::discover()?,
    };
    let command = match args.command {
        RuntimeCommand::Moderation { command } => return handle_moderation(&paths, command),
        other => other,
    };
    let prepare = matches!(
        &command,
        RuntimeCommand::Install { .. } | RuntimeCommand::Repair { .. } | RuntimeCommand::Verify { .. }
    );
    if prepare {
        swarm_cli::support_metrics::measure(&paths, swarm_cli::support_metrics::Metric::RuntimePrepare, || {
            run_command(&paths, command)
        })
    } else {
        run_command(&paths, command)
    }
}

fn run_command(paths: &DataPaths, command: RuntimeCommand) -> Result<()> {
    let storage = Storage::open(paths.root.clone())?;
    let installer = RuntimeInstaller::new(paths, &storage);

    match command {
        RuntimeCommand::Moderation { .. } => unreachable!("moderation is handled before storage opens"),
        RuntimeCommand::CompatibilityMatrix => unreachable!("handled before opening storage"),
        RuntimeCommand::Status { world } => {
            print_json(&installer.inspect(parse_world(&world)?)?)?;
        }
        RuntimeCommand::Plan { world } => {
            print_json(&installer.plan(parse_world(&world)?)?)?;
        }
        RuntimeCommand::Install { world, accept_eula, game_endpoint } => {
            let world = parse_world(&world)?;
            prepare_provider_mods(paths, &storage, world)?;
            let report =
                installer.install(world, RuntimeInstallOptions { accept_eula, game_endpoint }, print_progress)?;
            print_json(&report)?;
        }
        RuntimeCommand::Repair { world, accept_eula, game_endpoint } => {
            let world = parse_world(&world)?;
            prepare_provider_mods(paths, &storage, world)?;
            let report =
                installer.repair(world, RuntimeInstallOptions { accept_eula, game_endpoint }, print_progress)?;
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
            let mods = server_mods::evaluate_world_mods(paths, world, &world_config.compatibility)?;
            let mod_state = if mods.ready {
                ServerModsReadinessV1::Ready
            } else if mods.issues.iter().any(|issue| matches!(issue.kind, server_mods::ModIssueKind::MissingRequired)) {
                ServerModsReadinessV1::Missing
            } else {
                ServerModsReadinessV1::Incompatible
            };
            host_readiness::record_server_mod_readiness(paths, world, descriptor.compatibility_fingerprint, mod_state)?;

            // Runtime proof excludes third-party server-mod readiness. This is
            // what lets Host Readiness distinguish "runtime verified, mod
            // missing" from a genuinely unverified runtime.
            if runtime_platform_ready(&status) {
                let config = migration::load_runtime_config(paths, world)?;
                if status.manual_configuration {
                    let live =
                        host_readiness::local_runtime_readiness(paths, world, descriptor.compatibility_fingerprint)?;
                    if live != HostRuntimeReadinessV1::Ready {
                        anyhow::bail!(
                            "manual Advanced runtime is configured but not authoritatively verified; launch it through the shared SwarmCraft runtime once so the authenticated Fabric compatibility handshake can prove this exact configuration"
                        );
                    }
                } else {
                    host_readiness::record_runtime_verified(
                        paths,
                        world,
                        &config,
                        descriptor.compatibility_fingerprint,
                    )?;
                }
            }
            print_json(&status)?;
        }
        RuntimeCommand::InspectMod { path } => {
            print_json(&server_mods::inspect_fabric_mod(&path)?)?;
        }
        RuntimeCommand::AcquireMods { world } => {
            print_json(&provider_runtime::acquire_missing_server_mods(paths, &storage, parse_world(&world)?)?)?;
        }
        RuntimeCommand::ClientProfile { world, mods_dir } => {
            print_json(&client_profile::prepare_client_profile(&storage, parse_world(&world)?, mods_dir.as_deref())?)?;
        }
        RuntimeCommand::DiscoveryServe { listen, presence_only, bootstrap } => {
            swarm_cli::support_metrics::start_background(paths);
            tokio::runtime::Runtime::new()?.block_on(discovery::serve(
                paths.clone(),
                listen,
                presence_only,
                &bootstrap,
            ))?;
        }
        RuntimeCommand::DiscoverySearch { query, region, language, limit, bootstrap } => {
            let approximate_region =
                region.as_deref().map(swarm_protocol::listing_region).transpose().map_err(anyhow::Error::msg)?;
            let tags = language
                .as_deref()
                .map(swarm_protocol::listing_language_tag)
                .transpose()
                .map_err(anyhow::Error::msg)?
                .into_iter()
                .collect();
            let report = tokio::runtime::Runtime::new()?.block_on(discovery::search_public_worlds(
                paths,
                DiscoverySearchInputV1 { query, approximate_region, tags, limit, ..Default::default() },
                &bootstrap,
            ))?;
            print_json(&report)?;
        }
        RuntimeCommand::DiscoveryResolve { world, bootstrap } => {
            let report = tokio::runtime::Runtime::new()?.block_on(discovery::resolve_world(
                paths,
                parse_world(&world)?,
                &bootstrap,
            ))?;
            print_json(&report)?;
        }
        RuntimeCommand::Launch { world } => {
            let world = parse_world(&world)?;
            let status = installer.verify(world)?;
            if !status.ready {
                anyhow::bail!("managed runtime launch was requested before runtime verification reported Ready");
            }
            launch_guard::ensure_direct_launch_safe(&storage, world)?;
            let config = migration::load_runtime_config(paths, world)?;
            let options = migration::HostOptions {
                world,
                java: config.java,
                server_jar: config.server_jar,
                mod_jar: config.mod_jar,
                accept_eula: config.accept_eula,
            };
            tokio::runtime::Runtime::new()?.block_on(migration::run_authority_runtime(paths, &storage, options))?;
        }
    }
    Ok(())
}

fn prepare_provider_mods(paths: &DataPaths, storage: &Storage, world: WorldId) -> Result<()> {
    let report = provider_runtime::acquire_missing_server_mods(paths, storage, world)?;
    if !report.ready {
        let summary = report
            .artifacts
            .iter()
            .filter(|a| !matches!(a.state, "acquired" | "already_verified"))
            .map(|a| format!("{} {}: {}", a.mod_id, a.version, a.state))
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("Server mod preparation incomplete. Use Acquire required server mods or Supply required JAR in the world Mods panel. {summary}");
    }
    Ok(())
}

fn runtime_platform_ready(status: &RuntimeStatus) -> bool {
    status.components.iter().all(|component| {
        component.kind == RuntimeComponentKind::ServerMods || component.state == RuntimeComponentState::Ready
    })
}

fn handle_moderation(paths: &DataPaths, command: ModerationCommand) -> Result<()> {
    use swarm_cli::directory_moderation as policy;
    let result = match command {
        ModerationCommand::State => policy::state(paths)?,
        ModerationCommand::Hide { world, enabled } => policy::set_hidden(paths, parse_world(&world)?, enabled)?,
        ModerationCommand::Suppress { world, reason } => {
            policy::set_suppression(paths, parse_world(&world)?, Some(reason.parse()?))?
        }
        ModerationCommand::Restore { world } => policy::set_suppression(paths, parse_world(&world)?, None)?,
        ModerationCommand::Report { world, config, reason, bootstrap } => tokio::runtime::Runtime::new()?.block_on(
            policy::report_public_listing(paths, parse_world(&world)?, config.parse()?, reason.parse()?, &bootstrap),
        )?,
        ModerationCommand::ExportReport { report } => return print_json(&policy::export_report(paths, &report)?),
        ModerationCommand::ReviewReport { report_file, bootstrap } => {
            return print_json(&tokio::runtime::Runtime::new()?.block_on(policy::review_report(
                paths,
                &report_file,
                &bootstrap,
            ))?)
        }
        ModerationCommand::Dismiss { report } => policy::dismiss_report(paths, &report)?,
    };
    print_json(&result)
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
