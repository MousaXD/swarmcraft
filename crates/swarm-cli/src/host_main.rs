mod host;

use anyhow::{Context, Result};
use clap::Parser;
use std::{path::PathBuf, str::FromStr};
use swarm_core::DataPaths;
use swarm_protocol::WorldId;
use swarm_storage::Storage;
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
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .init();
    let args = Args::parse();
    let paths = match args.data_dir {
        Some(root) => DataPaths::from_root(root),
        None => DataPaths::discover()?,
    };
    let storage = Storage::open(paths.root.clone())?;
    let world = WorldId::from_str(&args.world).with_context(|| format!("invalid world ID: {}", args.world))?;
    tokio::runtime::Runtime::new()?.block_on(host::run(
        &paths,
        &storage,
        host::HostOptions {
            world,
            java: args.java,
            server_jar: args.server_jar,
            mod_jar: args.mod_jar,
            accept_eula: args.accept_eula,
        },
    ))
}
