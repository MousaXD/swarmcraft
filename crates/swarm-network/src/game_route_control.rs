//! Isolated, bounded game-route control protocol.
//!
//! The libp2p transport first authenticates the connection-bound application
//! identity. The daemon MUST still independently authorize world membership,
//! canonical quorum, hosting consent and live Fabric readiness before signing
//! an offer or challenge response. This protocol alone does not authorize a
//! game session and must never be treated as a replacement for RouteVerifier.

use crate::game_route::{
    MinecraftRouteChallengeV1, MinecraftRouteOfferV1, MinecraftRouteResponseV1, MAX_ROUTE_WIRE_BYTES,
};
use libp2p::{request_response, PeerId as TransportPeerId};
use serde::{Deserialize, Serialize};
use swarm_protocol::{Hash32, PeerId, WorldId};

pub const GAME_ROUTE_CONTROL_PROTOCOL: &str = "/swarmcraft/minecraft-route-control/1";
pub const GAME_ROUTE_CONTROL_MAX_REQUEST_BYTES: u64 = 1024;
pub const GAME_ROUTE_CONTROL_MAX_RESPONSE_BYTES: u64 = MAX_ROUTE_WIRE_BYTES as u64;
pub const GAME_ROUTE_CONTROL_MAX_OUTSTANDING: usize = 32;
pub const GAME_ROUTE_CONTROL_MAX_REQUESTS_PER_WINDOW: u32 = 8;
pub const GAME_ROUTE_CONTROL_EVENT_QUEUE: usize = 32;

/// Independent bounded subscription; adding route-control messages does not
/// change the exhaustive legacy NetworkEvent enum used by the daemon.
#[derive(Debug)]
pub enum GameRouteControlEvent {
    Inbound {
        transport_peer: TransportPeerId,
        application_peer: PeerId,
        request: GameRouteControlRequestV1,
        channel: request_response::ResponseChannel<GameRouteControlResponseV1>,
    },
    Response {
        transport_peer: TransportPeerId,
        request_id: request_response::OutboundRequestId,
        response: GameRouteControlResponseV1,
    },
    OutboundFailure {
        transport_peer: TransportPeerId,
        request_id: request_response::OutboundRequestId,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameRouteControlRequestV1 {
    /// Host is only an offer *candidate* until independent quorum validation.
    Offer { world_id: WorldId },
    /// A signed response must echo every field, including the requester
    /// application identity and the live transport identity, exactly.
    Challenge(Box<MinecraftRouteChallengeV1>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameRouteRefusalV1 {
    Unauthorized,
    QuorumUnavailable,
    ConflictingAuthority,
    HostUnready,
    FencedOrConsentWithdrawn,
    ExpiredOrMalformed,
    RateLimited,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameRouteControlResponseV1 {
    /// A signed offer is NOT a proof of current authority by itself.
    Offer(Box<MinecraftRouteOfferV1>),
    ChallengeResponse(Box<MinecraftRouteResponseV1>),
    Refused(GameRouteRefusalV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameRouteControlError {
    InvalidRequest,
    InvalidResponse,
}

impl GameRouteControlRequestV1 {
    pub fn world_id(&self) -> WorldId {
        match self {
            Self::Offer { world_id } => *world_id,
            Self::Challenge(challenge) => challenge.world_id,
        }
    }

    pub fn validate(&self) -> Result<(), GameRouteControlError> {
        match self {
            Self::Offer { world_id } if *world_id != WorldId::default() => Ok(()),
            Self::Challenge(challenge)
                if challenge.world_id != WorldId::default()
                    && challenge.offer_hash != Hash32::default()
                    && challenge.requester_peer_id != PeerId::default()
                    && challenge.nonce != [0; 32]
                    && !challenge.requester_transport_peer.is_empty()
                    && challenge.requester_transport_peer.len() <= 128
                    && TransportPeerId::from_bytes(&challenge.requester_transport_peer).is_ok() =>
            {
                Ok(())
            }
            _ => Err(GameRouteControlError::InvalidRequest),
        }
    }

    /// Match the exact requested world and challenge before forwarding any
    /// response to the authority verifier. Refusals are always fail-closed.
    pub fn accepts_response(&self, response: &GameRouteControlResponseV1) -> bool {
        match (self, response) {
            (_, GameRouteControlResponseV1::Refused(_)) => true,
            (Self::Offer { world_id }, GameRouteControlResponseV1::Offer(offer)) => {
                offer.world_id == *world_id && offer.verify_signature().is_ok()
            }
            (Self::Challenge(request), GameRouteControlResponseV1::ChallengeResponse(response)) => {
                response.challenge == **request && response.signing_bytes().is_ok() && response.signature.len() == 64
            }
            _ => false,
        }
    }
}

impl GameRouteControlResponseV1 {
    pub fn validate(&self) -> Result<(), GameRouteControlError> {
        match self {
            Self::Refused(_) => Ok(()),
            Self::Offer(offer) => {
                offer.verify_signature().map(|_| ()).map_err(|_| GameRouteControlError::InvalidResponse)
            }
            Self::ChallengeResponse(response) => {
                if response.signing_bytes().is_ok() && response.signature.len() == 64 {
                    Ok(())
                } else {
                    Err(GameRouteControlError::InvalidResponse)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    #[test]
    fn malformed_and_unbound_requests_are_rejected() {
        assert_eq!(
            GameRouteControlRequestV1::Offer { world_id: WorldId::default() }.validate(),
            Err(GameRouteControlError::InvalidRequest)
        );
        let mut challenge = MinecraftRouteChallengeV1 {
            world_id: WorldId([1; 32]),
            offer_hash: Hash32([2; 32]),
            requester_peer_id: PeerId([3; 32]),
            requester_transport_peer: Keypair::generate_ed25519().public().to_peer_id().to_bytes(),
            nonce: [4; 32],
        };
        assert!(GameRouteControlRequestV1::Challenge(Box::new(challenge.clone())).validate().is_ok());
        challenge.nonce = [0; 32];
        assert!(GameRouteControlRequestV1::Challenge(Box::new(challenge.clone())).validate().is_err());
        challenge.nonce = [4; 32];
        challenge.requester_transport_peer = vec![7; 129];
        assert!(GameRouteControlRequestV1::Challenge(Box::new(challenge)).validate().is_err());
    }

    #[test]
    fn wrong_response_type_never_counts_as_success() {
        let request = GameRouteControlRequestV1::Offer { world_id: WorldId([1; 32]) };
        assert!(!request.accepts_response(&GameRouteControlResponseV1::ChallengeResponse(Box::new(
            MinecraftRouteResponseV1 {
                challenge: MinecraftRouteChallengeV1 {
                    world_id: WorldId([1; 32]),
                    offer_hash: Hash32([2; 32]),
                    requester_peer_id: PeerId([3; 32]),
                    requester_transport_peer: vec![],
                    nonce: [4; 32],
                },
                authority_peer_id: PeerId([8; 32]),
                epoch: 1,
                fencing_token: 1,
                runtime_instance_nonce: [5; 32],
                issued_unix_ms: 10,
                expires_unix_ms: 100,
                signature: vec![0; 64],
            }
        ))));
        assert!(request.accepts_response(&GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unauthorized)));
    }
}
