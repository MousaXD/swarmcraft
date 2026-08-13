use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, str::FromStr};
use swarm_core::{create_world_genesis, verify_snapshot_signature, DataPaths, PeerIdentity};
use swarm_protocol::{WorldId, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION};
use swarm_storage::{Storage, WorldMetadataV1};
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
    /// Manage replicated worlds.
    World {
        #[command(subcommand)]
        command: WorldCommand,
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
        /// Stable compatibility material, for example a sorted mod/datapack fingerprint string.
        #[arg(long, default_value = "vanilla-fabric")]
        compatibility: String,
    },
    /// List locally known worlds.
    List,
    /// Show local world and snapshot status.
    Status { world: String },
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
        Command::World { command } => handle_world(command, &paths, &storage)?,
    }
    Ok(())
}

fn handle_world(command: WorldCommand, paths: &DataPaths, storage: &Storage) -> Result<()> {
    match command {
        WorldCommand::Create { name, minecraft, fabric_loader, compatibility } => {
            let identity = PeerIdentity::load_or_create(paths)?;
            let (world_id, genesis) =
                create_world_genesis(&identity, minecraft, fabric_loader, compatibility.as_bytes())?;
            storage.create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: name.clone(),
                world_id,
                genesis,
            })?;
            println!("Created world: {name}");
            println!("World ID: {world_id}");
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
        WorldCommand::Snapshot { world, source, epoch, sequence } => {
            let world = parse_world(&world)?;
            storage.load_world(world)?;
            let identity = PeerIdentity::load_or_create(paths)?;
            let number = storage.next_snapshot_number(world)?;
            let previous = storage.latest_snapshot(world)?;
            let previous_hash = previous.as_ref().map(|m| m.manifest_hash()).transpose()?;
            info!(world = %world, snapshot = number, source = %source.display(), "building snapshot");
            let mut manifest = storage.snapshot_directory(
                world,
                &source,
                number,
                epoch,
                sequence,
                previous_hash,
                identity.peer_id(),
                identity.public_key(),
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
