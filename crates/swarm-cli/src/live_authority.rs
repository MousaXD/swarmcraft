//! Signed, challenge-bound V1 quorum + Fabric authority verification.
//!
//! This is the trust adapter for the signed game-route protocol. The proof
//! provider must ask ACTUAL authenticated peers to sign a fresh challenge and
//! obtain the authority's signed, live Fabric IPC result. Discovery endpoints,
//! persisted Ready reports and configured Minecraft IPs are never evidence.
//!
//! Agent 3 owns the live network/event-loop provider and shared CLI module
//! wiring. Without such a provider, no instance of this adapter can be used.

use anyhow::{anyhow, bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use swarm_consensus::{
    membership::{validate_discovery_freshness_quorum, validate_discovery_membership_proof_shape},
    recovery::validate_recovery_certificate_shape,
};
use swarm_core::{
    discovery::{
        verify_discovery_freshness_vote, MAX_DISCOVERY_FRESHNESS_VOTES, MAX_DISCOVERY_MEMBERS,
        MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES,
    },
    protocol_v2::verify_world_config_signature,
    verify_membership_signature, verify_signature, verify_snapshot_signature, DataPaths, PeerIdentity,
};
use swarm_network::{
    game_route::{FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError},
    game_transport::GameAuthenticatedPeers,
    TransportPeerId,
};
use swarm_protocol::{
    peer_id_from_public_key, DiscoveryCanonicalHeadV1, DiscoveryFreshnessChallengeV1, DiscoveryFreshnessVoteV1,
    DiscoveryMembershipProofV1, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, PeerId, SnapshotManifestV1,
    WorldConfigV1, WorldId, PROTOCOL_VERSION,
};
use swarm_storage::{CanonicalSnapshotRefV1, Storage, StorageError};

use crate::{hosting_budget, live_fabric_readiness::VerifiedFabricRuntime};

const REALM_DOMAIN: &[u8] = b"swarmcraft/game-v1-realm/v1\0";
const QUORUM_CONTEXT_DOMAIN: &[u8] = b"swarmcraft/game-live-quorum-context/v1\0";
const CANONICAL_CERT_DOMAIN: &[u8] = b"swarmcraft/game-live-canonical-certificate/v1\0";
const HOST_FABRIC_SIGN_DOMAIN: &[u8] = b"swarmcraft/game-host-fabric-attestation/v1\0";
const MAX_FRESH_PROOF_MS: u64 = 3_000;
const MAX_SIGNED_RUNTIME_MS: u64 = 3_000;
const CLOCK_SKEW_MS: u64 = 500;

/// Exact, signed canonical head that was used to restore and authorize this
/// runtime instance. The daemon must derive it from current accepted records,
/// not from the launched process or a saved migration status report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCanonicalBinding {
    pub membership_hash: Hash32,
    pub config_hash: Hash32,
    pub checkpoint_hash: Hash32,
    pub state_root: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
}

/// The authority signs this only after receiving a live, nonce-bound result
/// from the *currently owned* token-authenticated FabricSession. A verifier
/// never accepts this signed attestation as a replacement for live quorum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedHostFabricAttestation {
    pub world_id: WorldId,
    pub realm_hash: Hash32,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub authority_transport_peer: Vec<u8>,
    pub requester_peer_id: PeerId,
    pub challenge_nonce: [u8; 32],
    pub canonical: RuntimeCanonicalBinding,
    pub compatibility_fingerprint: Hash32,
    pub runtime_instance_nonce: [u8; 32],
    pub runtime_ready_proof_hash: Hash32,
    pub game_server_port: u16,
    pub consent_revision: u64,
    pub consent_heap_mib: u64,
    pub issued_unix_ms: u64,
    pub expires_unix_ms: u64,
    pub signature: Vec<u8>,
}

impl SignedHostFabricAttestation {
    fn signing_bytes(&self) -> Result<Vec<u8>> {
        let unsigned = (
            (
                self.world_id,
                self.realm_hash,
                self.authority_peer_id,
                self.authority_public_key,
                &self.authority_transport_peer,
                self.requester_peer_id,
                self.challenge_nonce,
                self.canonical,
            ),
            (
                self.compatibility_fingerprint,
                self.runtime_instance_nonce,
                self.runtime_ready_proof_hash,
                self.game_server_port,
                self.consent_revision,
                self.consent_heap_mib,
                self.issued_unix_ms,
                self.expires_unix_ms,
            ),
        );
        let bytes = postcard::to_allocvec(&unsigned)?;
        if bytes.len() > 1024 {
            bail!("unbounded host Fabric attestation");
        }
        let mut signed = Vec::with_capacity(HOST_FABRIC_SIGN_DOMAIN.len() + bytes.len());
        signed.extend_from_slice(HOST_FABRIC_SIGN_DOMAIN);
        signed.extend_from_slice(&bytes);
        Ok(signed)
    }

    /// Verify the authority signature and its bounded, current lifetime.
    /// This alone is never a quorum, consent or canonical runtime permit.
    pub fn verify(&self, now: u64) -> Result<()> {
        if self.world_id == WorldId::default()
            || self.realm_hash == Hash32::default()
            || self.authority_peer_id == PeerId::default()
            || self.requester_peer_id == PeerId::default()
            || self.challenge_nonce == [0; 32]
            || self.canonical.membership_hash == Hash32::default()
            || self.canonical.config_hash == Hash32::default()
            || self.canonical.checkpoint_hash == Hash32::default()
            || self.canonical.state_root == Hash32::default()
            || self.canonical.epoch == 0
            || self.canonical.fencing_token == 0
            || self.runtime_instance_nonce == [0; 32]
            || self.runtime_ready_proof_hash == Hash32::default()
            || self.game_server_port == 0
            || self.consent_revision == 0
            || !(hosting_budget::MIN_HEAP_MIB..=hosting_budget::MAX_HEAP_MIB).contains(&self.consent_heap_mib)
            || self.authority_transport_peer.is_empty()
            || self.authority_transport_peer.len() > 128
            || self.expires_unix_ms <= self.issued_unix_ms
            || self.expires_unix_ms - self.issued_unix_ms > MAX_SIGNED_RUNTIME_MS
            || self.issued_unix_ms > now.saturating_add(CLOCK_SKEW_MS)
            || self.expires_unix_ms <= now
        {
            bail!("invalid, stale or unconsented host Fabric attestation");
        }
        verify_signature(self.authority_peer_id, self.authority_public_key, &self.signing_bytes()?, &self.signature)
            .context("invalid host Fabric attestation signature")
    }
}

/// Host-only action. The caller must issue a fresh challenge through its actual
/// FabricSession. This method loads LOCAL consent while holding the existing
/// consent-update lock: a stale consent object cannot be passed by its caller.
/// A persisted or synthetically constructed Ready cannot satisfy that contract.
#[allow(clippy::too_many_arguments)]
pub fn sign_live_host_fabric_attestation(
    paths: &DataPaths,
    identity: &PeerIdentity,
    transport_peer: &TransportPeerId,
    requester: PeerId,
    realm_hash: Hash32,
    canonical: RuntimeCanonicalBinding,
    fabric: &VerifiedFabricRuntime,
) -> Result<SignedHostFabricAttestation> {
    // The same cross-process lock is used by hosting_budget::set_contribution.
    // A concurrent revocation must win the lock before this proof is signed or
    // must be published afterwards (and invalidate subsequent rechecks).
    // Nonblocking acquisition refuses instead of holding a proof worker
    // indefinitely behind a slow consent update.
    let consent_lock_path = paths.root.join("control").join("hosting-consent-write.lock");
    let consent_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&consent_lock_path)
        .with_context(|| format!("cannot open hosting consent update lock {}", consent_lock_path.display()))?;
    consent_lock.try_lock_exclusive().context("hosting consent is being updated; refuse attestation")?;
    let consent = hosting_budget::load_consent(paths, fabric.world_id())?
        .context("hosting consent disappeared; cannot sign host readiness")?;
    if consent.world_id != fabric.world_id()
        || consent.peer_id != identity.peer_id()
        || !consent.enabled
        || consent.revision == 0
        || !(hosting_budget::MIN_HEAP_MIB..=hosting_budget::MAX_HEAP_MIB).contains(&consent.heap_mib)
        || realm_hash == Hash32::default()
        || canonical.membership_hash == Hash32::default()
        || canonical.config_hash == Hash32::default()
        || canonical.checkpoint_hash == Hash32::default()
        || canonical.state_root == Hash32::default()
        || canonical.epoch == 0
        || canonical.fencing_token == 0
        || requester == PeerId::default()
        || fabric.challenge_nonce() == [0; 32]
        || fabric.port() == 0
    {
        bail!("host Fabric readiness does not match unwithdrawn hosting consent");
    }
    // Caller-supplied timestamps are not a freshness oracle. Derive issuance
    // immediately before the signature from this machine's current clock.
    let issued_unix_ms = unix_ms()?;
    let mut attestation = SignedHostFabricAttestation {
        world_id: fabric.world_id(),
        realm_hash,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        authority_transport_peer: transport_peer.to_bytes(),
        requester_peer_id: requester,
        challenge_nonce: fabric.challenge_nonce(),
        canonical,
        compatibility_fingerprint: fabric.compatibility_fingerprint(),
        runtime_instance_nonce: fabric.runtime_instance_nonce(),
        runtime_ready_proof_hash: fabric.ready_proof_hash(),
        game_server_port: fabric.port(),
        consent_revision: consent.revision,
        consent_heap_mib: consent.heap_mib,
        issued_unix_ms,
        expires_unix_ms: issued_unix_ms
            .checked_add(MAX_SIGNED_RUNTIME_MS)
            .context("host attestation clock overflow")?,
        signature: Vec::new(),
    };
    attestation.signature = identity.sign(&attestation.signing_bytes()?);
    drop(consent_lock);
    Ok(attestation)
}

/// Raw signed material from actual authenticated peers; not a provider's
/// "safe" boolean. The source independently verifies all records and votes.
#[derive(Debug, Clone)]
pub struct LiveAuthorityObservation {
    pub membership: DiscoveryMembershipProofV1,
    pub epoch: EpochRecordV1,
    pub config: WorldConfigV1,
    pub snapshot: SnapshotManifestV1,
    pub quorum_challenge: DiscoveryFreshnessChallengeV1,
    pub quorum_votes: Vec<DiscoveryFreshnessVoteV1>,
    pub host_fabric: SignedHostFabricAttestation,
}

/// The daemon owner must obtain new signatures over the supplied nonce for
/// EVERY call. It MUST use authenticated P2P and actual live Fabric IPC (for
/// host attestation). The caller passes an absolute MONOTONIC deadline for
/// acquisition plus verification, so each blocking network/IPC step MUST use
/// a bounded timeout within that budget and return before the deadline. This
/// synchronous trait cannot interrupt a wedged provider by itself. Returning
/// cached votes fails the nonce check. Returning no material refuses the route.
pub trait LiveAuthorityProofProvider: Send + Sync {
    fn acquire(
        &self,
        world: WorldId,
        requester: PeerId,
        challenge_nonce: [u8; 32],
        deadline: Instant,
    ) -> Result<LiveAuthorityObservation>;
}

/// A fully signed V1 authority proof source; no synthetic Ready fallback.
///
/// The injected provider supplies *data*, not authorization. A real daemon
/// implementation must stay attached to its continuously polled SwarmNode
/// and live Fabric session. A private world never needs a public announcement.
pub struct SignedLiveAuthoritySource {
    paths: DataPaths,
    local_application_peer: PeerId,
    local_transport_peer: TransportPeerId,
    authenticated: GameAuthenticatedPeers,
    provider: Arc<dyn LiveAuthorityProofProvider>,
}

impl SignedLiveAuthoritySource {
    pub fn new(
        paths: DataPaths,
        local_application_peer: PeerId,
        local_transport_peer: TransportPeerId,
        authenticated: GameAuthenticatedPeers,
        provider: Arc<dyn LiveAuthorityProofProvider>,
    ) -> Self {
        Self { paths, local_application_peer, local_transport_peer, authenticated, provider }
    }

    fn verify_current(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
        proof: &LiveAuthorityObservation,
    ) -> Result<FreshAuthorityEvidence> {
        let now = unix_ms()?;
        let storage = Storage::open(self.paths.root.clone())?;
        let identity = PeerIdentity::load_or_create(&self.paths)?;
        if identity.peer_id() != self.local_application_peer || nonce == [0; 32] || world == WorldId::default() {
            bail!("local identity, world or proof challenge is invalid");
        }
        if proof.membership.world_id != world
            || proof.membership.protocol_version != PROTOCOL_VERSION
            || proof.membership.genesis.world_id()? != world
            || proof.membership.initial_membership.world_id != world
            || proof.membership.current_membership.world_id != world
            || proof.epoch.world_id != world
            || proof.config.world_id != world
            || proof.snapshot.world_id != world
        {
            bail!("live quorum material is not for this exact world");
        }
        proof.membership.genesis.validate_semantics()?;
        verify_genesis_anchored_membership(&proof.membership)?;
        let head = require_current_canonical_generation(&storage, world, proof)?;

        proof.epoch.validate_semantics()?;
        verify_signature(
            proof.epoch.authority_peer_id,
            proof.epoch.authority_public_key,
            &proof.epoch.signing_bytes()?,
            &proof.epoch.signature,
        )?;
        verify_world_config_signature(&proof.config)?;
        verify_snapshot_signature(&proof.snapshot)?;
        // A signed manifest alone does not prove that this replica still has
        // the exact readable checkpoint bytes. Validate the complete local
        // blob set before authorizing a route. This may be expensive for large
        // worlds, but a slow check is denied by the caller's proof deadline;
        // it must never silently degrade into a cached Ready assertion.
        storage.verify_snapshot(&proof.snapshot)?;
        let checkpoint_hash = proof.snapshot.manifest_hash()?;
        if head.snapshot_number != proof.snapshot.snapshot_number
            || head.manifest_hash != checkpoint_hash
            || head.epoch != proof.snapshot.epoch
            || head.sequence != proof.snapshot.sequence
            || proof.snapshot.epoch > proof.epoch.epoch_number
            || proof.config.authority_peer_id != proof.epoch.authority_peer_id
            || proof.config.authority_public_key != proof.epoch.authority_public_key
            || proof.membership.current_membership.authority_peer_id != proof.epoch.authority_peer_id
            || proof.membership.current_membership.authority_public_key != proof.epoch.authority_public_key
            || proof.membership.current_membership.epoch != proof.epoch.epoch_number
            || proof.config.compatibility_fingerprint()? != proof.membership.genesis.compatibility_fingerprint
        {
            bail!("checkpoint, membership, authority and configuration are not canonical");
        }
        match proof.epoch.mode {
            // The P0 route is quorum backed, not an implicit solo lease.
            EpochMode::Quorum => {}
            EpochMode::Recovery => verify_current_recovery_certificate(&storage, &proof.epoch, &proof.membership)?,
            EpochMode::Solo => bail!("solo-mode authority cannot authorize the multi-peer game route"),
        }
        let members = &proof.membership.current_membership.members;
        let authority = members
            .iter()
            .find(|member| member.peer_id == proof.epoch.authority_peer_id)
            .context("authority is not a current member")?;
        if authority.banned
            || !authority.authority_eligible
            || authority.public_key != proof.epoch.authority_public_key
            || !members.iter().any(|member| member.peer_id == requester && !member.banned)
        {
            bail!("requester or authority is not eligible in the current canonical membership");
        }
        let transport = TransportPeerId::from_bytes(&proof.host_fabric.authority_transport_peer)
            .context("host attestation contains an invalid transport identity")?;
        let authenticated = if proof.epoch.authority_peer_id == self.local_application_peer {
            transport == self.local_transport_peer
        } else {
            self.authenticated
                .read()
                .map_err(|_| anyhow!("authenticated-peer registry is unavailable"))?
                .get(&transport)
                .copied()
                == Some(proof.epoch.authority_peer_id)
        };
        if !authenticated {
            bail!("host transport has no live application-authenticated session");
        }

        let membership_hash = proof.membership.current_membership.record_hash()?;
        let config_hash = proof.config.config_hash()?;
        let canonical_head = DiscoveryCanonicalHeadV1 {
            snapshot_number: head.snapshot_number,
            manifest_hash: checkpoint_hash,
            epoch: head.epoch,
            sequence: head.sequence,
        };
        let quorum_hash = canonical_quorum_hash(world, membership_hash, &proof.epoch, config_hash, &canonical_head)?;
        let challenge = &proof.quorum_challenge;
        if challenge.protocol_version != PROTOCOL_VERSION
            || challenge.verifier_peer_id != self.local_application_peer
            || challenge.nonce != nonce
            || challenge.world_id != world
            || challenge.announcement_hash != quorum_hash
            || challenge.membership_sequence != proof.membership.current_membership.sequence
            || challenge.membership_hash != membership_hash
            || challenge.pending_membership_proposal_hash
                != proof.membership.pending_membership.as_ref().map(|p| p.proposal_hash()).transpose()?
            || challenge.authority_peer_id != proof.epoch.authority_peer_id
            || challenge.authority_epoch != proof.epoch.epoch_number
            || challenge.fencing_token != proof.epoch.fencing_token
            || challenge.config_sequence != proof.config.sequence
            || challenge.config_hash != config_hash
            || challenge.canonical_head != Some(canonical_head)
            || challenge.expires_unix_ms <= challenge.issued_unix_ms
            || challenge.expires_unix_ms - challenge.issued_unix_ms > MAX_FRESH_PROOF_MS
            || challenge.issued_unix_ms > now.saturating_add(CLOCK_SKEW_MS)
            || challenge.expires_unix_ms <= now
        {
            bail!("live canonical quorum challenge is stale, wrong-world or mismatched");
        }
        for vote in &proof.quorum_votes {
            verify_discovery_freshness_vote(vote, challenge).map_err(|e| anyhow!(e))?;
        }
        validate_discovery_freshness_quorum(&proof.membership, &proof.quorum_votes)?;

        let host = &proof.host_fabric;
        host.verify(now)?;
        let realm_hash = v1_realm_hash(world);
        let expected_runtime_canonical = RuntimeCanonicalBinding {
            membership_hash,
            config_hash,
            checkpoint_hash,
            state_root: proof.snapshot.state_root,
            epoch: proof.epoch.epoch_number,
            fencing_token: proof.epoch.fencing_token,
        };
        if host.world_id != world
            || host.realm_hash != realm_hash
            || host.authority_peer_id != proof.epoch.authority_peer_id
            || host.authority_public_key != proof.epoch.authority_public_key
            || host.requester_peer_id != requester
            || host.challenge_nonce != nonce
            || host.canonical != expected_runtime_canonical
            || host.compatibility_fingerprint != proof.config.compatibility_fingerprint()?
        {
            bail!("host Fabric attestation is not bound to current signed quorum");
        }
        if proof.epoch.authority_peer_id == self.local_application_peer {
            // A local authority must independently re-read its own consent.
            let consent =
                hosting_budget::load_consent(&self.paths, world)?.context("local hosting consent is absent")?;
            if !consent.enabled
                || consent.revision != host.consent_revision
                || consent.heap_mib != host.consent_heap_mib
            {
                bail!("local hosting consent was revised or withdrawn");
            }
        }
        let valid_until = host.expires_unix_ms.min(challenge.expires_unix_ms);
        let certificate_hash = Hash32::from_domain_bytes(
            CANONICAL_CERT_DOMAIN,
            &postcard::to_allocvec(&(quorum_hash, checkpoint_hash, proof.snapshot.state_root))?,
        );
        // Verify the same durable generation AGAIN after potentially slow
        // snapshot, signature, quorum and Fabric checks. In particular, a new
        // transfer, higher recovery promise, membership/config update or sleep
        // must not race the proof and leave an old generation authorized.
        if require_current_canonical_generation(&storage, world, proof)? != head {
            bail!("canonical head changed while verifying the live authority");
        }
        let host_still_authenticated = if proof.epoch.authority_peer_id == self.local_application_peer {
            transport == self.local_transport_peer
        } else {
            self.authenticated
                .read()
                .map_err(|_| anyhow!("authenticated-peer registry is unavailable"))?
                .get(&transport)
                .copied()
                == Some(proof.epoch.authority_peer_id)
        };
        if !host_still_authenticated {
            bail!("host authentication was withdrawn during proof verification");
        }
        if proof.epoch.authority_peer_id == self.local_application_peer {
            let consent =
                hosting_budget::load_consent(&self.paths, world)?.context("local hosting consent is absent")?;
            if !consent.enabled
                || consent.revision != host.consent_revision
                || consent.heap_mib != host.consent_heap_mib
            {
                bail!("local hosting consent changed during proof verification");
            }
        }
        if proof.epoch.mode == EpochMode::Recovery {
            verify_current_recovery_certificate(&storage, &proof.epoch, &proof.membership)?;
        }
        let verified_at = unix_ms()?;
        if verified_at >= valid_until || verified_at > now.saturating_add(MAX_FRESH_PROOF_MS) {
            bail!("quorum or runtime proof expired during final generation recheck");
        }
        Ok(FreshAuthorityEvidence {
            world_id: world,
            realm_hash,
            authority_peer_id: proof.epoch.authority_peer_id,
            authority_public_key: proof.epoch.authority_public_key,
            authority_transport_peer: transport,
            game_server_port: host.game_server_port,
            requester_is_member: true,
            authority_is_member: true,
            conflict_free: true,
            hosting_consent_valid: true,
            runtime_ready_verified: true,
            membership_hash,
            quorum_certificate_hash: certificate_hash,
            compatibility_fingerprint: host.compatibility_fingerprint,
            checkpoint_hash,
            state_root: proof.snapshot.state_root,
            epoch: proof.epoch.epoch_number,
            fencing_token: proof.epoch.fencing_token,
            consent_revision: host.consent_revision,
            runtime_instance_nonce: host.runtime_instance_nonce,
            runtime_ready_proof_hash: host.runtime_ready_proof_hash,
            challenge_nonce: nonce,
            verified_at_unix_ms: verified_at,
            valid_until_unix_ms: valid_until,
        })
    }
}

/// Compare the entire accepted canonical generation and every durable fence
/// both before and after verification. Never accept a missing/corrupt file as
/// permission to fall back to an earlier epoch or a persisted Ready status.
fn require_current_canonical_generation(
    storage: &Storage,
    world: WorldId,
    proof: &LiveAuthorityObservation,
) -> Result<CanonicalSnapshotRefV1> {
    if storage.load_world(world)?.genesis != proof.membership.genesis
        || storage.load_genesis_membership(world)? != proof.membership.initial_membership
        || storage.load_membership_record(world)? != proof.membership.current_membership
        || storage.load_membership_certificate_chain(world)? != proof.membership.membership_certificates
        || storage.load_epoch_record(world)? != proof.epoch
        || storage.load_world_config(world)? != proof.config
        || storage.latest_snapshot(world)?.as_ref() != Some(&proof.snapshot)
    {
        bail!("live authority transcript differs from locally accepted canonical records");
    }
    let head = storage.canonical_snapshot_head(world)?.head.context("no accepted canonical snapshot head")?;
    let current_pending = match storage.load_membership_promise(world) {
        Ok(value) => Some(value.proposal),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if current_pending != proof.membership.pending_membership {
        bail!("pending membership does not match durable promise");
    }
    match storage.load_sleep_record(world) {
        Ok(_) => bail!("world is durably sleeping"),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // Signed votes cannot silently hide a surviving solo branch or an
    // unresolved conflict preserved on this device. A corrupt conflict
    // ledger is also a denial, not an empty history.
    if !storage.list_solo_conflicts(world)?.is_empty() {
        bail!("unresolved solo world-history conflicts block the game route");
    }
    match storage.load_solo_branch(world) {
        Ok(_) => bail!("solo-unreplicated world history cannot authorize a quorum game route"),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match storage.load_recovery_promise(world) {
        Ok(promise)
            if (promise.ballot.target_epoch, promise.ballot.target_fencing_token)
                > (proof.epoch.epoch_number, proof.epoch.fencing_token) =>
        {
            bail!("higher recovery promise fences the offered authority")
        }
        Ok(_) => {}
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match storage.load_recovery_reservation(world) {
        Ok(reservation) => {
            swarm_core::verify_lease_signature(&reservation)?;
            if (reservation.epoch, reservation.fencing_token) > (proof.epoch.epoch_number, proof.epoch.fencing_token)
                || (reservation.epoch, reservation.fencing_token)
                    == (proof.epoch.epoch_number, proof.epoch.fencing_token)
                    && reservation.authority_peer_id != proof.epoch.authority_peer_id
            {
                bail!("a recovery reservation fences this authority");
            }
        }
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match storage.load_transfer_record(world) {
        Ok(transfer) => {
            swarm_core::verify_transfer_signature(&transfer)?;
            // Every active phase fences its exact source generation. A
            // historical completed handoff cannot fence a later authority
            // simply because its former source was elected again.
            let accepted = (proof.epoch.epoch_number, proof.epoch.fencing_token);
            let successor = (transfer.next_epoch, transfer.next_fencing_token);
            let completed = transfer.phase == swarm_protocol::TransferPhase::Committed
                && ((accepted == successor && proof.epoch.authority_peer_id == transfer.to_peer_id)
                    || (accepted.0 > successor.0 && accepted.1 > successor.1));
            if !completed {
                bail!("active, incomplete or inconsistent signed handoff fences this game route");
            }
        }
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(head)
}

impl FreshAuthoritySource for SignedLiveAuthoritySource {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        challenge: [u8; 32],
    ) -> std::result::Result<FreshAuthorityEvidence, GameRouteError> {
        if world == WorldId::default() || requester == PeerId::default() || challenge == [0; 32] {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        let started = Instant::now();
        let deadline =
            started.checked_add(Duration::from_millis(MAX_FRESH_PROOF_MS)).ok_or(GameRouteError::StaleAuthority)?;
        let observed = self.provider.acquire(world, requester, challenge, deadline).map_err(|error| {
            tracing::debug!(%world, %error, "independent live authority acquisition refused");
            GameRouteError::UnverifiedAuthority
        })?;
        require_monotonic_proof_deadline(started)?;
        let verified = self.verify_current(world, requester, challenge, &observed).map_err(|error| {
            tracing::debug!(%world, %error, "independent live authority verification refused");
            GameRouteError::UnverifiedAuthority
        })?;
        // Local snapshot verification and certificate validation are also
        // part of the proof budget. Wall-clock expiry alone is not enough:
        // a backwards clock jump must not authorize a slow/stale proof.
        require_monotonic_proof_deadline(started)?;
        Ok(verified)
    }
}

fn require_monotonic_proof_deadline(started: Instant) -> std::result::Result<(), GameRouteError> {
    if started.elapsed() >= Duration::from_millis(MAX_FRESH_PROOF_MS) {
        return Err(GameRouteError::StaleAuthority);
    }
    Ok(())
}

/// P0 V1 realms are anchored in the signed genesis world identifier. Unknown
/// V2 realm transitions are not silently reinterpreted as V1 game worlds.
pub fn v1_realm_hash(world: WorldId) -> Hash32 {
    Hash32::from_domain_bytes(REALM_DOMAIN, &world.0)
}

/// Stable digest of the signed canonical state. A live quorum must still
/// provide *new signatures* over the one-use discovery freshness challenge
/// carrying this digest; the digest by itself is never authority.
pub fn canonical_quorum_hash(
    world: WorldId,
    membership_hash: Hash32,
    epoch: &EpochRecordV1,
    config_hash: Hash32,
    head: &DiscoveryCanonicalHeadV1,
) -> Result<Hash32> {
    Ok(Hash32::from_domain_bytes(
        QUORUM_CONTEXT_DOMAIN,
        &postcard::to_allocvec(&(
            world,
            membership_hash,
            epoch.authority_peer_id,
            epoch.epoch_number,
            epoch.fencing_token,
            config_hash,
            head,
        ))?,
    ))
}

fn verify_genesis_anchored_membership(proof: &DiscoveryMembershipProofV1) -> Result<()> {
    if proof.protocol_version != PROTOCOL_VERSION || proof.genesis.world_id()? != proof.world_id {
        bail!("membership proof does not match signed world genesis");
    }
    // Refuse overlarge and noncanonical histories BEFORE expensive signature
    // checks, including on worlds which have not been publicly advertised.
    // The private-world route follows the same maximum proof envelope as
    // public discovery.
    if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES
        || proof.initial_membership.members.len() > MAX_DISCOVERY_MEMBERS
        || proof.current_membership.members.len() > MAX_DISCOVERY_MEMBERS
        || proof.membership_certificates.iter().any(|cert| {
            cert.proposal.previous.members.len() > MAX_DISCOVERY_MEMBERS
                || cert.proposal.proposed.members.len() > MAX_DISCOVERY_MEMBERS
                || cert.votes.len() > MAX_DISCOVERY_FRESHNESS_VOTES
        })
        || proof.pending_membership.as_ref().is_some_and(|pending| {
            pending.previous.members.len() > MAX_DISCOVERY_MEMBERS
                || pending.proposed.members.len() > MAX_DISCOVERY_MEMBERS
        })
    {
        bail!("live membership history exceeds supported proof limits");
    }
    validate_discovery_membership_proof_shape(proof)?;
    let initial = &proof.initial_membership;
    if initial.protocol_version != PROTOCOL_VERSION
        || initial.sequence != 0
        || initial.previous_membership_hash.is_some()
        || initial.authority_public_key != proof.genesis.creator_public_key
        || initial.authority_peer_id != peer_id_from_public_key(&proof.genesis.creator_public_key)
        || initial.members.len() != proof.genesis.initial_membership.len()
        || initial.members.iter().map(|member| member.peer_id).collect::<BTreeSet<_>>()
            != proof.genesis.initial_membership.iter().copied().collect::<BTreeSet<_>>()
    {
        bail!("membership history is not anchored to world genesis");
    }
    verify_membership_record_and_keys(initial)?;
    for certificate in &proof.membership_certificates {
        verify_membership_record_and_keys(&certificate.proposal.previous)?;
        verify_membership_record_and_keys(&certificate.proposal.proposed)?;
        let mut seen_voters = BTreeSet::new();
        for vote in &certificate.votes {
            // Legacy committed certificates are assembled from a HashMap,
            // so their vote order is not guaranteed. Uniqueness and exact
            // signatures matter; imposing sorted storage would strand real
            // accepted histories without making the quorum stronger.
            if !seen_voters.insert(vote.voter_peer_id)
                || peer_id_from_public_key(&vote.voter_public_key) != vote.voter_peer_id
                || !vote.matches_proposal(&certificate.proposal)?
            {
                bail!("membership certificate has mismatched or repeated voters");
            }
            verify_signature(vote.voter_peer_id, vote.voter_public_key, &vote.signing_bytes()?, &vote.signature)?;
        }
    }
    verify_membership_record_and_keys(&proof.current_membership)?;
    if let Some(pending) = &proof.pending_membership {
        verify_membership_record_and_keys(&pending.previous)?;
        verify_membership_record_and_keys(&pending.proposed)?;
    }
    Ok(())
}

fn verify_membership_record_and_keys(record: &MembershipRecordV1) -> Result<()> {
    record.validate_semantics()?;
    verify_membership_signature(record)?;
    for member in &record.members {
        if member.peer_id != peer_id_from_public_key(&member.public_key) {
            bail!("membership contains a key that does not match its canonical peer ID");
        }
    }
    Ok(())
}

pub fn verify_current_recovery_certificate(
    storage: &Storage,
    epoch: &EpochRecordV1,
    membership: &DiscoveryMembershipProofV1,
) -> Result<()> {
    let certificate = storage.load_recovery_certificate(epoch.world_id)?;
    let ballot = &certificate.ballot;
    swarm_core::protocol_v2::verify_recovery_ballot_signature(ballot)?;
    // A recovery ballot is signed BEFORE promotion of the membership
    // authority/epoch. Its hash must refer to that exact archived record,
    // not the successor's newly signed membership record.
    let base = storage.load_membership_by_hash(epoch.world_id, ballot.membership_hash)?;
    verify_membership_record_and_keys(&base)?;
    let base_proof = DiscoveryMembershipProofV1 {
        current_membership: base.clone(),
        membership_certificates: membership
            .membership_certificates
            .iter()
            .filter(|certificate| certificate.proposal.proposed.sequence <= base.sequence)
            .cloned()
            .collect(),
        pending_membership: None,
        ..membership.clone()
    };
    verify_genesis_anchored_membership(&base_proof)?;
    let candidate = base
        .members
        .iter()
        .find(|member| member.peer_id == ballot.candidate_peer_id)
        .context("recovery candidate was not in the certified voter set")?;
    if candidate.banned || !candidate.authority_eligible || candidate.public_key != ballot.candidate_public_key {
        bail!("recovery candidate was not eligible in the certified voter set");
    }
    let mut seen = BTreeSet::new();
    for vote in &certificate.votes {
        swarm_core::protocol_v2::verify_recovery_vote_signature(vote)?;
        if !seen.insert(vote.voter_peer_id)
            || !base.members.iter().any(|member| {
                !member.banned && member.peer_id == vote.voter_peer_id && member.public_key == vote.voter_public_key
            })
        {
            bail!("recovery vote does not identify a unique canonical voter");
        }
    }
    let voters = base.members.iter().filter(|member| !member.banned).map(|member| member.peer_id).collect::<Vec<_>>();
    validate_recovery_certificate_shape(&certificate, &voters)?;
    if ballot.world_id != epoch.world_id
        || ballot.target_epoch != epoch.epoch_number
        || ballot.target_fencing_token != epoch.fencing_token
        || ballot.candidate_peer_id != epoch.authority_peer_id
        || ballot.candidate_public_key != epoch.authority_public_key
        || ballot.base_state_hash != epoch.base_state_hash
        || base.epoch != ballot.base_epoch
        || base.record_hash()? != ballot.membership_hash
        // P0 recovery keeps the original voter set. A later membership
        // transition requires a separately certified playable generation.
        || base.members != membership.current_membership.members
        || membership.current_membership.epoch != epoch.epoch_number
        || membership.current_membership.previous_membership_hash != Some(ballot.membership_hash)
        || base.sequence.checked_add(1) != Some(membership.current_membership.sequence)
    {
        bail!("recovery certificate does not authorize this canonical generation");
    }
    Ok(())
}

/// Shared denial-only generation gate. Recovery is playable only with the
/// original durable certificate, the exact pre-promotion membership and the
/// genesis-anchored current membership. Callers must STILL collect fresh
/// quorum votes and live authenticated Fabric evidence for every route.
pub fn verify_playable_epoch(storage: &Storage, epoch: &EpochRecordV1) -> Result<()> {
    if storage.load_epoch_record(epoch.world_id)? != *epoch {
        bail!("playable epoch is no longer the locally accepted generation");
    }
    verify_signature(epoch.authority_peer_id, epoch.authority_public_key, &epoch.signing_bytes()?, &epoch.signature)?;
    if epoch.mode == EpochMode::Quorum {
        return Ok(());
    }
    if epoch.mode != EpochMode::Recovery {
        bail!("solo authority cannot grant a multiplayer route");
    }
    let world = epoch.world_id;
    let proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis: storage.load_world(world)?.genesis,
        initial_membership: storage.load_genesis_membership(world)?,
        membership_certificates: storage.load_membership_certificate_chain(world)?,
        current_membership: storage.load_membership_record(world)?,
        pending_membership: None,
    };
    verify_genesis_anchored_membership(&proof)?;
    if proof.current_membership.authority_peer_id != epoch.authority_peer_id
        || proof.current_membership.authority_public_key != epoch.authority_public_key
    {
        bail!("recovery membership does not match the accepted authority");
    }
    verify_current_recovery_certificate(storage, epoch, &proof)
}

fn unix_ms() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
#[path = "live_authority_tests.rs"]
mod tests;
