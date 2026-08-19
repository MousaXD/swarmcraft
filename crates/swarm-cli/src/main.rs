mod daemon;
mod invite;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, str::FromStr};
use swarm_cli::host_readiness;
use swarm_cli::migration::{self, RuntimeLaunchConfig, TransferPrepareResult};
use swarm_cli::server_mods;
use swarm_core::{
    create_world_genesis_with_fingerprint, random_nonce, sign_world_config, verify_membership_signature,
    verify_snapshot_signature, DataPaths, PeerIdentity,
};
use swarm_protocol::{
    AuthorityPolicyV1, EpochMode, InviteV1, JoinRequestV1, LeaveRequestV1, MembershipPolicyV1, MembershipRecordV1,
    PeerId, RuntimeCompatibilityManifestV1, WorldConfigV1, WorldDescriptorV1, WorldId, WorldMemberV1,
    WorldPresentationV1, WorldSafetyLevelV1, WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, WorldMetadataV1};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "swarmcraft", version, about = "Decentralized persistence for Minecraft worlds")]
struct Cli {
    /// Override the standard OS-local SwarmCraft data directory.
    #[arg(long, global = true, env = "SWARMCRAFT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize local storage and peer identity.
    Init,
    /// Display this device's persistent cryptographic peer identity.
    Identity,
    /// Run the authenticated replication coordinator and migration supervisor.
    Daemon {
        #[arg(long, default_value = "/ip4/0.0.0.0/udp/0/quic-v1")]
        listen: String,
    },
    /// Manage replicated worlds.
    World {
        #[command(subcommand)]
        command: WorldCommand,
    },
    /// Create signed world invitations.
    Invite {
        #[command(subcommand)]
        command: InviteCommand,
    },
    /// Display the authorized peers for a world.
    Peers { world: String },
}

#[derive(Debug, Subcommand)]
enum InviteCommand {
    /// Create an expiring signed invitation for an existing world.
    Create {
        world: String,
        #[arg(long, default_value_t = 60)]
        expires_minutes: u64,
        #[arg(long = "bootstrap")]
        bootstrap_addrs: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum WorldCommand {
    /// Create a world identity and local durable metadata.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        minecraft: String,
        #[arg(long, default_value = "unknown")]
        fabric_loader: String,
        /// Legacy exact compatibility material retained for CLI compatibility. Prefer the canonical manifest shown by `world compatibility`.
        #[arg(long, default_value = "vanilla-fabric")]
        compatibility: String,
        /// Exact third-party Fabric server mods to bind into this world runtime profile.
        #[arg(long = "server-mod")]
        server_mod: Vec<PathBuf>,
        /// private, unlisted, or public.
        #[arg(long, default_value = "private")]
        visibility: String,
    },
    /// Stage a signed authority-mediated join request from a scinvite token.
    Join { invite: String },
    /// Stage a signed authority-mediated leave request without deleting replicated data.
    Leave { world: String },
    /// List locally known worlds.
    List,
    /// Show local world and snapshot status.
    Status { world: String },
    /// Configure the local Minecraft/Fabric runtime used by recovery, transfer, and wake orchestration.
    RuntimeConfigure {
        world: String,
        #[arg(long, default_value = "java")]
        java: PathBuf,
        #[arg(long)]
        server_jar: PathBuf,
        #[arg(long)]
        mod_jar: PathBuf,
        #[arg(long)]
        accept_eula: bool,
        #[arg(long)]
        game_endpoint: Option<String>,
    },
    /// Show machine-readable migration/runtime state for Desktop or operators.
    MigrationStatus {
        world: String,
        #[arg(long)]
        json: bool,
    },
    /// Show the authoritative backend answer to whether this device may safely shut down.
    HostReadiness {
        world: String,
        #[arg(long)]
        json: bool,
    },
    /// Request a safe stop. Success is reported only after the Fabric save barrier, final checkpoint and durable sleep record complete.
    Stop { world: String },
    /// Request a safe wake of a sleeping world. Multi-member worlds remain blocked until a quorum transition exists.
    Wake { world: String },
    /// Request a final checkpoint and prepare a manual authority transfer.
    TransferPrepare { world: String, to: String },
    /// Export the locally durable signed transfer token.
    TransferExport { world: String },
    /// Accept a prepared transfer as its target and print the accepted token.
    TransferAccept { world: String, token: String },
    /// Commit an accepted transfer as the current authority and print the committed token.
    TransferCommit { world: String, token: String },
    /// Activate a committed transfer as the target and print the signed successor epoch token.
    TransferActivate { world: String, token: String },
    /// Observe the signed successor epoch on another quorum peer.
    TransferObserve { world: String, token: String },
    /// Inspect the canonical execution compatibility manifest and authority eligibility.
    Compatibility { world: String },
    /// Inspect local third-party server mods against the canonical world runtime profile.
    ModsStatus {
        world: String,
        #[arg(long)]
        json: bool,
    },
    /// Add the exact locally supplied JAR for a canonical server-mod requirement.
    ModsAdd { world: String, jar: PathBuf },
    /// Remove a locally installed third-party server mod by Fabric mod ID.
    ModsRemove { world: String, mod_id: String },
    /// Print the persistent per-world third-party server mods directory.
    ModsPath { world: String },
    /// Enable or disable background replica seeding while Minecraft is off.
    Seed { world: String, enabled: bool },
    /// List preserved conflicting solo-history branches requiring manual resolution.
    Conflicts { world: String },
    /// Create and commit a signed snapshot from a quiescent directory.
    Snapshot {
        world: String,
        source: PathBuf,
        #[arg(long, default_value_t = 0)]
        epoch: u64,
        #[arg(long, default_value_t = 0)]
        sequence: u64,
    },
    /// List committed snapshots.
    Snapshots { world: String },
    /// Verify blob hashes, state root, and authority signature for one or all snapshots.
    Verify {
        world: String,
        #[arg(long)]
        snapshot: Option<u64>,
    },
    /// Restore a selected snapshot into a normal Minecraft world folder.
    Recover { world: String, snapshot: u64, destination: PathBuf },
    /// Export the newest valid snapshot as a normal Minecraft world folder.
    Export { world: String, destination: PathBuf },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let paths = match cli.data_dir {
        Some(root) => DataPaths::from_root(root),
        None => DataPaths::discover()?,
    };
    let storage = Storage::open(paths.root.clone())?;

    match cli.command {
        Command::Init => {
            paths.ensure()?;
            let identity = PeerIdentity::load_or_create(&paths)?;
            println!("SwarmCraft initialized");
            println!("Data: {}", paths.root.display());
            println!("Peer ID: {}", identity.peer_id());
            println!("Protocol: {}", PROTOCOL_VERSION);
            println!("Storage schema: {}", STORAGE_SCHEMA_VERSION);
        }
        Command::Identity => {
            let identity = PeerIdentity::load_or_create(&paths)?;
            println!("Peer ID: {}", identity.peer_id());
            println!("Public key: {}", hex_string(&identity.public_key()));
        }
        Command::Daemon { listen } => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async {
                let supervisor = tokio::spawn(migration::supervise(paths.clone()));
                let result = daemon::run(&paths, &storage, &listen).await;
                supervisor.abort();
                result
            })?;
        }
        Command::World { command } => handle_world(command, &paths, &storage)?,
        Command::Invite { command } => handle_invite(command, &paths, &storage)?,
        Command::Peers { world } => show_peers(parse_world(&world)?, &storage)?,
    }
    Ok(())
}

fn handle_world(command: WorldCommand, paths: &DataPaths, storage: &Storage) -> Result<()> {
    match command {
        WorldCommand::Create { name, minecraft, fabric_loader, compatibility, server_mod, visibility } => {
            let identity = PeerIdentity::load_or_create(paths)?;
            let visibility = parse_visibility(&visibility)?;
            // The legacy compatibility text is retained only as a CLI compatibility input.
            // It is not a physical server-mod requirement and must never make a clean world
            // appear to be missing a fictional JAR.
            let _legacy_compatibility = compatibility;
            let required_server_mods = server_mods::requirements_from_jars(&server_mod)?;
            let manifest = RuntimeCompatibilityManifestV1 {
                minecraft_version: minecraft.clone(),
                loader_id: "fabric".into(),
                loader_version: fabric_loader.clone(),
                swarmcraft_protocol_version: PROTOCOL_VERSION,
                fabric_adapter_version: env!("CARGO_PKG_VERSION").into(),
                required_server_mods,
                required_client_mods: Vec::new(),
                datapacks: Vec::new(),
            };
            let fingerprint = manifest.fingerprint()?;
            let (world_id, genesis) =
                create_world_genesis_with_fingerprint(&identity, minecraft, fabric_loader, fingerprint)?;
            storage.create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: name.clone(),
                world_id,
                genesis: genesis.clone(),
            })?;
            let descriptor = WorldDescriptorV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id,
                compatibility_fingerprint: genesis.compatibility_fingerprint,
                members: vec![local_member(&identity)],
                preferred_replication_factor: 2,
            };
            storage.save_world_descriptor(&descriptor)?;
            let mut membership = MembershipRecordV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id,
                epoch: 0,
                sequence: 0,
                previous_membership_hash: None,
                members: descriptor.members.clone(),
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            identity.sign_membership(&mut membership)?;
            storage.save_membership_record(&membership)?;
            let mut config = WorldConfigV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id,
                sequence: 1,
                previous_config_hash: None,
                compatibility: manifest,
                visibility,
                authority_policy: AuthorityPolicyV1 {
                    allow_solo_advancement: true,
                    preferred_replication_factor: descriptor.preferred_replication_factor,
                },
                membership_policy: MembershipPolicyV1::InviteOnly,
                presentation: WorldPresentationV1 {
                    name: name.clone(),
                    description: String::new(),
                    tags: Vec::new(),
                    icon_hash: None,
                    approximate_region: None,
                },
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            sign_world_config(&identity, &mut config)?;
            storage.save_world_config(&config)?;

            // Seed an initial empty canonical snapshot. The shared authority runtime restores this
            // empty directory and Minecraft creates the actual world on first launch; from then on
            // all state is checkpointed through the normal canonical snapshot path.
            let initial_source = paths.root.join("initial-world").join(world_id.to_hex());
            std::fs::create_dir_all(&initial_source)?;
            let mut initial_snapshot = storage.snapshot_directory(
                &initial_source,
                swarm_storage::SnapshotContext {
                    world: world_id,
                    snapshot_number: 1,
                    epoch: 0,
                    sequence: 1,
                    previous_snapshot_hash: None,
                    authority_peer_id: identity.peer_id(),
                    authority_public_key: identity.public_key(),
                },
            )?;
            identity.sign_snapshot(&mut initial_snapshot)?;
            storage.commit_snapshot(&initial_snapshot)?;
            let _ = std::fs::remove_dir_all(&initial_source);
            // initial empty canonical snapshot

            for source in &server_mod {
                server_mods::add_local_mod(paths, world_id, &config.compatibility, source)?;
            }
            println!("Created world: {name}");
            println!("World ID: {world_id}");
        }
        WorldCommand::Join { invite: value } => {
            let invite = invite::decode(&value)?;
            let identity = PeerIdentity::load_or_create(paths)?;
            if storage.load_world(invite.world_id).is_err() {
                storage.create_world(&WorldMetadataV1 {
                    storage_schema_version: STORAGE_SCHEMA_VERSION,
                    display_name: invite.display_name.clone(),
                    world_id: invite.world_id,
                    genesis: invite.genesis.clone(),
                })?;
            }
            let descriptor = WorldDescriptorV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: invite.world_id,
                compatibility_fingerprint: invite.genesis.compatibility_fingerprint,
                members: vec![WorldMemberV1 {
                    peer_id: invite.inviter_peer_id,
                    public_key: invite.inviter_public_key,
                    authority_eligible: true,
                    banned: false,
                }],
                preferred_replication_factor: 2,
            };
            storage.save_world_descriptor(&descriptor)?;
            let mut request = JoinRequestV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: invite.world_id,
                invite: invite.clone(),
                joining_member: local_storage_member(&identity),
                nonce: random_nonce(),
                signature: Vec::new(),
            };
            identity.sign_join_request(&mut request)?;
            storage.save_pending_join(&request)?;
            println!("Join request staged for: {}", invite.display_name);
            println!("World ID: {}", invite.world_id);
            println!("Run `swarmcraft daemon` to contact the authority and complete membership.");
            if !invite.bootstrap_addrs.is_empty() {
                println!("Bootstrap addresses: {}", invite.bootstrap_addrs.join(", "));
            }
        }
        WorldCommand::Leave { world } => {
            let world = parse_world(&world)?;
            let identity = PeerIdentity::load_or_create(paths)?;
            let descriptor = storage.load_world_descriptor(world)?;
            let local = descriptor.member(identity.peer_id()).context("this peer is not an authorized world member")?;
            if local.banned || local.public_key != identity.public_key() {
                bail!("local identity does not match canonical membership");
            }
            let membership = storage.load_membership_record(world)?;
            verify_membership_signature(&membership)?;
            if membership.authority_peer_id == identity.peer_id() {
                bail!("the current authority must transfer authority before leaving");
            }
            let mut request = LeaveRequestV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                membership_hash: membership.record_hash()?,
                leaving_peer_id: identity.peer_id(),
                leaving_public_key: identity.public_key(),
                nonce: random_nonce(),
                signature: Vec::new(),
            };
            identity.sign_leave_request(&mut request)?;
            storage.save_pending_leave(&request)?;
            println!("Leave request staged for {world}; replicated snapshots remain on disk.");
            println!("Run `swarmcraft daemon` to have the current authority commit the membership change.");
        }
        WorldCommand::List => {
            let worlds = storage.list_worlds()?;
            if worlds.is_empty() {
                println!("No local worlds.");
            }
            for world in worlds {
                let snapshots = storage.list_snapshots(world.world_id)?.len();
                println!("{}  {}  snapshots={}", world.world_id, world.display_name, snapshots);
            }
        }
        WorldCommand::Status { world } => {
            let world = parse_world(&world)?;
            let metadata = storage.load_world(world)?;
            let latest = storage.latest_snapshot(world)?;
            println!("World: {}", metadata.display_name);
            println!("World ID: {world}");
            println!("Minecraft: {}", metadata.genesis.minecraft_version);
            println!("Fabric loader: {}", metadata.genesis.fabric_loader_version);
            println!("Compatibility: {}", metadata.genesis.compatibility_fingerprint);
            if let Ok(config) = storage.load_world_config(world) {
                println!("Visibility: {:?}", config.visibility);
                println!("Compatibility manifest: {}", config.compatibility_fingerprint()?);
            } else {
                println!("Visibility: unknown (configuration not synchronized)");
            }
            if let Ok(descriptor) = storage.load_world_descriptor(world) {
                println!("Authorized peers: {}", descriptor.members.len());
                println!("Preferred replicas: {}", descriptor.preferred_replication_factor);
            }
            let conflict = !storage.list_solo_conflicts(world)?.is_empty();
            let solo = storage.load_epoch_record(world).is_ok_and(|epoch| epoch.mode == EpochMode::Solo)
                || storage.load_solo_branch(world).is_ok();
            let safety = if conflict {
                WorldSafetyLevelV1::Conflict
            } else if solo {
                WorldSafetyLevelV1::SoloUnreplicated
            } else {
                WorldSafetyLevelV1::Canonical
            };
            println!("Safety: {:?}", safety);
            println!("Background seeding: {}", storage.background_seeding_enabled(world)?);
            if let Ok(status) = migration::load_migration_status(paths, world) {
                println!("Migration: {:?}", status.phase);
                println!("Runtime ready: {}", status.runtime_ready);
                if let Some(endpoint) = status.game_endpoint {
                    println!("Game endpoint: {endpoint}");
                }
                if let Some(reason) = status.failure_reason {
                    println!("Migration failure: {reason}");
                }
            }
            match latest {
                Some(snapshot) => {
                    println!("Latest snapshot: {}", snapshot.snapshot_number);
                    println!("Epoch: {}", snapshot.epoch);
                    println!("Sequence: {}", snapshot.sequence);
                    println!("State root: {}", snapshot.state_root);
                    println!("Authority: {}", snapshot.authority_peer_id);
                }
                None => println!("Latest snapshot: none"),
            }
        }
        WorldCommand::RuntimeConfigure { world, java, server_jar, mod_jar, accept_eula, game_endpoint } => {
            let world = parse_world(&world)?;
            storage.load_world(world)?;
            if !accept_eula {
                bail!("runtime configuration requires explicit --accept-eula after reviewing Mojang's EULA");
            }
            migration::save_runtime_config(
                paths,
                world,
                &RuntimeLaunchConfig { java, server_jar, mod_jar, accept_eula, game_endpoint },
            )?;
            println!("Runtime orchestration configured for {world}.");
            println!("Recovery and committed transfer launches will now be supervised by `swarmcraft daemon`.");
        }
        WorldCommand::MigrationStatus { world, json } => {
            let world = parse_world(&world)?;
            let status = migration::load_migration_status(paths, world)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("World: {}", status.world_id);
                println!("Authority: {}", status.authority_peer_id.as_deref().unwrap_or("unknown"));
                println!("Epoch: {}", status.epoch.map_or_else(|| "unknown".into(), |value| value.to_string()));
                println!(
                    "Fencing token: {}",
                    status.fencing_token.map_or_else(|| "unknown".into(), |value| value.to_string())
                );
                println!("Phase: {:?}", status.phase);
                println!("Runtime ready: {}", status.runtime_ready);
                println!("Game endpoint: {}", status.game_endpoint.as_deref().unwrap_or("unpublished"));
                if let Some(reason) = status.failure_reason {
                    println!("Failure: {reason}");
                }
            }
        }
        WorldCommand::HostReadiness { world, json } => {
            let world = parse_world(&world)?;
            storage.load_world(world)?;
            let report = match host_readiness::load_host_readiness_report(paths, world) {
                Ok(report) => report,
                Err(error) => {
                    let identity = PeerIdentity::load_or_create(paths)?;
                    host_readiness::unknown_report(
                        world,
                        identity.peer_id(),
                        storage.load_epoch_record(world).ok().map(|epoch| epoch.authority_peer_id),
                        format!("Host-readiness service has not produced a fresh report yet: {error}"),
                    )
                }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Shutdown safety: {:?}", report.state);
                println!("Safe to shut down: {}", report.safe_to_shutdown);
                println!("{}", report.detail);
                if let Some(successor) = report.successor_peer_id {
                    println!("Automatic successor: {successor}");
                } else if let Some(candidate) = report.handoff_candidate_peer_id {
                    println!("Transfer-first candidate: {candidate}");
                }
            }
        }
        WorldCommand::Stop { world } => {
            let world = parse_world(&world)?;
            migration::request_world_stop(paths, storage, world)?;
            println!("Safe stop requested for {world}; wait for migration status to become sleeping before treating shutdown as complete.");
        }
        WorldCommand::Wake { world } => {
            let world = parse_world(&world)?;
            migration::request_world_wake(paths, storage, world)?;
            println!("Wake requested for {world}.");
            println!(
                "The migration supervisor will launch only after a safe accepted authority generation is available."
            );
        }
        WorldCommand::TransferPrepare { world, to } => {
            let world = parse_world(&world)?;
            let target = PeerId::from_str(&to).with_context(|| format!("invalid peer ID: {to}"))?;
            match migration::prepare_manual_transfer(paths, storage, world, target)? {
                TransferPrepareResult::CheckpointRequested => {
                    println!("Transfer checkpoint requested.");
                    println!("After the runtime stops, run `swarmcraft world transfer-export {world}` to obtain the prepared token.");
                }
                TransferPrepareResult::Prepared(token) => println!("{token}"),
            }
        }
        WorldCommand::TransferExport { world } => {
            println!("{}", migration::export_transfer(storage, parse_world(&world)?)?);
        }
        WorldCommand::TransferAccept { world, token } => {
            println!("{}", migration::accept_manual_transfer(paths, storage, parse_world(&world)?, &token)?);
        }
        WorldCommand::TransferCommit { world, token } => {
            println!("{}", migration::commit_manual_transfer(paths, storage, parse_world(&world)?, &token)?);
        }
        WorldCommand::TransferActivate { world, token } => {
            let world = parse_world(&world)?;
            let epoch = migration::activate_manual_transfer(paths, storage, world, &token)?;
            println!("{epoch}");
            println!("Distribute this signed epoch token to a quorum of peers with `swarmcraft world transfer-observe {world} <token>`.");
        }
        WorldCommand::TransferObserve { world, token } => {
            let world = parse_world(&world)?;
            migration::observe_manual_transfer_epoch(paths, storage, world, &token)?;
            println!("Accepted manual authority epoch for {world}.");
        }
        WorldCommand::Compatibility { world } => {
            let world = parse_world(&world)?;
            let metadata = storage.load_world(world)?;
            match storage.load_world_config(world) {
                Ok(config) => {
                    let fingerprint = config.compatibility_fingerprint()?;
                    let mod_readiness = server_mods::evaluate_world_mods(paths, world, &config.compatibility)?;
                    println!("Compatibility fingerprint: {fingerprint}");
                    println!("Minecraft: {}", config.compatibility.minecraft_version);
                    println!("Loader: {} {}", config.compatibility.loader_id, config.compatibility.loader_version);
                    println!("Fabric adapter: {}", config.compatibility.fabric_adapter_version);
                    println!("Server mods: {}", config.compatibility.required_server_mods.len());
                    println!("Client mods: {}", config.compatibility.required_client_mods.len());
                    println!("Datapacks: {}", config.compatibility.datapacks.len());
                    println!("Genesis match: {}", fingerprint == metadata.genesis.compatibility_fingerprint);
                    println!("Server mods ready: {}", mod_readiness.ready);
                    let identity = PeerIdentity::load_or_create(paths)?;
                    let eligible = storage
                        .load_world_descriptor(world)
                        .ok()
                        .and_then(|descriptor| descriptor.member(identity.peer_id()).cloned())
                        .is_some_and(|member| member.authority_eligible && !member.banned);
                    if eligible && fingerprint == metadata.genesis.compatibility_fingerprint && mod_readiness.ready {
                        println!("Authority eligibility: Compatible");
                    } else if !mod_readiness.ready {
                        println!("Authority eligibility: Replica only: server mods missing or incompatible");
                    } else {
                        println!("Authority eligibility: Replica only: not authority eligible");
                    }
                }
                Err(_) => {
                    println!("Compatibility manifest: not yet synchronized");
                    println!("Authority eligibility: Replica only: not authority eligible");
                }
            }
        }
        WorldCommand::ModsStatus { world, json } => {
            let world = parse_world(&world)?;
            let config =
                storage.load_world_config(world).context("canonical runtime profile is not yet synchronized")?;
            let status = server_mods::evaluate_world_mods(paths, world, &config.compatibility)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("Server mods directory: {}", status.mods_dir.display());
                println!("Ready: {}", status.ready);
                println!("Canonical requirements: {}", status.required.len());
                for required in &status.required {
                    println!(
                        "required {} {} {} {:?}",
                        required.mod_id, required.version, required.artifact_hash, required.component_kind
                    );
                }
                for installed in &status.installed {
                    println!(
                        "installed {} {} {} {:?}",
                        installed.mod_id, installed.version, installed.artifact_hash, installed.environment
                    );
                }
                for issue in &status.issues {
                    println!("issue {:?}: {}", issue.kind, issue.message);
                }
            }
        }
        WorldCommand::ModsAdd { world, jar } => {
            let world = parse_world(&world)?;
            let config =
                storage.load_world_config(world).context("canonical runtime profile is not yet synchronized")?;
            let installed = server_mods::add_local_mod(paths, world, &config.compatibility, &jar)?;
            println!(
                "Installed {} {} with artifact hash {}",
                installed.mod_id, installed.version, installed.artifact_hash
            );
        }
        WorldCommand::ModsRemove { world, mod_id } => {
            let world = parse_world(&world)?;
            storage.load_world(world)?;
            let removed = server_mods::remove_local_mod(paths, world, &mod_id)?;
            if removed.is_empty() {
                println!("No local server mod with id {mod_id} was installed.");
            } else {
                println!("Removed {} from local server mods.", removed[0].display());
            }
        }
        WorldCommand::ModsPath { world } => {
            let world = parse_world(&world)?;
            storage.load_world(world)?;
            println!("{}", server_mods::mods_dir(paths, world).display());
        }
        WorldCommand::Seed { world, enabled } => {
            let world = parse_world(&world)?;
            storage.set_background_seeding(world, enabled)?;
            println!("Background seeding: {}", if enabled { "enabled" } else { "disabled" });
        }
        WorldCommand::Conflicts { world } => {
            let world = parse_world(&world)?;
            let conflicts = storage.list_solo_conflicts(world)?;
            if conflicts.is_empty() {
                println!("Solo history conflicts: none");
            } else {
                println!("Solo history conflicts: {}", conflicts.len());
                for branch in conflicts {
                    println!(
                        "{} writer={} epoch={} seq={} state={}",
                        branch.branch_hash()?,
                        branch.authority_peer_id,
                        branch.head_epoch,
                        branch.head_sequence,
                        branch.state_hash
                    );
                }
                println!(
                    "No automatic Minecraft world merge is attempted; preserve both branches until manually resolved."
                );
            }
        }
        WorldCommand::Snapshot { world, source, epoch, sequence } => {
            let world = parse_world(&world)?;
            storage.load_world(world)?;
            let identity = PeerIdentity::load_or_create(paths)?;
            let number = storage.next_snapshot_number(world)?;
            let previous = storage.latest_snapshot(world)?;
            let previous_hash = previous.as_ref().map(|m| m.manifest_hash()).transpose()?;
            info!(world = %world, snapshot = number, source = %source.display(), "building snapshot");
            let mut manifest = storage.snapshot_directory(
                &source,
                SnapshotContext {
                    world,
                    snapshot_number: number,
                    epoch,
                    sequence,
                    previous_snapshot_hash: previous_hash,
                    authority_peer_id: identity.peer_id(),
                    authority_public_key: identity.public_key(),
                },
            )?;
            identity.sign_snapshot(&mut manifest)?;
            storage.commit_snapshot(&manifest)?;
            println!("Committed snapshot #{}", manifest.snapshot_number);
            println!("State root: {}", manifest.state_root);
            println!("Files: {}", manifest.entries.len());
        }
        WorldCommand::Snapshots { world } => {
            let world = parse_world(&world)?;
            for snapshot in storage.list_snapshots(world)? {
                println!(
                    "#{} epoch={} seq={} files={} root={}",
                    snapshot.snapshot_number,
                    snapshot.epoch,
                    snapshot.sequence,
                    snapshot.entries.len(),
                    snapshot.state_root
                );
            }
        }
        WorldCommand::Verify { world, snapshot } => {
            let world = parse_world(&world)?;
            let snapshots = match snapshot {
                Some(number) => vec![storage.load_snapshot(world, number)?],
                None => storage.list_snapshots(world)?,
            };
            if snapshots.is_empty() {
                bail!("world has no committed snapshots");
            }
            let mut previous_hash = None;
            for manifest in snapshots {
                if snapshot.is_none() && manifest.previous_snapshot_hash != previous_hash {
                    bail!("snapshot #{} does not link to the previous committed manifest", manifest.snapshot_number);
                }
                storage.verify_snapshot(&manifest)?;
                verify_snapshot_signature(&manifest)?;
                previous_hash = Some(manifest.manifest_hash()?);
                println!("OK snapshot #{} {}", manifest.snapshot_number, manifest.state_root);
            }
        }
        WorldCommand::Recover { world, snapshot, destination } => {
            let world = parse_world(&world)?;
            let manifest = storage.load_snapshot(world, snapshot)?;
            storage.verify_snapshot(&manifest)?;
            verify_snapshot_signature(&manifest)?;
            ensure_empty_or_missing(&destination)?;
            storage.restore_snapshot(&manifest, &destination)?;
            println!("Recovered snapshot #{} to {}", snapshot, destination.display());
        }
        WorldCommand::Export { world, destination } => {
            let world = parse_world(&world)?;
            let manifest = storage.latest_snapshot(world)?.context("world has no committed snapshots")?;
            storage.verify_snapshot(&manifest)?;
            verify_snapshot_signature(&manifest)?;
            ensure_empty_or_missing(&destination)?;
            storage.restore_snapshot(&manifest, &destination)?;
            println!("Exported snapshot #{} to {}", manifest.snapshot_number, destination.display());
        }
    }
    Ok(())
}

fn handle_invite(command: InviteCommand, paths: &DataPaths, storage: &Storage) -> Result<()> {
    match command {
        InviteCommand::Create { world, expires_minutes, bootstrap_addrs } => {
            let world = parse_world(&world)?;
            let metadata = storage.load_world(world)?;
            let descriptor = storage.load_world_descriptor(world)?;
            let membership = storage.load_membership_record(world)?;
            verify_membership_signature(&membership)?;
            let identity = PeerIdentity::load_or_create(paths)?;
            let member =
                descriptor.member(identity.peer_id()).context("this peer is not an authorized member of the world")?;
            if member.banned || member.public_key != identity.public_key() {
                bail!("this peer is banned or its key does not match membership");
            }
            if membership.authority_peer_id != identity.peer_id()
                || membership.authority_public_key != identity.public_key()
            {
                bail!("only the current authority may create join invitations");
            }
            let lifetime_ms = expires_minutes.saturating_mul(60_000);
            let mut invite = InviteV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                display_name: metadata.display_name,
                genesis: metadata.genesis,
                inviter_peer_id: identity.peer_id(),
                inviter_public_key: identity.public_key(),
                bootstrap_addrs,
                expires_unix_ms: invite::unix_time_ms()?.saturating_add(lifetime_ms),
                nonce: random_nonce(),
                signature: Vec::new(),
            };
            identity.sign_invite(&mut invite)?;
            println!("{}", invite::encode(&invite)?);
        }
    }
    Ok(())
}

fn show_peers(world: WorldId, storage: &Storage) -> Result<()> {
    let descriptor = storage.load_world_descriptor(world)?;
    if descriptor.members.is_empty() {
        println!("No authorized peers.");
        return Ok(());
    }
    for member in descriptor.members {
        println!("{} authority_eligible={} banned={}", member.peer_id, member.authority_eligible, member.banned);
    }
    Ok(())
}

fn local_member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn local_storage_member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: false,
        banned: false,
    }
}

fn parse_visibility(value: &str) -> Result<WorldVisibilityV1> {
    match value.to_ascii_lowercase().as_str() {
        "private" => Ok(WorldVisibilityV1::Private),
        "unlisted" => Ok(WorldVisibilityV1::Unlisted),
        "public" => Ok(WorldVisibilityV1::Public),
        _ => bail!("visibility must be private, unlisted, or public"),
    }
}

fn parse_world(value: &str) -> Result<WorldId> {
    WorldId::from_str(value).with_context(|| format!("invalid world ID: {value}"))
}

fn ensure_empty_or_missing(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        let mut entries =
            std::fs::read_dir(path).with_context(|| format!("cannot inspect destination {}", path.display()))?;
        if entries.next().is_some() {
            bail!("destination must be empty or missing: {}", path.display());
        }
    }
    Ok(())
}

fn hex_string(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
