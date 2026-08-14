//! SwarmCraft local Fabric IPC schema and authenticated loopback transport.

#[path = "lib.rs"]
mod schema;
pub use schema::*;

pub mod transport;
pub use transport::{FabricBridgeListener, FabricSession, FabricWorldInfo, IpcLaunchConfig, IpcTransportError};
