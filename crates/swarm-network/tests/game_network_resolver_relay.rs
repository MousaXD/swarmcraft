#![allow(unreachable_code)] // libp2p's generated relay-event conversion is unreachable under newer Rust.
//! Full authenticated route-control and Minecraft TCP forwarding over a REAL
//! locally hosted libp2p relay. There are no direct SwarmCraft peer dials,
//! public Minecraft listeners, or unauthenticated endpoint fallbacks.
//!
//! Only the current canonical quorum and Fabric-ready proof are synthetic.
//! This is network acceptance, NOT a real Minecraft gameplay/NAT certification.

use ed25519_dalek::{Signer, SigningKey};
use futures::StreamExt;
use libp2p::{
    identify, identity,
    multiaddr::Protocol,
    noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, SwarmBuilder,
};
use rand_core::OsRng;
use std::{
    net::{Ipv4Addr, SocketAddrV4, TcpListener as StdTcpListener},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
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
    game_route_host_service::BoundedGameRouteHostResponder,
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

fn hello(key: &SigningKey, marker: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".to_owned()],
        nonce: [marker; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

#[derive(NetworkBehaviour)]
struct InfrastructureRelay {
    relay: relay::Behaviour,
    ping: ping::Behaviour,
    identify: identify::Behaviour,
}

struct Candidates(TransportPeerId);

impl GameRouteCandidateSource for Candidates {
    fn candidate_hosts(&self, _world: WorldId) -> Vec<TransportPeerId> {
        vec![self.0]
    }
}

/// TEST ONLY: the real daemon must obtain these fields from independent
/// intersecting signed-quorum history and authenticated live Fabric IPC.
struct SyntheticLiveAuthority {
    offer: MinecraftRouteOfferV1,
    player: PeerId,
    available: AtomicBool,
}

impl FreshAuthoritySource for SyntheticLiveAuthority {
    fn verify_fresh(
        &self,
        world: WorldId,
        requester: PeerId,
        nonce: [u8; 32],
    ) -> Result<FreshAuthorityEvidence, GameRouteError> {
        if !self.available.load(Ordering::SeqCst) || world != self.offer.world_id || requester != self.player {
            return Err(GameRouteError::UnverifiedAuthority);
        }
        let now = unix_ms();
        Ok(FreshAuthorityEvidence {
            world_id: self.offer.world_id,
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
            valid_until_unix_ms: now + 2_500,
        })
    }
}

async fn check_relay_echo(port: u16, payload: &[u8]) -> TcpStream {
    let mut tcp = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await.unwrap();
    tcp.write_all(payload).await.unwrap();
    let mut echoed = vec![0; payload.len()];
    timeout(Duration::from_secs(5), tcp.read_exact(&mut echoed))
        .await
        .expect("relayed authenticated game session must complete promptly")
        .unwrap();
    assert_eq!(&echoed, payload);
    tcp
}

#[tokio::test]
async fn signed_route_control_and_gateway_operate_over_authenticated_relay() {
    timeout(Duration::from_secs(45), async {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        // The relay is a third, infrastructure-only peer without world keys.
        let relay_key = identity::Keypair::generate_ed25519();
        let relay_id = relay_key.public().to_peer_id();
        let mut relay_server = SwarmBuilder::with_existing_identity(relay_key)
            .with_tokio()
            .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)
            .unwrap()
            .with_behaviour(move |key| InfrastructureRelay {
                relay: relay::Behaviour::new(
                    key.public().to_peer_id(),
                    relay::Config {
                        max_circuit_bytes: 8 * 1024 * 1024,
                        max_circuit_duration: Duration::from_secs(300),
                        ..Default::default()
                    },
                ),
                ping: ping::Behaviour::default(),
                identify: identify::Behaviour::new(identify::Config::new(
                    "/swarmcraft/relay-acceptance/1".to_owned(),
                    key.public(),
                )),
            })
            .unwrap()
            .build();
        relay_server.listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap()).unwrap();
        let relay_address = loop {
            if let SwarmEvent::NewListenAddr { address, .. } = relay_server.select_next_some().await {
                break address.with(Protocol::P2p(relay_id));
            }
        };
        relay_server.add_external_address(relay_address.clone());
        let relay_task = tokio::spawn(async move {
            loop {
                relay_server.select_next_some().await;
            }
        });

        let host_key = SigningKey::generate(&mut OsRng);
        let player_key = SigningKey::generate(&mut OsRng);
        let host_app = peer_id_from_public_key(&host_key.verifying_key().to_bytes());
        let player_app = peer_id_from_public_key(&player_key.verifying_key().to_bytes());
        let mut host = SwarmNode::new(generate_transport_key(), hello(&host_key, 1), host_key.clone()).unwrap();
        let mut player = SwarmNode::new(generate_transport_key(), hello(&player_key, 2), player_key).unwrap();
        let host_transport = host.local_transport_peer_id();
        let player_transport = player.local_transport_peer_id();
        let mut host_control_events = host.take_game_route_control_events().unwrap();
        let host_game_control = host.game_stream_control();
        let player_game_control = player.game_stream_control();
        let host_authenticated = host.game_authenticated_peers();
        let player_authenticated = player.game_authenticated_peers();
        host.configure_relay(relay_id, relay_address.clone()).unwrap();
        let circuit = timeout(Duration::from_secs(15), async {
            loop {
                if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                    if address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit)) {
                        break address;
                    }
                }
            }
        })
        .await
        .expect("host should obtain authenticated relay reservation");
        assert!(circuit.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit)));
        assert_eq!(host.application_peer(&relay_id), None);
        player.dial_via_relay(relay_id, relay_address, host_transport).unwrap();
        let mut host_saw_player = false;
        let mut player_saw_host = false;
        timeout(Duration::from_secs(15), async {
            while !host_saw_player || !player_saw_host {
                tokio::select! {
                    event = host.next_event() => {
                        if let NetworkEvent::Authenticated { transport_peer, application_peer } = event.unwrap() {
                            host_saw_player |= transport_peer == player_transport && application_peer == player_app;
                        }
                    }
                    event = player.next_event() => {
                        if let NetworkEvent::Authenticated { transport_peer, application_peer } = event.unwrap() {
                            player_saw_host |= transport_peer == host_transport && application_peer == host_app;
                        }
                    }
                }
            }
        })
        .await
        .expect("application proofs must complete across relay");
        assert_eq!(host_authenticated.read().unwrap().get(&relay_id), None);

        // The Minecraft surrogate listens ONLY on its host's loopback.
        let java = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let java_port = java.local_addr().unwrap().port();
        let java_task = tokio::spawn(async move {
            let mut sessions = tokio::task::JoinSet::new();
            while let Ok((mut socket, _)) = java.accept().await {
                sessions.spawn(async move {
                    let (mut read, mut write) = socket.split();
                    let _ = tokio::io::copy(&mut read, &mut write).await;
                });
            }
        });
        let now = unix_ms();
        let world = WorldId([48; 32]);
        let mut signed = MinecraftRouteOfferV1 {
            version: GAME_ROUTE_VERSION,
            world_id: world,
            realm_hash: Hash32([1; 32]),
            authority_peer_id: host_app,
            authority_public_key: host_key.verifying_key().to_bytes(),
            authority_transport_peer: host_transport.to_bytes(),
            game_server_port: java_port,
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
            offer_id: [9; 32],
            sequence: 1,
            issued_unix_ms: now,
            expires_unix_ms: now + 12_000,
            signature: Vec::new(),
        };
        signed.sign(&host_key).unwrap();
        let proof =
            Arc::new(SyntheticLiveAuthority { offer: signed, player: player_app, available: AtomicBool::new(true) });
        let signer = Arc::new(AuthenticatedGameRouteHostSigner::new(
            host_key,
            host_transport,
            proof.clone(),
            root.path().join("private-host-sequences"),
        ));
        let responder = Arc::new(BoundedGameRouteHostResponder::new(signer));
        let resolver = Arc::new(
            AuthenticatedGameRouteResolver::new(
                player_app,
                player_transport,
                player.game_route_rpc_client(),
                RouteVerifier::new(root.path().join("private-player-generation")),
                player_authenticated.clone(),
                proof.clone(),
                Arc::new(Candidates(host_transport)),
            )
            .unwrap(),
        );

        let host_driver = tokio::spawn(async move {
            let (reply_tx, mut reply_rx) = tokio::sync::mpsc::channel(32);
            let mut signing = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    event = host.next_event() => {
                        if event.is_err() { break; }
                    }
                    reply = reply_rx.recv() => {
                        if let Some((channel, response)) = reply {
                            host.respond_game_route(channel, response).unwrap();
                        }
                    }
                    done = signing.join_next(), if !signing.is_empty() => {
                        if let Some(Err(error)) = done {
                            panic!("host signer task panicked: {error}");
                        }
                    }
                    control = host_control_events.recv() => {
                        let Some(GameRouteControlEvent::Inbound {
                            transport_peer, application_peer, request, channel,
                        }) = control else { continue };
                        assert_eq!(transport_peer, player_transport);
                        assert_eq!(application_peer, player_app);
                        let responder = Arc::clone(&responder);
                        let replies = reply_tx.clone();
                        signing.spawn(async move {
                            let answer = responder.answer(application_peer, transport_peer, request).await.unwrap();
                            let _ = replies.send((channel, answer)).await;
                        });
                    }
                }
            }
        });
        let player_driver = tokio::spawn(async move { while player.next_event().await.is_ok() {} });
        let selected = resolver.resolve(world).await.expect("signed route must complete over live relay");
        assert_eq!(selected.world_id(), world);

        let receiver = Arc::new(
            GameStreamReceiver::new(
                world,
                host_transport,
                java_port,
                proof.clone(),
                host_authenticated,
                root.path().join("host-stream-replays"),
            )
            .unwrap(),
        );
        let gateway_client =
            Arc::new(GameStreamClient::new(player_app, player_transport, player_authenticated, proof.clone()).unwrap());
        let (stop_host, host_stop_rx) = watch::channel(false);
        let host_game = tokio::spawn(receiver.serve(host_game_control, host_stop_rx));
        let selected_port = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let gateway_port = selected_port.local_addr().unwrap().port();
        drop(selected_port);
        let registry = GameWorldPortRegistry::new(root.path().join("private-world-ports"));
        let gateway = StableLoopbackGateway::bind_reserved(world, gateway_port, &registry).await.unwrap();
        let (stop_gateway, gateway_stop_rx) = watch::channel(false);
        let dyn_resolver: Arc<dyn GatewayRouteResolver> = resolver.clone();
        let gateway_task =
            tokio::spawn(gateway.serve(dyn_resolver, gateway_client, player_game_control, gateway_stop_rx));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut old = check_relay_echo(gateway_port, b"fresh route over real relay").await;
        proof.available.store(false, Ordering::SeqCst);
        let mut bytes = [0u8; 16];
        let result = timeout(Duration::from_secs(4), old.read(&mut bytes)).await.unwrap();
        assert!(matches!(result, Ok(0) | Err(_)), "revoked quorum left old TCP stream forwarding");

        let mut denied = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, gateway_port)).await.unwrap();
        let _ = denied.write_all(b"must deny").await;
        let result = timeout(Duration::from_secs(3), denied.read(&mut bytes)).await.unwrap();
        assert!(matches!(result, Ok(0) | Err(_)), "revoked quorum forwarded relayed game data");

        proof.available.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(2_200)).await; // documented per-peer control limit
        let _rejoined = check_relay_echo(gateway_port, b"verified rejoin via fixed localhost port").await;
        assert_eq!(registry.lookup(world).unwrap(), Some(gateway_port));

        stop_gateway.send(true).unwrap();
        stop_host.send(true).unwrap();
        assert!(gateway_task.await.unwrap().is_ok());
        assert!(host_game.await.unwrap().is_ok());
        host_driver.abort();
        player_driver.abort();
        relay_task.abort();
        java_task.abort();
    })
    .await
    .expect("authenticated relayed game route acceptance must terminate");
}
