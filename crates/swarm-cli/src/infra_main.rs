//! Explicit opt-in infrastructure executable. Never opens a player world
//! store, loads membership, or starts a Minecraft runtime.
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use swarm_network::infrastructure::{serve_locator, InfrastructureState, PrivateRelay, PrivateRelayConfig};

#[derive(Parser)]
#[command(name = "swarmcraft-infra", version, about = "SwarmCraft locator and private transport relay")]
struct Args {
    /// Dedicated private service directory, NOT a SwarmCraft player data directory.
    #[arg(long)]
    state_dir: PathBuf,
    #[command(subcommand)]
    role: Role,
}

#[derive(Subcommand)]
enum Role {
    /// Run a locator-only DHT node with no world or presence publication.
    Locator {
        #[arg(long)]
        listen: String,
        #[arg(long)]
        advertise: Option<String>,
    },
    /// Run an allowlisted, bounded TCP/Noise relay.
    Relay {
        #[arg(long)]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    // Intentionally no tracing subscriber: detailed peer logs are
    // inappropriate for shared infrastructure service logs.
    match args.role {
        Role::Locator { listen, advertise } => {
            let state = InfrastructureState::open(&args.state_dir, "locator")?;
            serve_locator(&state, &listen, advertise.as_deref()).await
        }
        Role::Relay { config } => {
            let config = PrivateRelayConfig::load(&config)?;
            let state = InfrastructureState::open(&args.state_dir, "relay")?;
            let relay = PrivateRelay::bind(state.transport_key()?, &config).await?;
            println!("Relay address: {}", relay.advertised_address());
            relay.run().await
        }
    }
}
