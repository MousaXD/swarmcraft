//! Bound host-side route signing so a hung quorum or Fabric IPC verifier
//! cannot block the libp2p event loop or accumulate unbounded worker threads.
//!
//! This service DOES NOT fabricate evidence or grant authority. It delegates
//! to AuthenticatedGameRouteHostSigner, whose injected FreshAuthoritySource
//! must independently prove canonical quorum, membership, consent, fencing
//! and actual runtime readiness for each challenge.

use crate::{
    game_route::{GameRouteError, MAX_ROUTE_RESPONSE_AGE_MS},
    game_route_control::{GameRouteControlRequestV1, GameRouteControlResponseV1, GameRouteRefusalV1},
    game_route_host::AuthenticatedGameRouteHostSigner,
};
use libp2p::PeerId as TransportPeerId;
use std::{sync::Arc, time::Duration};
use swarm_protocol::PeerId;
use tokio::{
    sync::{oneshot, Semaphore},
    time::timeout,
};

/// At most this many authority/Fabric verifications can remain in flight
/// even when a source stalls forever. Each detached timed-out worker retains
/// its permit until it actually exits, never freeing an occupied slot early.
pub const MAX_CONCURRENT_HOST_ROUTE_SIGNINGS: usize = 4;
/// Below the route-control RPC's five-second maximum, leaving time to reply.
pub const MAX_HOST_ROUTE_SIGNING_WAIT: Duration = Duration::from_millis(MAX_ROUTE_RESPONSE_AGE_MS - 2_000);

pub struct BoundedGameRouteHostResponder {
    signer: Arc<AuthenticatedGameRouteHostSigner>,
    workers: Arc<Semaphore>,
    max_wait: Duration,
}

impl BoundedGameRouteHostResponder {
    pub fn new(signer: Arc<AuthenticatedGameRouteHostSigner>) -> Self {
        Self {
            signer,
            workers: Arc::new(Semaphore::new(MAX_CONCURRENT_HOST_ROUTE_SIGNINGS)),
            max_wait: MAX_HOST_ROUTE_SIGNING_WAIT,
        }
    }

    /// Call from the network controller's async event handler, never
    /// directly execute the synchronous signer on SwarmNode.next_event().
    ///
    /// When the proof source is hung, the request fails closed within a
    /// strict deadline. A stalled OS worker retains its permit until it
    /// actually exits; no Tokio blocking worker is held hostage during
    /// runtime shutdown. The application process may still terminate safely.
    pub async fn answer(
        &self,
        requester: PeerId,
        requester_transport: TransportPeerId,
        request: GameRouteControlRequestV1,
    ) -> Result<GameRouteControlResponseV1, GameRouteError> {
        if request.validate().is_err() {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ExpiredOrMalformed));
        }
        let Ok(permit) = Arc::clone(&self.workers).try_acquire_owned() else {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::RateLimited));
        };
        let signer = Arc::clone(&self.signer);
        let (sender, receiver) = oneshot::channel();
        // A bounded, detached OS thread avoids stalling Tokio's shutdown
        // when a third-party Fabric/quorum adapter never returns. With at
        // most four permits, this cannot cause unbounded detached threads.
        if std::thread::Builder::new()
            .name("swarm-game-route-proof".to_owned())
            .spawn(move || {
                let _slot = permit;
                let _ = sender.send(signer.answer(requester, requester_transport, request));
            })
            .is_err()
        {
            return Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unavailable));
        }
        match timeout(self.max_wait, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) | Err(_) => Ok(GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unavailable)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_route::{FreshAuthorityEvidence, FreshAuthoritySource, RouteVerifier};
    use ed25519_dalek::SigningKey;
    use libp2p::identity::Keypair;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Condvar, Mutex};
    use swarm_protocol::{peer_id_from_public_key, Hash32, WorldId};

    struct SlowUnavailableAuthority {
        started: AtomicBool,
        delay: Duration,
    }

    impl FreshAuthoritySource for SlowUnavailableAuthority {
        fn verify_fresh(
            &self,
            _world: WorldId,
            _requester: PeerId,
            _nonce: [u8; 32],
        ) -> Result<FreshAuthorityEvidence, GameRouteError> {
            self.started.store(true, Ordering::SeqCst);
            std::thread::sleep(self.delay);
            Err(GameRouteError::UnverifiedAuthority)
        }
    }

    #[derive(Default)]
    struct ControlledUnavailableAuthority {
        started: AtomicBool,
        released: Mutex<bool>,
        release: Condvar,
    }

    impl FreshAuthoritySource for ControlledUnavailableAuthority {
        fn verify_fresh(
            &self,
            _world: WorldId,
            _requester: PeerId,
            _nonce: [u8; 32],
        ) -> Result<FreshAuthorityEvidence, GameRouteError> {
            self.started.store(true, Ordering::SeqCst);
            // Test-only gate. The emergency timeout prevents orphaned
            // threads if the test panics before signaling release.
            let released = self.released.lock().unwrap();
            let _guard =
                self.release.wait_timeout_while(released, Duration::from_secs(10), |released| !*released).unwrap();
            Err(GameRouteError::UnverifiedAuthority)
        }
    }

    #[tokio::test]
    async fn stalled_quorum_verifier_does_not_block_reactor_or_unbound_worker_count() {
        let root = tempfile::tempdir().unwrap();
        let source = Arc::new(ControlledUnavailableAuthority::default());
        let signer = Arc::new(AuthenticatedGameRouteHostSigner::new(
            SigningKey::from_bytes(&[12; 32]),
            Keypair::generate_ed25519().public().to_peer_id(),
            source.clone(),
            root.path().join("unused-host-sequences"),
        ));
        let mut service = Arc::new(BoundedGameRouteHostResponder {
            signer,
            workers: Arc::new(Semaphore::new(1)),
            max_wait: Duration::from_millis(35),
        });
        let requester = PeerId([20; 32]);
        let transport = Keypair::generate_ed25519().public().to_peer_id();
        let request = GameRouteControlRequestV1::Offer { world_id: WorldId([6; 32]) };

        let worker = tokio::spawn({
            let service = Arc::clone(&service);
            let request = request.clone();
            async move { service.answer(requester, transport, request).await }
        });
        timeout(Duration::from_secs(3), async {
            while !source.started.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the real blocking source must have begun");
        let second = service.answer(requester, transport, request.clone()).await.unwrap();
        assert_eq!(second, GameRouteControlResponseV1::Refused(GameRouteRefusalV1::RateLimited));

        let first = timeout(Duration::from_secs(3), worker).await.unwrap().unwrap().unwrap();
        assert_eq!(first, GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unavailable));
        // A timed-out worker is NOT interrupted, and its permit remains
        // consumed until it exits. Exhaustion must remain fail-closed.
        assert_eq!(
            service.answer(requester, transport, request.clone()).await.unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::RateLimited)
        );

        // Release the verifier explicitly and observe its actual semaphore
        // return rather than assuming an OS thread ran during a fixed sleep.
        *source.released.lock().unwrap() = true;
        source.release.notify_all();
        timeout(Duration::from_secs(3), async {
            while service.workers.available_permits() != 1 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the released verifier must return its worker slot");
        // The next request can now acquire the slot, but a verifier that
        // supplies no proof must still refuse it. Give this post-release
        // assertion enough scheduling time on a loaded CI machine; the
        // actual stalled-request deadline above remains 35 milliseconds.
        Arc::get_mut(&mut service).expect("test owns the responder").max_wait = Duration::from_secs(3);
        let recovered = service.answer(requester, transport, request).await.unwrap();
        assert_eq!(recovered, GameRouteControlResponseV1::Refused(GameRouteRefusalV1::QuorumUnavailable));
    }

    #[tokio::test]
    async fn invalid_world_is_refused_without_spawning_authority_worker() {
        let root = tempfile::tempdir().unwrap();
        let source =
            Arc::new(SlowUnavailableAuthority { started: AtomicBool::new(false), delay: Duration::from_millis(100) });
        let signer = Arc::new(AuthenticatedGameRouteHostSigner::new(
            SigningKey::from_bytes(&[13; 32]),
            Keypair::generate_ed25519().public().to_peer_id(),
            source.clone(),
            root.path().join("unused-host-sequences"),
        ));
        let service = BoundedGameRouteHostResponder::new(signer);
        let result = service
            .answer(
                PeerId([20; 32]),
                Keypair::generate_ed25519().public().to_peer_id(),
                GameRouteControlRequestV1::Offer { world_id: WorldId::default() },
            )
            .await
            .unwrap();
        assert_eq!(result, GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ExpiredOrMalformed));
        assert!(!source.started.load(Ordering::SeqCst));
    }

    /// Test-only evidence fixture. This must NEVER be supplied to a real
    /// daemon: its membership/quorum/runtime are synthetic, not certificates.
    struct SyntheticReadyAuthority {
        world: WorldId,
        member: PeerId,
        key: [u8; 32],
        host_transport: TransportPeerId,
        ready: AtomicBool,
    }

    impl FreshAuthoritySource for SyntheticReadyAuthority {
        fn verify_fresh(
            &self,
            world: WorldId,
            requester: PeerId,
            nonce: [u8; 32],
        ) -> Result<FreshAuthorityEvidence, GameRouteError> {
            if world != self.world || requester != self.member {
                return Err(GameRouteError::UnverifiedAuthority);
            }
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
            Ok(FreshAuthorityEvidence {
                world_id: world,
                realm_hash: Hash32([1; 32]),
                authority_peer_id: peer_id_from_public_key(&self.key),
                authority_public_key: self.key,
                authority_transport_peer: self.host_transport,
                game_server_port: 25565,
                requester_is_member: true,
                authority_is_member: true,
                conflict_free: true,
                hosting_consent_valid: true,
                runtime_ready_verified: self.ready.load(Ordering::SeqCst),
                membership_hash: Hash32([2; 32]),
                quorum_certificate_hash: Hash32([3; 32]),
                compatibility_fingerprint: Hash32([4; 32]),
                checkpoint_hash: Hash32([5; 32]),
                state_root: Hash32([6; 32]),
                epoch: 4,
                fencing_token: 9,
                consent_revision: 2,
                runtime_instance_nonce: [7; 32],
                runtime_ready_proof_hash: Hash32([8; 32]),
                challenge_nonce: nonce,
                verified_at_unix_ms: now,
                valid_until_unix_ms: now + 4_000,
            })
        }
    }

    #[tokio::test]
    async fn bounded_responder_signs_only_test_verified_authority_and_consumes_challenge() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let key = SigningKey::from_bytes(&[22; 32]);
        let host_transport = Keypair::generate_ed25519().public().to_peer_id();
        let requester_transport = Keypair::generate_ed25519().public().to_peer_id();
        let requester = PeerId([21; 32]);
        let world = WorldId([10; 32]);
        let source = Arc::new(SyntheticReadyAuthority {
            world,
            member: requester,
            key: key.verifying_key().to_bytes(),
            host_transport,
            ready: AtomicBool::new(true),
        });
        let signer = Arc::new(AuthenticatedGameRouteHostSigner::new(
            key,
            host_transport,
            source.clone(),
            root.path().join("host-sequences"),
        ));
        let service = BoundedGameRouteHostResponder::new(signer);
        let offer = service
            .answer(requester, requester_transport, GameRouteControlRequestV1::Offer { world_id: world })
            .await
            .unwrap();
        let GameRouteControlResponseV1::Offer(offer) = offer else {
            panic!("the independent test authority should permit a signed offer");
        };
        assert_eq!(offer.verify_signature().unwrap(), host_transport);
        let verifier = RouteVerifier::new(root.path().join("accepted-generation-floors"));
        let challenge = verifier.issue_challenge(&offer, requester, requester_transport).unwrap();
        let answer = service
            .answer(requester, requester_transport, GameRouteControlRequestV1::Challenge(Box::new(challenge.clone())))
            .await
            .unwrap();
        let GameRouteControlResponseV1::ChallengeResponse(response) = answer else {
            panic!("the current signed offer must answer its one-time challenge");
        };
        assert_eq!(response.challenge, challenge);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let route = verifier.verify(&offer, &response, world, now, source.as_ref()).unwrap();
        assert!(route.is_fresh());
        assert_eq!(route.transport_peer(), host_transport);
        assert_eq!(
            service
                .answer(requester, requester_transport, GameRouteControlRequestV1::Challenge(Box::new(challenge)),)
                .await
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::ExpiredOrMalformed)
        );

        source.ready.store(false, Ordering::SeqCst);
        assert_eq!(
            service
                .answer(requester, requester_transport, GameRouteControlRequestV1::Offer { world_id: world },)
                .await
                .unwrap(),
            GameRouteControlResponseV1::Refused(GameRouteRefusalV1::HostUnready)
        );
    }
}
