//! Full synthetic authority handoff over a REAL local libp2p relay circuit.
//! Two possible SwarmCraft hosts and one player never dial application peers
//! directly. Both Minecraft-style servers bind only to their own loopback.
//! The stable local gateway reconnects to the newly verified host without
//! manual forwarding or raw-IP fallback. Quorum and Fabric readiness remain
//! SYNTHETIC, so this cannot certify a real Minecraft transfer, NAT/CGNAT,
//! actual world-state preservation, or a reachable external relay service.

use ed25519_dalek::{Signer, SigningKey};
use futures::StreamExt;
use libp2p::{
    identify, identity,
    multiaddr::Protocol,
    noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, SwarmBuilder,
};
use rand_core::OsRng;
use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddrV4, TcpListener as StdTcpListener},
    pin::Pin,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use swarm_network::{
    game_port_registry::GameWorldPortRegistry,
    game_route::{
        FreshAuthorityEvidence, FreshAuthoritySource, GameRouteError, MinecraftRouteOfferV1, MinecraftRouteResponseV1,
        RouteVerifier, GAME_ROUTE_VERSION,
    },
    game_transport::{
        GameStreamClient, GameStreamReceiver, GameTransportError, GatewayRouteResolver, StableLoopbackGateway,
        VerifiedForwardRoute,
    },
    generate_transport_key, NetworkEvent, SwarmNode, TransportPeerId,
};
use swarm_protocol::{peer_id_from_public_key, Hash32, PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
    time::timeout,
};

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock is configured").as_millis() as u64
}

#[derive(NetworkBehaviour)]
struct RelayBehaviour {
    relay: relay::Behaviour,
    ping: ping::Behaviour,
    identify: identify::Behaviour,
}

/// Independent infrastructure server; it owns no SwarmCraft application
/// signing key, world membership, Minecraft port, or authority certificate.
async fn start_local_relay() -> (TransportPeerId, Multiaddr, JoinHandle<()>) {
    let key = identity::Keypair::generate_ed25519();
    let id = key.public().to_peer_id();
    let mut server = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)
        .unwrap()
        .with_behaviour(move |local| RelayBehaviour {
            relay: relay::Behaviour::new(
                local.public().to_peer_id(),
                relay::Config {
                    max_circuit_duration: Duration::from_secs(5 * 60),
                    max_circuit_bytes: 8 * 1024 * 1024,
                    ..Default::default()
                },
            ),
            ping: ping::Behaviour::default(),
            identify: identify::Behaviour::new(identify::Config::new(
                "/swarmcraft/test-relay/1".to_owned(),
                local.public(),
            )),
        })
        .unwrap()
        .build();
    server.listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap()).unwrap();
    let address = loop {
        if let SwarmEvent::NewListenAddr { address, .. } = server.select_next_some().await {
            break address.with(Protocol::P2p(id));
        }
    };
    server.add_external_address(address.clone());
    let driver = tokio::spawn(async move {
        loop {
            server.select_next_some().await;
        }
    });
    (id, address, driver)
}

fn hello(key: &SigningKey, marker: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [marker; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

async fn echo_server(marker: u8) -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let mut sessions = tokio::task::JoinSet::new();
        while let Ok((mut socket, _)) = listener.accept().await {
            sessions.spawn(async move {
                let mut buf = [0; 1024];
                while let Ok(count) = socket.read(&mut buf).await {
                    if count == 0 {
                        break;
                    }
                    if socket.write_all(&[marker]).await.is_err() || socket.write_all(&buf[..count]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (port, task)
}

fn signed_offer(
    world: WorldId,
    key: &SigningKey,
    transport: TransportPeerId,
    minecraft_port: u16,
    epoch: u64,
    fence: u64,
) -> MinecraftRouteOfferV1 {
    let now = now_ms();
    let mut offer = MinecraftRouteOfferV1 {
        version: GAME_ROUTE_VERSION,
        world_id: world,
        realm_hash: Hash32([1; 32]),
        authority_peer_id: peer_id_from_public_key(&key.verifying_key().to_bytes()),
        authority_public_key: key.verifying_key().to_bytes(),
        authority_transport_peer: transport.to_bytes(),
        game_server_port: minecraft_port,
        membership_hash: Hash32([2; 32]),
        quorum_certificate_hash: Hash32([epoch as u8; 32]),
        compatibility_fingerprint: Hash32([4; 32]),
        checkpoint_hash: Hash32([5; 32]),
        state_root: Hash32([6; 32]),
        epoch,
        fencing_token: fence,
        consent_revision: epoch,
        runtime_instance_nonce: [epoch as u8; 32],
        runtime_ready_proof_hash: Hash32([9; 32]),
        offer_id: [epoch as u8; 32],
        sequence: 1,
        issued_unix_ms: now,
        expires_unix_ms: now + 15_000,
        signature: Vec::new(),
    };
    offer.sign(key).unwrap();
    offer
}

struct AuthorityState {
    offer: MinecraftRouteOfferV1,
    signing_key: SigningKey,
    ready_and_quorate: bool,
}

struct SyntheticAcceptedHead {
    current: RwLock<AuthorityState>,
    player: PeerId,
}

impl SyntheticAcceptedHead {
    fn update(&self, offer: MinecraftRouteOfferV1, signing_key: SigningKey, ready_and_quorate: bool) {
        *self.current.write().unwrap() = AuthorityState { offer, signing_key, ready_and_quorate };
    }
}

impl FreshAuthoritySource for SyntheticAcceptedHead {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError> {
        let state = self.current.read().unwrap();
        let offer = &state.offer;
        if !state.ready_and_quorate || requester != self.player || world != offer.world_id {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        let now = now_ms();
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

/// This test intentionally signs a response in-process. The deployed resolver
/// MUST request that signature from the host over the separately authenticated
/// game-route control channel; the network crate must never ship this mock.
struct SyntheticMovingResolver {
    state: Arc<SyntheticAcceptedHead>,
    verifier: RouteVerifier,
    player: PeerId,
    player_transport: TransportPeerId,
}

impl GatewayRouteResolver for SyntheticMovingResolver {
    fn resolve(
        &self,
        world: WorldId,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedForwardRoute, GameTransportError>> + Send + '_>> {
        Box::pin(async move {
            let (offer, signer) = {
                let current = self.state.current.read().unwrap();
                (current.offer.clone(), current.signing_key.clone())
            };
            if offer.world_id != world {
                return Err(GameTransportError::StaleRoute);
            }
            let challenge = self.verifier.issue_challenge(&offer, self.player, self.player_transport)?;
            let now = now_ms();
            let mut response = MinecraftRouteResponseV1 {
                challenge,
                authority_peer_id: offer.authority_peer_id,
                epoch: offer.epoch,
                fencing_token: offer.fencing_token,
                runtime_instance_nonce: offer.runtime_instance_nonce,
                issued_unix_ms: now,
                expires_unix_ms: now + 3_000,
                signature: Vec::new(),
            };
            response.sign(&signer)?;
            let approved = self.verifier.verify(&offer, &response, world, now_ms(), self.state.as_ref())?;
            VerifiedForwardRoute::new(approved, offer, response)
        })
    }
}

async fn join_local_gateway(port: u16, payload: &[u8], expected_host: u8) -> TcpStream {
    let mut tcp = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await.unwrap();
    tcp.write_all(payload).await.unwrap();
    let mut echoed = vec![0u8; payload.len() + 1];
    timeout(Duration::from_secs(4), tcp.read_exact(&mut echoed))
        .await
        .expect("verified successor should answer within bounded time")
        .unwrap();
    assert_eq!(echoed[0], expected_host, "traffic reached the wrong host");
    assert_eq!(&echoed[1..], payload);
    tcp
}

async fn rejected_new_join(port: u16) {
    let mut tcp = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await.unwrap();
    // A very fast fail-closed gateway is permitted to reset before the local
    // write completes. Either way, it must not return Minecraft server data.
    let _ = tcp.write_all(b"unauthorized").await;
    let mut buffer = [0u8; 8];
    let read =
        timeout(Duration::from_secs(3), tcp.read(&mut buffer)).await.expect("unverified join must fail promptly");
    assert!(matches!(read, Ok(0) | Err(_)), "unverified join leaked Minecraft data");
}

async fn run_relay_authority_handoff(crash_original_host: bool) {
    timeout(Duration::from_secs(55), async {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let world = WorldId([31; 32]);
        let key_a = SigningKey::generate(&mut OsRng);
        let key_b = SigningKey::generate(&mut OsRng);
        let key_player = SigningKey::generate(&mut OsRng);
        let app_a = peer_id_from_public_key(&key_a.verifying_key().to_bytes());
        let app_b = peer_id_from_public_key(&key_b.verifying_key().to_bytes());
        let app_player = peer_id_from_public_key(&key_player.verifying_key().to_bytes());
        let mut host_a = SwarmNode::new(generate_transport_key(), hello(&key_a, 1), key_a.clone()).unwrap();
        let mut host_b = SwarmNode::new(generate_transport_key(), hello(&key_b, 2), key_b.clone()).unwrap();
        let mut player = SwarmNode::new(generate_transport_key(), hello(&key_player, 3), key_player).unwrap();
        let transport_a = host_a.local_transport_peer_id();
        let transport_b = host_b.local_transport_peer_id();
        let player_transport = player.local_transport_peer_id();
        let (relay_peer, relay_address, relay_task) = start_local_relay().await;
        host_a.configure_relay(relay_peer, relay_address.clone()).unwrap();
        host_b.configure_relay(relay_peer, relay_address.clone()).unwrap();
        let mut a_reserved = false;
        let mut b_reserved = false;
        timeout(Duration::from_secs(15), async {
            while !a_reserved || !b_reserved {
                tokio::select! {
                    event = host_a.next_event(), if !a_reserved => {
                        if let NetworkEvent::Listening { address } = event.unwrap() {
                            a_reserved |= address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit));
                        }
                    }
                    event = host_b.next_event(), if !b_reserved => {
                        if let NetworkEvent::Listening { address } = event.unwrap() {
                            b_reserved |= address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit));
                        }
                    }
                }
            }
        })
        .await
        .expect("both authorized candidate hosts must reserve relay listeners");
        assert_eq!(host_a.application_peer(&relay_peer), None);
        assert_eq!(host_b.application_peer(&relay_peer), None);

        // Serializes discovery of the two hosts so the first circuit does
        // not race a duplicate infrastructure dial to the same relay.
        // There is never a direct peer address or Minecraft server IP.
        player.dial_via_relay(relay_peer, relay_address.clone(), transport_a).unwrap();
        let mut host_a_saw_player = false;
        let mut player_saw_a = false;
        timeout(Duration::from_secs(15), async {
            while !host_a_saw_player || !player_saw_a {
                tokio::select! {
                    event = host_a.next_event() => {
                        if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                            host_a_saw_player |= application_peer == app_player;
                        }
                    }
                    event = player.next_event() => {
                        if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                            player_saw_a |= application_peer == app_a;
                        }
                    }
                }
            }
        })
        .await
        .expect("player must authenticate with first candidate over relay");
        player.dial_via_relay(relay_peer, relay_address, transport_b).unwrap();
        let mut host_b_saw_player = false;
        let mut player_saw_b = false;
        timeout(Duration::from_secs(15), async {
            while !host_b_saw_player || !player_saw_b {
                tokio::select! {
                    event = host_b.next_event() => {
                        if let NetworkEvent::Authenticated {application_peer,..} = event.unwrap() {
                            host_b_saw_player |= application_peer == app_player;
                        }
                    }
                    event = player.next_event() => {
                        if let NetworkEvent::Authenticated {application_peer,..} = event.unwrap() {
                            player_saw_b |= application_peer == app_b;
                        }
                    }
                }
            }
        })
        .await
        .expect("player must authenticate with successor candidate over relay");

        let control_a = host_a.game_stream_control();
        let control_b = host_b.game_stream_control();
        let control_player = player.game_stream_control();
        let peers_a = host_a.game_authenticated_peers();
        let peers_b = host_b.game_authenticated_peers();
        let peers_player = player.game_authenticated_peers();
        let pump_a = tokio::spawn(async move { while host_a.next_event().await.is_ok() {} });
        let pump_b = tokio::spawn(async move { while host_b.next_event().await.is_ok() {} });
        let pump_player = tokio::spawn(async move { while player.next_event().await.is_ok() {} });

        let (port_a, server_a) = echo_server(b'A').await;
        let (port_b, server_b) = echo_server(b'B').await;
        let offer_a = signed_offer(world, &key_a, transport_a, port_a, 4, 9);
        let offer_b = signed_offer(world, &key_b, transport_b, port_b, 5, 10);
        let state = Arc::new(SyntheticAcceptedHead {
            current: RwLock::new(AuthorityState {
                offer: offer_a.clone(),
                signing_key: key_a.clone(),
                ready_and_quorate: true,
            }),
            player: app_player,
        });
        let receiver_a = Arc::new(
            GameStreamReceiver::new(
                world,
                transport_a,
                port_a,
                state.clone(),
                peers_a,
                root.path().join("host-a-replay"),
            )
            .unwrap(),
        );
        let receiver_b = Arc::new(
            GameStreamReceiver::new(
                world,
                transport_b,
                port_b,
                state.clone(),
                peers_b,
                root.path().join("host-b-replay"),
            )
            .unwrap(),
        );
        let (stop_a, rx_a) = watch::channel(false);
        let (stop_b, rx_b) = watch::channel(false);
        let task_a = tokio::spawn(receiver_a.serve(control_a, rx_a));
        let task_b = tokio::spawn(receiver_b.serve(control_b, rx_b));
        let client =
            Arc::new(GameStreamClient::new(app_player, player_transport, peers_player, state.clone()).unwrap());
        let resolver: Arc<dyn GatewayRouteResolver> = Arc::new(SyntheticMovingResolver {
            state: Arc::clone(&state),
            verifier: RouteVerifier::new(root.path().join("game-generation-floors")),
            player: app_player,
            player_transport,
        });
        let selected = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let stable_port = selected.local_addr().unwrap().port();
        drop(selected);
        let ports = GameWorldPortRegistry::new(root.path().join("durable-world-ports"));
        let gateway = StableLoopbackGateway::bind_reserved(world, stable_port, &ports).await.unwrap();
        let (stop_gateway, rx_gateway) = watch::channel(false);
        let task_gateway = tokio::spawn(gateway.serve(resolver, client, control_player, rx_gateway));
        tokio::time::sleep(Duration::from_millis(100)).await; // register host protocols

        let mut original = join_local_gateway(stable_port, b"first authority", b'A').await;
        if crash_original_host {
            // Simulate a real network/runtime process loss, not just an
            // application-level fencing flag. The host swarm, game-stream
            // supervisor, and its private Java-surrogate TCP listener all
            // stop abruptly while the player gateway stays on the SAME port.
            task_a.abort();
            pump_a.abort();
            server_a.abort();
        }
        // Synthetic transition says B is the accepted host, but B's runtime
        // is not yet ready. Old A MUST stop; no premature forwarding to B.
        state.update(offer_b.clone(), key_b.clone(), false);
        let mut scratch = [0u8; 16];
        let closed = timeout(Duration::from_secs(4), original.read(&mut scratch)).await.unwrap();
        assert!(matches!(closed, Ok(0) | Err(_)), "old authority stream continued after a fenced transition");
        rejected_new_join(stable_port).await;

        // Only a newly verified, ready B may accept the rejoined Java session.
        state.update(offer_b.clone(), key_b.clone(), true);
        let mut reconnected = join_local_gateway(stable_port, b"new authority", b'B').await;
        assert_eq!(ports.lookup(world).unwrap(), Some(stable_port));

        // A forged stale quorum view cannot regress the durably accepted epoch
        // after B's generation floor was committed.
        state.update(offer_a, key_a, true);
        let old_closed = timeout(Duration::from_secs(4), reconnected.read(&mut scratch)).await.unwrap();
        assert!(matches!(old_closed, Ok(0) | Err(_)));
        rejected_new_join(stable_port).await;

        state.update(offer_b, key_b, true);
        let _restored = join_local_gateway(stable_port, b"recovered B", b'B').await;
        stop_gateway.send(true).unwrap();
        if !crash_original_host {
            stop_a.send(true).unwrap();
        }
        stop_b.send(true).unwrap();
        assert!(task_gateway.await.unwrap().is_ok());
        if crash_original_host {
            assert!(task_a.await.unwrap_err().is_cancelled());
        } else {
            assert!(task_a.await.unwrap().is_ok());
        }
        assert!(task_b.await.unwrap().is_ok());
        pump_a.abort();
        pump_b.abort();
        pump_player.abort();
        server_a.abort();
        server_b.abort();
        relay_task.abort();
    })
    .await
    .expect("synthetic three-peer authority handoff over relay timed out");
}

#[tokio::test]
async fn relayed_hosts_are_fenced_and_fresh_successor_rejoins_same_local_gateway() {
    run_relay_authority_handoff(false).await;
}

#[tokio::test]
async fn crashed_relayed_host_recovers_via_new_authority_on_the_same_local_port() {
    run_relay_authority_handoff(true).await;
}
