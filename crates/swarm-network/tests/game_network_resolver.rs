//! Real authenticated QUIC route-offer/challenge traffic + real RouteVerifier,
//! with a synthetic fresh-authority source. This verifies the production
//! network resolver, NOT real quorum decisions, Java IPC or Minecraft gameplay.

use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::{
    net::{Ipv4Addr, SocketAddrV4, TcpListener as StdTcpListener},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use swarm_network::{
    game_port_registry::GameWorldPortRegistry,
    game_route::{
        FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError, MinecraftRouteOfferV1, RouteVerifier,
        GAME_ROUTE_VERSION,
    },
    game_route_control::GameRouteControlEvent,
    game_route_host::AuthenticatedGameRouteHostSigner,
    game_route_resolver::{AuthenticatedGameRouteResolver, GameRouteCandidateSource},
    game_transport::{GameStreamClient, GameStreamReceiver, GatewayRouteResolver, StableLoopbackGateway},
    generate_transport_key, NetworkEvent, SwarmNode, TransportPeerId,
};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::watch,
    time::timeout,
};

fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn signed_hello(key: &SigningKey, marker: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [marker; 32],
        signature: vec![],
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

struct CurrentCandidateHints {
    hosts: RwLock<Vec<TransportPeerId>>,
}

impl GameRouteCandidateSource for CurrentCandidateHints {
    fn candidate_hosts(&self, _world: WorldId) -> Vec<TransportPeerId> {
        self.hosts.read().unwrap().clone()
    }
}

struct SyntheticQuorum {
    offer: MinecraftRouteOfferV1,
    player: PeerId,
    quorum_ready: AtomicBool,
}

impl FreshAuthoritySource for SyntheticQuorum {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError> {
        if !self.quorum_ready.load(Ordering::SeqCst) || world != self.offer.world_id || requester != self.player {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        let now = unix_ms();
        let offer = &self.offer;
        Ok(FreshAuthorityEvidence {
            world_id: offer.world_id,
            realm_hash: offer.realm_hash,
            authority_peer_id: offer.authority_peer_id,
            authority_public_key: offer.authority_public_key,
            authority_transport_peer: TransportPeerId::from_bytes(&offer.authority_transport_peer).unwrap(),
            game_server_port: offer.game_server_port,
            requester_is_member: true,
            authority_is_member: true,
            conflict_free: true,
            hosting_consent_valid: true,
            runtime_ready_verified: true,
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
            verified_at_unix_ms: now,
            valid_until_unix_ms: now + 2_000,
        })
    }
}

#[tokio::test]
async fn network_resolver_requires_signed_host_challenge_and_fresh_independent_evidence() {
    timeout(Duration::from_secs(25), async {
        let root = tempfile::tempdir().unwrap();
        let host_key = SigningKey::generate(&mut OsRng);
        let unresponsive_key = SigningKey::generate(&mut OsRng);
        let player_key = SigningKey::generate(&mut OsRng);
        let host_app = peer_id_from_public_key(&host_key.verifying_key().to_bytes());
        let unresponsive_app = peer_id_from_public_key(&unresponsive_key.verifying_key().to_bytes());
        let player_app = peer_id_from_public_key(&player_key.verifying_key().to_bytes());
        let mut host = SwarmNode::new(generate_transport_key(), signed_hello(&host_key, 1), host_key.clone()).unwrap();
        let mut unresponsive =
            SwarmNode::new(generate_transport_key(), signed_hello(&unresponsive_key, 3), unresponsive_key).unwrap();
        let mut player =
            SwarmNode::new(generate_transport_key(), signed_hello(&player_key, 2), player_key).unwrap();
        let host_transport = host.local_transport_peer_id();
        let unresponsive_transport = unresponsive.local_transport_peer_id();
        let player_transport = player.local_transport_peer_id();
        let host_game_control = host.game_stream_control();
        let host_game_authenticated = host.game_authenticated_peers();
        let player_game_control = player.game_stream_control();
        let player_game_authenticated = player.game_authenticated_peers();
        let mut host_control_events = host.take_game_route_control_events().unwrap();
        let mut unresponsive_control_events = unresponsive.take_game_route_control_events().unwrap();
        host.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
        unresponsive.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
        let address = loop {
            if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                break address;
            }
        };
        player.dial(address).unwrap();
        let unresponsive_address = loop {
            if let NetworkEvent::Listening { address } = unresponsive.next_event().await.unwrap() {
                break address;
            }
        };
        player.dial(unresponsive_address).unwrap();
        let mut host_auth = false;
        let mut unresponsive_auth = false;
        let mut player_auth = false;
        let mut player_saw_unresponsive = false;
        while !(host_auth && unresponsive_auth && player_auth && player_saw_unresponsive) {
            tokio::select! {
                e = host.next_event() => {
                    if let NetworkEvent::Authenticated {application_peer,..} = e.unwrap() {
                        host_auth |= application_peer == player_app;
                    }
                }
                e = player.next_event() => {
                    if let NetworkEvent::Authenticated {application_peer,..} = e.unwrap() {
                        player_auth |= application_peer == host_app;
                        player_saw_unresponsive |= application_peer == unresponsive_app;
                    }
                }
                e = unresponsive.next_event() => {
                    if let NetworkEvent::Authenticated {application_peer,..} = e.unwrap() {
                        unresponsive_auth |= application_peer == player_app;
                    }
                }
            }
        }

        let world = WorldId([42; 32]);
        // Real echo server on the hosting peer's loopback interface. The
        // synthetic evidence provider below MUST bind this actual port to the
        // signed offer. There is no public Minecraft listener or raw-IP path.
        let java_surrogate = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let game_port = java_surrogate.local_addr().unwrap().port();
        let java_surrogate_task = tokio::spawn(async move {
            let mut sessions = tokio::task::JoinSet::new();
            while let Ok((mut stream, _)) = java_surrogate.accept().await {
                sessions.spawn(async move {
                    let mut bytes = [0u8; 2048];
                    while let Ok(count) = stream.read(&mut bytes).await {
                        if count == 0 || stream.write_all(&bytes[..count]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        let now = unix_ms();
        let mut offer = MinecraftRouteOfferV1 {
            version: GAME_ROUTE_VERSION,
            world_id: world,
            realm_hash: Hash32([1; 32]),
            authority_peer_id: host_app,
            authority_public_key: host_key.verifying_key().to_bytes(),
            authority_transport_peer: host_transport.to_bytes(),
            game_server_port: game_port,
            membership_hash: Hash32([2; 32]),
            quorum_certificate_hash: Hash32([3; 32]),
            compatibility_fingerprint: Hash32([4; 32]),
            checkpoint_hash: Hash32([5; 32]),
            state_root: Hash32([6; 32]),
            epoch: 4,
            fencing_token: 9,
            consent_revision: 2,
            runtime_instance_nonce: [8; 32],
            runtime_ready_proof_hash: Hash32([9; 32]),
            offer_id: [10; 32],
            sequence: 1,
            issued_unix_ms: now,
            expires_unix_ms: now + 12_000,
            signature: vec![],
        };
        offer.sign(&host_key).unwrap();
        let quorum = Arc::new(SyntheticQuorum {
            offer: offer.clone(),
            player: player_app,
            quorum_ready: AtomicBool::new(true),
        });
        let host_signer = AuthenticatedGameRouteHostSigner::new(
            host_key.clone(), host_transport, quorum.clone(),
            root.path().join("private-host-offer-sequences"),
        );
        let hints = Arc::new(CurrentCandidateHints { hosts: RwLock::new(vec![host_transport]) });
        let resolver = Arc::new(AuthenticatedGameRouteResolver::new(
            player_app,
            player_transport,
            player.game_route_rpc_client(),
            RouteVerifier::new(root.path().join("private-floors")),
            player.game_authenticated_peers(),
            quorum.clone(),
            hints.clone(),
        )
        .unwrap());

        let host_server = tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = host.next_event() => {
                        if event.is_err() { break; }
                    }
                    event = host_control_events.recv() => {
                        let Some(GameRouteControlEvent::Inbound { application_peer, transport_peer, request, channel }) = event else {
                            continue;
                        };
                        assert_eq!(application_peer, player_app);
                        assert_eq!(transport_peer, player_transport);
                        // Real host signing logic, with SYNTHETIC quorum and
                        // runtime evidence injected only for this test. The
                        // same handler refuses on withdrawal and re-verifies
                        // after its durable offer sequence has been written.
                        let response = host_signer.answer(application_peer, transport_peer, request).unwrap();
                        host.respond_game_route(channel, response).unwrap();
                    }
                }
            }
        });
        let blackhole = tokio::spawn(async move {
            // Keep authenticated request channels open without replying.
            // This is a live but unresponsive former-host candidate, not an
            // unauthenticated/disconnected peer that fails immediately.
            let mut held_channels = Vec::new();
            loop {
                tokio::select! {
                    event = unresponsive.next_event() => {
                        if event.is_err() { break; }
                    }
                    event = unresponsive_control_events.recv() => {
                        if let Some(GameRouteControlEvent::Inbound { channel, .. }) = event {
                            held_channels.push(channel);
                        }
                    }
                }
            }
        });
        let player_driver = tokio::spawn(async move { while player.next_event().await.is_ok() {} });

        let verified = resolver.resolve(world).await.expect("authenticated offer and fresh challenge must verify");
        assert_eq!(verified.world_id(), world);
        // Complete vertical slice: LOCAL Minecraft TCP client -> bounded
        // gateway -> fresh authenticated route RPC -> real QUIC game substream
        // -> separately authenticated host receiver -> loopback TCP server.
        let receiver = Arc::new(
            GameStreamReceiver::new(world, host_transport, game_port, quorum.clone(), host_game_authenticated)
                .unwrap(),
        );
        let (stop_host, host_rx) = watch::channel(false);
        let receiver_task = tokio::spawn(receiver.serve(host_game_control, host_rx));
        let gateway_client = Arc::new(
            GameStreamClient::new(player_app, player_transport, player_game_authenticated, quorum.clone()).unwrap(),
        );
        let reserved = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let stable_port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let registry = GameWorldPortRegistry::new(root.path().join("durable-client-ports"));
        let gateway = StableLoopbackGateway::bind_reserved(world, stable_port, &registry).await.unwrap();
        let (stop_gateway, gateway_rx) = watch::channel(false);
        let gateway_resolver: Arc<dyn GatewayRouteResolver> = resolver.clone();
        let gateway_task =
            tokio::spawn(gateway.serve(gateway_resolver, gateway_client, player_game_control, gateway_rx));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut session = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, stable_port)).await.unwrap();
        let data = b"authenticated route and game streams";
        session.write_all(data).await.unwrap();
        let mut echoed = vec![0u8; data.len()];
        timeout(Duration::from_secs(4), session.read_exact(&mut echoed)).await.unwrap().unwrap();
        assert_eq!(&echoed, data);

        // A correctly signed offer and response are insufficient when the
        // independently verified quorum/runtime lease disappears.
        quorum.quorum_ready.store(false, Ordering::SeqCst);
        let mut buffer = [0; 32];
        let ended = timeout(Duration::from_secs(4), session.read(&mut buffer)).await.unwrap();
        assert!(matches!(ended, Ok(0) | Err(_)), "active Minecraft stream outlived quorum withdrawal");
        let mut denied = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, stable_port)).await.unwrap();
        let _ = denied.write_all(b"must be refused").await;
        let rejected = timeout(Duration::from_secs(3), denied.read(&mut buffer)).await.unwrap();
        assert!(matches!(rejected, Ok(0) | Err(_)), "gateway served a player without current quorum");
        assert!(resolver.resolve(world).await.is_err());

        quorum.quorum_ready.store(true, Ordering::SeqCst);
        // Previous real sessions and refused joins have consumed the host's
        // bounded per-peer control-plane request budget. Recovery must wait
        // for that *existing* 2s admission window, not weaken the limiter.
        tokio::time::sleep(Duration::from_millis(2_200)).await;
        let mut rejoined = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, stable_port)).await.unwrap();
        rejoined.write_all(b"fresh Java login").await.unwrap();
        let mut reply = [0u8; 16];
        timeout(Duration::from_secs(4), rejoined.read_exact(&mut reply)).await.unwrap().unwrap();
        assert_eq!(&reply, b"fresh Java login");
        assert_eq!(registry.lookup(world).unwrap(), Some(stable_port));
        hints.hosts.write().unwrap().clear();
        assert!(resolver.resolve(world).await.is_err(), "missing host hint must fail closed");
        let unauthenticated_host = libp2p::identity::Keypair::generate_ed25519().public().to_peer_id();
        *hints.hosts.write().unwrap() = vec![unauthenticated_host];
        assert!(resolver.resolve(world).await.is_err(), "unconnected transport ID cannot authorize gameplay");
        *hints.hosts.write().unwrap() = vec![host_transport];
        assert!(resolver.resolve(WorldId([43; 32])).await.is_err(), "wrong world must never be routed");

        // A stale host can remain authenticated while refusing to respond.
        // An authorized successor must not be blocked behind its 5-second
        // control timeout: verify candidates concurrently and choose only
        // the independently certified authority.
        *hints.hosts.write().unwrap() = vec![unresponsive_transport, host_transport];
        let verified_successor = timeout(Duration::from_secs(3), resolver.resolve(world))
            .await
            .expect("stalled candidate must not block the current authority")
            .expect("fresh independently verified host should be selected");
        assert_eq!(verified_successor.world_id(), world);

        stop_gateway.send(true).unwrap();
        stop_host.send(true).unwrap();
        assert!(gateway_task.await.unwrap().is_ok());
        assert!(receiver_task.await.unwrap().is_ok());
        java_surrogate_task.abort();
        player_driver.abort();
        host_server.abort();
        blackhole.abort();
    })
    .await
    .expect("authenticated QUIC route resolution should complete");
}
