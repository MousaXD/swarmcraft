//! Bounded request-mailbox for a remote authority's LIVE, signed Fabric IPC
//! attestation. It does not grant a route. A requester must independently
//! obtain live majority signatures, verify this authority signature, and
//! compare every canonical world/epoch/consent/runtime field before routing.

use anyhow::{anyhow, bail, Result};
use std::{sync::mpsc as sync_mpsc, time::Duration};
use swarm_cli::live_authority::SignedHostFabricAttestation;
use swarm_protocol::{PeerId, WorldId};
use tokio::sync::mpsc;

pub(super) const REMOTE_HOST_RPC_CAPACITY: usize = 4;
pub(super) const REMOTE_HOST_RPC_TIMEOUT: Duration = Duration::from_millis(2_550);

pub(super) struct RemoteFabricRpcCommand {
    pub world: WorldId,
    pub authority: PeerId,
    pub nonce: [u8; 32],
    pub response: sync_mpsc::SyncSender<Result<SignedHostFabricAttestation>>,
}

#[derive(Clone)]
pub(super) struct RemoteFabricRpcClient {
    sender: mpsc::Sender<RemoteFabricRpcCommand>,
}

impl RemoteFabricRpcClient {
    pub fn collect_blocking(
        &self,
        world: WorldId,
        authority: PeerId,
        nonce: [u8; 32],
    ) -> Result<SignedHostFabricAttestation> {
        if world == WorldId::default() || authority == PeerId::default() || nonce == [0; 32] {
            bail!("invalid remote host Fabric proof request");
        }
        let (response, receiver) = sync_mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteFabricRpcCommand { world, authority, nonce, response })
            .map_err(|_| anyhow!("authenticated remote authority RPC unavailable or saturated"))?;
        receiver
            .recv_timeout(REMOTE_HOST_RPC_TIMEOUT)
            .map_err(|_| anyhow!("remote live Fabric proof did not arrive within the deadline"))?
    }
}

pub(super) fn channel() -> (RemoteFabricRpcClient, mpsc::Receiver<RemoteFabricRpcCommand>) {
    let (sender, receiver) = mpsc::channel(REMOTE_HOST_RPC_CAPACITY);
    (RemoteFabricRpcClient { sender }, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_requester_refuses_without_fallback() {
        let (client, mut rx) = channel();
        rx.close();
        assert!(client.collect_blocking(WorldId([1; 32]), PeerId([2; 32]), [3; 32]).is_err());
        assert!(client.collect_blocking(WorldId([1; 32]), PeerId([2; 32]), [0; 32]).is_err());
    }
}
