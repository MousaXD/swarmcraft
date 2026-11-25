#[path = "lib.rs"]
mod base;
pub use base::*;

pub mod hosting_policy_migration;
pub mod membership;
pub mod migration;
pub mod recovery;
pub mod simulator;
pub mod solo;
pub mod v2_migration_quorum;

// A non-activatable design model for a possible, separate two-device policy.
#[cfg(test)]
mod exclusive_turn_model;

pub use membership::*;
pub use recovery::*;
pub use solo::*;

pub mod ownerless;

pub mod ownerless_authority;
