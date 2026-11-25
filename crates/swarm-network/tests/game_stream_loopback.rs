//! Real loopback TCP -> real Noise-authenticated libp2p QUIC stream -> loopback
//! TCP, with an explicitly SYNTHETIC canonical authority evidence source.
//! This verifies transport plumbing and revocation, NOT a real Fabric server,
//! real majority votes, or certified remote home-network relay behavior.

use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddrV4, TcpListener as StdTcpListener},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use swarm_network::{
    game_route::{
        FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError, MinecraftRouteOfferV1, MinecraftRouteResponseV1,
        RouteVerifier, GAME_ROUTE_VERSION,
    },
    game_transport::{
        GameStreamClient, GameStreamReceiver, GameTransportError, GatewayRouteResolver, StableLoopbackGateway,
        VerifiedForwardRoute,
    },
    generate_transport_key, NetworkEvent, SwarmNode,
};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    time::timeout,
};

fn millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn hello(key: &SigningKey, n: u8) -> PeerHelloV1 {
    let public = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public),
        public_key: public,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [n; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

/// For tests ONLY. No production adapter is allowed to derive these bools from
/// a signed offer, cached status, unverified app config, or local IP.
struct SyntheticQuorumAndRuntime {
    offer: MinecraftRouteOfferV1,
    member: PeerId,
    allowed: AtomicBool,
}

impl FreshAuthoritySource for SyntheticQuorumAndRuntime {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        challenge: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError> {
        if world != self.offer.world_id || requester != self.member || !self.allowed.load(Ordering::SeqCst) {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        let now = millis();
        Ok(FreshAuthorityEvidence {
            world_id: self.offer.world_id,
            realm_hash: self.offer.realm_hash,
            authority_peer_id: self.offer.authority_peer_id,
            authority_public_key: self.offer.authority_public_key,
            authority_transport_peer: swarm_network::TransportPeerId::from_bytes(&self.offer.authority_transport_peer)
                .unwrap(),
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
            challenge_nonce: challenge,
            verified_at_unix_ms: now,
            valid_until_unix_ms: now + 3000,
        })
    }
}

struct SyntheticResolver {
    offer: MinecraftRouteOfferV1,
    signer: SigningKey,
    requester: PeerId,
    requester_transport: swarm_network::TransportPeerId,
    verifier: RouteVerifier,
    source: Arc<SyntheticQuorumAndRuntime>,
}

impl GatewayRouteResolver for SyntheticResolver {
    fn resolve(
        &self,
        world: WorldId,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedForwardRoute, GameTransportError>> + Send + '_>> {
        Box::pin(async move {
            if world != self.offer.world_id {
                return Err(GameTransportError::StaleRoute);
            }
            let challenge = self.verifier.issue_challenge(&self.offer, self.requester, self.requester_transport)?;
            let mut response = MinecraftRouteResponseV1 {
                challenge,
                authority_peer_id: self.offer.authority_peer_id,
                epoch: self.offer.epoch,
                fencing_token: self.offer.fencing_token,
                runtime_instance_nonce: self.offer.runtime_instance_nonce,
                issued_unix_ms: millis(),
                expires_unix_ms: millis() + 3000,
                signature: Vec::new(),
            };
            response.sign(&self.signer)?;
            let route = self.verifier.verify(&self.offer, &response, world, millis(), self.source.as_ref())?;
            VerifiedForwardRoute::new(route, self.offer.clone(), response)
        })
    }
}

#[tokio::test]
async fn authenticated_quic_game_stream_round_trip_and_consent_withdrawal() {
    timeout(Duration::from_secs(25), async {
        let temp = TempDir::new().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let host_key = SigningKey::generate(&mut OsRng);
        let player_key = SigningKey::generate(&mut OsRng);
        let host_hello = hello(&host_key, 1);
        let player_hello = hello(&player_key, 2);
        let host_app = host_hello.peer_id;
        let player_app = player_hello.peer_id;
        let host_transport_key = generate_transport_key();
        let player_transport_key = generate_transport_key();
        let host_transport = host_transport_key.public().to_peer_id();
        let player_transport = player_transport_key.public().to_peer_id();

        let mut host = SwarmNode::new(host_transport_key, host_hello, host_key.clone()).unwrap();
        let mut player = SwarmNode::new(player_transport_key, player_hello, player_key).unwrap();
        host.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
        let host_addr = loop {
            if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                break address;
            }
        };
        player.dial(host_addr).unwrap();
        let mut host_authenticated = false;
        let mut player_authenticated = false;
        while !host_authenticated || !player_authenticated {
            tokio::select! {
                event = host.next_event() => {
                    if let NetworkEvent::Authenticated {application_peer,..} = event.unwrap() {
                        host_authenticated |= application_peer == player_app;
                    }
                }
                event = player.next_event() => {
                    if let NetworkEvent::Authenticated {application_peer,..} = event.unwrap() {
                        player_authenticated |= application_peer == host_app;
                    }
                }
            }
        }
        let host_activity = host.game_traffic_activity();
        let player_activity = player.game_traffic_activity();
        assert!(!host_activity.active() && !player_activity.active());
        let host_control = host.game_stream_control();
        let player_control = player.game_stream_control();
        let host_auth = host.game_authenticated_peers();
        let player_auth = player.game_authenticated_peers();
        let host_pump = tokio::spawn(async move { while host.next_event().await.is_ok() {} });
        let player_pump = tokio::spawn(async move { while player.next_event().await.is_ok() {} });

        let mc_server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mc_port = mc_server.local_addr().unwrap().port();
        let mc_task = tokio::spawn(async move {
            while let Ok((mut conn, _)) = mc_server.accept().await {
                tokio::spawn(async move {
                    let (mut r, mut w) = conn.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });

        let world = WorldId([1; 32]);
        let now = millis();
        let mut offer = MinecraftRouteOfferV1 {
            version: GAME_ROUTE_VERSION,
            world_id: world,
            realm_hash: Hash32([2; 32]),
            authority_peer_id: host_app,
            authority_public_key: host_key.verifying_key().to_bytes(),
            authority_transport_peer: host_transport.to_bytes(),
            game_server_port: mc_port,
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
        offer.sign(&host_key).unwrap();
        let source = Arc::new(SyntheticQuorumAndRuntime {
            offer: offer.clone(),
            member: player_app,
            allowed: AtomicBool::new(true),
        });
        let receiver = Arc::new(
            GameStreamReceiver::new(
                world,
                host_transport,
                mc_port,
                source.clone(),
                host_auth,
                temp.path().join("host-stream-replays"),
            )
            .unwrap()
            .with_activity(host_activity.clone()),
        );
        let client = Arc::new(
            GameStreamClient::new(player_app, player_transport, player_auth, source.clone())
                .unwrap()
                .with_activity(player_activity.clone()),
        );
        let resolver: Arc<dyn GatewayRouteResolver> = Arc::new(SyntheticResolver {
            offer: offer.clone(),
            signer: host_key.clone(),
            requester: player_app,
            requester_transport: player_transport,
            verifier: RouteVerifier::new(temp.path().join("route-floors")),
            source: source.clone(),
        });
        let (_host_stop, host_rx) = watch::channel(false);
        let receiver_task = tokio::spawn(receiver.serve(host_control, host_rx));
        let (_player_stop, player_rx) = watch::channel(false);
        let local = StdTcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let stable_port = local.local_addr().unwrap().port();
        drop(local);
        let gateway = StableLoopbackGateway::bind(world, stable_port).await.unwrap();
        let gateway_task =
            tokio::spawn(gateway.serve(Arc::clone(&resolver), Arc::clone(&client), player_control.clone(), player_rx));
        // Register the inbound libp2p game protocol before the first open.
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut minecraft = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, stable_port)).await.unwrap();
        minecraft.write_all(b"minecraft synthetic roundtrip").await.unwrap();
        let mut answer = [0; 29];
        minecraft.read_exact(&mut answer).await.unwrap();
        assert_eq!(&answer, b"minecraft synthetic roundtrip");
        assert!(host_activity.active() && player_activity.active());

        // Exercise sustained bidirectional TCP forwarding across multiple
        // live authority checks, not just a single tiny packet. A recreated
        // copy future could lose already-read buffered bytes at those ticks.
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        let bulk: Vec<u8> = (0..(1024 * 1024)).map(|i| (i % 251) as u8).collect();
        let mut echoed_bulk = vec![0; bulk.len()];
        {
            let (mut minecraft_reader, mut minecraft_writer) = minecraft.split();
            let (sent, received) =
                tokio::join!(minecraft_writer.write_all(&bulk), minecraft_reader.read_exact(&mut echoed_bulk));
            sent.expect("large Minecraft data must pass to the host");
            received.expect("large Minecraft echo must return intact");
            assert_eq!(echoed_bulk, bulk, "game transport must preserve all bytes and order");
        }

        // Withdraw consent while a connection is already forwarding. An
        // existing game stream must close, not just new admissions.
        source.allowed.store(false, Ordering::SeqCst);
        let mut buf = [0u8; 16];
        let closed = timeout(Duration::from_secs(4), minecraft.read(&mut buf)).await.unwrap();
        assert!(matches!(closed, Ok(0) | Err(_)), "old game session survived revoked consent");

        let mut attempted = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, stable_port)).await.unwrap();
        attempted.write_all(b"forbidden").await.unwrap();
        let result = timeout(Duration::from_secs(3), attempted.read(&mut buf)).await.unwrap();
        assert!(matches!(result, Ok(0) | Err(_)), "revoked route unexpectedly forwarded");
        timeout(Duration::from_secs(3), async {
            while host_activity.active() || player_activity.active() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("revoked/denied streams must release pacing activity");
        // Recovered consent does not resurrect the dead TCP stream. A fresh
        // session through the SAME address is allowed after fresh verification.
        source.allowed.store(true, Ordering::SeqCst);
        let mut rejoined = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, stable_port)).await.unwrap();
        rejoined.write_all(b"rejoined minecraft world").await.unwrap();
        let mut rejoined_data = [0; 24];
        rejoined.read_exact(&mut rejoined_data).await.unwrap();
        assert_eq!(&rejoined_data, b"rejoined minecraft world");
        assert!(host_activity.active() && player_activity.active());

        // Abruptly abort the gateway supervisor, as can happen during a
        // world switch or daemon task failure. No detached child may keep the
        // OLD world forwarding after this task terminates.
        gateway_task.abort();
        assert!(gateway_task.await.unwrap_err().is_cancelled());
        let abruptly_closed = timeout(Duration::from_secs(3), rejoined.read(&mut buf)).await.unwrap();
        assert!(matches!(abruptly_closed, Ok(0) | Err(_)), "aborted gateway left an old-world TCP session alive");

        timeout(Duration::from_secs(3), async {
            while host_activity.active() || player_activity.active() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("aborted gateway must release pacing activity");
        // Recreate the gateway at the SAME port with a new RouteVerifier
        // reading the prior durable generation floor.
        let restarted_resolver: Arc<dyn GatewayRouteResolver> = Arc::new(SyntheticResolver {
            offer,
            signer: host_key,
            requester: player_app,
            requester_transport: player_transport,
            verifier: RouteVerifier::new(temp.path().join("route-floors")),
            source: Arc::clone(&source),
        });
        let restarted_gateway = StableLoopbackGateway::bind(world, stable_port).await.unwrap();
        let (restart_stop, restart_rx) = watch::channel(false);
        let restart_task =
            tokio::spawn(restarted_gateway.serve(restarted_resolver, client, player_control, restart_rx));
        let mut after_restart = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, stable_port)).await.unwrap();
        after_restart.write_all(b"restart reconnect").await.unwrap();
        let mut after_restart_bytes = [0; 17];
        after_restart.read_exact(&mut after_restart_bytes).await.unwrap();
        assert_eq!(&after_restart_bytes, b"restart reconnect");
        // The hosting receiver's supervisor has the same lifetime guarantee:
        // aborting it must cut its connected Minecraft TCP sessions.
        receiver_task.abort();
        assert!(receiver_task.await.unwrap_err().is_cancelled());
        let host_cut_off = timeout(Duration::from_secs(4), after_restart.read(&mut buf)).await.unwrap();
        assert!(matches!(host_cut_off, Ok(0) | Err(_)), "aborted host receiver left a game stream alive");
        restart_stop.send(true).unwrap();
        assert!(restart_task.await.unwrap().is_ok());
        timeout(Duration::from_secs(3), async {
            while host_activity.active() || player_activity.active() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("aborted receiver must release pacing activity");
        host_pump.abort();
        player_pump.abort();
        mc_task.abort();
    })
    .await
    .expect("real authenticated loopback stream should complete in time");
}
