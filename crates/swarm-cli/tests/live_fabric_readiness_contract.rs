// The live Fabric adapter is now part of the real CLI library. Reuse that
// compiled module for this legacy scoped authority test so the test does not
// compile a duplicate unused authenticated-session entrypoint. The library
// suite independently exercises the adapter's one-use challenge tests.
pub use swarm_cli::{hosting_budget, live_fabric_readiness};
#[path = "../src/live_authority.rs"]
mod live_authority;
