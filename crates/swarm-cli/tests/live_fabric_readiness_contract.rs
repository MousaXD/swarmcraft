// Agent 1 owns the adapter but Agent 3 owns shared CLI module wiring.
// Compile and exercise the standalone adapter without editing lib.rs/main.rs.
#[path = "../src/live_fabric_readiness.rs"]
mod live_fabric_readiness;

pub use swarm_cli::hosting_budget;
#[path = "../src/live_authority.rs"]
mod live_authority;
