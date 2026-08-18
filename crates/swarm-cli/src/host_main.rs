mod host;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::{
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};
use swarm_cli::{authority_permit::PermitWatch, launch_guard};
use swarm_consensus::AuthorityGeneration;
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::WorldId;
use swarm_storage::Storage;
use tokio::time::sleep;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "swarmcraft-host", version, about = "Run the local SwarmCraft Minecraft authority runtime")]
struct Args {
    #[arg(long, env = "SWARMCRAFT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    world: String,
    #[arg(long, default_value = "java")]
    java: PathBuf,
    #[arg(long)]
    server_jar: PathBuf,
    #[arg(long)]
    mod_jar: PathBuf,
    #[arg(long)]
    accept_eula: bool,
    /// Stay dormant until this peer is the accepted authority and has a changing majority-backed permit.
    #[arg(long)]
    standby: bool,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .init();
    let args = Args::parse();
    let paths = match args.data_dir.clone() {
        Some(root) => DataPaths::from_root(root),
        None => DataPaths::discover()?,
    };
    let storage = Storage::open(paths.root.clone())?;
    let world = WorldId::from_str(&args.world).with_context(|| format!("invalid world ID: {}", args.world))?;

    if args.standby {
        return tokio::runtime::Runtime::new()?.block_on(run_standby(&paths, &storage, &args, world));
    }

    launch_guard::ensure_direct_launch_safe(&storage, world)?;
    tokio::runtime::Runtime::new()?.block_on(host_once(&paths, &storage, &args, world))
}

async fn run_standby(paths: &DataPaths, storage: &Storage, args: &Args, world: WorldId) -> Result<()> {
    let identity = PeerIdentity::load_or_create(paths)?;
    info!(%world, peer = %identity.peer_id(), "standby authority supervisor started");
    loop {
        wait_until_ready(paths, storage, &identity, world).await?;
        info!(%world, "standby peer has a live authority permit; starting Minecraft");
        match host_once(paths, storage, args, world).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                warn!(%world, %error, "authority runtime stopped; returning to standby");
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn host_once(paths: &DataPaths, storage: &Storage, args: &Args, world: WorldId) -> Result<()> {
    host::run(
        paths,
        storage,
        host::HostOptions {
            world,
            java: args.java.clone(),
            server_jar: args.server_jar.clone(),
            mod_jar: args.mod_jar.clone(),
            accept_eula: args.accept_eula,
        },
    )
    .await
}

async fn wait_until_ready(paths: &DataPaths, storage: &Storage, identity: &PeerIdentity, world: WorldId) -> Result<()> {
    let mut watched_generation = None;
    let mut permit_watch = None;
    loop {
        if storage.load_sleep_record(world).is_ok() {
            watched_generation = None;
            permit_watch = None;
            sleep(Duration::from_millis(500)).await;
            continue;
        }

        let descriptor = storage.load_world_descriptor(world)?;
        let Some(local_member) = descriptor.member(identity.peer_id()) else {
            bail!("local peer is no longer a member of this world");
        };
        if local_member.banned || !local_member.authority_eligible || local_member.public_key != identity.public_key() {
            bail!("local peer is not eligible to host this world");
        }

        let epoch = storage.load_epoch_record(world)?;
        if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {
            watched_generation = None;
            permit_watch = None;
            sleep(Duration::from_millis(500)).await;
            continue;
        }
        if descriptor.members.len() <= 1 {
            return Ok(());
        }

        let generation = AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token };
        if watched_generation != Some(generation) {
            watched_generation = Some(generation);
            permit_watch = Some(PermitWatch::new(generation));
        }
        if let Some(watch) = permit_watch.as_mut() {
            if watch.observe(paths, world, Instant::now()).unwrap_or(false) {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(250)).await;
    }
}
