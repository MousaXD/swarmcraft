//! Host-side, fail-closed route offer and challenge-response signing.
//!
//! This does NOT establish quorum or Fabric readiness. It only signs if a
//! trusted FreshAuthoritySource independently verifies current signed quorum,
//! requester membership, the active host's consent and an authenticated live
//! Fabric runtime for a fresh nonce. No production trust-source adapter exists
//! in the network crate. This handler MUST NOT use persisted Ready or IP hints.

use crate::{
    game_route::{
        ensure_private_floor_directory, private_floor_options, replace_floor_atomically, sync_directory,
        FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError, MinecraftRouteOfferV1, MinecraftRouteResponseV1,
        GAME_ROUTE_VERSION, MAX_FRESH_EVIDENCE_AGE_MS, MAX_FRESH_EVIDENCE_LEASE_MS, MAX_ROUTE_OFFER_AGE_MS,
        MAX_ROUTE_RESPONSE_AGE_MS,
    },
    game_route_control::{GameRouteControlRequestV1, GameRouteControlResponseV1, GameRouteRefusalV1},
};
use ed25519_dalek::SigningKey;
use fs2::FileExt;
use libp2p::PeerId as TransportPeerId;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerId, WorldId};

const OFFER_SEQUENCER_VERSION: u16 = 1;
const MAX_ISSUED_OFFERS: usize = 256;
const MAX_OFFER_SEQUENCE_BYTES: u64 = 128;
const CLOCK_SKEW_MS: u64 = 2_000;

/// A separate, persistent per-world monotonic SIGNING sequence. Persisting
/// the client's accepted generation floor is not enough: the host must also
/// avoid reusing sequence #1 after its own process restarts.
pub struct DurableHostOfferSequencer {
    directory: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct HostSequenceRecord {
    version: u16,
    world_id: WorldId,
    realm_hash: Hash32,
    epoch: u64,
    fencing_token: u64,
    next_sequence: u64,
}

impl DurableHostOfferSequencer {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self { directory: directory.into() }
    }

    pub fn reserve_next(
        &self,
        world_id: WorldId,
        realm_hash: Hash32,
        epoch: u64,
        fencing_token: u64,
    ) -> Result<u64, GameRouteError> {
        if world_id == WorldId::default() || realm_hash == Hash32::default() || epoch == 0 || fencing_token == 0 {
            return Err(GameRouteError::InvalidTranscript);
        }
        ensure_private_floor_directory(&self.directory)?;
        let name = world_id.to_hex();
        let lock_path = self.directory.join(format!("{name}.sequence.lock"));
        let path = self.directory.join(format!("{name}.sequence"));
        crate::game_durable_path::validate_private_state_file(&lock_path)
            .map_err(|_| GameRouteError::InvalidTranscript)?;
        let lock = private_floor_options().read(true).write(true).create(true).truncate(false).open(&lock_path)?;
        lock.lock_exclusive()?;
        let result = (|| {
            let exists = crate::game_durable_path::validate_private_state_file(&path)
                .map_err(|_| GameRouteError::InvalidTranscript)?;
            let previous = if exists {
                let mut bytes = Vec::new();
                File::open(&path)?.take(MAX_OFFER_SEQUENCE_BYTES + 1).read_to_end(&mut bytes)?;
                if bytes.is_empty() || bytes.len() as u64 > MAX_OFFER_SEQUENCE_BYTES {
                    return Err(GameRouteError::InvalidTranscript);
                }
                let (value, remainder) = postcard::take_from_bytes::<HostSequenceRecord>(&bytes)
                    .map_err(|_| GameRouteError::InvalidTranscript)?;
                if !remainder.is_empty() {
                    return Err(GameRouteError::InvalidTranscript);
                }
                Some(value)
            } else {
                None
            };
            let sequence = match previous {
                Some(old) => {
                    if old.version != OFFER_SEQUENCER_VERSION
                        || old.world_id != world_id
                        || old.realm_hash != realm_hash
                        || old.epoch == 0
                        || old.fencing_token == 0
                        || old.next_sequence == 0
                        || epoch < old.epoch
                        || fencing_token < old.fencing_token
                        || (epoch == old.epoch && fencing_token != old.fencing_token)
                        || (epoch > old.epoch && fencing_token <= old.fencing_token)
                    {
                        return Err(GameRouteError::Downgrade);
                    }
                    if epoch == old.epoch {
                        old.next_sequence.checked_add(1).ok_or(GameRouteError::InvalidTranscript)?
                    } else {
                        1
                    }
                }
                None => 1,
            };
            let next = HostSequenceRecord {
                version: OFFER_SEQUENCER_VERSION,
                world_id,
                realm_hash,
                epoch,
                fencing_token,
                next_sequence: sequence,
            };
            let bytes = postcard::to_allocvec(&next).map_err(|_| GameRouteError::InvalidTranscript)?;
            let mut salt = [0u8; 8];
            OsRng.fill_bytes(&mut salt);
            let tmp = self.directory.join(format!(".{name}-{}.tmp", u64::from_be_bytes(salt)));
            let mut file = private_floor_options().write(true).create_new(true).open(&tmp)?;
            let commit = (|| -> Result<(), GameRouteError> {
                file.write_all(&bytes)?;
                file.sync_all()?;
                drop(file);
                replace_floor_atomically(&tmp, &path)?;
                sync_directory(&self.directory)?;
                Ok(())
            })();
            if commit.is_err() {
                let _ = fs::remove_file(&tmp);
            }
            commit?;
            Ok(sequence)
        })();
        let unlock = FileExt::unlock(&lock).map_err(GameRouteError::Storage);
        unlock?;
        result
    }
}

struct IssuedOffer {
    offer: MinecraftRouteOfferV1,
    requester: PeerId,
    requester_transport: TransportPeerId,
}

/// Use this service inside the hosting peer's authenticated route-control
/// handler. The source is intentionally injected and mandatory: without a
/// certified current quorum/Fabric adapter this cannot sign a route.
pub struct AuthenticatedGameRouteHostSigner {
    key: SigningKey,
    application_peer: PeerId,
    transport_peer: TransportPeerId,
    authority: Arc<dyn FreshAuthoritySource>,
    sequencer: DurableHostOfferSequencer,
    issued: Mutex<HashMap<Hash32, IssuedOffer>>,
}

impl AuthenticatedGameRouteHostSigner {
    pub fn new(
        key: SigningKey,
        transport_peer: TransportPeerId,
        authority: Arc<dyn FreshAuthoritySource>,
        sequence_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            application_peer: peer_id_from_public_key(&key.verifying_key().to_bytes()),
            key,
            transport_peer,
            authority,
            sequencer: DurableHostOfferSequencer::new(sequence_directory),
            issued: Mutex::new(HashMap::new()),
        }
    }

    /// Called ONLY for an application-authenticated inbound control event.
    /// Bind the exact sender identity supplied by the SwarmNode handshake;
    /// do not take an identity from the untrusted request body.
    pub fn answer(
        &self,
        requester: PeerId,
        requester_transport: TransportPeerId,
        request: GameRouteControlRequestV1,
    ) -> Result<GameRouteControlResponseV1, GameRouteError> {
        if request.validate().is_err() {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ExpiredOrMalformed));
        }
        if requester == PeerId::default() {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unauthorized));
        }
        match request {
            GameRouteControlRequestV1::Offer { world_id } => self.issue_offer(world_id, requester, requester_transport),
            GameRouteControlRequestV1::Challenge(challenge) => {
                self.answer_challenge(requester, requester_transport, *challenge)
            }
        }
    }

    fn evidence(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
        now: u64,
    ) -> Result<FreshAuthorityEvidence, GameRouteRefusalV1> {
        let evidence =
            self.authority.verify_fresh(world, requester, nonce).map_err(|_| GameRouteRefusalV1::QuorumUnavailable)?;
        if evidence.world_id != world
            || evidence.challenge_nonce != nonce
            || evidence.authority_peer_id != self.application_peer
            || evidence.authority_public_key != self.key.verifying_key().to_bytes()
            || evidence.authority_transport_peer != self.transport_peer
        {
            return Err(GameRouteRefusalV1::FencedOrConsentWithdrawn);
        }
        if !evidence.requester_is_member || !evidence.authority_is_member {
            return Err(GameRouteRefusalV1::Unauthorized);
        }
        if !evidence.conflict_free {
            return Err(GameRouteRefusalV1::ConflictingAuthority);
        }
        if !evidence.hosting_consent_valid {
            return Err(GameRouteRefusalV1::FencedOrConsentWithdrawn);
        }
        if !evidence.runtime_ready_verified {
            return Err(GameRouteRefusalV1::HostUnready);
        }
        if evidence.verified_at_unix_ms > now.saturating_add(CLOCK_SKEW_MS)
            || now.saturating_sub(evidence.verified_at_unix_ms) > MAX_FRESH_EVIDENCE_AGE_MS
            || evidence.valid_until_unix_ms <= now
            || evidence.valid_until_unix_ms > now.saturating_add(MAX_FRESH_EVIDENCE_LEASE_MS)
        {
            return Err(GameRouteRefusalV1::ExpiredOrMalformed);
        }
        Ok(evidence)
    }

    fn issue_offer(
        &self,
        world: WorldId,
        requester: PeerId,
        requester_transport: TransportPeerId,
    ) -> Result<GameRouteControlResponseV1, GameRouteError> {
        let now = unix_ms()?;
        let mut proof_nonce = [0; 32];
        OsRng.fill_bytes(&mut proof_nonce);
        let evidence = match self.evidence(world, requester, proof_nonce, now) {
            Ok(proof) => proof,
            Err(reason) => return Ok(GameRouteControlResponseV1::Refused(reason)),
        };
        let mut issued = self.issued.lock().map_err(|_| GameRouteError::StaleAuthority)?;
        issued.retain(|_, pending| pending.offer.expires_unix_ms > now);
        if issued.len() >= MAX_ISSUED_OFFERS {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::RateLimited));
        }
        // A durable increasing sequence is required even after host process
        // death; no offer is signed before the updated sequence reaches disk.
        let sequence =
            self.sequencer.reserve_next(world, evidence.realm_hash, evidence.epoch, evidence.fencing_token)?;
        // The disk commit above may block behind another writer. Re-prove
        // current quorum/consent/runtime with an independent nonce after it
        // completes, and refuse to sign the original head if anything moved.
        let signed_at = unix_ms()?;
        let mut second_nonce = [0; 32];
        OsRng.fill_bytes(&mut second_nonce);
        let post_commit = match self.evidence(world, requester, second_nonce, signed_at) {
            Ok(proof) => proof,
            Err(reason) => return Ok(GameRouteControlResponseV1::Refused(reason)),
        };
        if !same_authority_state(&evidence, &post_commit) {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::FencedOrConsentWithdrawn));
        }
        let expires = signed_at
            .saturating_add(MAX_ROUTE_OFFER_AGE_MS)
            .min(evidence.valid_until_unix_ms)
            .min(post_commit.valid_until_unix_ms);
        if expires <= signed_at {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ExpiredOrMalformed));
        }
        let mut offer_id = [0; 32];
        OsRng.fill_bytes(&mut offer_id);
        let mut offer = MinecraftRouteOfferV1 {
            version: GAME_ROUTE_VERSION,
            world_id: world,
            realm_hash: evidence.realm_hash,
            authority_peer_id: self.application_peer,
            authority_public_key: self.key.verifying_key().to_bytes(),
            authority_transport_peer: self.transport_peer.to_bytes(),
            game_server_port: evidence.game_server_port,
            membership_hash: evidence.membership_hash,
            quorum_certificate_hash: evidence.quorum_certificate_hash,
            compatibility_fingerprint: evidence.compatibility_fingerprint,
            checkpoint_hash: evidence.checkpoint_hash,
            state_root: evidence.state_root,
            epoch: evidence.epoch,
            fencing_token: evidence.fencing_token,
            consent_revision: evidence.consent_revision,
            runtime_instance_nonce: evidence.runtime_instance_nonce,
            runtime_ready_proof_hash: evidence.runtime_ready_proof_hash,
            offer_id,
            sequence,
            issued_unix_ms: signed_at,
            expires_unix_ms: expires,
            signature: Vec::new(),
        };
        offer.sign(&self.key)?;
        issued.insert(offer.offer_hash()?, IssuedOffer { offer: offer.clone(), requester, requester_transport });
        Ok(GameRouteControlResponseV1::Offer(Box::new(offer)))
    }

    fn answer_challenge(
        &self,
        requester: PeerId,
        requester_transport: TransportPeerId,
        challenge: crate::game_route::MinecraftRouteChallengeV1,
    ) -> Result<GameRouteControlResponseV1, GameRouteError> {
        if challenge.requester_peer_id != requester
            || challenge.requester_transport_peer != requester_transport.to_bytes()
        {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unauthorized));
        }
        let mut issued_offers = self.issued.lock().map_err(|_| GameRouteError::StaleAuthority)?;
        // Do NOT let a different authenticated peer spend somebody else's
        // pending offer merely by knowing its public signed offer hash.
        // Check the exact application and transport identity BEFORE remove.
        if let Some(pending) = issued_offers.get(&challenge.offer_hash) {
            if pending.requester != requester
                || pending.requester_transport != requester_transport
                || challenge.world_id != pending.offer.world_id
            {
                return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unauthorized));
            }
        }
        let issued = issued_offers.remove(&challenge.offer_hash);
        drop(issued_offers);
        let Some(issued) = issued else {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ExpiredOrMalformed));
        };
        let offer = issued.offer;
        let now = unix_ms()?;
        if issued.requester != requester
            || issued.requester_transport != requester_transport
            || challenge.world_id != offer.world_id
            || now >= offer.expires_unix_ms
        {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unauthorized));
        }
        let evidence = match self.evidence(offer.world_id, requester, challenge.nonce, now) {
            Ok(evidence) => evidence,
            Err(reason) => return Ok(GameRouteControlResponseV1::Refused(reason)),
        };
        if evidence.realm_hash != offer.realm_hash
            || evidence.membership_hash != offer.membership_hash
            || evidence.quorum_certificate_hash != offer.quorum_certificate_hash
            || evidence.compatibility_fingerprint != offer.compatibility_fingerprint
            || evidence.checkpoint_hash != offer.checkpoint_hash
            || evidence.state_root != offer.state_root
            || evidence.epoch != offer.epoch
            || evidence.fencing_token != offer.fencing_token
            || evidence.consent_revision != offer.consent_revision
            || evidence.runtime_instance_nonce != offer.runtime_instance_nonce
            || evidence.runtime_ready_proof_hash != offer.runtime_ready_proof_hash
            || evidence.game_server_port != offer.game_server_port
        {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::FencedOrConsentWithdrawn));
        }
        let expires =
            now.saturating_add(MAX_ROUTE_RESPONSE_AGE_MS).min(offer.expires_unix_ms).min(evidence.valid_until_unix_ms);
        if expires <= now {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ExpiredOrMalformed));
        }
        let mut response = MinecraftRouteResponseV1 {
            challenge,
            authority_peer_id: self.application_peer,
            epoch: offer.epoch,
            fencing_token: offer.fencing_token,
            runtime_instance_nonce: offer.runtime_instance_nonce,
            issued_unix_ms: now,
            expires_unix_ms: expires,
            signature: Vec::new(),
        };
        response.sign(&self.key)?;
        Ok(GameRouteControlResponseV1::ChallengeResponse(Box::new(response)))
    }
}

fn same_authority_state(a: &FreshAuthorityEvidence, b: &FreshAuthorityEvidence) -> bool {
    a.world_id == b.world_id
        && a.realm_hash == b.realm_hash
        && a.authority_peer_id == b.authority_peer_id
        && a.authority_public_key == b.authority_public_key
        && a.authority_transport_peer == b.authority_transport_peer
        && a.game_server_port == b.game_server_port
        && a.membership_hash == b.membership_hash
        && a.quorum_certificate_hash == b.quorum_certificate_hash
        && a.compatibility_fingerprint == b.compatibility_fingerprint
        && a.checkpoint_hash == b.checkpoint_hash
        && a.state_root == b.state_root
        && a.epoch == b.epoch
        && a.fencing_token == b.fencing_token
        && a.consent_revision == b.consent_revision
        && a.runtime_instance_nonce == b.runtime_instance_nonce
        && a.runtime_ready_proof_hash == b.runtime_ready_proof_hash
}

fn unix_ms() -> Result<u64, GameRouteError> {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| GameRouteError::Expired)?.as_millis();
    u64::try_from(ms).map_err(|_| GameRouteError::Expired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_route::RouteVerifier;
    use libp2p::identity::Keypair;
    use tempfile::TempDir;

    struct SyntheticAuthority {
        state: Mutex<FreshAuthorityEvidence>,
        requester: PeerId,
        available: std::sync::atomic::AtomicBool,
        flip_after_first_proof: std::sync::atomic::AtomicBool,
        observations: std::sync::atomic::AtomicUsize,
    }

    impl FreshAuthoritySource for SyntheticAuthority {
        fn verify_fresh(
            &self,
            world: WorldId,
            requester: PeerId,
            nonce: [u8; 32],
        ) -> Result<FreshAuthorityEvidence, GameRouteError> {
            if !self.available.load(std::sync::atomic::Ordering::SeqCst) || requester != self.requester {
                return Err(GameRouteError::UnverifiedAuthority);
            }
            let ordinal = self.observations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if ordinal == 1 && self.flip_after_first_proof.load(std::sync::atomic::Ordering::SeqCst) {
                let mut state = self.state.lock().unwrap();
                state.epoch += 1;
                state.fencing_token += 1;
            }
            let mut state = self.state.lock().unwrap().clone();
            if state.world_id != world {
                return Err(GameRouteError::WrongAuthority);
            }
            let now = unix_ms().unwrap();
            state.challenge_nonce = nonce;
            state.verified_at_unix_ms = now;
            state.valid_until_unix_ms = now + 4_000;
            Ok(state)
        }
    }

    struct Fixture {
        root: TempDir,
        key: SigningKey,
        source: Arc<SyntheticAuthority>,
        host_transport: TransportPeerId,
        requester: PeerId,
        requester_transport: TransportPeerId,
        world: WorldId,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            }
            let key = SigningKey::from_bytes(&[42; 32]);
            let host_transport = Keypair::generate_ed25519().public().to_peer_id();
            let requester_transport = Keypair::generate_ed25519().public().to_peer_id();
            let requester = PeerId([15; 32]);
            let world = WorldId([21; 32]);
            let source = Arc::new(SyntheticAuthority {
                requester,
                available: std::sync::atomic::AtomicBool::new(true),
                flip_after_first_proof: std::sync::atomic::AtomicBool::new(false),
                observations: std::sync::atomic::AtomicUsize::new(0),
                state: Mutex::new(FreshAuthorityEvidence {
                    world_id: world,
                    realm_hash: Hash32([1; 32]),
                    authority_peer_id: peer_id_from_public_key(&key.verifying_key().to_bytes()),
                    authority_public_key: key.verifying_key().to_bytes(),
                    authority_transport_peer: host_transport,
                    game_server_port: 25565,
                    requester_is_member: true,
                    authority_is_member: true,
                    conflict_free: true,
                    hosting_consent_valid: true,
                    runtime_ready_verified: true,
                    membership_hash: Hash32([2; 32]),
                    quorum_certificate_hash: Hash32([3; 32]),
                    compatibility_fingerprint: Hash32([4; 32]),
                    checkpoint_hash: Hash32([5; 32]),
                    state_root: Hash32([6; 32]),
                    epoch: 5,
                    fencing_token: 9,
                    consent_revision: 2,
                    runtime_instance_nonce: [7; 32],
                    runtime_ready_proof_hash: Hash32([8; 32]),
                    challenge_nonce: [0; 32],
                    verified_at_unix_ms: 0,
                    valid_until_unix_ms: 0,
                }),
            });
            Self { root, key, source, host_transport, requester, requester_transport, world }
        }

        fn signer(&self) -> AuthenticatedGameRouteHostSigner {
            AuthenticatedGameRouteHostSigner::new(
                self.key.clone(),
                self.host_transport,
                self.source.clone(),
                self.root.path().join("durable-host-sequences"),
            )
        }

        fn offer(&self, signer: &AuthenticatedGameRouteHostSigner) -> MinecraftRouteOfferV1 {
            let reply = signer
                .answer(
                    self.requester,
                    self.requester_transport,
                    GameRouteControlRequestV1::Offer { world_id: self.world },
                )
                .unwrap();
            let GameRouteControlResponseV1::Offer(offer) = reply else { panic!("expected a signed offer") };
            *offer
        }
    }

    #[test]
    fn independently_verified_host_signs_one_use_challenge_and_restart_monotonicity() {
        let f = Fixture::new();
        let signer = f.signer();
        let offer = f.offer(&signer);
        assert_eq!(offer.verify_signature().unwrap(), f.host_transport);
        assert_eq!(offer.sequence, 1);

        let verifier = RouteVerifier::new(f.root.path().join("accepted-floors"));
        let challenge = verifier.issue_challenge(&offer, f.requester, f.requester_transport).unwrap();
        let response = signer
            .answer(
                f.requester,
                f.requester_transport,
                GameRouteControlRequestV1::Challenge(Box::new(challenge.clone())),
            )
            .unwrap();
        let GameRouteControlResponseV1::ChallengeResponse(response) = response else {
            panic!("expected a signed response");
        };
        let route = verifier.verify(&offer, &response, f.world, unix_ms().unwrap(), f.source.as_ref()).unwrap();
        assert!(route.is_fresh());
        assert_eq!(route.transport_peer(), f.host_transport);
        assert_eq!(route.epoch(), 5);

        // A one-use issued offer is consumed; even the exact same valid
        // signature and challenge cannot be reissued by a restarted sender.
        assert!(matches!(
            signer
                .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Challenge(Box::new(challenge)),)
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ExpiredOrMalformed)
        ));
        let restarted = f.signer();
        let second = f.offer(&restarted);
        assert_eq!(second.sequence, 2);
        assert_ne!(second.offer_hash().unwrap(), offer.offer_hash().unwrap());
    }

    #[test]
    fn missing_quorum_or_consent_or_runtime_readiness_cannot_be_signed() {
        let f = Fixture::new();
        let signer = f.signer();
        f.source.available.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            signer
                .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Offer { world_id: f.world })
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::QuorumUnavailable)
        ));
        f.source.available.store(true, std::sync::atomic::Ordering::SeqCst);
        f.source.state.lock().unwrap().requester_is_member = false;
        assert!(matches!(
            signer
                .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Offer { world_id: f.world })
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unauthorized)
        ));
        f.source.state.lock().unwrap().requester_is_member = true;
        f.source.state.lock().unwrap().hosting_consent_valid = false;
        assert!(matches!(
            signer
                .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Offer { world_id: f.world })
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::FencedOrConsentWithdrawn)
        ));
        f.source.state.lock().unwrap().hosting_consent_valid = true;
        f.source.state.lock().unwrap().runtime_ready_verified = false;
        assert!(matches!(
            signer
                .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Offer { world_id: f.world })
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::HostUnready)
        ));
        f.source.state.lock().unwrap().runtime_ready_verified = true;
        f.source.state.lock().unwrap().conflict_free = false;
        assert!(matches!(
            signer
                .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Offer { world_id: f.world })
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ConflictingAuthority)
        ));
    }

    #[test]
    fn host_denies_wrong_requester_and_fenced_history_after_offer() {
        let f = Fixture::new();
        let signer = f.signer();
        let offer = f.offer(&signer);
        let verifier = RouteVerifier::new(f.root.path().join("client-floors"));
        let challenge = verifier.issue_challenge(&offer, f.requester, f.requester_transport).unwrap();
        assert!(matches!(
            signer
                .answer(
                    PeerId([88; 32]),
                    f.requester_transport,
                    GameRouteControlRequestV1::Challenge(Box::new(challenge.clone())),
                )
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unauthorized)
        ));
        // Accepted authority changed between the offer and challenge; do NOT
        // sign a response for a now-superseded host.
        let mut state = f.source.state.lock().unwrap();
        state.epoch += 1;
        state.fencing_token += 1;
        drop(state);
        assert!(matches!(
            signer
                .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Challenge(Box::new(challenge)),)
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::FencedOrConsentWithdrawn)
        ));
        let new_offer = f.offer(&signer);
        assert_eq!(new_offer.epoch, offer.epoch + 1);
        assert_eq!(new_offer.fencing_token, offer.fencing_token + 1);
        assert_eq!(new_offer.sequence, 1);
    }

    #[test]
    fn foreign_member_cannot_consume_another_requesters_signed_offer() {
        let f = Fixture::new();
        let signer = f.signer();
        let offer = f.offer(&signer);
        let verifier = RouteVerifier::new(f.root.path().join("client-floors"));
        let original = verifier.issue_challenge(&offer, f.requester, f.requester_transport).unwrap();

        // A different authenticated SwarmCraft connection can learn a signed
        // offer hash but must not be allowed to spend the pending challenge.
        let attacker = PeerId([88; 32]);
        let attacker_transport = Keypair::generate_ed25519().public().to_peer_id();
        let mut forged = original.clone();
        forged.requester_peer_id = attacker;
        forged.requester_transport_peer = attacker_transport.to_bytes();
        assert!(matches!(
            signer
                .answer(attacker, attacker_transport, GameRouteControlRequestV1::Challenge(Box::new(forged)))
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unauthorized)
        ));

        // The original authenticated requester must still be able to consume
        // its exact one-time offer after the attack attempt.
        let response = signer
            .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Challenge(Box::new(original)))
            .unwrap();
        let GameRouteControlResponseV1::ChallengeResponse(response) = response else {
            panic!("the rightful requester must retain its pending challenge");
        };
        verifier.verify(&offer, &response, f.world, unix_ms().unwrap(), f.source.as_ref()).unwrap();
    }

    #[test]
    fn host_rechecks_authority_after_sequence_commit_before_signing_offer() {
        let f = Fixture::new();
        let signer = f.signer();
        f.source.flip_after_first_proof.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            signer
                .answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Offer { world_id: f.world },)
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::FencedOrConsentWithdrawn)
        ));
        assert_eq!(f.source.observations.load(std::sync::atomic::Ordering::SeqCst), 2);
        // Sequence is consumed durably even if no signed offer was issued.
        // A retry at the newer generation begins at sequence 1, not a replay.
        let approved = f.offer(&signer);
        assert_eq!(approved.epoch, 6);
        assert_eq!(approved.fencing_token, 10);
        assert_eq!(approved.sequence, 1);
    }

    #[test]
    fn corrupt_sequence_storage_fails_closed_without_signing() {
        let f = Fixture::new();
        let signer = f.signer();
        let offer = f.offer(&signer);
        let path = f.root.path().join("durable-host-sequences").join(format!("{}.sequence", f.world.to_hex()));
        fs::write(path, b"truncated garbage").unwrap();
        assert!(matches!(
            signer.answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Offer { world_id: f.world },),
            Err(GameRouteError::InvalidTranscript)
        ));
        assert_eq!(offer.sequence, 1);
    }

    #[cfg(unix)]
    #[test]
    fn permissive_persisted_signing_sequence_cannot_issue_a_new_offer() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let signer = f.signer();
        f.offer(&signer);
        let path = f.root.path().join("durable-host-sequences").join(format!("{}.sequence", f.world.to_hex()));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(matches!(
            signer.answer(f.requester, f.requester_transport, GameRouteControlRequestV1::Offer { world_id: f.world }),
            Err(GameRouteError::InvalidTranscript)
        ));
    }

    #[test]
    fn parallel_persistent_sequence_allocations_never_reuse_numbers() {
        let f = Fixture::new();
        let sequence_dir = f.root.path().join("multi-process-sequencer");
        let mut threads = Vec::new();
        for _ in 0..8 {
            let directory = sequence_dir.clone();
            let world = f.world;
            threads.push(std::thread::spawn(move || {
                DurableHostOfferSequencer::new(directory).reserve_next(world, Hash32([1; 32]), 1, 1).unwrap()
            }));
        }
        let mut values = threads.into_iter().map(|thread| thread.join().unwrap()).collect::<Vec<_>>();
        values.sort_unstable();
        assert_eq!(values, (1..=8).collect::<Vec<_>>());
    }

    #[cfg(unix)]
    #[test]
    fn signing_sequence_cannot_be_allocated_under_replaceable_storage_parent() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let f = Fixture::new();
        let shared = f.root.path().join("shared-parent");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o770)).unwrap();
        let unsafe_dir = shared.join("private-sequences");
        let sequencer = DurableHostOfferSequencer::new(&unsafe_dir);
        assert!(matches!(
            sequencer.reserve_next(f.world, Hash32([1; 32]), 1, 1),
            Err(GameRouteError::InvalidTranscript)
        ));
        assert!(!unsafe_dir.exists());

        let private = f.root.path().join("trusted");
        fs::create_dir(&private).unwrap();
        let link = f.root.path().join("linked-parent");
        symlink(private.clone(), &link).unwrap();
        let sequencer = DurableHostOfferSequencer::new(link.join("sequences"));
        assert!(matches!(
            sequencer.reserve_next(f.world, Hash32([1; 32]), 1, 1),
            Err(GameRouteError::InvalidTranscript)
        ));
        assert!(!private.join("sequences").exists());
    }
}
