//! Authenticate route control *over the network*, then independently verify
//! the returned signed authority before allowing a Minecraft gateway join.
//!
//! Discovery and a configured candidate ID are untrusted hints, never proof.
//! Every session performs the full host offer -> fresh requester challenge ->
//! signed host response -> independent quorum/runtime/fencing verification.
//! In particular, this resolver never dials a raw Minecraft IP address or
//! treats a persisted "Ready" status as authorization.

use crate::{
    game_route::{FreshAuthoritySource, GameRouteError, RouteVerifier},
    game_route_control::{GameRouteControlRequestV1, GameRouteControlResponseV1, GameRouteRpcClient},
    game_transport::{GameAuthenticatedPeers, GameTransportError, GatewayRouteResolver, VerifiedForwardRoute},
};
use futures::{stream::FuturesUnordered, StreamExt};
use libp2p::PeerId as TransportPeerId;
use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_protocol::{PeerId, WorldId};

pub const MAX_GAME_ROUTE_CANDIDATES: usize = 8;

/// Supplies only candidate transport IDs, for example from the current world
/// membership/discovery view. The list may be stale or adversarial. Supplying
/// a host ID here NEVER makes the host authoritative or ready.
pub trait GameRouteCandidateSource: Send + Sync {
    fn candidate_hosts(&self, world: WorldId) -> Vec<TransportPeerId>;
}

/// Production-ready network mechanics with an intentionally unimplemented
/// *trust source*: the application must inject an authority adapter that
/// independently verifies a CURRENT accepted quorum, membership, fencing,
/// hosting consent and cryptographic Fabric runtime readiness. No adapter,
/// no verified route.
pub struct AuthenticatedGameRouteResolver {
    requester_application: PeerId,
    requester_transport: TransportPeerId,
    rpc: GameRouteRpcClient,
    verifier: RouteVerifier,
    authenticated: GameAuthenticatedPeers,
    authority: Arc<dyn FreshAuthoritySource>,
    candidates: Arc<dyn GameRouteCandidateSource>,
}

impl AuthenticatedGameRouteResolver {
    pub fn new(
        requester_application: PeerId,
        requester_transport: TransportPeerId,
        rpc: GameRouteRpcClient,
        verifier: RouteVerifier,
        authenticated: GameAuthenticatedPeers,
        authority: Arc<dyn FreshAuthoritySource>,
        candidates: Arc<dyn GameRouteCandidateSource>,
    ) -> Result<Self, GameTransportError> {
        if requester_application == PeerId::default() {
            return Err(GameTransportError::Unauthorized);
        }
        Ok(Self { requester_application, requester_transport, rpc, verifier, authenticated, authority, candidates })
    }

    /// This is only the application-authenticated transport identity. It
    /// must also equal the signer in a current, quorum-backed route offer.
    fn is_authenticated_as(&self, transport: TransportPeerId, application: PeerId) -> bool {
        self.authenticated.read().ok().and_then(|map| map.get(&transport).copied()) == Some(application)
    }

    async fn resolve_candidate(
        &self,
        world: WorldId,
        transport: TransportPeerId,
    ) -> Result<VerifiedForwardRoute, GameTransportError> {
        let response = self
            .rpc
            .request(transport, GameRouteControlRequestV1::Offer { world_id: world })
            .await
            .map_err(|_| GameTransportError::Unavailable)?;
        let GameRouteControlResponseV1::Offer(offer) = response else {
            return Err(GameTransportError::Unauthorized);
        };
        let signer_transport = offer.verify_signature()?;
        if offer.world_id != world
            || signer_transport != transport
            || !self.is_authenticated_as(transport, offer.authority_peer_id)
        {
            return Err(GameTransportError::Unauthorized);
        }
        // A challenge is unique, expires quickly and is consumed on the first
        // verification attempt, including an invalid or replayed answer.
        let challenge = self.verifier.issue_challenge(&offer, self.requester_application, self.requester_transport)?;
        let response = self
            .rpc
            .request(transport, GameRouteControlRequestV1::Challenge(Box::new(challenge)))
            .await
            .map_err(|_| GameTransportError::Unavailable)?;
        let GameRouteControlResponseV1::ChallengeResponse(response) = response else {
            return Err(GameTransportError::Unauthorized);
        };
        let now_ms = current_unix_ms()?;
        let verified = self.verifier.verify(&offer, &response, world, now_ms, self.authority.as_ref())?;
        if !self.is_authenticated_as(transport, offer.authority_peer_id) {
            return Err(GameTransportError::Unauthorized);
        }
        VerifiedForwardRoute::new(verified, *offer, *response)
    }
}

impl GatewayRouteResolver for AuthenticatedGameRouteResolver {
    fn resolve(
        &self,
        world: WorldId,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedForwardRoute, GameTransportError>> + Send + '_>> {
        Box::pin(async move {
            if world == WorldId::default() {
                return Err(GameTransportError::StaleRoute);
            }
            let candidates = self.candidates.candidate_hosts(world);
            if candidates.is_empty() {
                return Err(GameTransportError::Unavailable);
            }
            if candidates.len() > MAX_GAME_ROUTE_CANDIDATES {
                return Err(GameTransportError::Overloaded);
            }
            let mut visited = HashSet::with_capacity(candidates.len());
            let mut attempts = FuturesUnordered::new();
            for candidate in candidates {
                if candidate == self.requester_transport || !visited.insert(candidate) {
                    continue;
                }
                // Query authenticated candidates concurrently. A stale peer
                // may still hold an encrypted connection but never reply to
                // the route-control challenge. Serial probing could exhaust
                // the gateway's bounded join deadline before the current
                // host is even asked. The verifier still independently
                // proves one accepted canonical head before ANY forwarding.
                attempts.push(self.resolve_candidate(world, candidate));
            }
            while let Some(result) = attempts.next().await {
                match result {
                    Ok(route) => return Ok(route),
                    // Corrupt local anti-rollback state and I/O failures are
                    // fatal. Trying another candidate cannot heal them.
                    Err(GameTransportError::Route(GameRouteError::InvalidTranscript))
                    | Err(GameTransportError::Route(GameRouteError::Storage(_))) => {
                        return Err(GameTransportError::StaleRoute);
                    }
                    // Bad/refusing/stale candidates never grant gameplay.
                    Err(_) => continue,
                }
            }
            Err(GameTransportError::StaleRoute)
        })
    }
}

fn current_unix_ms() -> Result<u64, GameTransportError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| GameTransportError::StaleRoute)?;
    u64::try_from(elapsed.as_millis()).map_err(|_| GameTransportError::StaleRoute)
}
