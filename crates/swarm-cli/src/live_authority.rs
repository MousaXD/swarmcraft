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
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use swarm_consensus::{
    membership::{validate_discovery_freshness_quorum, validate_discovery_membership_proof_shape},
    recovery::validate_recovery_certificate_shape,
};
use swarm_core::{
    discovery::verify_discovery_freshness_vote, protocol_v2::verify_world_config_signature,
    verify_membership_signature, verify_signature, verify_snapshot_signature, DataPaths, PeerIdentity,
};
use swarm_network::{
    game_route::{FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError},
    game_transport::GameAuthenticatedPeers,
    TransportPeerId,
};
use swarm_protocol::{
    peer_id_from_public_key, DiscoveryCanonicalHeadV1, DiscoveryFreshnessChallengeV1, DiscoveryFreshnessVoteV1,
    DiscoveryMembershipProofV1, EpochMode, EpochRecordV1, Hash32, PeerId, SnapshotManifestV1, WorldConfigV1, WorldId,
    PROTOCOL_VERSION,
};
use swarm_storage::{Storage, StorageError};

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

    fn verify(&self, now: u64) -> Result<()> {
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

/// Host-only action. The caller must first read and verify local current
/// hosting consent and issue a fresh challenge through its actual FabricSession.
/// A persisted or synthetically constructed Ready cannot satisfy that contract.
#[allow(clippy::too_many_arguments)]
pub fn sign_live_host_fabric_attestation(
    identity: &PeerIdentity,
    transport_peer: &TransportPeerId,
    requester: PeerId,
    realm_hash: Hash32,
    canonical: RuntimeCanonicalBinding,
    consent: &hosting_budget::HostingConsentV1,
    fabric: &VerifiedFabricRuntime,
    now: u64,
) -> Result<SignedHostFabricAttestation> {
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
        issued_unix_ms: now,
        expires_unix_ms: now.checked_add(MAX_SIGNED_RUNTIME_MS).context("host attestation clock overflow")?,
        signature: Vec::new(),
    };
    attestation.signature = identity.sign(&attestation.signing_bytes()?);
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
/// host attestation); deadlines/retries are bounded independently. Returning
/// cached votes fails the nonce check. Returning no material refuses the route.
pub trait LiveAuthorityProofProvider: Send + Sync {
    fn acquire(&self, world: WorldId, requester: PeerId, challenge_nonce: [u8; 32])
        -> Result<LiveAuthorityObservation>;
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
        if storage.load_world(world)?.genesis != proof.membership.genesis
            || storage.load_membership_record(world)? != proof.membership.current_membership
            || storage.load_membership_certificate_chain(world)? != proof.membership.membership_certificates
            || storage.load_epoch_record(world)? != proof.epoch
            || storage.load_world_config(world)? != proof.config
            || storage.latest_snapshot(world)?.as_ref() != Some(&proof.snapshot)
        {
            bail!("live authority transcript differs from locally accepted canonical records");
        }
        let current_pending = match storage.load_membership_promise(world) {
            Ok(value) => Some(value.proposal),
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if current_pending != proof.membership.pending_membership {
            bail!("pending membership does not match durable promise");
        }
        // A present sleep record can never be treated as an awake authority.
        match storage.load_sleep_record(world) {
            Ok(_) => bail!("world is durably sleeping"),
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // A promised higher recovery round/fence cannot be ignored even when
        // the old epoch file still exists.
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
                if (reservation.epoch, reservation.fencing_token)
                    > (proof.epoch.epoch_number, proof.epoch.fencing_token)
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
                if transfer.phase == swarm_protocol::TransferPhase::Committed
                    && transfer.from_peer_id == proof.epoch.authority_peer_id
                {
                    bail!("the source authority has already committed a transfer");
                }
            }
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

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
        let head = storage.canonical_snapshot_head(world)?.head.context("no accepted canonical snapshot head")?;
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
        let verified_at = unix_ms()?;
        if verified_at >= valid_until || verified_at > now.saturating_add(MAX_FRESH_PROOF_MS) {
            bail!("quorum or Fabric proof expired while verifying");
        }
        let certificate_hash = Hash32::from_domain_bytes(
            CANONICAL_CERT_DOMAIN,
            &postcard::to_allocvec(&(quorum_hash, checkpoint_hash, proof.snapshot.state_root))?,
        );
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
        let observed =
            self.provider.acquire(world, requester, challenge).map_err(|_| GameRouteError::UnverifiedAuthority)?;
        if started.elapsed() >= Duration::from_millis(MAX_FRESH_PROOF_MS) {
            return Err(GameRouteError::StaleAuthority);
        }
        self.verify_current(world, requester, challenge, &observed).map_err(|_| GameRouteError::UnverifiedAuthority)
    }
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
    verify_membership_signature(initial)?;
    for member in &initial.members {
        if member.peer_id != peer_id_from_public_key(&member.public_key) {
            bail!("initial member key and peer ID do not match");
        }
    }
    for certificate in &proof.membership_certificates {
        verify_membership_signature(&certificate.proposal.previous)?;
        verify_membership_signature(&certificate.proposal.proposed)?;
        for vote in &certificate.votes {
            verify_signature(vote.voter_peer_id, vote.voter_public_key, &vote.signing_bytes()?, &vote.signature)?;
        }
    }
    verify_membership_signature(&proof.current_membership)?;
    if let Some(pending) = &proof.pending_membership {
        verify_membership_signature(&pending.previous)?;
        verify_membership_signature(&pending.proposed)?;
    }
    for member in &proof.current_membership.members {
        if member.peer_id != peer_id_from_public_key(&member.public_key) {
            bail!("current member key does not match its canonical peer ID");
        }
    }
    Ok(())
}

fn verify_current_recovery_certificate(
    storage: &Storage,
    epoch: &EpochRecordV1,
    membership: &DiscoveryMembershipProofV1,
) -> Result<()> {
    let certificate = storage.load_recovery_certificate(epoch.world_id)?;
    let ballot = &certificate.ballot;
    swarm_core::protocol_v2::verify_recovery_ballot_signature(ballot)?;
    for vote in &certificate.votes {
        swarm_core::protocol_v2::verify_recovery_vote_signature(vote)?;
    }
    let voters = membership
        .current_membership
        .members
        .iter()
        .filter(|member| !member.banned)
        .map(|member| member.peer_id)
        .collect::<Vec<_>>();
    validate_recovery_certificate_shape(&certificate, &voters)?;
    if ballot.world_id != epoch.world_id
        || ballot.target_epoch != epoch.epoch_number
        || ballot.target_fencing_token != epoch.fencing_token
        || ballot.candidate_peer_id != epoch.authority_peer_id
        || ballot.candidate_public_key != epoch.authority_public_key
        || ballot.membership_hash != membership.current_membership.record_hash()?
    {
        bail!("recovery certificate does not authorize this canonical generation");
    }
    Ok(())
}

fn unix_ms() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
#[path = "live_authority_tests.rs"]
mod tests;
