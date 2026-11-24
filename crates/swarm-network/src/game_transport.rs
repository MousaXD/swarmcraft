//! Bounded Minecraft TCP over a separate authenticated libp2p stream.
//!
//! This is NOT a raw-IP fallback. The only dial at the hosting peer is to its
//! own configured loopback Minecraft listener, after an independent fresh
//! canonical/quorum, consent and Fabric-readiness check. Both sides drop live
//! sessions when the generation changes, hosting is withdrawn or the quorum
//! ceases to be fresh. The existing SwarmNode MUST continue polling alongside
//! these tasks or no libp2p streams can make progress.

use crate::game_port_registry::{GamePortRegistryError, GameWorldPortRegistry};
use crate::game_route::{
    FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError, MinecraftRouteOfferV1, MinecraftRouteResponseV1,
    VerifiedGameRoute, GAME_STREAM_PROTOCOL, MAX_FRESH_EVIDENCE_AGE_MS, MAX_FRESH_EVIDENCE_LEASE_MS,
    MAX_ROUTE_WIRE_BYTES,
};
use crate::game_stream_replay::{DurableGameStreamReplayLedger, GameStreamReplayError};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures::{AsyncReadExt as _, AsyncWriteExt as _, StreamExt as _};
use libp2p::{PeerId as TransportPeerId, Stream, StreamProtocol};
use libp2p_stream::{Control, IncomingStreams};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    io,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use swarm_protocol::{PeerId, WorldId};
use thiserror::Error;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore},
    task::{self, JoinSet},
    time::{sleep_until, timeout},
};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{debug, warn};

const GAME_ACK: u8 = 0xa5;
pub const MAX_GAME_STREAMS: usize = 16;
pub const MAX_GAME_STREAMS_PER_PEER: usize = 4;
pub const MAX_GAME_HEADER_WAIT: Duration = Duration::from_secs(4);
/// Fresh quorum and signed Fabric IPC require multiple sequential signed
/// offer/challenge steps. Do not use the four-second *frame I/O* timeout as a
/// route-verification budget: that would make an honest three-peer route
/// impossible before the Minecraft client can even send its first packet.
pub const MAX_GAME_ROUTE_RESOLVE_WAIT: Duration = Duration::from_secs(18);
pub const MAX_GAME_DIAL_WAIT: Duration = Duration::from_secs(3);
pub const MAX_GAME_SESSION: Duration = Duration::from_secs(4 * 60 * 60);
pub const GAME_REVALIDATE_INTERVAL: Duration = Duration::from_secs(1);
/// A hung Fabric IPC or quorum check must not stall the libp2p reactor.
pub const MAX_GAME_LIVE_PROOF_WAIT: Duration = Duration::from_secs(3);
pub const MAX_GAME_LIVE_PROOF_WORKERS: usize = 8;
/// Bound to both the application-authenticated requester and the encrypted
/// libp2p transport identity. A stream doesn't authorize membership by itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GameStreamOpenV1 {
    version: u16,
    offer: MinecraftRouteOfferV1,
    response: MinecraftRouteResponseV1,
    requester_application_peer: PeerId,
    requester_transport_peer: Vec<u8>,
    nonce: [u8; 32],
}

#[derive(Debug, Error)]
pub enum GameTransportError {
    #[error("unauthorized game connection or stale authority")]
    Unauthorized,
    #[error("game transport resource limit reached")]
    Overloaded,
    #[error("game stream or local server is unavailable")]
    Unavailable,
    #[error("game transport deadline exceeded")]
    Timeout,
    #[error("invalid or oversized game stream handshake")]
    InvalidFrame,
    #[error("verified route is stale")]
    StaleRoute,
    #[error(transparent)]
    PortRegistry(#[from] GamePortRegistryError),
    #[error(transparent)]
    Route(#[from] GameRouteError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Return only proof produced by the real, one-time RouteVerifier; callers
/// cannot construct VerifiedGameRoute from a configured IP or persisted status.
#[derive(Debug)]
pub struct VerifiedForwardRoute {
    route: VerifiedGameRoute,
    offer: MinecraftRouteOfferV1,
    response: MinecraftRouteResponseV1,
}

impl VerifiedForwardRoute {
    pub fn new(
        route: VerifiedGameRoute,
        offer: MinecraftRouteOfferV1,
        response: MinecraftRouteResponseV1,
    ) -> Result<Self, GameTransportError> {
        if !route.is_fresh()
            || route.world_id() != offer.world_id
            || route.authority_peer_id() != offer.authority_peer_id
            || route.transport_peer().to_bytes() != offer.authority_transport_peer
            || route.epoch() != offer.epoch
            || route.fencing_token() != offer.fencing_token
            || route.offer_hash() != offer.offer_hash()?
            || response.challenge.offer_hash != route.offer_hash()
            || response.challenge.world_id != route.world_id()
            || response.epoch != route.epoch()
            || response.fencing_token != route.fencing_token()
        {
            return Err(GameTransportError::StaleRoute);
        }
        verify_response_signature(&offer, &response)?;
        Ok(Self { route, offer, response })
    }

    pub fn world_id(&self) -> WorldId {
        self.route.world_id()
    }

    /// A read-only status projection from an *already verified*, one-use
    /// signed route and the anti-rollback RouteVerifier. It does not allow
    /// callers to construct a route or extend its authority lease.
    pub fn verified_status(&self, now_unix_ms: u64) -> Option<VerifiedGameRouteStatus> {
        if !self.route.is_fresh()
            || self.offer.world_id != self.route.world_id()
            || self.offer.authority_peer_id != self.route.authority_peer_id()
            || self.response.authority_peer_id != self.route.authority_peer_id()
            || self.response.epoch != self.route.epoch()
            || self.response.fencing_token != self.route.fencing_token()
        {
            return None;
        }
        let expires = self
            .offer
            .expires_unix_ms
            .min(self.response.expires_unix_ms)
            .min(now_unix_ms.saturating_add(u64::try_from(self.route.time_remaining().as_millis()).ok()?));
        if expires <= now_unix_ms {
            return None;
        }
        Some(VerifiedGameRouteStatus {
            world_id: self.route.world_id(),
            authority_peer_id: self.route.authority_peer_id(),
            transport_peer: self.route.transport_peer(),
            consent_revision: self.offer.consent_revision,
            epoch: self.route.epoch(),
            fencing_token: self.route.fencing_token(),
            checkpoint_hash: self.offer.checkpoint_hash,
            verified_at_unix_ms: now_unix_ms,
            valid_until_unix_ms: expires,
        })
    }
}

/// Can only be projected from an independently verified signed forward route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedGameRouteStatus {
    world_id: WorldId,
    authority_peer_id: PeerId,
    transport_peer: TransportPeerId,
    consent_revision: u64,
    epoch: u64,
    fencing_token: u64,
    checkpoint_hash: swarm_protocol::Hash32,
    verified_at_unix_ms: u64,
    valid_until_unix_ms: u64,
}

impl VerifiedGameRouteStatus {
    pub fn world_id(&self) -> WorldId {
        self.world_id
    }
    pub fn authority_peer_id(&self) -> PeerId {
        self.authority_peer_id
    }
    pub fn transport_peer(&self) -> TransportPeerId {
        self.transport_peer
    }
    pub fn consent_revision(&self) -> u64 {
        self.consent_revision
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn fencing_token(&self) -> u64 {
        self.fencing_token
    }
    pub fn checkpoint_hash(&self) -> swarm_protocol::Hash32 {
        self.checkpoint_hash
    }
    pub fn verified_at_unix_ms(&self) -> u64 {
        self.verified_at_unix_ms
    }
    pub fn valid_until_unix_ms(&self) -> u64 {
        self.valid_until_unix_ms
    }
}

/// Generated by the existing connection-bound PeerHello authentication.
/// Passing a transport ID alone without an authenticated application ID never
/// admits a game stream. All reads re-check the registry on every tick.
pub type GameAuthenticatedPeers = Arc<RwLock<HashMap<TransportPeerId, PeerId>>>;

/// The hosting peer accepts dedicated libp2p substreams and forwards them only
/// to its own managed 127.0.0.1 Minecraft listener. No externally supplied
/// hostname or IP is ever dialed.
pub struct GameStreamReceiver {
    world_id: WorldId,
    local_transport_peer: TransportPeerId,
    managed_minecraft_port: u16,
    evidence: Arc<dyn FreshAuthoritySource>,
    authenticated: GameAuthenticatedPeers,
    active: Arc<Semaphore>,
    peers: Arc<Mutex<HashMap<TransportPeerId, usize>>>,
    replay: Arc<DurableGameStreamReplayLedger>,
    replay_workers: Arc<Semaphore>,
    proof_workers: Arc<Semaphore>,
}

impl GameStreamReceiver {
    pub fn new(
        world_id: WorldId,
        local_transport_peer: TransportPeerId,
        managed_minecraft_port: u16,
        evidence: Arc<dyn FreshAuthoritySource>,
        authenticated: GameAuthenticatedPeers,
        replay_directory: impl Into<PathBuf>,
    ) -> Result<Self, GameTransportError> {
        if world_id == WorldId::default() || managed_minecraft_port == 0 {
            return Err(GameTransportError::InvalidFrame);
        }
        Ok(Self {
            world_id,
            local_transport_peer,
            managed_minecraft_port,
            evidence,
            authenticated,
            active: Arc::new(Semaphore::new(MAX_GAME_STREAMS)),
            peers: Arc::new(Mutex::new(HashMap::new())),
            replay: Arc::new(DurableGameStreamReplayLedger::new(replay_directory)),
            replay_workers: Arc::new(Semaphore::new(MAX_GAME_LIVE_PROOF_WORKERS)),
            proof_workers: Arc::new(Semaphore::new(MAX_GAME_LIVE_PROOF_WORKERS)),
        })
    }

    /// Only one receiver may register the game protocol on a SwarmNode.
    /// Dropping this future or setting shutdown=true closes new admission.
    /// Existing sessions also receive shutdown and close promptly.
    pub async fn serve(
        self: Arc<Self>,
        mut control: Control,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), GameTransportError> {
        if *shutdown.borrow() {
            return Ok(());
        }
        let mut incoming: IncomingStreams =
            control.accept(StreamProtocol::new(GAME_STREAM_PROTOCOL)).map_err(|_| GameTransportError::Unavailable)?;
        // Tie every accepted game stream to this receiver's lifetime. Dropping
        // or aborting the supervisor MUST abort its spawned sessions as well;
        // detached tokio::spawn tasks would otherwise survive world switches.
        let mut sessions = JoinSet::new();
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                }
                result = sessions.join_next(), if !sessions.is_empty() => {
                    if let Some(Err(error)) = result {
                        warn!(%error, "game receiver task failed");
                    }
                }
                item = incoming.next() => {
                    let Some((peer, stream)) = item else { return Ok(()); };
                    let Ok(global) = Arc::clone(&self.active).try_acquire_owned() else {
                        debug!(%peer, "game stream capacity exhausted");
                        continue;
                    };
                    let Ok(peer_slot) = PeerSlot::acquire(Arc::clone(&self.peers), peer) else {
                        continue;
                    };
                    let receiver = Arc::clone(&self);
                    let stopped = shutdown.clone();
                    sessions.spawn(async move {
                        let _global = global;
                        let _peer = peer_slot;
                        if let Err(error) = receiver.handle_incoming(peer, stream, stopped).await {
                            debug!(%peer, %error, "game stream refused or disconnected");
                        }
                    });
                }
            }
        }
    }

    async fn handle_incoming(
        &self,
        remote: TransportPeerId,
        mut stream: Stream,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), GameTransportError> {
        if *shutdown.borrow() {
            return Err(GameTransportError::StaleRoute);
        }
        let frame = timeout(MAX_GAME_HEADER_WAIT, read_game_open(&mut stream))
            .await
            .map_err(|_| GameTransportError::Timeout)??;
        let now = unix_ms()?;
        let app_peer = authenticated_peer(&self.authenticated, remote).ok_or(GameTransportError::Unauthorized)?;
        if frame.version != 1
            || frame.offer.world_id != self.world_id
            || frame.offer.game_server_port != self.managed_minecraft_port
            || frame.requester_application_peer != app_peer
            || frame.requester_transport_peer != remote.to_bytes()
            || frame.response.challenge.requester_peer_id != app_peer
            || frame.response.challenge.requester_transport_peer != remote.to_bytes()
            || frame.nonce == [0; 32]
            || frame.offer.authority_transport_peer != self.local_transport_peer.to_bytes()
        {
            return Err(GameTransportError::Unauthorized);
        }
        verify_response_signature(&frame.offer, &frame.response)?;
        validate_timestamps(&frame.offer, &frame.response, now)?;
        // The signed response challenge was already used by the host route
        // signer. Obtain a new independent quorum/Fabric proof for admission
        // instead of replaying that consumed nonce. The durable replay ledger
        // below STILL consumes the original signed transcript challenge.
        let mut admission_nonce = [0; 32];
        OsRng.fill_bytes(&mut admission_nonce);
        validate_live_evidence_bounded(
            Arc::clone(&self.evidence),
            Arc::clone(&self.proof_workers),
            &frame.offer,
            app_peer,
            admission_nonce,
            now,
        )
        .await?;
        // The client-chosen open nonce is NOT sufficient replay protection:
        // an attacker could modify it while replaying a still-valid signed
        // route response. Persist the SIGNED challenge before admitting any
        // Minecraft stream; an interrupted host process must not forget it.
        self.consume_open_nonce(frame.response.challenge.nonce, frame.response.expires_unix_ms, unix_ms()?).await?;

        // The only TCP dial on the host is a locally configured loopback port.
        // Never trust remote endpoint hints, DNS, persisted Ready, or raw IP.
        let destination = SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.managed_minecraft_port);
        let mut server = timeout(MAX_GAME_DIAL_WAIT, TcpStream::connect(destination))
            .await
            .map_err(|_| GameTransportError::Timeout)??;
        server.set_nodelay(true)?;
        // A loopback dial can outlive a consent/fence transition. Recheck
        // independently *after* the dial and before acknowledging admission
        // or forwarding the first Minecraft byte. Never rely on the earlier
        // admission check after an awaited network operation.
        if *shutdown.borrow() || authenticated_peer(&self.authenticated, remote) != Some(app_peer) {
            return Err(GameTransportError::Unauthorized);
        }
        let mut post_dial_challenge = [0; 32];
        OsRng.fill_bytes(&mut post_dial_challenge);
        let post_dial_now = unix_ms()?;
        validate_timestamps(&frame.offer, &frame.response, post_dial_now)?;
        let mut proof_deadline = validate_live_evidence_bounded(
            Arc::clone(&self.evidence),
            Arc::clone(&self.proof_workers),
            &frame.offer,
            app_peer,
            post_dial_challenge,
            post_dial_now,
        )
        .await?;
        // An independently verified proof is not a permit to reuse an
        // already-expired signed route. Its blocking validation can consume
        // the remaining host handshake lease; recheck at the forwarding edge.
        validate_timestamps(&frame.offer, &frame.response, unix_ms()?)?;
        if *shutdown.borrow()
            || Instant::now() >= proof_deadline
            || authenticated_peer(&self.authenticated, remote) != Some(app_peer)
        {
            return Err(GameTransportError::StaleRoute);
        }
        timeout(MAX_GAME_HEADER_WAIT, async {
            stream.write_all(&[GAME_ACK]).await?;
            stream.flush().await
        })
        .await
        .map_err(|_| GameTransportError::Timeout)??;
        let mut stream = stream.compat();
        if Instant::now() >= proof_deadline {
            return Err(GameTransportError::StaleRoute);
        }
        // Preserve copy state between heartbeat ticks; recreating
        // copy_bidirectional would silently discard buffered Minecraft bytes.
        let copying = tokio::io::copy_bidirectional(&mut stream, &mut server);
        tokio::pin!(copying);
        let transfer = async {
            loop {
                tokio::select! {
                    biased;
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { return Err(GameTransportError::StaleRoute); }
                    }
                    _ = sleep_until(next_proof_check(proof_deadline)) => {
                        if authenticated_peer(&self.authenticated, remote) != Some(app_peer) {
                            return Err(GameTransportError::Unauthorized);
                        }
                        let mut fresh_nonce = [0; 32];
                        OsRng.fill_bytes(&mut fresh_nonce);
                        proof_deadline = validate_live_evidence_bounded(
                            Arc::clone(&self.evidence), Arc::clone(&self.proof_workers),
                            &frame.offer, app_peer, fresh_nonce, unix_ms()?,
                        ).await?;
                    }
                    copied = &mut copying => {
                        copied?;
                        return Ok(());
                    }
                }
            }
        };
        timeout(MAX_GAME_SESSION, transfer).await.map_err(|_| GameTransportError::Timeout)??;
        Ok(())
    }

    async fn consume_open_nonce(
        &self,
        nonce: [u8; 32],
        expires_ms: u64,
        now_ms: u64,
    ) -> Result<(), GameTransportError> {
        // A disk lock/fsync may block behind a competing receiver process.
        // Never block the network reactor, spawn unbounded workers or let an
        // I/O deadline bypass replay admission.
        let slot = Arc::clone(&self.replay_workers).try_acquire_owned().map_err(|_| GameTransportError::Overloaded)?;
        let ledger = Arc::clone(&self.replay);
        let world = self.world_id;
        let result = timeout(
            MAX_GAME_LIVE_PROOF_WAIT,
            task::spawn_blocking(move || {
                let _slot = slot;
                ledger.consume(world, nonce, expires_ms, now_ms)
            }),
        )
        .await
        .map_err(|_| GameTransportError::Timeout)?
        .map_err(|_| GameTransportError::Unavailable)?;
        match result {
            Ok(()) => Ok(()),
            Err(GameStreamReplayError::Replayed) => Err(GameTransportError::Unauthorized),
            Err(GameStreamReplayError::Capacity) => Err(GameTransportError::Overloaded),
            Err(GameStreamReplayError::InvalidState | GameStreamReplayError::Storage(_)) => {
                Err(GameTransportError::Unavailable)
            }
        }
    }
}

struct PeerSlot {
    counts: Arc<Mutex<HashMap<TransportPeerId, usize>>>,
    peer: TransportPeerId,
}

impl PeerSlot {
    fn acquire(
        counts: Arc<Mutex<HashMap<TransportPeerId, usize>>>,
        peer: TransportPeerId,
    ) -> Result<Self, GameTransportError> {
        {
            let mut map = counts.lock().map_err(|_| GameTransportError::Unavailable)?;
            let count = map.entry(peer).or_default();
            if *count >= MAX_GAME_STREAMS_PER_PEER {
                return Err(GameTransportError::Overloaded);
            }
            *count += 1;
        }
        Ok(Self { counts, peer })
    }
}

impl Drop for PeerSlot {
    fn drop(&mut self) {
        if let Ok(mut counts) = self.counts.lock() {
            if let Some(count) = counts.get_mut(&self.peer) {
                *count -= 1;
                if *count == 0 {
                    counts.remove(&self.peer);
                }
            }
        }
    }
}

/// All game stream handshakes are length-prefixed and bounded. Neither the
/// receiver nor the gateway allocates buffers based on untrusted unbounded
/// length fields. These frames are NOT sent over snapshot WireRequest.
async fn read_game_open<R: futures::AsyncRead + Unpin>(stream: &mut R) -> Result<GameStreamOpenV1, GameTransportError> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_ROUTE_WIRE_BYTES {
        return Err(GameTransportError::InvalidFrame);
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await?;
    // postcard::from_bytes intentionally ignores trailing data. A framed
    // game capability must be ONE canonical transcript, not a valid prefix
    // followed by attacker-controlled uninterpreted bytes.
    let (frame, trailing) =
        postcard::take_from_bytes::<GameStreamOpenV1>(&body).map_err(|_| GameTransportError::InvalidFrame)?;
    if !trailing.is_empty() {
        return Err(GameTransportError::InvalidFrame);
    }
    Ok(frame)
}

async fn write_game_open<W: futures::AsyncWrite + Unpin>(
    stream: &mut W,
    frame: &GameStreamOpenV1,
) -> Result<(), GameTransportError> {
    let body = postcard::to_allocvec(frame).map_err(|_| GameTransportError::InvalidFrame)?;
    if body.is_empty() || body.len() > MAX_ROUTE_WIRE_BYTES {
        return Err(GameTransportError::InvalidFrame);
    }
    let len = u32::try_from(body.len()).map_err(|_| GameTransportError::InvalidFrame)?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

fn unix_ms() -> Result<u64, GameTransportError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| GameTransportError::StaleRoute)?;
    u64::try_from(elapsed.as_millis()).map_err(|_| GameTransportError::StaleRoute)
}

fn authenticated_peer(registry: &GameAuthenticatedPeers, transport: TransportPeerId) -> Option<PeerId> {
    registry.read().ok()?.get(&transport).copied()
}

fn verify_response_signature(
    offer: &MinecraftRouteOfferV1,
    response: &MinecraftRouteResponseV1,
) -> Result<(), GameTransportError> {
    offer.verify_signature()?;
    if response.challenge.offer_hash != offer.offer_hash()?
        || response.challenge.world_id != offer.world_id
        || response.authority_peer_id != offer.authority_peer_id
        || response.epoch != offer.epoch
        || response.fencing_token != offer.fencing_token
        || response.runtime_instance_nonce != offer.runtime_instance_nonce
    {
        return Err(GameTransportError::StaleRoute);
    }
    let key = VerifyingKey::from_bytes(&offer.authority_public_key).map_err(|_| GameTransportError::Unauthorized)?;
    let signature = Signature::from_slice(&response.signature).map_err(|_| GameTransportError::Unauthorized)?;
    key.verify(&response.signing_bytes()?, &signature).map_err(|_| GameTransportError::Unauthorized)
}

fn validate_timestamps(
    offer: &MinecraftRouteOfferV1,
    response: &MinecraftRouteResponseV1,
    now: u64,
) -> Result<(), GameTransportError> {
    if now < offer.issued_unix_ms.saturating_sub(2_000)
        || now >= offer.expires_unix_ms
        || now < response.issued_unix_ms.saturating_sub(2_000)
        || now >= response.expires_unix_ms
    {
        debug!(
            now,
            offer_expires = offer.expires_unix_ms,
            response_expires = response.expires_unix_ms,
            "signed game handshake expired or not yet valid"
        );
        return Err(GameTransportError::StaleRoute);
    }
    Ok(())
}

/// The source MUST verify quorum, accepted history, membership, hosting
/// consent, and fresh authenticated Fabric readiness for this NEW nonce.
/// Nonce is deliberately renewed during active game streaming. If the source
/// cannot prove any of these, the session closes rather than using cached
/// transport or an old signed offer.
fn validate_live_evidence(
    source: &dyn FreshAuthoritySource,
    offer: &MinecraftRouteOfferV1,
    requester: PeerId,
    nonce: [u8; 32],
    now: u64,
) -> Result<Instant, GameTransportError> {
    let started = Instant::now();
    let proof: FreshAuthorityEvidence = source.verify_fresh(offer.world_id, requester, nonce)?;
    // Never authorize an expired runtime/quorum lease merely because the
    // source blocked after its caller sampled the clock. The monotonic
    // elapsed-time bound also fails closed across a backwards system clock
    // adjustment.
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let effective_now = now.saturating_add(elapsed_ms).max(unix_ms()?);
    if proof.world_id != offer.world_id
        || proof.realm_hash != offer.realm_hash
        || proof.authority_peer_id != offer.authority_peer_id
        || proof.authority_public_key != offer.authority_public_key
        || proof.authority_transport_peer.to_bytes() != offer.authority_transport_peer
        || proof.game_server_port != offer.game_server_port
        || proof.membership_hash != offer.membership_hash
        || proof.quorum_certificate_hash != offer.quorum_certificate_hash
        || proof.compatibility_fingerprint != offer.compatibility_fingerprint
        || proof.checkpoint_hash != offer.checkpoint_hash
        || proof.state_root != offer.state_root
        || proof.epoch != offer.epoch
        || proof.fencing_token != offer.fencing_token
        || proof.consent_revision != offer.consent_revision
        || proof.runtime_instance_nonce != offer.runtime_instance_nonce
        || proof.runtime_ready_proof_hash != offer.runtime_ready_proof_hash
        || proof.challenge_nonce != nonce
    {
        debug!("fresh game evidence does not match the signed canonical route");
        return Err(GameTransportError::StaleRoute);
    }
    if !proof.requester_is_member
        || !proof.authority_is_member
        || !proof.conflict_free
        || !proof.hosting_consent_valid
        || !proof.runtime_ready_verified
    {
        return Err(GameTransportError::Unauthorized);
    }
    // Evidence is collected after the caller captures now; allow only bounded
    // wall-clock advancement during verification, never arbitrary future proof.
    if proof.verified_at_unix_ms > effective_now.saturating_add(2_000)
        || effective_now.saturating_sub(proof.verified_at_unix_ms) > MAX_FRESH_EVIDENCE_AGE_MS
        || proof.valid_until_unix_ms <= effective_now
        || proof.valid_until_unix_ms > effective_now.saturating_add(MAX_FRESH_EVIDENCE_LEASE_MS)
    {
        debug!(effective_now, proof_expires = proof.valid_until_unix_ms, "fresh game proof expired or out of bounds");
        return Err(GameTransportError::StaleRoute);
    }
    let remaining = Duration::from_millis(proof.valid_until_unix_ms - effective_now);
    Instant::now().checked_add(remaining).ok_or(GameTransportError::StaleRoute)
}

/// Separate the synchronous authority, membership and authenticated Fabric
/// readiness proof from the Tokio networking reactor. A blocking adapter may
/// outlive this request's deadline: its owned permit stays in the OS worker
/// until the adapter actually returns, so repeated timeouts never create an
/// unbounded number of blocked threads. No proof or available slot = no game.
async fn validate_live_evidence_bounded(
    evidence: Arc<dyn FreshAuthoritySource>,
    workers: Arc<Semaphore>,
    offer: &MinecraftRouteOfferV1,
    requester: PeerId,
    nonce: [u8; 32],
    sampled_unix_ms: u64,
) -> Result<Instant, GameTransportError> {
    let slot = workers.try_acquire_owned().map_err(|_| GameTransportError::Overloaded)?;
    let offer = offer.clone();
    // A permanently stuck external IPC verifier must not hold a Tokio
    // blocking-pool thread (and thus a desktop daemon's shutdown) forever.
    // Use a detached OS worker with strict semaphore accounting. At most
    // MAX_GAME_LIVE_PROOF_WORKERS exist per client/receiver at a time.
    let (reply_tx, reply_rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("swarm-game-live-proof".to_owned())
        .spawn(move || {
            let _slot = slot;
            let _ = reply_tx.send(validate_live_evidence(evidence.as_ref(), &offer, requester, nonce, sampled_unix_ms));
        })
        .map_err(|_| GameTransportError::Unavailable)?;
    let deadline = timeout(MAX_GAME_LIVE_PROOF_WAIT, reply_rx)
        .await
        .map_err(|_| GameTransportError::Timeout)?
        .map_err(|_| GameTransportError::Unavailable)??;
    if Instant::now() >= deadline {
        return Err(GameTransportError::StaleRoute);
    }
    Ok(deadline)
}

fn next_proof_check(deadline: Instant) -> tokio::time::Instant {
    tokio::time::Instant::from_std(deadline.min(Instant::now() + GAME_REVALIDATE_INTERVAL))
}

/// Gateway side of a dedicated authenticated game stream. The caller must
/// supply a fresh RouteVerifier-produced capability and the associated signed
/// offer/response for this exact requester transport identity.
pub struct GameStreamClient {
    application_peer: PeerId,
    transport_peer: TransportPeerId,
    authenticated: GameAuthenticatedPeers,
    evidence: Arc<dyn FreshAuthoritySource>,
    proof_workers: Arc<Semaphore>,
}

impl GameStreamClient {
    pub fn new(
        application_peer: PeerId,
        transport_peer: TransportPeerId,
        authenticated: GameAuthenticatedPeers,
        evidence: Arc<dyn FreshAuthoritySource>,
    ) -> Result<Self, GameTransportError> {
        if application_peer == PeerId::default() {
            return Err(GameTransportError::Unauthorized);
        }
        Ok(Self {
            application_peer,
            transport_peer,
            authenticated,
            evidence,
            proof_workers: Arc::new(Semaphore::new(MAX_GAME_LIVE_PROOF_WORKERS)),
        })
    }

    pub async fn forward(
        &self,
        mut minecraft_client: TcpStream,
        authorization: VerifiedForwardRoute,
        mut control: Control,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), GameTransportError> {
        if *shutdown.borrow() {
            return Err(GameTransportError::StaleRoute);
        }
        let VerifiedForwardRoute { route, offer, response } = authorization;
        let local_authority =
            route.transport_peer() == self.transport_peer && route.authority_peer_id() == self.application_peer;
        if !route.is_fresh()
            || response.challenge.requester_peer_id != self.application_peer
            || response.challenge.requester_transport_peer != self.transport_peer.to_bytes()
            || (!local_authority
                && authenticated_peer(&self.authenticated, route.transport_peer()) != Some(route.authority_peer_id()))
        {
            return Err(GameTransportError::Unauthorized);
        }
        let now = unix_ms()?;
        validate_timestamps(&offer, &response, now)?;
        let mut nonce = [0; 32];
        OsRng.fill_bytes(&mut nonce);
        // RouteVerifier has already independently verified every canonical
        // field, persisted the generation floor, and supplied a still-fresh
        // one-use capability. Use that proof only to begin the authenticated
        // control handshake. Repeating the same pre-dial proof here used up
        // the response's hard five-second lifetime before host admission on
        // larger recovered worlds. No Minecraft bytes are forwarded until
        // the host has independently admitted the stream AND the client has
        // obtained a separate fresh post-ACK (or post-local-dial) proof.
        let frame = GameStreamOpenV1 {
            version: 1,
            offer,
            response,
            requester_application_peer: self.application_peer,
            requester_transport_peer: self.transport_peer.to_bytes(),
            nonce,
        };
        if local_authority {
            // The host's OWN Minecraft client uses the same immutable world
            // gateway. Self-dialing libp2p is intentionally unsupported;
            // instead this direct 127.0.0.1 hop requires the exact same
            // independently verified signed route and continuous fresh
            // quorum/Fabric/consent checks as a remote game stream.
            return self.forward_local_authority(minecraft_client, route, frame, shutdown).await;
        }
        let target = route.transport_peer();
        if !route.is_fresh()
            || *shutdown.borrow()
            || authenticated_peer(&self.authenticated, target) != Some(route.authority_peer_id())
        {
            return Err(GameTransportError::StaleRoute);
        }
        let mut game_stream =
            timeout(MAX_GAME_HEADER_WAIT, control.open_stream(target, StreamProtocol::new(GAME_STREAM_PROTOCOL)))
                .await
                .map_err(|_| GameTransportError::Timeout)?
                .map_err(|_| GameTransportError::Unavailable)?;
        timeout(MAX_GAME_HEADER_WAIT, write_game_open(&mut game_stream, &frame))
            .await
            .map_err(|_| GameTransportError::Timeout)??;
        let mut ack = [0; 1];
        timeout(MAX_GAME_HEADER_WAIT, game_stream.read_exact(&mut ack))
            .await
            .map_err(|_| GameTransportError::Timeout)??;
        if ack != [GAME_ACK] {
            return Err(GameTransportError::Unauthorized);
        }
        // Opening a libp2p stream and waiting for the host ACK can outlast
        // the gateway's short-lived verified route. Re-check the capability,
        // authenticated host identity and current canonical/runtime evidence
        // before forwarding any Minecraft payload, not only before dialing.
        if *shutdown.borrow() || authenticated_peer(&self.authenticated, target) != Some(route.authority_peer_id()) {
            return Err(GameTransportError::StaleRoute);
        }
        let post_ack_now = unix_ms()?;
        validate_timestamps(&frame.offer, &frame.response, post_ack_now)?;
        let mut post_ack_nonce = [0; 32];
        OsRng.fill_bytes(&mut post_ack_nonce);
        let mut proof_deadline = validate_live_evidence_bounded(
            Arc::clone(&self.evidence),
            Arc::clone(&self.proof_workers),
            &frame.offer,
            self.application_peer,
            post_ack_nonce,
            post_ack_now,
        )
        .await?;
        // The new independent proof replaces the earlier short-lived
        // observation, not the signed transcript. The transcript must
        // still be unexpired and the connection identity unchanged.
        if *shutdown.borrow() || authenticated_peer(&self.authenticated, target) != Some(route.authority_peer_id()) {
            return Err(GameTransportError::StaleRoute);
        }
        validate_timestamps(&frame.offer, &frame.response, unix_ms()?)?;
        minecraft_client.set_nodelay(true)?;
        let mut upstream = game_stream.compat();
        if Instant::now() >= proof_deadline {
            return Err(GameTransportError::StaleRoute);
        }
        let copying = tokio::io::copy_bidirectional(&mut minecraft_client, &mut upstream);
        tokio::pin!(copying);
        let transfer = async {
            loop {
                tokio::select! {
                    biased;
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { return Err(GameTransportError::StaleRoute); }
                    }
                    _ = sleep_until(next_proof_check(proof_deadline)) => {
                        if authenticated_peer(&self.authenticated, target) != Some(route.authority_peer_id()) {
                            return Err(GameTransportError::Unauthorized);
                        }
                        let mut fresh_nonce = [0; 32];
                        OsRng.fill_bytes(&mut fresh_nonce);
                        proof_deadline = validate_live_evidence_bounded(
                            Arc::clone(&self.evidence), Arc::clone(&self.proof_workers),
                            &frame.offer, self.application_peer, fresh_nonce, unix_ms()?,
                        ).await?;
                    }
                    copied = &mut copying => {
                        copied?;
                        return Ok(());
                    }
                }
            }
        };
        timeout(MAX_GAME_SESSION, transfer).await.map_err(|_| GameTransportError::Timeout)??;
        Ok(())
    }

    async fn forward_local_authority(
        &self,
        mut minecraft_client: TcpStream,
        route: VerifiedGameRoute,
        frame: GameStreamOpenV1,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), GameTransportError> {
        if *shutdown.borrow()
            || !route.is_fresh()
            || route.transport_peer() != self.transport_peer
            || route.authority_peer_id() != self.application_peer
            || frame.offer.authority_transport_peer != self.transport_peer.to_bytes()
            || frame.offer.authority_peer_id != self.application_peer
        {
            return Err(GameTransportError::Unauthorized);
        }
        validate_timestamps(&frame.offer, &frame.response, unix_ms()?)?;
        // Never dial a configured hostname, public address or IP supplied by
        // a participant. This port was proven by live authenticated Fabric
        // and bound into the signed local host offer/challenge transcript.
        let destination = SocketAddrV4::new(Ipv4Addr::LOCALHOST, frame.offer.game_server_port);
        let mut server = timeout(MAX_GAME_DIAL_WAIT, TcpStream::connect(destination))
            .await
            .map_err(|_| GameTransportError::Timeout)??;
        let mut post_dial_nonce = [0; 32];
        OsRng.fill_bytes(&mut post_dial_nonce);
        let mut proof_deadline = validate_live_evidence_bounded(
            Arc::clone(&self.evidence),
            Arc::clone(&self.proof_workers),
            &frame.offer,
            self.application_peer,
            post_dial_nonce,
            unix_ms()?,
        )
        .await?;
        if *shutdown.borrow() {
            return Err(GameTransportError::StaleRoute);
        }
        // Fresh post-dial proof was mandatory; only the signed transcript
        // deadline remains binding from the initial route verification.
        validate_timestamps(&frame.offer, &frame.response, unix_ms()?)?;
        minecraft_client.set_nodelay(true)?;
        server.set_nodelay(true)?;
        if Instant::now() >= proof_deadline {
            return Err(GameTransportError::StaleRoute);
        }
        let copying = tokio::io::copy_bidirectional(&mut minecraft_client, &mut server);
        tokio::pin!(copying);
        let transfer = async {
            loop {
                tokio::select! {
                    biased;
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return Err(GameTransportError::StaleRoute);
                        }
                    }
                    _ = sleep_until(next_proof_check(proof_deadline)) => {
                        let mut fresh_nonce = [0; 32];
                        OsRng.fill_bytes(&mut fresh_nonce);
                        proof_deadline = validate_live_evidence_bounded(
                            Arc::clone(&self.evidence), Arc::clone(&self.proof_workers),
                            &frame.offer, self.application_peer, fresh_nonce, unix_ms()?,
                        ).await?;
                    }
                    copied = &mut copying => {
                        copied?;
                        return Ok(());
                    }
                }
            }
        };
        timeout(MAX_GAME_SESSION, transfer).await.map_err(|_| GameTransportError::Timeout)??;
        Ok(())
    }
}

/// Implemented by the daemon's asynchronous, authenticated route-offer /
/// challenge exchange. The resolver MUST use RouteVerifier::verify and a real
/// FreshAuthoritySource. No default resolver or cached-IP fallback is supplied.
pub trait GatewayRouteResolver: Send + Sync {
    fn resolve(
        &self,
        world_id: WorldId,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedForwardRoute, GameTransportError>> + Send + '_>>;
}

/// A per-world, fixed-port loopback-only Minecraft TCP listener. The caller
/// persists the port in its local per-world settings. Starting a different
/// world requires stopping the old gateway; no prior route can be inherited.
pub struct StableLoopbackGateway {
    listener: TcpListener,
    world_id: WorldId,
}

impl StableLoopbackGateway {
    /// Reserve a durable per-world port before binding. An occupied saved port
    /// fails closed; no arbitrary replacement endpoint is selected.
    pub async fn bind_reserved(
        world_id: WorldId,
        preferred_port: u16,
        registry: &GameWorldPortRegistry,
    ) -> Result<Self, GameTransportError> {
        let stable_port = registry.reserve(world_id, preferred_port)?;
        Self::bind(world_id, stable_port).await
    }

    pub async fn bind(world_id: WorldId, stable_port: u16) -> Result<Self, GameTransportError> {
        if world_id == WorldId::default() || stable_port == 0 {
            return Err(GameTransportError::InvalidFrame);
        }
        // Explicit IPv4 loopback only. Do not bind to 0.0.0.0 or discover
        // Minecraft servers on the LAN; already-bound ports fail closed.
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, stable_port);
        let listener = TcpListener::bind(address).await?;
        Ok(Self { listener, world_id })
    }

    pub fn local_addr(&self) -> Result<SocketAddrV4, GameTransportError> {
        let address = self.listener.local_addr()?;
        match address {
            std::net::SocketAddr::V4(address) if address.ip().is_loopback() && address.port() != 0 => Ok(address),
            _ => Err(GameTransportError::Unauthorized),
        }
    }

    /// Every new Minecraft TCP session re-runs independent route resolution.
    /// A dropped/closed route never reuses previous host addresses. shutdown is
    /// mandatory: setting it true drops admission and all active sessions.
    pub async fn serve(
        self,
        resolver: Arc<dyn GatewayRouteResolver>,
        client: Arc<GameStreamClient>,
        control: Control,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), GameTransportError> {
        if *shutdown.borrow() {
            return Ok(());
        }
        let slots = Arc::new(Semaphore::new(MAX_GAME_STREAMS));
        // Session ownership is scoped to this gateway, not to the Tokio
        // runtime. A cancelled gateway supervisor cannot leave an old-world
        // Minecraft connection forwarding in a detached child.
        let mut sessions = JoinSet::new();
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                }
                result = sessions.join_next(), if !sessions.is_empty() => {
                    if let Some(Err(error)) = result {
                        warn!(%error, "gateway session task failed");
                    }
                }
                accepted = self.listener.accept() => {
                    let (tcp, _) = accepted?;
                    let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                        // Drop immediately; never unboundedly queue join requests.
                        drop(tcp);
                        continue;
                    };
                    let resolver = Arc::clone(&resolver);
                    let client = Arc::clone(&client);
                    let control = control.clone();
                    let stopped = shutdown.clone();
                    let world_id = self.world_id;
                    sessions.spawn(async move {
                        let _slot: OwnedSemaphorePermit = slot;
                        // Verification is bounded and always happens for this
                        // new session, including after planned/crash migration.
                        let result = timeout(MAX_GAME_ROUTE_RESOLVE_WAIT, resolver.resolve(world_id)).await;
                        match result {
                            Ok(Ok(verified)) if verified.world_id() == world_id => {
                                if let Err(error) = client.forward(tcp, verified, control, stopped).await {
                                    debug!(%error, "game gateway session ended without fallback");
                                }
                            }
                            Ok(Ok(_)) => warn!("game gateway resolver returned wrong world"),
                            Ok(Err(error)) => debug!(%error, "game route resolver refused connection"),
                            Err(_) => debug!("game route resolver timed out"),
                        }
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;
    use libp2p::identity::Keypair;
    use std::{
        net::TcpListener as StdTcpListener,
        sync::atomic::{AtomicU64, Ordering},
    };
    use swarm_protocol::{Hash32, WorldId};
    use tokio::sync::Notify;

    fn peer() -> TransportPeerId {
        Keypair::generate_ed25519().public().to_peer_id()
    }

    #[tokio::test]
    async fn invalid_and_oversized_frames_fail_before_allocation_or_forwarding() {
        for length in [0, (MAX_ROUTE_WIRE_BYTES + 1) as u32, u32::MAX] {
            let mut stream = Cursor::new(length.to_be_bytes().to_vec());
            assert!(matches!(read_game_open(&mut stream).await, Err(GameTransportError::InvalidFrame)));
        }
        let mut stream = Cursor::new(vec![0, 0, 0, 3, 0xff, 0xff, 0xff]);
        assert!(matches!(read_game_open(&mut stream).await, Err(GameTransportError::InvalidFrame)));
        let mut truncated = Cursor::new(vec![0, 0, 0, 5, 1, 2]);
        assert!(matches!(read_game_open(&mut truncated).await, Err(GameTransportError::Io(_))));
    }

    #[tokio::test]
    async fn game_open_rejects_valid_postcard_prefix_with_trailing_payload() {
        let key = ed25519_dalek::SigningKey::from_bytes(&[11; 32]);
        let application = swarm_protocol::peer_id_from_public_key(&key.verifying_key().to_bytes());
        let transport = peer();
        let world = WorldId([7; 32]);
        let offer = MinecraftRouteOfferV1 {
            version: 1,
            world_id: world,
            realm_hash: Hash32([1; 32]),
            authority_peer_id: application,
            authority_public_key: key.verifying_key().to_bytes(),
            authority_transport_peer: transport.to_bytes(),
            game_server_port: 25565,
            membership_hash: Hash32([2; 32]),
            quorum_certificate_hash: Hash32([3; 32]),
            compatibility_fingerprint: Hash32([4; 32]),
            checkpoint_hash: Hash32([5; 32]),
            state_root: Hash32([6; 32]),
            epoch: 3,
            fencing_token: 4,
            consent_revision: 2,
            runtime_instance_nonce: [8; 32],
            runtime_ready_proof_hash: Hash32([9; 32]),
            offer_id: [10; 32],
            sequence: 1,
            issued_unix_ms: 1000,
            expires_unix_ms: 5000,
            signature: vec![0; 64],
        };
        let response = MinecraftRouteResponseV1 {
            challenge: crate::game_route::MinecraftRouteChallengeV1 {
                world_id: world,
                offer_hash: Hash32([12; 32]),
                requester_peer_id: PeerId([13; 32]),
                requester_transport_peer: peer().to_bytes(),
                nonce: [14; 32],
            },
            authority_peer_id: application,
            epoch: 3,
            fencing_token: 4,
            runtime_instance_nonce: [8; 32],
            issued_unix_ms: 1100,
            expires_unix_ms: 4000,
            signature: vec![0; 64],
        };
        let frame = GameStreamOpenV1 {
            version: 1,
            offer,
            response,
            requester_application_peer: PeerId([13; 32]),
            requester_transport_peer: peer().to_bytes(),
            nonce: [15; 32],
        };
        let bytes = postcard::to_allocvec(&frame).unwrap();
        let mut good_wire = Vec::from(u32::try_from(bytes.len()).unwrap().to_be_bytes());
        good_wire.extend_from_slice(&bytes);
        let good = read_game_open(&mut Cursor::new(good_wire)).await.unwrap();
        assert_eq!(good.offer.world_id, world);

        let mut contaminated = bytes;
        contaminated.extend_from_slice(&[0xff, 0x00]);
        let mut bad_wire = Vec::from(u32::try_from(contaminated.len()).unwrap().to_be_bytes());
        bad_wire.extend_from_slice(&contaminated);
        assert!(matches!(read_game_open(&mut Cursor::new(bad_wire)).await, Err(GameTransportError::InvalidFrame)));
    }

    #[tokio::test]
    async fn per_peer_admission_and_nonce_replay_are_bounded() {
        let counts = Arc::new(Mutex::new(HashMap::new()));
        let attacker = peer();
        let mut slots = Vec::new();
        for _ in 0..MAX_GAME_STREAMS_PER_PEER {
            slots.push(PeerSlot::acquire(Arc::clone(&counts), attacker).unwrap());
        }
        assert!(matches!(PeerSlot::acquire(Arc::clone(&counts), attacker), Err(GameTransportError::Overloaded)));
        assert!(PeerSlot::acquire(Arc::clone(&counts), peer()).is_ok());
        drop(slots);
        assert!(counts.lock().unwrap().get(&attacker).is_none());

        struct NoEvidence;
        impl FreshAuthoritySource for NoEvidence {
            fn verify_fresh(
                &self,
                _: WorldId,
                _: PeerId,
                _: [u8; 32],
            ) -> Result<FreshAuthorityEvidence, GameRouteError> {
                Err(GameRouteError::UnverifiedAuthority)
            }
        }
        let world = WorldId([1; 32]);
        let private = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(private.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let replay_path = private.path().join("durable-host-replays");
        let receiver = GameStreamReceiver::new(
            world,
            peer(),
            25001,
            Arc::new(NoEvidence),
            Arc::new(RwLock::new(HashMap::new())),
            &replay_path,
        )
        .unwrap();
        receiver.consume_open_nonce([12; 32], 2_000, 1_000).await.unwrap();
        assert!(matches!(
            receiver.consume_open_nonce([12; 32], 2_000, 1_000).await,
            Err(GameTransportError::Unauthorized)
        ));
        let restarted = GameStreamReceiver::new(
            world,
            peer(),
            25001,
            Arc::new(NoEvidence),
            Arc::new(RwLock::new(HashMap::new())),
            &replay_path,
        )
        .unwrap();
        assert!(matches!(
            restarted.consume_open_nonce([12; 32], 2_000, 1_000).await,
            Err(GameTransportError::Unauthorized)
        ));
        assert!(matches!(
            GameStreamReceiver::new(
                world,
                peer(),
                0,
                Arc::new(NoEvidence),
                Arc::new(RwLock::new(HashMap::new())),
                replay_path
            ),
            Err(GameTransportError::InvalidFrame)
        ));
    }

    #[tokio::test]
    async fn gateway_only_binds_loopback_and_refuses_port_collision_and_invalid_world() {
        assert!(matches!(
            StableLoopbackGateway::bind(WorldId::default(), 25565).await,
            Err(GameTransportError::InvalidFrame)
        ));
        assert!(matches!(
            StableLoopbackGateway::bind(WorldId([1; 32]), 0).await,
            Err(GameTransportError::InvalidFrame)
        ));
        let reserved = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let gateway = StableLoopbackGateway::bind(WorldId([1; 32]), port).await.unwrap();
        assert_eq!(gateway.local_addr().unwrap().ip(), &Ipv4Addr::LOCALHOST);
        assert!(StableLoopbackGateway::bind(WorldId([2; 32]), port).await.is_err());
        drop(gateway);
        StableLoopbackGateway::bind(WorldId([2; 32]), port).await.unwrap();
    }

    #[tokio::test]
    async fn registered_gateway_never_changes_a_worlds_saved_local_port() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let registry = GameWorldPortRegistry::new(root.path().join("gateway-ports"));
        let world = WorldId([20; 32]);
        let other = WorldId([21; 32]);
        let reserved = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let original = reserved.local_addr().unwrap().port();
        drop(reserved);
        let gateway = StableLoopbackGateway::bind_reserved(world, original, &registry).await.unwrap();
        assert_eq!(gateway.local_addr().unwrap().port(), original);
        drop(gateway);

        // A different preference on restart does not silently change the
        // connection entry already saved in a Minecraft client.
        let new_preference = if original == u16::MAX { original - 1 } else { original + 1 };
        let restarted = StableLoopbackGateway::bind_reserved(world, new_preference, &registry).await.unwrap();
        assert_eq!(restarted.local_addr().unwrap().port(), original);
        drop(restarted);
        assert!(matches!(
            StableLoopbackGateway::bind_reserved(other, original, &registry).await,
            Err(GameTransportError::PortRegistry(GamePortRegistryError::PortReservedForAnotherWorld))
        ));
    }

    #[test]
    fn unauthenticated_transport_is_not_application_admission() {
        let authenticated: GameAuthenticatedPeers = Arc::new(RwLock::new(HashMap::new()));
        let target = peer();
        let world_member = PeerId([11; 32]);
        assert_eq!(authenticated_peer(&authenticated, target), None);
        authenticated.write().unwrap().insert(target, world_member);
        assert_eq!(authenticated_peer(&authenticated, target), Some(world_member));
        authenticated.write().unwrap().remove(&target);
        assert_eq!(authenticated_peer(&authenticated, target), None);
        // No route can be fabricated from this registry; it holds identity
        // only, not quorum, runtime readiness, or the capability itself.
        assert_ne!(Hash32([1; 32]), Hash32::default());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_game_quorum_checks_are_bounded_off_reactor_even_after_timeout() {
        struct DelayedUnavailableProof {
            delay_ms: AtomicU64,
            started: Notify,
        }

        impl FreshAuthoritySource for DelayedUnavailableProof {
            fn verify_fresh(
                &self,
                _world: WorldId,
                _requester: PeerId,
                _nonce: [u8; 32],
            ) -> Result<FreshAuthorityEvidence, GameRouteError> {
                self.started.notify_one();
                std::thread::sleep(Duration::from_millis(self.delay_ms.load(Ordering::SeqCst)));
                Err(GameRouteError::UnverifiedAuthority)
            }
        }

        // No authorization can be granted from these intentionally incomplete
        // test fields; the source ALWAYS refuses after a blocking delay.
        let authority = peer();
        let offer = MinecraftRouteOfferV1 {
            version: 1,
            world_id: WorldId([1; 32]),
            realm_hash: Hash32([2; 32]),
            authority_peer_id: PeerId([3; 32]),
            authority_public_key: [4; 32],
            authority_transport_peer: authority.to_bytes(),
            game_server_port: 25565,
            membership_hash: Hash32([5; 32]),
            quorum_certificate_hash: Hash32([6; 32]),
            compatibility_fingerprint: Hash32([7; 32]),
            checkpoint_hash: Hash32([8; 32]),
            state_root: Hash32([9; 32]),
            epoch: 1,
            fencing_token: 1,
            consent_revision: 1,
            runtime_instance_nonce: [10; 32],
            runtime_ready_proof_hash: Hash32([11; 32]),
            offer_id: [12; 32],
            sequence: 1,
            issued_unix_ms: 1,
            expires_unix_ms: 2,
            signature: vec![],
        };
        let source = Arc::new(DelayedUnavailableProof { delay_ms: AtomicU64::new(350), started: Notify::new() });
        let workers = Arc::new(Semaphore::new(1));
        let proof = source.clone();
        let slots = Arc::clone(&workers);
        let candidate = offer.clone();
        let started_at = Instant::now();
        let first = tokio::spawn(async move {
            validate_live_evidence_bounded(proof, slots, &candidate, PeerId([14; 32]), [15; 32], 100).await
        });
        timeout(Duration::from_secs(2), source.started.notified()).await.expect("proof must begin in a worker");
        assert!(
            started_at.elapsed() < Duration::from_millis(300),
            "blocking proof must not occupy the Tokio networking thread for 350ms"
        );
        assert!(matches!(
            validate_live_evidence_bounded(
                source.clone(),
                Arc::clone(&workers),
                &offer,
                PeerId([14; 32]),
                [16; 32],
                100,
            )
            .await,
            Err(GameTransportError::Overloaded)
        ));
        assert!(matches!(first.await.unwrap(), Err(GameTransportError::Route(GameRouteError::UnverifiedAuthority))));

        source.delay_ms.store(3_700, Ordering::SeqCst);
        let proof = source.clone();
        let slots = Arc::clone(&workers);
        let candidate = offer.clone();
        let timed = tokio::spawn(async move {
            validate_live_evidence_bounded(proof, slots, &candidate, PeerId([14; 32]), [17; 32], 100).await
        });
        timeout(Duration::from_secs(2), source.started.notified()).await.expect("stalled proof must start");
        assert!(matches!(
            timeout(Duration::from_millis(3_200), timed)
                .await
                .expect("stalled proof must return before deadline")
                .unwrap(),
            Err(GameTransportError::Timeout)
        ));
        assert!(matches!(
            validate_live_evidence_bounded(
                source.clone(),
                Arc::clone(&workers),
                &offer,
                PeerId([14; 32]),
                [18; 32],
                100,
            )
            .await,
            Err(GameTransportError::Overloaded)
        ));
        // The timed-out OS worker cannot be killed, so wait until it exits
        // before dropping this test's fake authority environment.
        tokio::time::sleep(Duration::from_millis(850)).await;
        assert_eq!(workers.available_permits(), 1);
    }

    #[tokio::test]
    async fn same_host_gateway_needs_signed_local_route_and_closes_on_live_consent_loss() {
        use crate::{
            game_route::RouteVerifier,
            game_route_control::{GameRouteControlRequestV1, GameRouteControlResponseV1},
            game_route_host::AuthenticatedGameRouteHostSigner,
        };
        use ed25519_dalek::SigningKey;
        use std::sync::atomic::{AtomicBool, Ordering};
        use swarm_protocol::peer_id_from_public_key;
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        struct LocalTestAuthority {
            world: WorldId,
            authority: PeerId,
            key: [u8; 32],
            transport: TransportPeerId,
            port: u16,
            consent: AtomicBool,
        }

        impl FreshAuthoritySource for LocalTestAuthority {
            fn verify_fresh(
                &self,
                world: WorldId,
                requester: PeerId,
                challenge: [u8; 32],
            ) -> Result<FreshAuthorityEvidence, GameRouteError> {
                if !self.consent.load(Ordering::SeqCst)
                    || requester != self.authority
                    || world != self.world
                    || challenge == [0; 32]
                {
                    return Err(GameRouteError::UnverifiedAuthority);
                }
                // A realistic sequence of independent observations can
                // outlast the initial proof lease without relying on it.
                std::thread::sleep(Duration::from_millis(200));
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
                Ok(FreshAuthorityEvidence {
                    world_id: world,
                    realm_hash: Hash32([1; 32]),
                    authority_peer_id: self.authority,
                    authority_public_key: self.key,
                    authority_transport_peer: self.transport,
                    game_server_port: self.port,
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
                    epoch: 3,
                    fencing_token: 7,
                    consent_revision: 2,
                    runtime_instance_nonce: [8; 32],
                    runtime_ready_proof_hash: Hash32([9; 32]),
                    challenge_nonce: challenge,
                    verified_at_unix_ms: now,
                    valid_until_unix_ms: now + 300,
                })
            }
        }

        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let world = WorldId([17; 32]);
        let key = SigningKey::from_bytes(&[33; 32]);
        let local_id = peer_id_from_public_key(&key.verifying_key().to_bytes());
        let transport = peer();
        let managed = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = managed.local_addr().unwrap().port();
        let source = Arc::new(LocalTestAuthority {
            world,
            authority: local_id,
            key: key.verifying_key().to_bytes(),
            transport,
            port,
            consent: AtomicBool::new(true),
        });
        let signer = AuthenticatedGameRouteHostSigner::new(
            key,
            transport,
            source.clone(),
            root.path().join("signed-offer-sequences"),
        );
        let verifier = RouteVerifier::new(root.path().join("route-verifier-floor"));
        let GameRouteControlResponseV1::Offer(offer) =
            signer.answer(local_id, transport, GameRouteControlRequestV1::Offer { world_id: world }).unwrap()
        else {
            panic!("local host must prove a signed offer")
        };
        let challenge = verifier.issue_challenge(&offer, local_id, transport).unwrap();
        let GameRouteControlResponseV1::ChallengeResponse(response) =
            signer.answer(local_id, transport, GameRouteControlRequestV1::Challenge(Box::new(challenge))).unwrap()
        else {
            panic!("local host must prove the same signed generation")
        };
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let capability = verifier.verify(&offer, &response, world, now, source.as_ref()).unwrap();
        let forwarded = VerifiedForwardRoute::new(capability, *offer, *response).unwrap();
        assert!(forwarded.verified_status(now).is_some());

        // A genuine client connects to the permanent *loopback gateway* while
        // its signed authority happens to be the same local process.
        let gateway = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut minecraft = TcpStream::connect(gateway.local_addr().unwrap()).await.unwrap();
        let (local_tcp, _) = gateway.accept().await.unwrap();
        let (stop_tx, stop_rx) = watch::channel(false);
        let client =
            GameStreamClient::new(local_id, transport, Arc::new(RwLock::new(HashMap::new())), source.clone()).unwrap();
        let game_control = libp2p_stream::Behaviour::new().new_control();
        let forwarding = tokio::spawn(async move { client.forward(local_tcp, forwarded, game_control, stop_rx).await });
        let game = tokio::spawn(async move {
            let (mut server, _) = managed.accept().await.unwrap();
            let mut input = [0u8; 4];
            server.read_exact(&mut input).await.unwrap();
            assert_eq!(&input, b"ping");
            server.write_all(b"pong").await.unwrap();
            let _ = server.read(&mut input).await;
        });
        minecraft.write_all(b"ping").await.unwrap();
        let mut reply = [0u8; 4];
        timeout(Duration::from_secs(5), minecraft.read_exact(&mut reply)).await.unwrap().unwrap();
        assert_eq!(&reply, b"pong");

        // Withdrawal invalidates subsequent live proof and disconnects a
        // running world, despite an otherwise signed, recently valid offer.
        source.consent.store(false, Ordering::SeqCst);
        assert!(timeout(Duration::from_secs(5), forwarding).await.unwrap().unwrap().is_err());
        timeout(Duration::from_secs(2), game).await.unwrap().unwrap();
        let _ = stop_tx.send(true);
    }
}
