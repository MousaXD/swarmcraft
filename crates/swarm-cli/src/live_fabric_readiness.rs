//! Strict decoder for the live, authenticated Fabric IPC readiness challenge.
//!
//! The response is NOT a quorum certificate or hosting permission. It is only
//! usable when received from the already authenticated, currently owned
//! `FabricSession` after sending a new unpredictable challenge. The daemon must
//! independently establish canonical signed quorum, membership, consent and
//! fencing, and must reject IPC session/process replacement before publishing
//! FreshAuthorityEvidence. Never call this on a cached file or network payload.

use anyhow::{bail, Context, Result};
use std::{path::Path, str::FromStr, time::Duration};
use swarm_ipc::FabricSession;
use swarm_protocol::{Hash32, WorldId};

const RUNTIME_PROOF_DOMAIN: &[u8] = b"swarmcraft/live-fabric-ipc-ready/v1\0";
const MAX_READY_LINE_BYTES: usize = 16 * 1024;

/// The expected port must come from the managed loopback listener reservation,
/// never `RuntimeLaunchConfig.game_endpoint` or a supplied remote IP.
#[derive(Debug, Clone)]
pub struct ExpectedFabricRuntime<'a> {
    pub world_id: WorldId,
    pub world_directory: &'a Path,
    pub compatibility_fingerprint: Hash32,
    pub managed_loopback_port: u16,
    pub request_id: u64,
    pub challenge_nonce: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedFabricRuntime {
    world_id: WorldId,
    port: u16,
    compatibility_fingerprint: Hash32,
    runtime_instance_nonce: [u8; 32],
    challenge_nonce: [u8; 32],
    ready_proof_hash: Hash32,
}

impl VerifiedFabricRuntime {
    /// Only the readiness reply validator in this module can create this
    /// evidence. Consumers may read verified fields but cannot mint one from
    /// stale configuration, migration status or a disconnected process.
    pub fn world_id(&self) -> WorldId {
        self.world_id
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn compatibility_fingerprint(&self) -> Hash32 {
        self.compatibility_fingerprint
    }

    pub fn runtime_instance_nonce(&self) -> [u8; 32] {
        self.runtime_instance_nonce
    }

    pub fn challenge_nonce(&self) -> [u8; 32] {
        self.challenge_nonce
    }

    pub fn ready_proof_hash(&self) -> Hash32 {
        self.ready_proof_hash
    }
}

/// Decode the reply to a fresh challenge on a live, token-authenticated IPC
/// session. This function does *not* authenticate the channel on its own.
pub fn verify_runtime_reply(line: &str, expected: &ExpectedFabricRuntime<'_>) -> Result<VerifiedFabricRuntime> {
    if line.len() > MAX_READY_LINE_BYTES
        || expected.world_id == WorldId::default()
        || expected.challenge_nonce == [0; 32]
        || expected.managed_loopback_port == 0
        || expected.compatibility_fingerprint == Hash32::default()
    {
        bail!("invalid Fabric readiness challenge context");
    }
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != 7 || fields[0] != "RUNTIME_READY" {
        bail!("unexpected Fabric readiness response");
    }
    if fields[1] != expected.request_id.to_string() || fields[2] != hex::encode(expected.challenge_nonce) {
        bail!("Fabric readiness request or challenge does not match");
    }
    let runtime_instance_nonce: [u8; 32] = hex::decode(fields[3])
        .context("invalid Fabric runtime instance nonce")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Fabric runtime instance nonce must be 32 bytes"))?;
    if runtime_instance_nonce == [0; 32] {
        bail!("Fabric runtime instance nonce is missing");
    }
    let port = fields[4].parse::<u16>().context("invalid Fabric listener port")?;
    if port != expected.managed_loopback_port {
        bail!("live Fabric port does not match reserved loopback listener");
    }
    let world_directory = String::from_utf8(hex::decode(fields[5]).context("invalid Fabric world path encoding")?)
        .context("Fabric world path is not UTF-8")?;
    if world_directory.is_empty() || Path::new(&world_directory) != expected.world_directory {
        bail!("live Fabric world directory does not match the managed world");
    }
    let compatibility_fingerprint = Hash32::from_str(fields[6]).context("invalid Fabric compatibility fingerprint")?;
    if compatibility_fingerprint != expected.compatibility_fingerprint {
        bail!("live Fabric compatibility differs from canonical world");
    }

    // The reply is challenge-bound by the exact comparison above. The stable
    // instance hash deliberately does NOT include the one-use challenge: route
    // offer and subsequent response verification must agree on the *same*
    // running Java instance across independent fresh challenges. Neither the
    // hash nor this decoder constitutes a quorum or consent authorization.
    let mut bytes = Vec::with_capacity(32 * 3 + world_directory.len() + 12);
    bytes.extend_from_slice(&expected.world_id.0);
    bytes.extend_from_slice(&runtime_instance_nonce);
    bytes.extend_from_slice(&port.to_be_bytes());
    bytes.extend_from_slice(&(world_directory.len() as u64).to_be_bytes());
    bytes.extend_from_slice(world_directory.as_bytes());
    bytes.extend_from_slice(&compatibility_fingerprint.0);
    Ok(VerifiedFabricRuntime {
        world_id: expected.world_id,
        port,
        compatibility_fingerprint,
        runtime_instance_nonce,
        challenge_nonce: expected.challenge_nonce,
        ready_proof_hash: Hash32::from_domain_bytes(RUNTIME_PROOF_DOMAIN, &bytes),
    })
}

/// Acquire the verified runtime instance only over the supervised controller's
/// EXISTING authenticated FabricSession, not a new socket or persisted status.
/// The supervisor must additionally check the managed Java PID, active permit,
/// consent and canonical authority both before and after this call. This is a
/// runtime signal, NOT a signed quorum or permission to expose a game route.
pub async fn challenge_authenticated_session(
    session: &mut FabricSession,
    expected: &ExpectedFabricRuntime<'_>,
    deadline: Duration,
) -> Result<VerifiedFabricRuntime> {
    if session.world_info().compatibility_fingerprint != expected.compatibility_fingerprint
        || Path::new(&session.world_info().world_directory) != expected.world_directory
    {
        bail!("authenticated Fabric session identity does not match managed runtime");
    }
    let reply = session.runtime_challenge(expected.request_id, expected.challenge_nonce, deadline).await?;
    verify_runtime_reply(&reply, expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected<'a>(world_directory: &'a Path) -> ExpectedFabricRuntime<'a> {
        ExpectedFabricRuntime {
            world_id: WorldId([7; 32]),
            world_directory,
            compatibility_fingerprint: Hash32([8; 32]),
            managed_loopback_port: 25567,
            request_id: 42,
            challenge_nonce: [9; 32],
        }
    }

    fn response() -> String {
        format!(
            "RUNTIME_READY\t42\t{}\t{}\t25567\t{}\t{}",
            hex::encode([9; 32]),
            hex::encode([10; 32]),
            hex::encode("/private/world"),
            Hash32([8; 32]),
        )
    }

    #[test]
    fn live_nonce_and_exact_world_are_required() {
        let directory = Path::new("/private/world");
        let proof = verify_runtime_reply(&response(), &expected(directory)).unwrap();
        assert_eq!(proof.runtime_instance_nonce, [10; 32]);
        assert_eq!(proof.port, 25567);
        assert_ne!(proof.ready_proof_hash, Hash32::default());

        let mut stale = expected(directory);
        stale.challenge_nonce = [11; 32];
        assert!(verify_runtime_reply(&response(), &stale).is_err());
        stale = expected(directory);
        stale.world_directory = Path::new("/private/other");
        assert!(verify_runtime_reply(&response(), &stale).is_err());
        stale = expected(directory);
        stale.managed_loopback_port = 25568;
        assert!(verify_runtime_reply(&response(), &stale).is_err());
        stale = expected(directory);
        stale.compatibility_fingerprint = Hash32([12; 32]);
        assert!(verify_runtime_reply(&response(), &stale).is_err());
        stale = expected(directory);
        stale.request_id = 43;
        assert!(verify_runtime_reply(&response(), &stale).is_err());
    }

    #[test]
    fn malformed_and_unauthenticated_shapes_cannot_count_as_ready() {
        let expected = expected(Path::new("/private/world"));
        for line in [
            "",
            "READY",
            "ERROR\t42\tRUNTIME_UNAVAILABLE",
            "RUNTIME_READY\t42",
            "RUNTIME_READY\t42\textra\tfields\tare\tnot\tproof\there",
        ] {
            assert!(verify_runtime_reply(line, &expected).is_err());
        }
        let zero_nonce = response().replace(&hex::encode([10; 32]), &hex::encode([0; 32]));
        assert!(verify_runtime_reply(&zero_nonce, &expected).is_err());
        assert!(verify_runtime_reply(&response().replace("25567", "25565"), &expected).is_err());
    }

    #[test]
    fn stable_instance_hash_is_not_a_reusable_challenge_response() {
        let directory = Path::new("/private/world");
        let first = verify_runtime_reply(&response(), &expected(directory)).unwrap();
        let mut second_expected = expected(directory);
        second_expected.challenge_nonce = [11; 32];
        second_expected.request_id = 43;
        let second_response = response().replacen("RUNTIME_READY\t42\t", "RUNTIME_READY\t43\t", 1).replacen(
            &hex::encode([9; 32]),
            &hex::encode([11; 32]),
            1,
        );
        let second = verify_runtime_reply(&second_response, &second_expected).unwrap();
        assert_eq!(first.ready_proof_hash, second.ready_proof_hash);
        assert_ne!(first.challenge_nonce, second.challenge_nonce);
        assert!(verify_runtime_reply(&response(), &second_expected).is_err());
    }
}
