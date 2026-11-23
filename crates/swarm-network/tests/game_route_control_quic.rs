//! Real application-authenticated game-route control transport with synthetic
//! canonical quorum/runtime evidence. This is NOT production authorization.

use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use swarm_network::{
    game_route::{
        FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError, MinecraftRouteChallengeV1, MinecraftRouteOfferV1,
        MinecraftRouteResponseV1, RouteVerifier, GAME_ROUTE_VERSION,
    },
    game_route_control::{
        GameRouteControlEvent, GameRouteControlRequestV1 as Request, GameRouteControlResponseV1 as Response,
        GameRouteRefusalV1, GAME_ROUTE_CONTROL_MAX_OUTSTANDING,
    },
    generate_transport_key, NetworkEvent, SwarmNode, TransportPeerId, WireRequest, WireResponse,
};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tokio::sync::mpsc;
use tokio::time::timeout;

fn clock() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn signed_hello(key: &SigningKey, nonce: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [nonce; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

async fn authenticate(host: &mut SwarmNode, player: &mut SwarmNode, host_peer: PeerId, player_peer: PeerId) {
    let mut host_authenticated = false;
    let mut player_authenticated = false;
    timeout(Duration::from_secs(12), async {
        while !host_authenticated || !player_authenticated {
            tokio::select! {
                event = host.next_event() => {
                    if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                        host_authenticated |= application_peer == player_peer;
                    }
                }
                event = player.next_event() => {
                    if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                        player_authenticated |= application_peer == host_peer;
                    }
                }
            }
        }
    })
    .await
    .expect("both peers must authenticate over QUIC");
}

struct AuthenticatedRouteExchange<'a> {
    host: &'a mut SwarmNode,
    player: &'a mut SwarmNode,
    host_events: &'a mut mpsc::Receiver<GameRouteControlEvent>,
    player_events: &'a mut mpsc::Receiver<GameRouteControlEvent>,
    host_transport: TransportPeerId,
    player_app: PeerId,
    signed_offer: &'a MinecraftRouteOfferV1,
    key: &'a SigningKey,
}

impl AuthenticatedRouteExchange<'_> {
    async fn roundtrip(&mut self, requested: Request) -> Response {
        let id = self.player.send_game_route_request(&self.host_transport, requested.clone()).unwrap();
        timeout(Duration::from_secs(12), async {
            loop {
                tokio::select! {
                    event = self.host.next_event() => {
                        event.unwrap();
                    }
                    event = self.player.next_event() => {
                        event.unwrap();
                    }
                    event = self.host_events.recv() => {
                        if let Some(GameRouteControlEvent::Inbound { application_peer, request, channel, .. }) = event {
                            assert_eq!(application_peer, self.player_app);
                            assert_eq!(request, requested);
                            let reply = match request {
                                Request::Offer { .. } => Response::Offer(Box::new(self.signed_offer.clone())),
                                Request::Challenge(challenge) => {
                                    let now = clock();
                                    let mut response = MinecraftRouteResponseV1 {
                                        challenge: *challenge,
                                        authority_peer_id: self.signed_offer.authority_peer_id,
                                        epoch: self.signed_offer.epoch,
                                        fencing_token: self.signed_offer.fencing_token,
                                        runtime_instance_nonce: self.signed_offer.runtime_instance_nonce,
                                        issued_unix_ms: now,
                                        expires_unix_ms: now + 3000,
                                        signature: Vec::new(),
                                    };
                                    response.sign(self.key).unwrap();
                                    Response::ChallengeResponse(Box::new(response))
                                }
                            };
                            self.host.respond_game_route(channel, reply).unwrap();
                        }
                    }
                    event = self.player_events.recv() => {
                        match event.expect("player route event channel must stay alive") {
                            GameRouteControlEvent::Response { request_id, response, .. } if request_id == id => {
                                return response;
                            }
                            GameRouteControlEvent::OutboundFailure { request_id, error, .. } if request_id == id => {
                                panic!("game route control failed: {error}");
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
        .await
        .expect("game route response must traverse an authenticated QUIC stream")
    }
}

struct SyntheticCurrentAuthority {
    offer: MinecraftRouteOfferV1,
    requester: PeerId,
    available: bool,
}

impl FreshAuthoritySource for SyntheticCurrentAuthority {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError> {
        if !self.available || requester != self.requester || world != self.offer.world_id {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        let now = clock();
        Ok(FreshAuthorityEvidence {
            world_id: world,
            realm_hash: self.offer.realm_hash,
            authority_peer_id: self.offer.authority_peer_id,
            authority_public_key: self.offer.authority_public_key,
            authority_transport_peer: TransportPeerId::from_bytes(&self.offer.authority_transport_peer).unwrap(),
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
            challenge_nonce: nonce,
            verified_at_unix_ms: now,
            valid_until_unix_ms: now + 2500,
        })
    }
}

#[tokio::test]
async fn signed_offer_and_challenge_traverse_separate_authenticated_control_channel() {
    let key_host = SigningKey::generate(&mut OsRng);
    let key_player = SigningKey::generate(&mut OsRng);
    let app_host = peer_id_from_public_key(&key_host.verifying_key().to_bytes());
    let app_player = peer_id_from_public_key(&key_player.verifying_key().to_bytes());
    let mut host = SwarmNode::new(generate_transport_key(), signed_hello(&key_host, 1), key_host.clone()).unwrap();
    let mut player = SwarmNode::new(generate_transport_key(), signed_hello(&key_player, 2), key_player).unwrap();
    let mut host_route_events = host.take_game_route_control_events().unwrap();
    let mut player_route_events = player.take_game_route_control_events().unwrap();
    let host_transport = host.local_transport_peer_id();
    let player_transport = player.local_transport_peer_id();
    let world_id = WorldId([1; 32]);
    assert!(player.send_game_route_request(&host_transport, Request::Offer { world_id }).is_err());
    host.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let address = timeout(Duration::from_secs(10), async {
        loop {
            if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                break address;
            }
        }
    })
    .await
    .unwrap();
    player.dial(address).unwrap();
    authenticate(&mut host, &mut player, app_host, app_player).await;

    let now = clock();
    let mut offer = MinecraftRouteOfferV1 {
        version: GAME_ROUTE_VERSION,
        world_id,
        realm_hash: Hash32([2; 32]),
        authority_peer_id: app_host,
        authority_public_key: key_host.verifying_key().to_bytes(),
        authority_transport_peer: host_transport.to_bytes(),
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
        issued_unix_ms: now,
        expires_unix_ms: now + 12_000,
        signature: Vec::new(),
    };
    offer.sign(&key_host).unwrap();
    {
        let mut exchange = AuthenticatedRouteExchange {
            host: &mut host,
            player: &mut player,
            host_events: &mut host_route_events,
            player_events: &mut player_route_events,
            host_transport,
            player_app: app_player,
            signed_offer: &offer,
            key: &key_host,
        };
        let response = exchange.roundtrip(Request::Offer { world_id }).await;
        let Response::Offer(remote_offer) = response else {
            panic!("expected signed authority offer");
        };
        assert_eq!(remote_offer.world_id, world_id);
        remote_offer.verify_signature().unwrap();

        let root = tempfile::tempdir().unwrap();
        let verifier = RouteVerifier::new(root.path());
        let challenge = verifier.issue_challenge(&remote_offer, app_player, player_transport).unwrap();
        let mut impersonating = challenge.clone();
        impersonating.requester_peer_id = app_host;
        assert!(exchange
            .player
            .send_game_route_request(&host_transport, Request::Challenge(Box::new(impersonating)))
            .is_err());
        let answer = exchange.roundtrip(Request::Challenge(Box::new(challenge.clone()))).await;
        let Response::ChallengeResponse(answer) = answer else {
            panic!("expected signed challenge answer");
        };
        assert_eq!(answer.challenge, challenge);
        let source = SyntheticCurrentAuthority { offer: offer.clone(), requester: app_player, available: false };
        // A valid signature or authenticated transport is NEVER enough to route
        // when the independent canonical authority is unavailable.
        assert!(matches!(
            verifier.verify(&offer, &answer, world_id, clock(), &source),
            Err(GameRouteError::UnverifiedAuthority)
        ));
        // A failed verifier consumes the challenge; it cannot be replayed.
        assert!(matches!(
            verifier.verify(&offer, &answer, world_id, clock(), &source),
            Err(GameRouteError::InvalidChallenge)
        ));
        let fresh = verifier.issue_challenge(&offer, app_player, player_transport).unwrap();
        let Response::ChallengeResponse(fresh_answer) = exchange.roundtrip(Request::Challenge(Box::new(fresh))).await
        else {
            panic!("expected fresh response");
        };
        let allowed = SyntheticCurrentAuthority { available: true, ..source };
        let ticket = verifier.verify(&offer, &fresh_answer, world_id, clock(), &allowed).unwrap();
        assert!(ticket.is_fresh());
        assert_eq!(ticket.transport_peer(), host_transport);
    }

    // Route control traffic must leave the legacy snapshot protocol usable.
    let ping = player.send_request(&host_transport, WireRequest::Ping { nonce: 0x55aa }).unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                event = host.next_event() => {
                    if let NetworkEvent::InboundRequest {
                        request: WireRequest::Ping { nonce: 0x55aa }, channel, ..
                    } = event.unwrap() {
                        host.respond(channel, WireResponse::Pong { nonce: 0x55aa }).unwrap();
                    }
                }
                event = player.next_event() => {
                    if let NetworkEvent::Response {
                        request_id, response: WireResponse::Pong { nonce: 0x55aa }, ..
                    } = event.unwrap() {
                        if request_id == ping { break; }
                    }
                }
            }
        }
    })
    .await
    .unwrap();

    // Never accumulate more than a bounded number of unanswered offer
    // requests. Snapshot Ping above remains independent and functional.
    for _ in 0..GAME_ROUTE_CONTROL_MAX_OUTSTANDING {
        player.send_game_route_request(&host_transport, Request::Offer { world_id }).unwrap();
    }
    assert!(player.send_game_route_request(&host_transport, Request::Offer { world_id }).is_err());
}

#[tokio::test]
async fn unsubscribed_or_dropped_route_controller_cannot_send_control_requests() {
    let key = SigningKey::generate(&mut OsRng);
    let mut node = SwarmNode::new(generate_transport_key(), signed_hello(&key, 1), key).unwrap();
    let peer = generate_transport_key().public().to_peer_id();
    let request = Request::Offer { world_id: WorldId([9; 32]) };
    assert!(node.send_game_route_request(&peer, request.clone()).is_err());
    let events = node.take_game_route_control_events().unwrap();
    assert!(node.take_game_route_control_events().is_none());
    drop(events);
    assert!(node.send_game_route_request(&peer, request).is_err());
}

#[tokio::test]
async fn route_control_refusals_are_not_successful_authority_proofs() {
    assert!(!Request::Offer { world_id: WorldId([1; 32]) }.accepts_response(&Response::ChallengeResponse(Box::new(
        MinecraftRouteResponseV1 {
            challenge: MinecraftRouteChallengeV1 {
                world_id: WorldId([1; 32]),
                offer_hash: Hash32([2; 32]),
                requester_peer_id: PeerId([3; 32]),
                requester_transport_peer: vec![],
                nonce: [4; 32],
            },
            authority_peer_id: PeerId([5; 32]),
            epoch: 1,
            fencing_token: 1,
            runtime_instance_nonce: [6; 32],
            issued_unix_ms: 1,
            expires_unix_ms: 5,
            signature: vec![0; 64],
        }
    ))));
    assert!(Response::Refused(GameRouteRefusalV1::QuorumUnavailable).validate().is_ok());
}
