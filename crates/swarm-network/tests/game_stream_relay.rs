//! Locally hosted three-process-style relay acceptance.
//!
//! Player and host have NO direct application connection or manually
//! forwarded Minecraft port: they authenticate across a dedicated libp2p
//! TCP/Noise relay circuit and exchange bytes on the game substream.
//! This proves relay protocol plumbing on localhost, NOT consumer NAT,
//! CGNAT, production relay availability, real Minecraft login or quorum.

use ed25519_dalek::{Signer, SigningKey};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::{multiaddr::Protocol, Multiaddr, StreamProtocol};
use rand_core::OsRng;
use std::time::{Duration, Instant};
use swarm_network::{
    game_route::GAME_STREAM_PROTOCOL,
    generate_transport_key,
    infrastructure::{PrivateRelay, PrivateRelayConfig},
    NetworkEvent, SwarmNode, TransportPeerId, WireRequest, WireResponse,
};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PROTOCOL_VERSION};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};

async fn start_relay(allowed: &[TransportPeerId]) -> (TransportPeerId, Multiaddr, JoinHandle<()>) {
    start_relay_with_budget(allowed, 8 * 1024 * 1024).await
}

async fn start_relay_with_budget(
    allowed: &[TransportPeerId],
    max_circuit_bytes: u64,
) -> (TransportPeerId, Multiaddr, JoinHandle<()>) {
    start_relay_with_limits(allowed, max_circuit_bytes, 300).await
}

async fn start_relay_with_limits(
    allowed: &[TransportPeerId],
    max_circuit_bytes: u64,
    max_circuit_seconds: u64,
) -> (TransportPeerId, Multiaddr, JoinHandle<()>) {
    let key = generate_transport_key();
    let peer = key.public().to_peer_id();
    let relay = PrivateRelay::bind(
        key,
        &PrivateRelayConfig {
            listen: "/ip4/127.0.0.1/tcp/0".into(),
            advertise: None,
            allowed_transport_peers: allowed.iter().map(ToString::to_string).collect(),
            max_circuit_bytes,
            max_circuit_seconds,
            max_circuits: 4,
        },
    )
    .await
    .unwrap();
    let address = relay.advertised_address().clone();
    let task = tokio::spawn(async move { relay.run().await.unwrap() });
    (peer, address, task)
}

async fn pump(mut node: SwarmNode, mut stop: oneshot::Receiver<()>) -> SwarmNode {
    loop {
        tokio::select! {
            _ = &mut stop => return node,
            event = node.next_event() => { event.unwrap(); }
        }
    }
}

fn hello(key: &SigningKey, nonce: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut value = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [nonce; 32],
        signature: Vec::new(),
    };
    value.signature = key.sign(&value.signing_bytes().unwrap()).to_bytes().to_vec();
    value
}

#[tokio::test]
async fn encrypted_game_substream_survives_a_real_local_libp2p_relay_circuit() {
    timeout(Duration::from_secs(80), async {
        let host_key = SigningKey::generate(&mut OsRng);
        let player_key = SigningKey::generate(&mut OsRng);
        let host_app = peer_id_from_public_key(&host_key.verifying_key().to_bytes());
        let player_app = peer_id_from_public_key(&player_key.verifying_key().to_bytes());
        let mut host = SwarmNode::new(generate_transport_key(), hello(&host_key, 1), host_key).unwrap();
        let mut player = SwarmNode::new(generate_transport_key(), hello(&player_key, 2), player_key).unwrap();
        let host_transport = host.local_transport_peer_id();
        let player_transport = player.local_transport_peer_id();
        let (relay_peer, relay_address, relay_task) = start_relay(&[host_transport, player_transport]).await;

        // Host does not listen publicly for game traffic; only to its
        // authenticated relay circuit. No raw-IP dial or fallback is used.
        host.configure_relay(relay_peer, relay_address.clone()).unwrap();
        let reserved_address = timeout(Duration::from_secs(15), async {
            loop {
                if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                    if address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit)) {
                        break address;
                    }
                }
            }
        })
        .await
        .expect("host must reserve an application relay circuit");
        assert!(reserved_address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit)));
        assert_eq!(host.application_peer(&relay_peer), None, "relay server is not an authenticated world member");

        player.dial_via_relay(relay_peer, relay_address.clone(), host_transport).unwrap();
        let mut host_authenticated = false;
        let mut player_authenticated = false;
        timeout(Duration::from_secs(18), async {
            while !host_authenticated || !player_authenticated {
                tokio::select! {
                    event = host.next_event() => {
                        if let NetworkEvent::Authenticated { transport_peer, application_peer } = event.unwrap() {
                            host_authenticated |= transport_peer == player_transport && application_peer == player_app;
                        }
                    }
                    event = player.next_event() => {
                        if let NetworkEvent::Authenticated { transport_peer, application_peer } = event.unwrap() {
                            player_authenticated |= transport_peer == host_transport && application_peer == host_app;
                        }
                    }
                }
            }
        })
        .await
        .expect("application identities must authenticate across relay");

        let authenticated_host = host.game_authenticated_peers();
        assert_eq!(authenticated_host.read().unwrap().get(&player_transport), Some(&player_app));
        assert_eq!(authenticated_host.read().unwrap().get(&relay_peer), None);
        let mut host_control = host.game_stream_control();
        let mut player_control = player.game_stream_control();
        let mut incoming = host_control.accept(StreamProtocol::new(GAME_STREAM_PROTOCOL)).unwrap();
        // A real control RPC and game substream use the same authenticated
        // relayed application connection, not a direct IP fallback.
        assert!(host.connectivity_diagnostics().relay_connectivity);
        assert!(!host.connectivity_diagnostics().direct_connectivity);
        assert!(player.connectivity_diagnostics().relay_connectivity);
        assert!(!player.connectivity_diagnostics().direct_connectivity);
        let request = player.send_request(&host_transport, WireRequest::Ping { nonce: 17 }).unwrap();
        timeout(Duration::from_secs(8), async {
            loop {
                tokio::select! {
                    event = host.next_event() => {
                        if let NetworkEvent::InboundRequest { transport_peer, request: WireRequest::Ping { nonce }, channel } = event.unwrap() {
                            assert_eq!(transport_peer, player_transport);
                            host.respond(channel, WireResponse::Pong { nonce }).unwrap();
                        }
                    }
                    event = player.next_event() => {
                        if let NetworkEvent::Response { transport_peer, request_id, response: WireResponse::Pong { nonce: 17 } } = event.unwrap() {
                            assert_eq!(transport_peer, host_transport);
                            assert_eq!(request_id, request);
                            break;
                        }
                    }
                }
            }
        }).await.unwrap();

        let (host_stop, host_rx) = oneshot::channel();
        let (player_stop, player_rx) = oneshot::channel();
        let host_task = tokio::spawn(pump(host, host_rx));
        let player_task = tokio::spawn(pump(player, player_rx));

        // An unknown transport identity cannot reserve or authenticate
        // through the actual production relay, even with a valid PeerHello.
        let rogue_key = SigningKey::generate(&mut OsRng);
        let mut rogue = SwarmNode::new(generate_transport_key(), hello(&rogue_key, 3), rogue_key).unwrap();
        rogue.configure_relay(relay_peer, relay_address.clone()).unwrap();
        let admitted = timeout(Duration::from_secs(3), async {
            loop {
                if let NetworkEvent::Listening { address } = rogue.next_event().await.unwrap() {
                    if address.iter().any(|part| matches!(part, Protocol::P2pCircuit)) { break; }
                }
            }
        }).await;
        assert!(admitted.is_err(), "unlisted transport obtained a relay reservation");
        assert!(!rogue.connectivity_diagnostics().relay_connectivity);
        drop(rogue);

        let start = Instant::now();
        timeout(Duration::from_secs(25), async {
            let send = async {
                let mut stream = player_control
                    .open_stream(host_transport, StreamProtocol::new(GAME_STREAM_PROTOCOL))
                    .await
                    .expect("game protocol must negotiate over authenticated relay circuit");
                for sequence in 0..256 {
                    let data = [sequence as u8; 4096];
                    stream.write_all(&data).await.unwrap();
                    stream.flush().await.unwrap();
                    let mut echoed = [0; 4096];
                    stream.read_exact(&mut echoed).await.unwrap();
                    assert_eq!(echoed, data, "relay must preserve all game bytes and backpressure");
                }
            };
            let receive = async {
                let (peer, mut stream) = incoming.next().await.unwrap();
                assert_eq!(peer, player_transport);
                assert_eq!(authenticated_host.read().unwrap().get(&peer), Some(&player_app));
                for sequence in 0..256 {
                    let mut bytes = [0; 4096];
                    stream.read_exact(&mut bytes).await.unwrap();
                    assert_eq!(bytes, [sequence as u8; 4096]);
                    stream.write_all(&bytes).await.unwrap();
                    stream.flush().await.unwrap();
                }
            };
            tokio::join!(send, receive);
        })
        .await
        .expect("Minecraft substream must cross real libp2p relay");
        eprintln!("capacity transport=relay-tcp-noise acknowledged_bytes=1048576 elapsed_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0);
        // A known infrastructure relay must remain connected beyond the
        // application PeerHello 10-second deadline. Only real SwarmCraft
        // application peers get that challenge; the relay itself has no
        // authority to read or provide world traffic.
        tokio::time::sleep(Duration::from_secs(11)).await;
        timeout(Duration::from_secs(12), async {
            let send = async {
                let mut stream = player_control
                    .open_stream(host_transport, StreamProtocol::new(GAME_STREAM_PROTOCOL))
                    .await
                    .expect("relay reservation must remain usable after application auth timeout");
                stream.write_all(b"still-reserved").await.unwrap();
                stream.flush().await.unwrap();
                let mut echoed = [0; 14];
                stream.read_exact(&mut echoed).await.unwrap();
                assert_eq!(&echoed, b"still-reserved");
            };
            let receive = async {
                let (peer, mut stream) = incoming.next().await.expect("a fresh relayed game stream");
                assert_eq!(peer, player_transport);
                let mut data = [0; 14];
                stream.read_exact(&mut data).await.unwrap();
                assert_eq!(&data, b"still-reserved");
                stream.write_all(&data).await.unwrap();
                stream.flush().await.unwrap();
            };
            tokio::join!(send, receive);
        })
        .await
        .expect("relay infrastructure must not be cut off by the application auth timeout");

        // Simulate an actual relay outage. A relayed SwarmCraft connection
        // must lose its application-authentication mapping, and a Minecraft
        // stream must NOT silently fall back to a raw IP or stale circuit.
        relay_task.abort();
        timeout(Duration::from_secs(8), async {
            while authenticated_host.read().unwrap().contains_key(&player_transport) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("relay outage must revoke application authentication");
        let reopened = timeout(
            Duration::from_secs(3),
            player_control.open_stream(host_transport, StreamProtocol::new(GAME_STREAM_PROTOCOL)),
        )
        .await;
        assert!(!matches!(reopened, Ok(Ok(_))), "game stream unexpectedly opened after the only relay went down");
        let _ = host_stop.send(());
        let _ = player_stop.send(());
        let mut host = host_task.await.unwrap();
        let mut player = player_task.await.unwrap();
        assert_eq!(host.local_transport_peer_id(), host_transport);
        assert_eq!(player.local_transport_peer_id(), player_transport);
        assert!(host.application_peer(&player_transport).is_none());

        // Explicit operator relay replacement: retain the exact application
        // and transport identities; never reinterpret the relay as authority.
        let (replacement, replacement_address, replacement_task) = start_relay(&[host_transport, player_transport]).await;
        host.configure_relay(replacement, replacement_address.clone()).unwrap();
        timeout(Duration::from_secs(12), async {
            loop {
                if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                    if address.iter().any(|part| matches!(part, Protocol::P2p(peer) if peer == replacement))
                        && address.iter().any(|part| matches!(part, Protocol::P2pCircuit)) { break; }
                }
            }
        }).await.expect("replacement relay reservation");
        player.dial_via_relay(replacement, replacement_address, host_transport).unwrap();
        timeout(Duration::from_secs(15), async {
            loop {
                if host.application_peer(&player_transport) == Some(player_app)
                    && player.application_peer(&host_transport) == Some(host_app) { break; }
                tokio::select! {
                    event = host.next_event() => { event.unwrap(); }
                    event = player.next_event() => { event.unwrap(); }
                }
            }
        }).await.expect("replacement circuit must freshly authenticate both original peers");
        assert_eq!(host.application_peer(&replacement), None);
        assert_eq!(player.application_peer(&replacement), None);
        assert!(host.connectivity_diagnostics().relay_connectivity);
        assert!(!host.connectivity_diagnostics().direct_connectivity);

        let host_task = tokio::spawn(async move { while host.next_event().await.is_ok() {} });
        let player_task = tokio::spawn(async move { while player.next_event().await.is_ok() {} });
        timeout(Duration::from_secs(8), async {
            let send = async {
                let mut stream = player_control.open_stream(host_transport, StreamProtocol::new(GAME_STREAM_PROTOCOL)).await.unwrap();
                stream.write_all(b"replacement").await.unwrap();
                stream.flush().await.unwrap();
                let mut response = [0; 11];
                stream.read_exact(&mut response).await.unwrap();
                assert_eq!(&response, b"replacement");
            };
            let receive = async {
                let (peer, mut stream) = incoming.next().await.unwrap();
                assert_eq!(peer, player_transport);
                assert_eq!(authenticated_host.read().unwrap().get(&peer), Some(&player_app));
                let mut data = [0; 11];
                stream.read_exact(&mut data).await.unwrap();
                stream.write_all(&data).await.unwrap();
                stream.flush().await.unwrap();
            };
            tokio::join!(send, receive);
        }).await.expect("fresh game stream over replacement relay");
        host_task.abort();
        player_task.abort();
        replacement_task.abort();
    })
    .await
    .expect("bounded local relay acceptance must finish");
}

async fn connect_isolated_relay_pair(
    host: &mut SwarmNode,
    player: &mut SwarmNode,
    relay: TransportPeerId,
    address: Multiaddr,
) -> Instant {
    let host_id = host.local_transport_peer_id();
    let player_id = player.local_transport_peer_id();
    host.configure_relay(relay, address.clone()).unwrap();
    timeout(Duration::from_secs(12), async {
        loop {
            if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                if address.iter().any(|part| matches!(part, Protocol::P2pCircuit)) {
                    break;
                }
            }
        }
    })
    .await
    .expect("independent host relay reservation");
    let dialed = Instant::now();
    player.dial_via_relay(relay, address, host_id).unwrap();
    timeout(Duration::from_secs(15), async {
        while host.application_peer(&player_id).is_none() || player.application_peer(&host_id).is_none() {
            tokio::select! {
                event = host.next_event() => { event.unwrap(); }
                event = player.next_event() => { event.unwrap(); }
            }
        }
    })
    .await
    .expect("independent pair authentication");
    for node in [host, player] {
        assert!(node.connectivity_diagnostics().relay_connectivity);
        assert!(!node.connectivity_diagnostics().direct_connectivity);
        assert_eq!(node.application_peer(&relay), None);
    }
    dialed
}

#[tokio::test]
async fn private_relay_caps_bulk_circuit_without_disconnect_of_independent_pair() {
    timeout(Duration::from_secs(60), async {
        let make_node = |nonce| {
            let key = SigningKey::generate(&mut OsRng);
            SwarmNode::new(generate_transport_key(), hello(&key, nonce), key).unwrap()
        };
        let mut bulk_host = make_node(1);
        let mut bulk_client = make_node(2);
        let mut small_host = make_node(3);
        let mut small_client = make_node(4);
        let bulk_host_id = bulk_host.local_transport_peer_id();
        let bulk_client_id = bulk_client.local_transport_peer_id();
        let small_host_id = small_host.local_transport_peer_id();
        let small_client_id = small_client.local_transport_peer_id();
        const BUDGET: u64 = 256 * 1024;
        let (relay, address, relay_task) =
            start_relay_with_budget(&[bulk_host_id, bulk_client_id, small_host_id, small_client_id], BUDGET).await;
        connect_isolated_relay_pair(&mut bulk_host, &mut bulk_client, relay, address.clone()).await;
        let bulk_authenticated = bulk_host.game_authenticated_peers();
        let mut bulk_host_control = bulk_host.game_stream_control();
        let mut bulk_client_control = bulk_client.game_stream_control();
        let mut bulk_incoming = bulk_host_control.accept(StreamProtocol::new(GAME_STREAM_PROTOCOL)).unwrap();
        let bulk_host_task = tokio::spawn(async move { while bulk_host.next_event().await.is_ok() {} });
        let bulk_client_task = tokio::spawn(async move { while bulk_client.next_event().await.is_ok() {} });

        connect_isolated_relay_pair(&mut small_host, &mut small_client, relay, address).await;
        let small_authenticated = small_host.game_authenticated_peers();
        let mut small_host_control = small_host.game_stream_control();
        let mut small_client_control = small_client.game_stream_control();
        let mut small_incoming = small_host_control.accept(StreamProtocol::new(GAME_STREAM_PROTOCOL)).unwrap();
        let small_host_task = tokio::spawn(async move { while small_host.next_event().await.is_ok() {} });
        let small_client_task = tokio::spawn(async move { while small_client.next_event().await.is_ok() {} });

        // A small circuit has its own budget and must remain useful while
        // an unrelated transport pair attempts to consume the relay.
        let small_echo = async {
            let (peer, mut stream) = small_incoming.next().await.unwrap();
            assert_eq!(peer, small_client_id);
            let mut data = [0; 16];
            for _ in 0..2 {
                stream.read_exact(&mut data).await.unwrap();
                stream.write_all(&data).await.unwrap();
                stream.flush().await.unwrap();
            }
        };
        let small_check = async {
            let mut stream = small_client_control
                .open_stream(small_host_id, StreamProtocol::new(GAME_STREAM_PROTOCOL))
                .await
                .unwrap();
            stream.write_all(&[42; 16]).await.unwrap();
            stream.flush().await.unwrap();
            let mut echo = [0; 16];
            stream.read_exact(&mut echo).await.unwrap();
            assert_eq!(echo, [42; 16]);

            // Count acknowledged bytes, not writes buffered by the client.
            // Exceed the *encrypted circuit* budget with bounded chunks.
            let bulk_send = async {
                let mut stream = bulk_client_control
                    .open_stream(bulk_host_id, StreamProtocol::new(GAME_STREAM_PROTOCOL))
                    .await
                    .unwrap();
                let data = [9; 4096];
                let mut acknowledged = 0u64;
                let mut echoed = [0; 4096];
                loop {
                    if stream.write_all(&data).await.is_err()
                        || stream.flush().await.is_err()
                        || stream.read_exact(&mut echoed).await.is_err()
                    {
                        break acknowledged;
                    }
                    assert_eq!(echoed, data);
                    acknowledged += data.len() as u64;
                    assert!(acknowledged < 2 * BUDGET, "bulk circuit exceeded configured cap");
                }
            };
            let bulk_echo = async {
                let (peer, mut stream) = bulk_incoming.next().await.unwrap();
                assert_eq!(peer, bulk_client_id);
                let mut data = [0; 4096];
                while stream.read_exact(&mut data).await.is_ok() {
                    if stream.write_all(&data).await.is_err() || stream.flush().await.is_err() {
                        break;
                    }
                }
            };
            let (acknowledged, ()) = tokio::join!(bulk_send, bulk_echo);
            eprintln!(
                "capacity relay_circuits=2 configured_circuit_bytes={BUDGET} bulk_acknowledged_bytes={acknowledged}"
            );
            assert!(acknowledged > 0, "bulk circuit never transferred bytes");
            assert!(acknowledged <= BUDGET, "circuit byte limit was not enforced");
            timeout(Duration::from_secs(8), async {
                while bulk_authenticated.read().unwrap().contains_key(&bulk_client_id) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("budget exhaustion must remove old application authentication");

            stream.write_all(&[43; 16]).await.unwrap();
            stream.flush().await.unwrap();
            stream.read_exact(&mut echo).await.unwrap();
            assert_eq!(echo, [43; 16]);
            assert!(small_authenticated.read().unwrap().contains_key(&small_client_id));
            println!("bulk acknowledged bytes before circuit cap: {acknowledged}; independent circuit remained live");
        };
        timeout(Duration::from_secs(20), async {
            tokio::join!(small_echo, small_check);
        })
        .await
        .expect("bounded bulk disconnect and independent small stream progress");
        for task in [bulk_host_task, bulk_client_task, small_host_task, small_client_task, relay_task] {
            task.abort();
        }
    })
    .await
    .expect("bounded relay resource acceptance");
}

struct TimedCircuit {
    dialed: Instant,
    authenticated: Instant,
    closed: Instant,
    chunk_bytes: usize,
    samples: Vec<(Instant, Duration)>,
}

impl TimedCircuit {
    fn report(&self, label: &str, overlap_start: Instant, overlap_end: Instant) {
        let mut latency: Vec<_> = self.samples.iter().map(|(_, rtt)| *rtt).collect();
        latency.sort_unstable();
        let p95 = latency[(latency.len() * 95).div_ceil(100) - 1];
        let bytes = self.samples.len() * self.chunk_bytes;
        let overlap = self.samples.iter().filter(|(at, _)| *at >= overlap_start && *at < overlap_end).count();
        println!(
            "relay_duration {label} acknowledged_bytes={bytes} samples={} overlap_samples={overlap} dial_to_close_ms={} authenticated_to_close_ms={} payload_kib_per_s={:.2} p95_echo_ms={:.3}",
            self.samples.len(), self.closed.duration_since(self.dialed).as_millis(),
            self.closed.duration_since(self.authenticated).as_millis(),
            bytes as f64 / 1024.0 / self.closed.duration_since(self.authenticated).as_secs_f64(),
            p95.as_secs_f64() * 1000.0,
        );
    }
}

/// A real encrypted relay circuit under sustained, bounded application load.
/// No Minecraft login, signed world admission or WAN fairness is inferred.
async fn paced_circuit_until_expiry(
    host: SwarmNode,
    player: SwarmNode,
    dialed: Instant,
    chunk_bytes: usize,
    interval: Duration,
    byte_budget: u64,
) -> TimedCircuit {
    let authenticated = Instant::now();
    let host_id = host.local_transport_peer_id();
    let player_id = player.local_transport_peer_id();
    let player_app = host.application_peer(&player_id).unwrap();
    let host_app = player.application_peer(&host_id).unwrap();
    let host_registry = host.game_authenticated_peers();
    let player_registry = player.game_authenticated_peers();
    let mut host_control = host.game_stream_control();
    let mut player_control = player.game_stream_control();
    let mut incoming = host_control.accept(StreamProtocol::new(GAME_STREAM_PROTOCOL)).unwrap();
    let (host_stop, host_rx) = oneshot::channel();
    let (player_stop, player_rx) = oneshot::channel();
    let host_driver = tokio::spawn(pump(host, host_rx));
    let player_driver = tokio::spawn(pump(player, player_rx));
    let (samples, closed) = timeout(Duration::from_secs(35), async {
        let echo = async {
            let (peer, mut stream) = incoming.next().await.expect("incoming paced circuit");
            assert_eq!(peer, player_id);
            assert_eq!(host_registry.read().unwrap().get(&peer), Some(&player_app));
            let mut data = vec![0; chunk_bytes];
            while stream.read_exact(&mut data).await.is_ok() {
                if stream.write_all(&data).await.is_err() || stream.flush().await.is_err() {
                    break;
                }
            }
        };
        let send = async {
            let mut stream =
                player_control.open_stream(host_id, StreamProtocol::new(GAME_STREAM_PROTOCOL)).await.unwrap();
            assert_eq!(player_registry.read().unwrap().get(&host_id), Some(&host_app));
            let mut samples = Vec::new();
            let mut data = vec![0x61; chunk_bytes];
            let mut response = vec![0; chunk_bytes];
            loop {
                // Independent, sequence-tagged exact echo. Keep the
                // bidirectional payload well below the encrypted byte cap;
                // expiration must not be confused with volume exhaustion.
                let sequence = samples.len() as u64;
                data[..8].copy_from_slice(&sequence.to_le_bytes());
                let started = Instant::now();
                let exchange = timeout(Duration::from_secs(5), async {
                    stream.write_all(&data).await?;
                    stream.flush().await?;
                    stream.read_exact(&mut response).await
                })
                .await
                .expect("paced relay exchange stalled without a transport close");
                if exchange.is_err() {
                    break (samples, Instant::now());
                }
                assert_eq!(data, response);
                samples.push((Instant::now(), started.elapsed()));
                assert!(
                    samples.len() * chunk_bytes * 2 < byte_budget as usize / 2,
                    "byte cap would confound duration test"
                );
                tokio::time::sleep(interval).await;
            }
        };
        let (measured, ()) = tokio::join!(send, echo);
        measured
    })
    .await
    .expect("duration cap must close both ends of the still-active stream");
    timeout(Duration::from_secs(8), async {
        while host_registry.read().unwrap().contains_key(&player_id)
            || player_registry.read().unwrap().contains_key(&host_id)
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("expired circuit must retire application authentication at both endpoints");
    let _ = host_stop.send(());
    let _ = player_stop.send(());
    let host = host_driver.await.unwrap();
    let player = player_driver.await.unwrap();
    assert!(host.application_peer(&player_id).is_none() && player.application_peer(&host_id).is_none());
    assert!(samples.len() >= 10, "circuit expired before sustained traffic");
    TimedCircuit { dialed, authenticated, closed, chunk_bytes, samples }
}

#[tokio::test]
async fn private_relay_expires_staggered_active_circuits_without_cross_disconnect() {
    timeout(Duration::from_secs(85), async {
        const SECONDS: u64 = 20;
        const BYTES: u64 = 64 * 1024 * 1024;
        let make = |nonce| {
            let key = SigningKey::generate(&mut OsRng);
            SwarmNode::new(generate_transport_key(), hello(&key, nonce), key).unwrap()
        };
        let (mut bulk_host, mut bulk_player, mut interactive_host, mut interactive_player) = (make(1), make(2), make(3), make(4));
        let allowed = [&bulk_host, &bulk_player, &interactive_host, &interactive_player]
            .map(SwarmNode::local_transport_peer_id);
        let (relay, address, relay_task) = start_relay_with_limits(&allowed, BYTES, SECONDS).await;
        let bulk_dial = connect_isolated_relay_pair(&mut bulk_host, &mut bulk_player, relay, address.clone()).await;
        let bulk = tokio::spawn(paced_circuit_until_expiry(bulk_host, bulk_player, bulk_dial, 8192, Duration::from_millis(25), BYTES));
        tokio::time::sleep(Duration::from_secs(6)).await;
        let interactive_dial = connect_isolated_relay_pair(&mut interactive_host, &mut interactive_player, relay, address).await;
        let interactive = tokio::spawn(paced_circuit_until_expiry(interactive_host, interactive_player, interactive_dial, 64, Duration::from_millis(100), BYTES));
        let (bulk, interactive) = tokio::join!(bulk, interactive);
        let (bulk, interactive) = (bulk.unwrap(), interactive.unwrap());
        for sample in [&bulk, &interactive] {
            // Dial starts before relay circuit establishment. Authentication
            // finishes after it. These bracket the actual timer origin.
            assert!(sample.closed.duration_since(sample.dialed) >= Duration::from_secs(SECONDS - 1), "premature circuit close");
            assert!(sample.closed.duration_since(sample.authenticated) <= Duration::from_secs(SECONDS + 5), "duration cap did not bound lifetime");
            assert!(sample.closed.duration_since(sample.samples.last().unwrap().0) <= Duration::from_secs(5), "stream was not active before close");
        }
        let overlap_start = interactive.samples.first().unwrap().0;
        let overlap_end = bulk.closed;
        assert!(overlap_end > overlap_start, "circuits never overlapped");
        assert!(bulk.samples.iter().filter(|(at, _)| *at >= overlap_start).count() >= 10, "bulk circuit did not progress during overlap");
        assert!(interactive.samples.iter().filter(|(at, _)| *at < overlap_end).count() >= 10, "interactive circuit did not progress during overlap");
        assert!(interactive.samples.iter().filter(|(at, _)| *at > bulk.closed).count() >= 5, "first expiration interrupted the independent circuit");
        bulk.report("bulk", overlap_start, overlap_end);
        interactive.report("interactive", overlap_start, overlap_end);
        println!("relay_duration configured_seconds={SECONDS} byte_budget={BYTES} overlap_ms={} independent_post_expiry_samples={} authentication_revoked_both_pairs=true", overlap_end.duration_since(overlap_start).as_millis(), interactive.samples.iter().filter(|(at, _)| *at > overlap_end).count());
        relay_task.abort();
    }).await.expect("bounded staggered relay duration gate");
}
