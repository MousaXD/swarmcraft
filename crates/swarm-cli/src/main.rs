mod daemon;
mod invite;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, str::FromStr};
use swarm_core::{
    create_world_genesis, random_nonce, verify_membership_signature, verify_snapshot_signature, DataPaths, PeerIdentity,
};
use swarm_protocol::{
    InviteV1, JoinRequestV1, LeaveRequestV1, MembershipRecordV1, WorldDescriptorV1, WorldId, WorldMemberV1,
    PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
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
    /// Run the authenticated replication coordinator.
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
        /// Stable compatibility material, for example a sorted mod/datapack fingerprint string.
        #[arg(long, default_value = "vanilla-fabric")]
        compatibility: String,
    },
    /// Stage a signed authority-mediated join request from a scinvite token.
    Join { invite: String },
    /// Stage a signed authority-mediated leave request without deleting replicated data.
    Leave { world: String },
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
        Command::Daemon { listen } => {
            tokio::runtime::Runtime::new()?.block_on(daemon::run(&paths, &storage, &listen))?;
        }
        Command::World { command } => handle_world(command, &paths, &storage)?,
        Command::Invite { command } => handle_invite(command, &paths, &storage)?,
        Command::Peers { world } => show_peers(parse_world(&world)?, &storage)?,
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
                joining_member: local_member(&identity),
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
            if let Ok(descriptor) = storage.load_world_descriptor(world) {
                println!("Authorized peers: {}", descriptor.members.len());
                println!("Preferred replicas: {}", descriptor.preferred_replication_factor);
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
