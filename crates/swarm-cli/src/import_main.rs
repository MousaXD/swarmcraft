use anyhow::{bail, Result};
use clap::Parser;
use std::path::PathBuf;
use swarm_cli::world_import::{import_reviewed_world, import_world, preview_import, ImportWorldRequest};
use swarm_core::DataPaths;
use swarm_protocol::WorldVisibilityV1;

#[derive(Debug, Parser)]
#[command(name = "swarmcraft-import", version, about = "Safely import an existing Minecraft save into SwarmCraft")]
struct Args {
    /// Override the standard OS-local SwarmCraft data directory.
    #[arg(long, env = "SWARMCRAFT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Existing Minecraft world directory. The source is never mutated.
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    name: String,
    /// Exact Minecraft version used by this save.
    #[arg(long)]
    minecraft: String,
    /// Exact Fabric Loader version required when this world is hosted.
    #[arg(long)]
    fabric_loader: String,
    /// private, unlisted, or public.
    #[arg(long, default_value = "private")]
    visibility: String,
    /// Exact third-party Fabric server mods required by the imported world.
    #[arg(long = "server-mod")]
    server_mods: Vec<PathBuf>,
    /// Explicitly declare that the imported world requires no third-party server mods.
    #[arg(long, conflicts_with = "server_mods")]
    no_server_mods: bool,
    /// Inspect source and compatibility without creating or changing a world.
    #[arg(long, conflicts_with = "expected_compatibility")]
    preview: bool,
    /// Require the exact compatibility fingerprint from the reviewed preview.
    #[arg(long)]
    expected_compatibility: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let request = ImportWorldRequest {
        source: args.source,
        name: args.name,
        minecraft_version: args.minecraft,
        fabric_loader_version: args.fabric_loader,
        visibility: parse_visibility(&args.visibility)?,
        server_mod_jars: args.server_mods,
        confirm_no_server_mods: args.no_server_mods,
    };
    if args.preview {
        println!("{}", serde_json::to_string_pretty(&preview_import(&request))?);
        return Ok(());
    }
    let paths = match args.data_dir {
        Some(root) => DataPaths::from_root(root),
        None => DataPaths::discover()?,
    };
    let result = match args.expected_compatibility.as_deref() {
        Some(expected) => import_reviewed_world(&paths, &request, expected)?,
        None => import_world(&paths, &request)?,
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn parse_visibility(value: &str) -> Result<WorldVisibilityV1> {
    match value.to_ascii_lowercase().as_str() {
        "private" => Ok(WorldVisibilityV1::Private),
        "unlisted" => Ok(WorldVisibilityV1::Unlisted),
        "public" => Ok(WorldVisibilityV1::Public),
        _ => bail!("visibility must be private, unlisted, or public"),
    }
}
