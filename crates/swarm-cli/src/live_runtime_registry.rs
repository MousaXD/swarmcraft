//! In-process rendezvous with a JVM owned by the migration supervisor.
//!
//! A game-route proof worker must NEVER open a second Fabric control socket,
//! trust a PID from a status file, or use an old Ready report. Instead it sends
//! a bounded one-use challenge to the supervisor that owns the authenticated
//! FabricSession and the managed Child handle. The supervisor independently
//! checks the current permit, generation, consent and PID before replying.
//! This registry is transport for live evidence, NOT route authorization.

use crate::live_fabric_readiness::VerifiedFabricRuntime;
use anyhow::{anyhow, bail, Context, Result};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use swarm_consensus::AuthorityGeneration;
use swarm_protocol::{Hash32, WorldId};
use tokio::sync::{mpsc, oneshot};

const CHALLENGE_RESPONSE_WAIT: Duration = Duration::from_millis(1_900);
const COMMAND_QUEUE: usize = 4;

#[derive(Debug)]
pub struct RuntimeChallengeCommand {
    pub request_id: u64,
    pub nonce: [u8; 32],
    pub response: oneshot::Sender<Result<VerifiedFabricRuntime>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveRuntimeIdentity {
    pub world: WorldId,
    pub pid: u32,
    pub generation: AuthorityGeneration,
    pub fingerprint: Hash32,
    pub managed_port: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct SupervisedFabricEvidence {
    pub identity: LiveRuntimeIdentity,
    pub fabric: VerifiedFabricRuntime,
}

struct LiveEntry {
    registration_id: u64,
    identity: LiveRuntimeIdentity,
    sender: mpsc::Sender<RuntimeChallengeCommand>,
}

#[derive(Clone, Default)]
pub struct LiveFabricRuntimeRegistry {
    entries: Arc<Mutex<HashMap<WorldId, LiveEntry>>>,
    sequence: Arc<AtomicU64>,
}

pub struct LiveRuntimeRegistration {
    world: WorldId,
    registration_id: u64,
    registry: LiveFabricRuntimeRegistry,
}

impl Drop for LiveRuntimeRegistration {
    fn drop(&mut self) {
        // A former JVM must not unregister a newer process's registration.
        if let Ok(mut entries) = self.registry.entries.lock() {
            if entries.get(&self.world).is_some_and(|entry| entry.registration_id == self.registration_id) {
                entries.remove(&self.world);
            }
        }
    }
}

impl LiveFabricRuntimeRegistry {
    pub fn register(
        &self,
        identity: LiveRuntimeIdentity,
    ) -> Result<(LiveRuntimeRegistration, mpsc::Receiver<RuntimeChallengeCommand>)> {
        if identity.world == WorldId::default()
            || identity.pid == 0
            || identity.generation.epoch == 0
            || identity.generation.fencing_token == 0
            || identity.fingerprint == Hash32::default()
            || identity.managed_port == 0
        {
            bail!("invalid managed Fabric registration identity");
        }
        let id = self.next_id()?;
        let (sender, receiver) = mpsc::channel(COMMAND_QUEUE);
        let mut entries = self.entries.lock().map_err(|_| anyhow!("Fabric session registry unavailable"))?;
        if entries.contains_key(&identity.world) {
            bail!("another live Fabric session is already registered for this world");
        }
        entries.insert(identity.world, LiveEntry { registration_id: id, identity, sender });
        Ok((LiveRuntimeRegistration { world: identity.world, registration_id: id, registry: self.clone() }, receiver))
    }

    pub async fn challenge(&self, world: WorldId, nonce: [u8; 32]) -> Result<SupervisedFabricEvidence> {
        if world == WorldId::default() || nonce == [0; 32] {
            bail!("invalid live Fabric challenge");
        }
        let (registration_id, identity, sender) = {
            let entries = self.entries.lock().map_err(|_| anyhow!("Fabric session registry unavailable"))?;
            let entry = entries.get(&world).context("no live managed Fabric process registered for this world")?;
            (entry.registration_id, entry.identity, entry.sender.clone())
        };
        let request_id = self.next_id()?;
        let (reply_sender, reply_receiver) = oneshot::channel();
        sender
            .try_send(RuntimeChallengeCommand { request_id, nonce, response: reply_sender })
            .map_err(|_| anyhow!("managed Fabric controller is unavailable or busy"))?;
        let proof = tokio::time::timeout(CHALLENGE_RESPONSE_WAIT, reply_receiver)
            .await
            .context("live Fabric supervisor challenge timed out")?
            .context("live Fabric supervisor dropped its active session")??;
        if proof.world_id() != world
            || proof.port() != identity.managed_port
            || proof.compatibility_fingerprint() != identity.fingerprint
            || proof.challenge_nonce() != nonce
        {
            bail!("live Fabric response does not match its registered managed process");
        }
        // Check after the round trip: a process swap during the challenge
        // invalidates the old signed/unsigned runtime evidence immediately.
        let entries = self.entries.lock().map_err(|_| anyhow!("Fabric session registry unavailable"))?;
        if !entries
            .get(&world)
            .is_some_and(|entry| entry.registration_id == registration_id && entry.identity == identity)
        {
            bail!("managed Fabric runtime was replaced during proof collection");
        }
        Ok(SupervisedFabricEvidence { identity, fabric: proof })
    }

    fn next_id(&self) -> Result<u64> {
        let previous = self
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| anyhow!("live Fabric request sequence exhausted"))?;
        Ok(previous + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> LiveRuntimeIdentity {
        LiveRuntimeIdentity {
            world: WorldId([5; 32]),
            pid: 1234,
            generation: AuthorityGeneration { epoch: 3, fencing_token: 8 },
            fingerprint: Hash32([7; 32]),
            managed_port: 25567,
        }
    }

    #[tokio::test]
    async fn nonexistent_and_zero_nonce_challenges_fail_closed() {
        let registry = LiveFabricRuntimeRegistry::default();
        assert!(registry.challenge(identity().world, [1; 32]).await.is_err());
        let (_registration, _rx) = registry.register(identity()).unwrap();
        assert!(registry.challenge(identity().world, [0; 32]).await.is_err());
        assert!(registry.register(identity()).is_err());
    }

    #[tokio::test]
    async fn stale_registration_and_missing_runtime_reply_refuse() {
        let registry = LiveFabricRuntimeRegistry::default();
        let (registration, mut commands) = registry.register(identity()).unwrap();
        let task = tokio::spawn({
            let registry = registry.clone();
            async move { registry.challenge(identity().world, [1; 32]).await }
        });
        let command = commands.recv().await.unwrap();
        assert_eq!(command.nonce, [1; 32]);
        assert_ne!(command.request_id, 0);
        drop(registration);
        let _ = command.response.send(Err(anyhow!("supervisor observed stopped JVM")));
        assert!(task.await.unwrap().is_err());
        assert!(registry.challenge(identity().world, [2; 32]).await.is_err());
        let (_replacement, _rx) = registry.register(identity()).unwrap();
    }
}
