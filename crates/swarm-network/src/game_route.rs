//! Authenticated Minecraft route transcripts and durable generation fencing.
//!
//! A valid signature is NOT authority. A caller MUST provide fresh, independently
//! verified canonical quorum, membership, consent and Fabric runtime evidence.
//! Do not derive that evidence from discovery, configured IPs or persisted Ready.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use fs2::FileExt;
use libp2p::PeerId as TransportPeerId;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerId, WorldId};
use thiserror::Error;

pub const GAME_ROUTE_VERSION: u16 = 1;
pub const GAME_STREAM_PROTOCOL: &str = "/swarmcraft/minecraft-game/1";
pub const MAX_ROUTE_OFFER_AGE_MS: u64 = 15_000;
pub const MAX_ROUTE_RESPONSE_AGE_MS: u64 = 5_000;
pub const MAX_FRESH_EVIDENCE_AGE_MS: u64 = 5_000;
/// A quorum/readiness attestation is a short lease, never a durable Ready flag.
pub const MAX_FRESH_EVIDENCE_LEASE_MS: u64 = 30_000;
pub const MAX_ROUTE_WIRE_BYTES: usize = 4096;
const MAX_PEER_BYTES: usize = 128;
const OFFER_DOMAIN: &[u8] = b"swarmcraft/minecraft-route-offer/v1\0";
const RESPONSE_DOMAIN: &[u8] = b"swarmcraft/minecraft-route-response/v1\0";
const OFFER_HASH_DOMAIN: &[u8] = b"swarmcraft/minecraft-route-offer-hash/v1\0";
const CLOCK_SKEW_MS: u64 = 2_000;

#[derive(Debug, Error)]
pub enum GameRouteError {
    #[error("invalid route signature")]
    InvalidSignature,
    #[error("invalid application or transport identity")]
    InvalidIdentity,
    #[error("invalid, unsupported or unbounded route transcript")]
    InvalidTranscript,
    #[error("expired or future route")]
    Expired,
    #[error("route does not match current world, realm or authority")]
    WrongAuthority,
    #[error("challenge not issued here, already used or expired")]
    InvalidChallenge,
    #[error("independent live quorum, membership, consent or runtime proof is missing")]
    UnverifiedAuthority,
    #[error("canonical authority is stale, conflicting, fenced or withdrawn")]
    StaleAuthority,
    #[error("route would roll back the durable generation floor")]
    Downgrade,
    #[error("route floor I/O error: {0}")]
    Storage(#[from] std::io::Error),
}

/// The current authority signs ALL fields, including the game transport peer.
/// No raw IP, DNS or runtime configuration is a route authorization source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftRouteOfferV1 {
    pub version: u16,
    pub world_id: WorldId,
    pub realm_hash: Hash32,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub authority_transport_peer: Vec<u8>,
    /// Actual local Minecraft listener port proved by authenticated Fabric IPC.
    /// The host-side game receiver must dial only 127.0.0.1 at this port.
    pub game_server_port: u16,
    pub membership_hash: Hash32,
    pub quorum_certificate_hash: Hash32,
    pub compatibility_fingerprint: Hash32,
    pub checkpoint_hash: Hash32,
    pub state_root: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub consent_revision: u64,
    pub runtime_instance_nonce: [u8; 32],
    pub runtime_ready_proof_hash: Hash32,
    pub offer_id: [u8; 32],
    pub sequence: u64,
    pub issued_unix_ms: u64,
    pub expires_unix_ms: u64,
    pub signature: Vec<u8>,
}

impl MinecraftRouteOfferV1 {
    fn validate(&self) -> Result<TransportPeerId, GameRouteError> {
        if self.version != GAME_ROUTE_VERSION
            || self.world_id == WorldId::default()
            || self.realm_hash == Hash32::default()
            || self.membership_hash == Hash32::default()
            || self.quorum_certificate_hash == Hash32::default()
            || self.compatibility_fingerprint == Hash32::default()
            || self.checkpoint_hash == Hash32::default()
            || self.state_root == Hash32::default()
            || self.runtime_ready_proof_hash == Hash32::default()
            || self.runtime_instance_nonce == [0; 32]
            || self.offer_id == [0; 32]
            || self.epoch == 0
            || self.fencing_token == 0
            || self.consent_revision == 0
            || self.sequence == 0
            || self.expires_unix_ms <= self.issued_unix_ms
            || self.expires_unix_ms - self.issued_unix_ms > MAX_ROUTE_OFFER_AGE_MS
            || self.authority_transport_peer.is_empty()
            || self.authority_transport_peer.len() > MAX_PEER_BYTES
            || self.game_server_port == 0
            || self.signature.len() > 64
        {
            return Err(GameRouteError::InvalidTranscript);
        }
        if peer_id_from_public_key(&self.authority_public_key) != self.authority_peer_id {
            return Err(GameRouteError::InvalidIdentity);
        }
        TransportPeerId::from_bytes(&self.authority_transport_peer).map_err(|_| GameRouteError::InvalidIdentity)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, GameRouteError> {
        self.validate()?;
        let mut unsigned = self.clone();
        unsigned.signature.clear();
        domain_encode(OFFER_DOMAIN, &unsigned)
    }

    pub fn sign(&mut self, key: &SigningKey) -> Result<(), GameRouteError> {
        if key.verifying_key().to_bytes() != self.authority_public_key {
            return Err(GameRouteError::InvalidIdentity);
        }
        self.signature = key.sign(&self.signing_bytes()?).to_bytes().to_vec();
        Ok(())
    }

    pub fn verify_signature(&self) -> Result<TransportPeerId, GameRouteError> {
        let peer = self.validate()?;
        let key = VerifyingKey::from_bytes(&self.authority_public_key).map_err(|_| GameRouteError::InvalidSignature)?;
        let sig = Signature::from_slice(&self.signature).map_err(|_| GameRouteError::InvalidSignature)?;
        key.verify(&self.signing_bytes()?, &sig).map_err(|_| GameRouteError::InvalidSignature)?;
        Ok(peer)
    }

    pub fn offer_hash(&self) -> Result<Hash32, GameRouteError> {
        self.verify_signature()?;
        let mut bytes = self.signing_bytes()?;
        bytes.extend_from_slice(&self.signature);
        Ok(Hash32::from_domain_bytes(OFFER_HASH_DOMAIN, &bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftRouteChallengeV1 {
    pub world_id: WorldId,
    pub offer_hash: Hash32,
    pub requester_peer_id: PeerId,
    pub requester_transport_peer: Vec<u8>,
    pub nonce: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftRouteResponseV1 {
    pub challenge: MinecraftRouteChallengeV1,
    pub authority_peer_id: PeerId,
    pub epoch: u64,
    pub fencing_token: u64,
    pub runtime_instance_nonce: [u8; 32],
    pub issued_unix_ms: u64,
    pub expires_unix_ms: u64,
    pub signature: Vec<u8>,
}

impl MinecraftRouteResponseV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, GameRouteError> {
        if self.challenge.nonce == [0; 32]
            || self.challenge.requester_peer_id == PeerId::default()
            || self.challenge.requester_transport_peer.is_empty()
            || self.challenge.requester_transport_peer.len() > MAX_PEER_BYTES
            || TransportPeerId::from_bytes(&self.challenge.requester_transport_peer).is_err()
            || self.epoch == 0
            || self.fencing_token == 0
            || self.runtime_instance_nonce == [0; 32]
            || self.expires_unix_ms <= self.issued_unix_ms
            || self.expires_unix_ms - self.issued_unix_ms > MAX_ROUTE_RESPONSE_AGE_MS
            || self.signature.len() > 64
        {
            return Err(GameRouteError::InvalidTranscript);
        }
        let mut unsigned = self.clone();
        unsigned.signature.clear();
        domain_encode(RESPONSE_DOMAIN, &unsigned)
    }

    pub fn sign(&mut self, key: &SigningKey) -> Result<(), GameRouteError> {
        if peer_id_from_public_key(&key.verifying_key().to_bytes()) != self.authority_peer_id {
            return Err(GameRouteError::InvalidIdentity);
        }
        self.signature = key.sign(&self.signing_bytes()?).to_bytes().to_vec();
        Ok(())
    }
}

fn domain_encode<T: Serialize>(domain: &[u8], value: &T) -> Result<Vec<u8>, GameRouteError> {
    let encoded = postcard::to_allocvec(value).map_err(|_| GameRouteError::InvalidTranscript)?;
    if encoded.len() + domain.len() > MAX_ROUTE_WIRE_BYTES {
        return Err(GameRouteError::InvalidTranscript);
    }
    let mut bytes = Vec::with_capacity(encoded.len() + domain.len());
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&encoded);
    Ok(bytes)
}

/// The source implementing this type MUST independently verify signed quorum
/// history, current membership, consent and an authenticated live Fabric
/// readiness exchange for the SAME unpredictable challenge. This type is a
/// readout of that verification; it does not itself certify any external data.
#[derive(Debug, Clone)]
pub struct FreshAuthorityEvidence {
    pub world_id: WorldId,
    pub realm_hash: Hash32,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub authority_transport_peer: TransportPeerId,
    pub game_server_port: u16,
    pub requester_is_member: bool,
    pub authority_is_member: bool,
    pub conflict_free: bool,
    pub hosting_consent_valid: bool,
    pub runtime_ready_verified: bool,
    pub membership_hash: Hash32,
    pub quorum_certificate_hash: Hash32,
    pub compatibility_fingerprint: Hash32,
    pub checkpoint_hash: Hash32,
    pub state_root: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub consent_revision: u64,
    pub runtime_instance_nonce: [u8; 32],
    pub runtime_ready_proof_hash: Hash32,
    pub challenge_nonce: [u8; 32],
    pub verified_at_unix_ms: u64,
    pub valid_until_unix_ms: u64,
}

/// No production implementation is supplied until the daemon can prove a
/// live intersecting canonical quorum and verified runtime/hosting consent.
pub trait FreshAuthoritySource: Send + Sync {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        challenge: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError>;
}

/// Not constructible or persistable by a caller: only the full verifier mints it.
#[derive(Debug)]
pub struct VerifiedGameRoute {
    world_id: WorldId,
    authority_peer_id: PeerId,
    transport_peer: TransportPeerId,
    epoch: u64,
    fencing_token: u64,
    offer_hash: Hash32,
    expires_at: Instant,
}

impl VerifiedGameRoute {
    pub fn world_id(&self) -> WorldId {
        self.world_id
    }
    pub fn authority_peer_id(&self) -> PeerId {
        self.authority_peer_id
    }
    pub fn transport_peer(&self) -> TransportPeerId {
        self.transport_peer
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn fencing_token(&self) -> u64 {
        self.fencing_token
    }
    pub fn offer_hash(&self) -> Hash32 {
        self.offer_hash
    }
    pub fn is_fresh(&self) -> bool {
        Instant::now() < self.expires_at
    }
    pub fn time_remaining(&self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }
}

struct PendingChallenge {
    challenge: MinecraftRouteChallengeV1,
    issued_at: Instant,
}

pub struct RouteVerifier {
    floor_dir: PathBuf,
    pending: Mutex<HashMap<[u8; 32], PendingChallenge>>,
}

impl RouteVerifier {
    pub fn new(floor_dir: impl Into<PathBuf>) -> Self {
        Self { floor_dir: floor_dir.into(), pending: Mutex::new(HashMap::new()) }
    }

    /// Only send this to the offered host over authenticated libp2p.
    pub fn issue_challenge(
        &self,
        offer: &MinecraftRouteOfferV1,
        requester_peer_id: PeerId,
        requester_transport_peer: TransportPeerId,
    ) -> Result<MinecraftRouteChallengeV1, GameRouteError> {
        if requester_peer_id == PeerId::default() {
            return Err(GameRouteError::InvalidIdentity);
        }
        let offer_hash = offer.offer_hash()?;
        let mut pending = self.pending.lock().map_err(|_| GameRouteError::InvalidChallenge)?;
        pending.retain(|_, entry| entry.issued_at.elapsed() < Duration::from_millis(MAX_ROUTE_RESPONSE_AGE_MS));
        if pending.len() >= 256 {
            return Err(GameRouteError::InvalidChallenge);
        }
        let mut nonce = [0; 32];
        OsRng.fill_bytes(&mut nonce);
        let challenge = MinecraftRouteChallengeV1 {
            world_id: offer.world_id,
            offer_hash,
            requester_peer_id,
            requester_transport_peer: requester_transport_peer.to_bytes(),
            nonce,
        };
        pending.insert(nonce, PendingChallenge { challenge: challenge.clone(), issued_at: Instant::now() });
        Ok(challenge)
    }

    /// The challenge is consumed even on failure. Persist the floor before
    /// granting any traffic; failure to persist always blocks the route.
    pub fn verify(
        &self,
        offer: &MinecraftRouteOfferV1,
        response: &MinecraftRouteResponseV1,
        expected_world: WorldId,
        now_unix_ms: u64,
        source: &(impl FreshAuthoritySource + ?Sized),
    ) -> Result<VerifiedGameRoute, GameRouteError> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| GameRouteError::InvalidChallenge)?
            .remove(&response.challenge.nonce)
            .ok_or(GameRouteError::InvalidChallenge)?;
        if pending.challenge != response.challenge
            || pending.issued_at.elapsed() >= Duration::from_millis(MAX_ROUTE_RESPONSE_AGE_MS)
        {
            return Err(GameRouteError::InvalidChallenge);
        }
        let transport_peer = offer.verify_signature()?;
        if offer.world_id != expected_world
            || response.challenge.world_id != expected_world
            || response.challenge.offer_hash != offer.offer_hash()?
            || response.authority_peer_id != offer.authority_peer_id
            || response.epoch != offer.epoch
            || response.fencing_token != offer.fencing_token
            || response.runtime_instance_nonce != offer.runtime_instance_nonce
        {
            return Err(GameRouteError::WrongAuthority);
        }
        if now_unix_ms < offer.issued_unix_ms.saturating_sub(CLOCK_SKEW_MS)
            || now_unix_ms >= offer.expires_unix_ms
            || now_unix_ms < response.issued_unix_ms.saturating_sub(CLOCK_SKEW_MS)
            || now_unix_ms >= response.expires_unix_ms
        {
            return Err(GameRouteError::Expired);
        }
        let key =
            VerifyingKey::from_bytes(&offer.authority_public_key).map_err(|_| GameRouteError::InvalidSignature)?;
        let signature = Signature::from_slice(&response.signature).map_err(|_| GameRouteError::InvalidSignature)?;
        key.verify(&response.signing_bytes()?, &signature).map_err(|_| GameRouteError::InvalidSignature)?;

        let evidence =
            source.verify_fresh(expected_world, response.challenge.requester_peer_id, response.challenge.nonce)?;
        if evidence.world_id != offer.world_id
            || evidence.realm_hash != offer.realm_hash
            || evidence.authority_peer_id != offer.authority_peer_id
            || evidence.authority_public_key != offer.authority_public_key
            || evidence.authority_transport_peer != transport_peer
            || evidence.game_server_port != offer.game_server_port
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
            || evidence.challenge_nonce != response.challenge.nonce
        {
            return Err(GameRouteError::WrongAuthority);
        }
        if !evidence.requester_is_member
            || !evidence.authority_is_member
            || !evidence.runtime_ready_verified
            || !evidence.hosting_consent_valid
        {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        if !evidence.conflict_free
            || evidence.verified_at_unix_ms > now_unix_ms.saturating_add(CLOCK_SKEW_MS)
            || now_unix_ms.saturating_sub(evidence.verified_at_unix_ms) > MAX_FRESH_EVIDENCE_AGE_MS
            || evidence.valid_until_unix_ms <= now_unix_ms
            || evidence.valid_until_unix_ms > now_unix_ms.saturating_add(MAX_FRESH_EVIDENCE_LEASE_MS)
        {
            return Err(GameRouteError::StaleAuthority);
        }
        let expires_ms = evidence.valid_until_unix_ms.min(offer.expires_unix_ms).min(response.expires_unix_ms);
        self.advance_floor(offer)?;
        Ok(VerifiedGameRoute {
            world_id: offer.world_id,
            authority_peer_id: offer.authority_peer_id,
            transport_peer,
            epoch: offer.epoch,
            fencing_token: offer.fencing_token,
            offer_hash: response.challenge.offer_hash,
            expires_at: Instant::now() + Duration::from_millis(expires_ms - now_unix_ms),
        })
    }

    fn advance_floor(&self, offer: &MinecraftRouteOfferV1) -> Result<(), GameRouteError> {
        ensure_private_floor_directory(&self.floor_dir)?;
        let name = offer.world_id.to_hex();
        let path = self.floor_dir.join(format!("{name}.floor"));
        let lock_path = self.floor_dir.join(format!("{name}.lock"));
        reject_symlink(&lock_path)?;
        let lock = private_floor_options().read(true).write(true).create(true).truncate(false).open(&lock_path)?;
        lock.lock_exclusive()?;
        let outcome = (|| {
            reject_symlink(&path)?;
            let floor = if path.exists() {
                let file = File::open(&path)?;
                let mut data = Vec::new();
                file.take(256).read_to_end(&mut data)?;
                if data.len() > 128 {
                    return Err(GameRouteError::InvalidTranscript);
                }
                let (value, remainder) = postcard::take_from_bytes::<GenerationFloor>(&data)
                    .map_err(|_| GameRouteError::InvalidTranscript)?;
                if !remainder.is_empty() {
                    return Err(GameRouteError::InvalidTranscript);
                }
                Some(value)
            } else {
                None
            };
            if let Some(old) = floor {
                if old.world_id != offer.world_id
                    || old.realm_hash != offer.realm_hash
                    || offer.epoch < old.epoch
                    || offer.fencing_token < old.fencing_token
                    || (offer.epoch == old.epoch && offer.fencing_token != old.fencing_token)
                    || (offer.epoch > old.epoch && offer.fencing_token <= old.fencing_token)
                    || (offer.epoch == old.epoch && offer.sequence < old.sequence)
                {
                    return Err(GameRouteError::Downgrade);
                }
                if offer.epoch == old.epoch && offer.sequence == old.sequence {
                    if old.offer_hash == offer.offer_hash()? {
                        return Ok(());
                    }
                    return Err(GameRouteError::Downgrade);
                }
            }
            let next = GenerationFloor {
                world_id: offer.world_id,
                realm_hash: offer.realm_hash,
                epoch: offer.epoch,
                fencing_token: offer.fencing_token,
                sequence: offer.sequence,
                offer_hash: offer.offer_hash()?,
            };
            let bytes = postcard::to_allocvec(&next).map_err(|_| GameRouteError::InvalidTranscript)?;
            let mut rand = [0; 8];
            OsRng.fill_bytes(&mut rand);
            let tmp = self.floor_dir.join(format!(".{name}-{}.tmp", u64::from_be_bytes(rand)));
            let mut file = private_floor_options().write(true).create_new(true).open(&tmp)?;
            let commit = (|| -> Result<(), GameRouteError> {
                file.write_all(&bytes)?;
                file.sync_all()?;
                drop(file);
                replace_floor_atomically(&tmp, &path)?;
                sync_directory(&self.floor_dir)?;
                Ok(())
            })();
            if commit.is_err() {
                let _ = fs::remove_file(&tmp);
            }
            commit
        })();
        let _ = FileExt::unlock(&lock);
        outcome
    }
}

#[derive(Serialize, Deserialize)]
struct GenerationFloor {
    world_id: WorldId,
    realm_hash: Hash32,
    epoch: u64,
    fencing_token: u64,
    sequence: u64,
    offer_hash: Hash32,
}

fn reject_symlink(path: &Path) -> Result<(), GameRouteError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(GameRouteError::InvalidTranscript),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(GameRouteError::Storage(error)),
    }
}

/// Persistent anti-rollback evidence must not live in a world-writable
/// directory where another local user could unlink it and force a downgrade.
/// Never follow a symlink as the final floor directory component.
fn ensure_private_floor_directory(path: &Path) -> Result<(), GameRouteError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        builder.mode(0o700);
        builder.create(path)?;
        let meta = fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() || meta.permissions().mode() & 0o022 != 0 {
            return Err(GameRouteError::InvalidTranscript);
        }
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)?;
        reject_symlink(path)?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), GameRouteError> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn private_floor_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Windows std::fs::rename does not reliably replace an existing destination.
/// This floor MUST advance after the first accepted offer, including on
/// Windows. Match swarm-storage's write-through replacement semantics instead
/// of deleting the old floor first (which would open a rollback window).
#[cfg(windows)]
fn replace_floor_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
    }

    let source = source.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    let destination = destination.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    // SAFETY: Both pointers refer to NUL-terminated UTF-16 buffers that
    // remain alive for the duration of the synchronous Windows API call.
    let result = unsafe {
        MoveFileExW(source.as_ptr(), destination.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_floor_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;
    use tempfile::TempDir;

    struct Fixture {
        root: TempDir,
        key: SigningKey,
        offer: MinecraftRouteOfferV1,
        client_peer: PeerId,
        client_transport: TransportPeerId,
    }

    struct EvidenceSource {
        evidence: FreshAuthorityEvidence,
        refuse: bool,
    }

    impl FreshAuthoritySource for EvidenceSource {
        fn verify_fresh(
            &self,
            world: WorldId,
            requester: PeerId,
            challenge: [u8; 32],
        ) -> Result<FreshAuthorityEvidence, GameRouteError> {
            if self.refuse || world != self.evidence.world_id || requester != self.evidence_authorized_requester() {
                return Err(GameRouteError::UnverifiedAuthority);
            }
            let mut evidence = self.evidence.clone();
            evidence.challenge_nonce = challenge;
            Ok(evidence)
        }
    }

    impl EvidenceSource {
        fn evidence_authorized_requester(&self) -> PeerId {
            // Nonzero test member. Production checks the signed membership chain.
            PeerId([12; 32])
        }
    }

    impl Fixture {
        fn new() -> Self {
            let key = SigningKey::from_bytes(&[7; 32]);
            let authority_peer_id = peer_id_from_public_key(&key.verifying_key().to_bytes());
            let authority_transport_peer = Keypair::generate_ed25519().public().to_peer_id().to_bytes();
            let offer = MinecraftRouteOfferV1 {
                version: GAME_ROUTE_VERSION,
                world_id: WorldId([1; 32]),
                realm_hash: Hash32([2; 32]),
                authority_peer_id,
                authority_public_key: key.verifying_key().to_bytes(),
                authority_transport_peer,
                game_server_port: 25570,
                membership_hash: Hash32([3; 32]),
                quorum_certificate_hash: Hash32([4; 32]),
                compatibility_fingerprint: Hash32([5; 32]),
                checkpoint_hash: Hash32([6; 32]),
                state_root: Hash32([7; 32]),
                epoch: 4,
                fencing_token: 9,
                consent_revision: 2,
                runtime_instance_nonce: [8; 32],
                runtime_ready_proof_hash: Hash32([9; 32]),
                offer_id: [10; 32],
                sequence: 1,
                issued_unix_ms: 10_000,
                expires_unix_ms: 20_000,
                signature: Vec::new(),
            };
            // tempfile::tempdir uses the process umask (often 0o002), and
            // thus can produce 0o775 on this machine. Tests exercising a
            // durable anti-rollback floor require a genuinely private path.
            let root = tempfile::tempdir().unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self {
                root,
                key,
                offer,
                client_peer: PeerId([12; 32]),
                client_transport: Keypair::generate_ed25519().public().to_peer_id(),
            }
        }

        fn sign(&mut self) {
            self.offer.sign(&self.key).unwrap();
        }

        fn source(&self) -> EvidenceSource {
            EvidenceSource {
                refuse: false,
                evidence: FreshAuthorityEvidence {
                    world_id: self.offer.world_id,
                    realm_hash: self.offer.realm_hash,
                    authority_peer_id: self.offer.authority_peer_id,
                    authority_public_key: self.offer.authority_public_key,
                    authority_transport_peer: self.offer.verify_signature().unwrap(),
                    game_server_port: self.offer.game_server_port,
                    requester_is_member: true,
                    authority_is_member: true,
                    conflict_free: true,
                    hosting_consent_valid: true,
                    runtime_ready_verified: true,
                    membership_hash: self.offer.membership_hash,
                    quorum_certificate_hash: self.offer.quorum_certificate_hash,
                    compatibility_fingerprint: self.offer.compatibility_fingerprint,
                    checkpoint_hash: self.offer.checkpoint_hash,
                    state_root: self.offer.state_root,
                    epoch: self.offer.epoch,
                    fencing_token: self.offer.fencing_token,
                    consent_revision: self.offer.consent_revision,
                    runtime_instance_nonce: self.offer.runtime_instance_nonce,
                    runtime_ready_proof_hash: self.offer.runtime_ready_proof_hash,
                    challenge_nonce: [0; 32],
                    verified_at_unix_ms: 10_100,
                    valid_until_unix_ms: 14_000,
                },
            }
        }

        fn response(&self, challenge: MinecraftRouteChallengeV1) -> MinecraftRouteResponseV1 {
            let mut response = MinecraftRouteResponseV1 {
                challenge,
                authority_peer_id: self.offer.authority_peer_id,
                epoch: self.offer.epoch,
                fencing_token: self.offer.fencing_token,
                runtime_instance_nonce: self.offer.runtime_instance_nonce,
                issued_unix_ms: 10_100,
                expires_unix_ms: 14_000,
                signature: Vec::new(),
            };
            response.sign(&self.key).unwrap();
            response
        }

        fn verify(
            &self,
            verifier: &RouteVerifier,
            source: &EvidenceSource,
        ) -> Result<VerifiedGameRoute, GameRouteError> {
            let challenge = verifier.issue_challenge(&self.offer, self.client_peer, self.client_transport)?;
            let response = self.response(challenge);
            verifier.verify(&self.offer, &response, self.offer.world_id, 10_150, source)
        }
    }

    #[test]
    fn signed_current_route_requires_fresh_evidence_and_durable_floor() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        let route = f.verify(&verifier, &f.source()).unwrap();
        assert_eq!(route.world_id(), f.offer.world_id);
        assert_eq!(route.authority_peer_id(), f.offer.authority_peer_id);
        assert_eq!(route.epoch(), 4);
        assert_eq!(route.fencing_token(), 9);
        assert!(route.is_fresh());
        assert_eq!(route.transport_peer().to_bytes(), f.offer.authority_transport_peer);

        // Different verifier instance (simulated process restart) never forgets floor.
        let restarted = RouteVerifier::new(f.root.path());
        let old = f.offer.clone();
        f.offer.epoch = 5;
        f.offer.fencing_token = 10;
        f.offer.sequence = 1;
        f.sign();
        f.verify(&restarted, &f.source()).unwrap();
        f.offer = old;
        assert!(matches!(f.verify(&restarted, &f.source()), Err(GameRouteError::Downgrade)));
    }

    #[test]
    fn same_generation_rejects_lower_offer_sequence() {
        let mut f = Fixture::new();
        f.offer.sequence = 2;
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        f.verify(&verifier, &f.source()).unwrap();
        f.offer.sequence = 1;
        f.sign();
        assert!(matches!(f.verify(&verifier, &f.source()), Err(GameRouteError::Downgrade)));
    }

    #[test]
    fn equal_generation_and_sequence_rejects_equivocating_offer() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        f.verify(&verifier, &f.source()).unwrap();
        f.offer.offer_id = [88; 32];
        f.sign();
        assert!(matches!(f.verify(&verifier, &f.source()), Err(GameRouteError::Downgrade)));
    }

    #[test]
    fn higher_epoch_requires_strictly_new_fencing_token() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        f.verify(&verifier, &f.source()).unwrap();
        f.offer.epoch += 1;
        f.sign();
        assert!(matches!(f.verify(&verifier, &f.source()), Err(GameRouteError::Downgrade)));
    }

    #[test]
    fn challenge_cannot_be_replayed_or_changed() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        let challenge = verifier.issue_challenge(&f.offer, f.client_peer, f.client_transport).unwrap();
        let response = f.response(challenge.clone());
        assert!(verifier.verify(&f.offer, &response, f.offer.world_id, 10_150, &f.source()).is_ok());
        assert!(matches!(
            verifier.verify(&f.offer, &response, f.offer.world_id, 10_150, &f.source()),
            Err(GameRouteError::InvalidChallenge)
        ));
        let fresh = verifier.issue_challenge(&f.offer, f.client_peer, f.client_transport).unwrap();
        let mut bad = f.response(fresh);
        bad.challenge.nonce = challenge.nonce;
        assert!(matches!(
            verifier.verify(&f.offer, &bad, f.offer.world_id, 10_150, &f.source()),
            Err(GameRouteError::InvalidChallenge)
        ));
    }

    #[test]
    fn rejects_wrong_world_tampering_signature_and_expiry() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        let challenge = verifier.issue_challenge(&f.offer, f.client_peer, f.client_transport).unwrap();
        let response = f.response(challenge);
        assert!(matches!(
            verifier.verify(&f.offer, &response, WorldId([99; 32]), 10_150, &f.source()),
            Err(GameRouteError::WrongAuthority)
        ));
        let mut tampered = f.offer.clone();
        tampered.epoch += 1;
        assert!(matches!(tampered.verify_signature(), Err(GameRouteError::InvalidSignature)));
        let challenge = verifier.issue_challenge(&f.offer, f.client_peer, f.client_transport).unwrap();
        let response = f.response(challenge);
        assert!(matches!(
            verifier.verify(&f.offer, &response, f.offer.world_id, 30_000, &f.source()),
            Err(GameRouteError::Expired)
        ));
    }

    #[test]
    fn missing_quorum_membership_consent_conflict_and_unready_runtime_fail_closed() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        let mut source = f.source();
        source.refuse = true;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::UnverifiedAuthority)));
        source.refuse = false;
        source.evidence.requester_is_member = false;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::UnverifiedAuthority)));
        source.evidence.requester_is_member = true;
        source.evidence.authority_is_member = false;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::UnverifiedAuthority)));
        source.evidence.authority_is_member = true;
        source.evidence.hosting_consent_valid = false;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::UnverifiedAuthority)));
        source.evidence.hosting_consent_valid = true;
        source.evidence.runtime_ready_verified = false;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::UnverifiedAuthority)));
        source.evidence.runtime_ready_verified = true;
        source.evidence.conflict_free = false;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::StaleAuthority)));
        source.evidence.conflict_free = true;
        source.evidence.valid_until_unix_ms = 10_000;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::StaleAuthority)));
        source.evidence.valid_until_unix_ms = 14_000;
        source.evidence.epoch += 1;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::WrongAuthority)));
    }

    #[test]
    fn independently_generated_evidence_allows_bounded_time_advance_only() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        let mut source = f.source();
        // The source runs after the caller captured its wall-clock sample.
        source.evidence.verified_at_unix_ms = 10_151;
        assert!(f.verify(&verifier, &source).is_ok());
        // A purported future proof beyond the allowed skew is never accepted.
        source.evidence.verified_at_unix_ms = 12_151;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::StaleAuthority)));
        // Nor can an expired freshness attestation be replayed.
        source.evidence.verified_at_unix_ms = 4_000;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::StaleAuthority)));
        // An attestation cannot claim arbitrarily long validity even while
        // its observation timestamp is fresh and signatures are valid.
        source.evidence.verified_at_unix_ms = 10_150;
        source.evidence.valid_until_unix_ms = 10_150 + MAX_FRESH_EVIDENCE_LEASE_MS + 1;
        assert!(matches!(f.verify(&verifier, &source), Err(GameRouteError::StaleAuthority)));
    }

    #[test]
    fn incompatible_realm_wrong_key_and_invalid_transport_rejected() {
        let mut f = Fixture::new();
        f.sign();
        let mut tampered = f.offer.clone();
        tampered.realm_hash = Hash32::default();
        assert!(matches!(tampered.verify_signature(), Err(GameRouteError::InvalidTranscript)));
        tampered = f.offer.clone();
        tampered.authority_transport_peer = vec![0; 255];
        assert!(matches!(tampered.verify_signature(), Err(GameRouteError::InvalidTranscript)));
        tampered = f.offer.clone();
        tampered.version += 1;
        assert!(matches!(tampered.verify_signature(), Err(GameRouteError::InvalidTranscript)));
        tampered = f.offer.clone();
        assert!(matches!(tampered.sign(&SigningKey::from_bytes(&[19; 32])), Err(GameRouteError::InvalidIdentity)));
    }

    #[test]
    fn corrupted_floor_does_not_silently_reset_generation() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        f.verify(&verifier, &f.source()).unwrap();
        fs::write(f.root.path().join(format!("{}.floor", f.offer.world_id.to_hex())), b"corrupted").unwrap();
        assert!(matches!(f.verify(&verifier, &f.source()), Err(GameRouteError::InvalidTranscript)));
    }

    #[test]
    fn trailing_bytes_after_valid_durable_generation_are_never_ignored() {
        let mut f = Fixture::new();
        f.sign();
        let verifier = RouteVerifier::new(f.root.path());
        f.verify(&verifier, &f.source()).unwrap();
        let path = f.root.path().join(format!("{}.floor", f.offer.world_id.to_hex()));
        OpenOptions::new().append(true).open(path).unwrap().write_all(b"junk").unwrap();
        assert!(matches!(f.verify(&verifier, &f.source()), Err(GameRouteError::InvalidTranscript)));
    }

    #[cfg(unix)]
    #[test]
    fn world_writable_floor_directory_cannot_reset_the_monotonic_generation() {
        use std::os::unix::fs::PermissionsExt;
        let mut f = Fixture::new();
        f.sign();
        let shared = f.root.path().join("untrusted");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o777)).unwrap();
        let verifier = RouteVerifier::new(shared);
        assert!(matches!(f.verify(&verifier, &f.source()), Err(GameRouteError::InvalidTranscript)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_write_through_floor_replacement_overwrites_an_existing_floor() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("generation.floor");
        let next = root.path().join("generation.new");
        fs::write(&old, b"epoch 3").unwrap();
        fs::write(&next, b"epoch 4").unwrap();
        replace_floor_atomically(&next, &old).unwrap();
        assert_eq!(fs::read(&old).unwrap(), b"epoch 4");
        assert!(!next.exists());
    }
}
