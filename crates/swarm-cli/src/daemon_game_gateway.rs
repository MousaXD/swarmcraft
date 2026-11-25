//! One explicitly selected world per running daemon may own a stable local
//! Minecraft gateway. Every player connection independently resolves a fresh
//! signed route and travels through the Noise-authenticated libp2p game stream;
//! no configured IP, migration Ready file or old endpoint is a fallback.

use anyhow::{anyhow, bail, Context, Result};
use std::{
    future::Future,
    net::SocketAddrV4,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use swarm_cli::{
    game_route_api::{GameRouteDiagnostic, GameRouteStatusV1},
    live_authority::SignedLiveAuthoritySource,
};
use swarm_core::{verify_signature, DataPaths, PeerIdentity};
use swarm_network::{
    game_port_registry::GameWorldPortRegistry,
    game_route::RouteVerifier,
    game_route_control::{GameRouteControlRequestV1, GameRouteControlResponseV1},
    game_route_host::AuthenticatedGameRouteHostSigner,
    game_route_resolver::{AuthenticatedGameRouteResolver, GameRouteCandidateSource},
    game_transport::{
        GameSessionDiagnostic, GameSessionDiagnostics, GameStreamClient, GameTransportError, GatewayRouteResolver,
        StableLoopbackGateway, VerifiedForwardRoute, VerifiedGameRouteStatus,
    },
    SwarmNode, TransportPeerId,
};
use swarm_protocol::{PeerId, WorldId};
use swarm_storage::Storage;
use tokio::{
    sync::{oneshot, watch, Semaphore},
    task::JoinHandle,
    time::timeout,
};

const GATEWAY_PORT_BASE: u16 = 30_000;
const GATEWAY_PORT_SPAN: u16 = 20_000;
pub(super) const STATUS_PROOF_TIMEOUT: Duration = Duration::from_secs(16);
const MAX_SELF_ROUTE_WORKERS: usize = 4;
const MAX_SELF_ROUTE_PROOF_WAIT: Duration = Duration::from_secs(14);

pub(super) fn preferred_gateway_port(world: WorldId) -> u16 {
    // Stable world-identity hint ONLY. The durable registry is authoritative
    // about the original port assignment and refuses collisions/busy ports.
    GATEWAY_PORT_BASE + (u16::from_be_bytes([world.0[0], world.0[1]]) % GATEWAY_PORT_SPAN)
}

struct CanonicalRouteCandidates {
    paths: DataPaths,
    authenticated: swarm_network::game_transport::GameAuthenticatedPeers,
}

impl GameRouteCandidateSource for CanonicalRouteCandidates {
    fn candidate_hosts(&self, world: WorldId) -> Vec<TransportPeerId> {
        let candidates = (|| -> Result<Vec<TransportPeerId>> {
            let storage = Storage::open(self.paths.root.clone())?;
            let epoch = storage.load_epoch_record(world)?;
            let descriptor = storage.load_world_descriptor(world)?;
            swarm_cli::live_authority::verify_playable_epoch(&storage, &epoch)?;
            verify_signature(
                epoch.authority_peer_id,
                epoch.authority_public_key,
                &epoch.signing_bytes()?,
                &epoch.signature,
            )?;
            let member =
                descriptor.member(epoch.authority_peer_id).context("accepted authority is absent from this world")?;
            if member.banned || !member.authority_eligible || member.public_key != epoch.authority_public_key {
                bail!("candidate authority is not currently eligible");
            }
            let authenticated =
                self.authenticated.read().map_err(|_| anyhow!("authenticated peers registry unavailable"))?;
            let mut connected: Vec<_> = authenticated
                .iter()
                .filter_map(|(transport, application)| (*application == epoch.authority_peer_id).then_some(*transport))
                .collect();
            connected.sort_by_key(|peer| peer.to_string());
            Ok(connected)
        })();
        candidates.unwrap_or_default()
    }
}

struct ActiveGateway {
    world: WorldId,
    address: SocketAddrV4,
    resolver: Arc<dyn GatewayRouteResolver>,
    diagnostics: GameSessionDiagnostics,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<Result<(), swarm_network::game_transport::GameTransportError>>,
}

#[derive(Default)]
pub(super) struct GameGatewayController {
    revision: u64,
    active: Option<ActiveGateway>,
}

impl Drop for GameGatewayController {
    fn drop(&mut self) {
        self.stop();
    }
}

impl GameGatewayController {
    fn stop(&mut self) {
        if let Some(old) = self.active.take() {
            let _ = old.shutdown.send(true);
            old.task.abort();
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn active_world(&self) -> Option<WorldId> {
        self.active.as_ref().and_then(|running| (!running.task.is_finished()).then_some(running.world))
    }

    pub fn status_source(&self, world: WorldId) -> Option<(Arc<dyn GatewayRouteResolver>, SocketAddrV4)> {
        self.active
            .as_ref()
            .filter(|running| running.world == world && !running.task.is_finished())
            .map(|running| (Arc::clone(&running.resolver), running.address))
    }

    pub fn recent_session(&self, world: WorldId) -> Option<GameSessionDiagnostic> {
        self.active
            .as_ref()
            .filter(|running| running.world == world && !running.task.is_finished())
            .and_then(|running| running.diagnostics.recent())
    }

    /// Selecting another world immediately revokes and cancels all old-world
    /// gateway sessions. An unavailable new world never keeps an old gateway
    /// alive, and port conflicts NEVER silently allocate a different port.
    /// The independently owned live daemon, identity, authority and route
    /// signer inputs must not be collapsed into a cached readiness boolean.
    #[allow(clippy::too_many_arguments)]
    pub async fn select(
        &mut self,
        paths: &DataPaths,
        storage: &Storage,
        identity: &PeerIdentity,
        node: &SwarmNode,
        proof: Arc<SignedLiveAuthoritySource>,
        signer: Arc<AuthenticatedGameRouteHostSigner>,
        world: WorldId,
    ) -> Result<()> {
        if world == WorldId::default() {
            bail!("invalid selected game world");
        }
        if self.active_world() == Some(world) {
            return Ok(());
        }
        self.revision = self.revision.checked_add(1).context("local game selection generation exhausted")?;
        self.stop();
        storage.load_world(world)?;
        let descriptor = storage.load_world_descriptor(world)?;
        let local =
            descriptor.member(identity.peer_id()).context("local player does not belong to the selected world")?;
        if local.banned || local.public_key != identity.public_key() {
            bail!("local selected-world membership identity is not eligible");
        }
        let directory = paths.root.join("game-gateway-ports");
        let reserved = tokio::task::spawn_blocking(move || {
            GameWorldPortRegistry::new(directory).reserve(world, preferred_gateway_port(world))
        });
        let port = timeout(Duration::from_secs(2), reserved)
            .await
            .context("stable gateway port registry could not respond")??
            .context("stable gateway port assignment is not available")?;
        let gateway = StableLoopbackGateway::bind(world, port)
            .await
            .context("reserved Minecraft gateway port is already occupied; refusing fallback")?;
        let address = gateway.local_addr()?;
        let diagnostics = gateway.session_diagnostics();
        let authenticated = node.game_authenticated_peers();
        let remote = Arc::new(AuthenticatedGameRouteResolver::new(
            identity.peer_id(),
            node.local_transport_peer_id(),
            node.game_route_rpc_client(),
            RouteVerifier::new(paths.root.join("game-route-verifier-floor")),
            authenticated.clone(),
            proof.clone(),
            Arc::new(CanonicalRouteCandidates { paths: paths.clone(), authenticated: authenticated.clone() }),
        )?);
        let local = Arc::new(LocalSignedGameResolver {
            requester: identity.peer_id(),
            transport: node.local_transport_peer_id(),
            signer,
            verifier: Arc::new(RouteVerifier::new(paths.root.join("game-route-verifier-floor"))),
            authority: proof.clone(),
            workers: Arc::new(Semaphore::new(MAX_SELF_ROUTE_WORKERS)),
        });
        let resolver: Arc<dyn GatewayRouteResolver> =
            Arc::new(SelectedWorldResolver { paths: paths.clone(), requester: identity.peer_id(), remote, local });
        let client = Arc::new(
            GameStreamClient::new(identity.peer_id(), node.local_transport_peer_id(), authenticated, proof)?
                .with_activity(node.game_traffic_activity()),
        );
        let (shutdown, rx) = watch::channel(false);
        let task = tokio::spawn(gateway.serve(resolver.clone(), client, node.game_stream_control(), rx));
        self.active = Some(ActiveGateway { world, address, resolver, diagnostics, shutdown, task });
        Ok(())
    }
}

pub(super) async fn resolve_status(
    world: WorldId,
    resolver: Arc<dyn GatewayRouteResolver>,
    address: SocketAddrV4,
) -> (GameRouteStatusV1, Option<VerifiedGameRouteStatus>) {
    let found = timeout(STATUS_PROOF_TIMEOUT, resolver.resolve(world)).await;
    let reason = if matches!(&found, Err(_) | Ok(Err(GameTransportError::Timeout))) {
        GameRouteDiagnostic::RouteCheckTimedOut
    } else {
        GameRouteDiagnostic::HostProofUnavailable
    };
    let verified = match found {
        Ok(Ok(route)) if route.world_id() == world => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|t| u64::try_from(t.as_millis()).ok());
            now.and_then(|now| route.verified_status(now))
        }
        _ => None,
    };
    match verified.and_then(|proof| GameRouteStatusV1::verified(proof, address).map(|status| (status, proof))) {
        Some(pair) => (pair.0, Some(pair.1)),
        None => (GameRouteStatusV1::unavailable_with_reason(world, reason), None),
    }
}

/// The accepted signed epoch selects a candidate transport path, never
/// authorizes gameplay. Both the self-host and remote branches must produce
/// a new RouteVerifier capability with fresh, independently signed evidence.
struct SelectedWorldResolver {
    paths: DataPaths,
    requester: PeerId,
    remote: Arc<AuthenticatedGameRouteResolver>,
    local: Arc<LocalSignedGameResolver>,
}

impl GatewayRouteResolver for SelectedWorldResolver {
    fn resolve(
        &self,
        world: WorldId,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedForwardRoute, GameTransportError>> + Send + '_>> {
        Box::pin(async move {
            let storage = Storage::open(self.paths.root.clone()).map_err(|_| GameTransportError::StaleRoute)?;
            let epoch = storage.load_epoch_record(world).map_err(|_| GameTransportError::StaleRoute)?;
            swarm_cli::live_authority::verify_playable_epoch(&storage, &epoch)
                .map_err(|_| GameTransportError::StaleRoute)?;
            if epoch.authority_peer_id == self.requester {
                self.local.resolve(world).await
            } else {
                self.remote.resolve(world).await
            }
        })
    }
}

/// The local host's own player uses the SAME loopback gateway, signed
/// offer/challenge, one-use RouteVerifier and durable generation floor. The
/// self path must never pretend its own libp2p identity is a remote-authenticated
/// PeerHello session. Only this locally owned signing key can self-attest.
struct LocalSignedGameResolver {
    requester: PeerId,
    transport: TransportPeerId,
    signer: Arc<AuthenticatedGameRouteHostSigner>,
    verifier: Arc<RouteVerifier>,
    authority: Arc<SignedLiveAuthoritySource>,
    workers: Arc<Semaphore>,
}

impl GatewayRouteResolver for LocalSignedGameResolver {
    fn resolve(
        &self,
        world: WorldId,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedForwardRoute, GameTransportError>> + Send + '_>> {
        Box::pin(async move {
            let permit = Arc::clone(&self.workers).try_acquire_owned().map_err(|_| GameTransportError::Overloaded)?;
            let signer = Arc::clone(&self.signer);
            let verifier = Arc::clone(&self.verifier);
            let authority = Arc::clone(&self.authority);
            let requester = self.requester;
            let transport = self.transport;
            let (tx, rx) = oneshot::channel();
            // Keep ownership of the worker permit until the proof actually
            // finishes; a delayed timeout must never spawn unbounded threads.
            std::thread::Builder::new()
                .name("swarm-local-game-route-proof".into())
                .spawn(move || {
                    let _permit = permit;
                    let result = (|| -> Result<VerifiedForwardRoute, GameTransportError> {
                        let offer = signer.answer(
                            requester,
                            transport,
                            GameRouteControlRequestV1::Offer { world_id: world },
                        )?;
                        let GameRouteControlResponseV1::Offer(offer) = offer else {
                            return Err(GameTransportError::Unauthorized);
                        };
                        let challenge = verifier.issue_challenge(&offer, requester, transport)?;
                        let response = signer.answer(
                            requester,
                            transport,
                            GameRouteControlRequestV1::Challenge(Box::new(challenge)),
                        )?;
                        let GameRouteControlResponseV1::ChallengeResponse(response) = response else {
                            return Err(GameTransportError::Unauthorized);
                        };
                        let now =
                            SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| GameTransportError::StaleRoute)?;
                        let now_ms = u64::try_from(now.as_millis()).map_err(|_| GameTransportError::StaleRoute)?;
                        let verified = verifier.verify(&offer, &response, world, now_ms, authority.as_ref())?;
                        VerifiedForwardRoute::new(verified, *offer, *response)
                    })();
                    let _ = tx.send(result);
                })
                .map_err(|_| GameTransportError::Unavailable)?;
            timeout(MAX_SELF_ROUTE_PROOF_WAIT, rx)
                .await
                .map_err(|_| GameTransportError::Timeout)?
                .map_err(|_| GameTransportError::Unavailable)?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_port_hint_is_deterministic_bounded_and_does_not_authorize() {
        let a = preferred_gateway_port(WorldId([1; 32]));
        assert_eq!(a, preferred_gateway_port(WorldId([1; 32])));
        assert!((GATEWAY_PORT_BASE..GATEWAY_PORT_BASE + GATEWAY_PORT_SPAN).contains(&a));
        assert_ne!(a, preferred_gateway_port(WorldId([2; 32])));
        let controller = GameGatewayController::default();
        assert!(controller.active_world().is_none());
        assert!(controller.status_source(WorldId([1; 32])).is_none());
    }
}
