//! Test the real host route signer against adversarial requests and durable
//! generation sequencing. FreshAuthoritySource below is SYNTHETIC: this does
//! not certify real accepted quorum, Fabric IPC readiness or Java gameplay.

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_network::{
    game_route::{
        FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError, MinecraftRouteChallengeV1, MinecraftRouteOfferV1,
        RouteVerifier, GAME_ROUTE_VERSION,
    },
    game_route_control::{
        GameRouteControlRequestV1 as Request, GameRouteControlResponseV1 as Response, GameRouteRefusalV1 as Refusal,
    },
    game_route_host::{AuthenticatedGameRouteHostSigner, DurableHostOfferSequencer},
    TransportPeerId,
};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerId, WorldId};

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

struct SyntheticLiveAuthority {
    offer_fields: MinecraftRouteOfferV1,
    member: PeerId,
    quorum: AtomicBool,
    conflict_free: AtomicBool,
    consent: AtomicBool,
    runtime_ready: AtomicBool,
}

impl FreshAuthoritySource for SyntheticLiveAuthority {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError> {
        if requester != self.member || world != self.offer_fields.world_id || !self.quorum.load(Ordering::SeqCst) {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        let offer = &self.offer_fields;
        let current = now_ms();
        Ok(FreshAuthorityEvidence {
            world_id: offer.world_id,
            realm_hash: offer.realm_hash,
            authority_peer_id: offer.authority_peer_id,
            authority_public_key: offer.authority_public_key,
            authority_transport_peer: TransportPeerId::from_bytes(&offer.authority_transport_peer).unwrap(),
            game_server_port: offer.game_server_port,
            requester_is_member: true,
            authority_is_member: true,
            conflict_free: self.conflict_free.load(Ordering::SeqCst),
            hosting_consent_valid: self.consent.load(Ordering::SeqCst),
            runtime_ready_verified: self.runtime_ready.load(Ordering::SeqCst),
            membership_hash: offer.membership_hash,
            quorum_certificate_hash: offer.quorum_certificate_hash,
            compatibility_fingerprint: offer.compatibility_fingerprint,
            checkpoint_hash: offer.checkpoint_hash,
            state_root: offer.state_root,
            epoch: offer.epoch,
            fencing_token: offer.fencing_token,
            consent_revision: offer.consent_revision,
            runtime_instance_nonce: offer.runtime_instance_nonce,
            runtime_ready_proof_hash: offer.runtime_ready_proof_hash,
            challenge_nonce: nonce,
            verified_at_unix_ms: current,
            valid_until_unix_ms: current + 2_000,
        })
    }
}

struct Fixture {
    host_key: SigningKey,
    host_transport: TransportPeerId,
    member: PeerId,
    member_transport: TransportPeerId,
    world: WorldId,
    source: Arc<SyntheticLiveAuthority>,
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let host_key = SigningKey::generate(&mut OsRng);
        let host_transport = libp2p::identity::Keypair::generate_ed25519().public().to_peer_id();
        let member_transport = libp2p::identity::Keypair::generate_ed25519().public().to_peer_id();
        let member = PeerId([21; 32]);
        let world = WorldId([30; 32]);
        let template = MinecraftRouteOfferV1 {
            version: GAME_ROUTE_VERSION,
            world_id: world,
            realm_hash: Hash32([2; 32]),
            authority_peer_id: peer_id_from_public_key(&host_key.verifying_key().to_bytes()),
            authority_public_key: host_key.verifying_key().to_bytes(),
            authority_transport_peer: host_transport.to_bytes(),
            game_server_port: 25581,
            membership_hash: Hash32([3; 32]),
            quorum_certificate_hash: Hash32([4; 32]),
            compatibility_fingerprint: Hash32([5; 32]),
            checkpoint_hash: Hash32([6; 32]),
            state_root: Hash32([7; 32]),
            epoch: 7,
            fencing_token: 14,
            consent_revision: 3,
            runtime_instance_nonce: [8; 32],
            runtime_ready_proof_hash: Hash32([9; 32]),
            offer_id: [10; 32],
            sequence: 1,
            issued_unix_ms: now_ms(),
            expires_unix_ms: now_ms() + 10_000,
            signature: vec![],
        };
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            host_key,
            host_transport,
            member,
            member_transport,
            world,
            source: Arc::new(SyntheticLiveAuthority {
                offer_fields: template,
                member,
                quorum: AtomicBool::new(true),
                conflict_free: AtomicBool::new(true),
                consent: AtomicBool::new(true),
                runtime_ready: AtomicBool::new(true),
            }),
            root,
        }
    }

    fn signer(&self) -> AuthenticatedGameRouteHostSigner {
        AuthenticatedGameRouteHostSigner::new(
            self.host_key.clone(),
            self.host_transport,
            self.source.clone(),
            self.root.path().join("private-host-sequences"),
        )
    }

    fn offer(&self, signer: &AuthenticatedGameRouteHostSigner) -> MinecraftRouteOfferV1 {
        match signer.answer(self.member, self.member_transport, Request::Offer { world_id: self.world }).unwrap() {
            Response::Offer(offer) => *offer,
            other => panic!("expected current signed offer; got {other:?}"),
        }
    }

    fn challenge(&self, verifier: &RouteVerifier, offer: &MinecraftRouteOfferV1) -> MinecraftRouteChallengeV1 {
        verifier.issue_challenge(offer, self.member, self.member_transport).unwrap()
    }
}

#[test]
fn host_signs_only_current_member_and_one_time_challenge_and_client_verifies_it() {
    let f = Fixture::new();
    let signer = f.signer();
    let verifier = RouteVerifier::new(f.root.path().join("private-client-floor"));
    let offer = f.offer(&signer);
    offer.verify_signature().unwrap();
    let challenge = f.challenge(&verifier, &offer);

    // A foreign authenticated requester cannot steal another member's
    // signed one-time response. This denial must not consume the real offer.
    assert!(matches!(
        signer.answer(PeerId([55; 32]), f.member_transport, Request::Challenge(Box::new(challenge.clone()))).unwrap(),
        Response::Refused(Refusal::Unauthorized)
    ));
    let response =
        match signer.answer(f.member, f.member_transport, Request::Challenge(Box::new(challenge.clone()))).unwrap() {
            Response::ChallengeResponse(response) => *response,
            other => panic!("expected a signed response; got {other:?}"),
        };
    assert_eq!(response.challenge, challenge);
    verifier.verify(&offer, &response, f.world, now_ms(), f.source.as_ref()).unwrap();
    assert!(matches!(
        signer.answer(f.member, f.member_transport, Request::Challenge(Box::new(challenge))).unwrap(),
        Response::Refused(Refusal::ExpiredOrMalformed)
    ));
    assert!(matches!(
        signer.answer(f.member, f.member_transport, Request::Offer { world_id: WorldId([77; 32]) }).unwrap(),
        Response::Refused(Refusal::QuorumUnavailable)
    ));
}

#[test]
fn quorum_conflict_consent_and_fabric_unreadiness_refuse_offers_and_challenges() {
    let f = Fixture::new();
    let signer = f.signer();
    f.source.quorum.store(false, Ordering::SeqCst);
    assert!(matches!(
        signer.answer(f.member, f.member_transport, Request::Offer { world_id: f.world }).unwrap(),
        Response::Refused(Refusal::QuorumUnavailable)
    ));
    f.source.quorum.store(true, Ordering::SeqCst);
    f.source.conflict_free.store(false, Ordering::SeqCst);
    assert!(matches!(
        signer.answer(f.member, f.member_transport, Request::Offer { world_id: f.world }).unwrap(),
        Response::Refused(Refusal::ConflictingAuthority)
    ));
    f.source.conflict_free.store(true, Ordering::SeqCst);
    f.source.consent.store(false, Ordering::SeqCst);
    assert!(matches!(
        signer.answer(f.member, f.member_transport, Request::Offer { world_id: f.world }).unwrap(),
        Response::Refused(Refusal::FencedOrConsentWithdrawn)
    ));
    f.source.consent.store(true, Ordering::SeqCst);
    let offer = f.offer(&signer);
    let verifier = RouteVerifier::new(f.root.path().join("private-client-floor"));
    let challenge = f.challenge(&verifier, &offer);
    f.source.runtime_ready.store(false, Ordering::SeqCst);
    assert!(matches!(
        signer.answer(f.member, f.member_transport, Request::Challenge(Box::new(challenge))).unwrap(),
        Response::Refused(Refusal::HostUnready)
    ));
}

#[test]
fn durable_host_offer_sequence_survives_restart_and_refuses_rollback() {
    let f = Fixture::new();
    let first = f.offer(&f.signer());
    let second = f.offer(&f.signer());
    assert!(second.sequence > first.sequence, "a process restart must not reuse a signed sequence");
    assert_ne!(second.offer_hash().unwrap(), first.offer_hash().unwrap());

    let sequencer = DurableHostOfferSequencer::new(f.root.path().join("private-host-sequences"));
    assert!(matches!(
        sequencer.reserve_next(f.world, first.realm_hash, first.epoch - 1, first.fencing_token - 1),
        Err(GameRouteError::Downgrade)
    ));
    assert!(matches!(
        sequencer.reserve_next(f.world, first.realm_hash, first.epoch + 1, first.fencing_token),
        Err(GameRouteError::Downgrade)
    ));
}
