//! Host-local real signed-quorum and authenticated Fabric proof collector.
//! A remote authority cannot be attested here: no network-cache/Ready fallback.

use super::live_quorum::LiveQuorumClient;
use super::remote_fabric::RemoteFabricRpcClient;
use anyhow::{anyhow, bail, Context, Result};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use swarm_cli::{
    live_authority::{
        canonical_quorum_hash, sign_live_host_fabric_attestation, v1_realm_hash, LiveAuthorityObservation,
        LiveAuthorityProofProvider, RuntimeCanonicalBinding, SignedHostFabricAttestation,
    },
    live_runtime_registry::LiveFabricRuntimeRegistry,
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_network::TransportPeerId;
use swarm_protocol::{
    DiscoveryCanonicalHeadV1, DiscoveryFreshnessChallengeV1, DiscoveryMembershipProofV1, EpochMode, EpochRecordV1,
    PeerId, SnapshotManifestV1, WorldConfigV1, WorldId, PROTOCOL_VERSION,
};
use swarm_storage::{Storage, StorageError};
use tokio::runtime::Handle;

const MAX_CHALLENGE_MS: u64 = 2_700;

struct SignedContext {
    membership: DiscoveryMembershipProofV1,
    epoch: EpochRecordV1,
    config: WorldConfigV1,
    snapshot: SnapshotManifestV1,
    quorum_challenge: DiscoveryFreshnessChallengeV1,
    canonical: RuntimeCanonicalBinding,
}

pub(super) struct DaemonLocalLiveAuthority {
    paths: DataPaths,
    application_peer: PeerId,
    transport_peer: TransportPeerId,
    quorum: LiveQuorumClient,
    remote_fabric: RemoteFabricRpcClient,
    runtime: LiveFabricRuntimeRegistry,
    reactor: Handle,
}

impl DaemonLocalLiveAuthority {
    pub fn new(
        paths: DataPaths,
        application_peer: PeerId,
        transport_peer: TransportPeerId,
        quorum: LiveQuorumClient,
        remote_fabric: RemoteFabricRpcClient,
        runtime: LiveFabricRuntimeRegistry,
        reactor: Handle,
    ) -> Self {
        Self { paths, application_peer, transport_peer, quorum, remote_fabric, runtime, reactor }
    }

    fn signed_context(&self, storage: &Storage, world: WorldId) -> Result<SignedContext> {
        let metadata = storage.load_world(world)?;
        let current_membership = storage.load_membership_record(world)?;
        let membership_certificates = storage.load_membership_certificate_chain(world)?;
        let initial_membership = membership_certificates
            .first()
            .map(|certificate| certificate.proposal.previous.clone())
            .unwrap_or_else(|| current_membership.clone());
        let pending_membership = match storage.load_membership_promise(world) {
            Ok(_) => bail!("pending canonical membership change blocks game routing"),
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let membership = DiscoveryMembershipProofV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            genesis: metadata.genesis,
            initial_membership,
            membership_certificates,
            current_membership,
            pending_membership,
        };
        let epoch = storage.load_epoch_record(world)?;
        let config = storage.load_world_config(world)?;
        let snapshot = storage.latest_snapshot(world)?.context("no accepted canonical checkpoint")?;
        if epoch.mode != EpochMode::Quorum {
            bail!("game routing requires a signed quorum authority generation");
        }
        let accepted = storage.canonical_snapshot_head(world)?.head.context("no accepted canonical snapshot head")?;
        let head = DiscoveryCanonicalHeadV1 {
            snapshot_number: accepted.snapshot_number,
            manifest_hash: accepted.manifest_hash,
            epoch: accepted.epoch,
            sequence: accepted.sequence,
        };
        let membership_hash = membership.current_membership.record_hash()?;
        let config_hash = config.config_hash()?;
        let canonical = RuntimeCanonicalBinding {
            membership_hash,
            config_hash,
            checkpoint_hash: head.manifest_hash,
            state_root: snapshot.state_root,
            epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
        };
        let quorum_challenge = DiscoveryFreshnessChallengeV1 {
            protocol_version: PROTOCOL_VERSION,
            verifier_peer_id: self.application_peer,
            nonce: [0; 32], // Replaced by caller's one-use challenge before requesting votes.
            world_id: world,
            announcement_hash: canonical_quorum_hash(world, membership_hash, &epoch, config_hash, &head)?,
            membership_sequence: membership.current_membership.sequence,
            membership_hash,
            pending_membership_proposal_hash: None,
            authority_peer_id: epoch.authority_peer_id,
            authority_epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
            config_sequence: config.sequence,
            config_hash,
            canonical_head: Some(head),
            issued_unix_ms: 0,
            expires_unix_ms: 0,
        };
        Ok(SignedContext { membership, epoch, config, snapshot, quorum_challenge, canonical })
    }

    /// An authenticated peer may request fresh host Fabric evidence, but this
    /// is never a replacement for independent majority-signed quorum proof.
    /// Execute on a bounded blocking worker, NOT the SwarmNode reactor.
    pub fn attest_remote_requester(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
    ) -> Result<SignedHostFabricAttestation> {
        if requester == PeerId::default() || world == WorldId::default() || nonce == [0; 32] {
            bail!("invalid host Fabric challenge");
        }
        let storage = Storage::open(self.paths.root.clone())?;
        let identity = PeerIdentity::load_or_create(&self.paths)?;
        if identity.peer_id() != self.application_peer {
            bail!("local signed host identity changed");
        }
        let before = self.signed_context(&storage, world)?;
        if before.epoch.authority_peer_id != self.application_peer {
            bail!("remote Fabric report may only be signed by its currently accepted authority");
        }
        if !before.membership.current_membership.members.iter().any(|member| {
            member.peer_id == requester
                && !member.banned
                && swarm_protocol::peer_id_from_public_key(&member.public_key) == requester
        }) {
            bail!("remote requester is not a canonical member");
        }
        // Challenge the original supervised, token-authenticated Fabric socket.
        // The live runtime registry itself enforces one-use nonces, advancing
        // permits, current process, consent and fence both before and after.
        let observed = self.reactor.block_on(self.runtime.challenge(world, nonce))?;
        if observed.identity.generation.epoch != before.epoch.epoch_number
            || observed.identity.generation.fencing_token != before.epoch.fencing_token
            || observed.identity.fingerprint != before.membership.genesis.compatibility_fingerprint
        {
            bail!("live managed JVM and accepted canonical authority generation differ");
        }
        let after = self.signed_context(&storage, world)?;
        if before.membership != after.membership
            || before.epoch != after.epoch
            || before.config != after.config
            || before.snapshot != after.snapshot
            || before.canonical != after.canonical
        {
            bail!("signed canonical state changed during Fabric IPC proof");
        }
        sign_live_host_fabric_attestation(
            &self.paths,
            &identity,
            &self.transport_peer,
            requester,
            v1_realm_hash(world),
            after.canonical,
            &observed.fabric,
        )
    }
}

impl LiveAuthorityProofProvider for DaemonLocalLiveAuthority {
    fn acquire(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
        deadline: Instant,
    ) -> Result<LiveAuthorityObservation> {
        if world == WorldId::default()
            || requester == PeerId::default()
            || nonce == [0; 32]
            || Instant::now() >= deadline
        {
            bail!("invalid or expired route proof request");
        }
        let storage = Storage::open(self.paths.root.clone())?;
        let identity = PeerIdentity::load_or_create(&self.paths)?;
        if identity.peer_id() != self.application_peer {
            bail!("daemon application identity changed");
        }
        let mut context = self.signed_context(&storage, world)?;
        if !context
            .membership
            .current_membership
            .members
            .iter()
            .any(|member| member.peer_id == requester && !member.banned)
        {
            bail!("requester is not a current canonical member");
        }
        let issued = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
        context.quorum_challenge.nonce = nonce;
        context.quorum_challenge.issued_unix_ms = issued;
        context.quorum_challenge.expires_unix_ms =
            issued.checked_add(MAX_CHALLENGE_MS).context("quorum challenge expiry overflow")?;
        let challenge = context.quorum_challenge.clone();
        // Both independent REAL sources are polled concurrently, staying
        // under the verifier's three-second monotonic acquisition budget.
        let (quorum_votes, host_fabric) = std::thread::scope(|scope| -> Result<_> {
            let vote_task = scope.spawn(|| self.quorum.collect_blocking(challenge));
            let host_fabric = if context.epoch.authority_peer_id == self.application_peer {
                let live = self.reactor.block_on(self.runtime.challenge(world, nonce))?;
                if live.identity.generation.epoch != context.canonical.epoch
                    || live.identity.generation.fencing_token != context.canonical.fencing_token
                    || live.identity.fingerprint != context.membership.genesis.compatibility_fingerprint
                {
                    bail!("managed Fabric JVM and signed canonical authority generation differ");
                }
                sign_live_host_fabric_attestation(
                    &self.paths,
                    &identity,
                    &self.transport_peer,
                    requester,
                    v1_realm_hash(world),
                    context.canonical,
                    &live.fabric,
                )?
            } else {
                self.remote_fabric.collect_blocking(world, context.epoch.authority_peer_id, nonce)?
            };
            let votes = vote_task.join().map_err(|_| anyhow!("quorum worker panicked"))??;
            Ok((votes, host_fabric))
        })?;
        if Instant::now() >= deadline {
            bail!("quorum or Fabric acquisition exceeded the monotonic deadline");
        }
        if Instant::now() >= deadline {
            bail!("host Fabric signature exceeded the monotonic deadline");
        }
        Ok(LiveAuthorityObservation {
            membership: context.membership,
            epoch: context.epoch,
            config: context.config,
            snapshot: context.snapshot,
            quorum_challenge: context.quorum_challenge,
            quorum_votes,
            host_fabric,
        })
    }
}
