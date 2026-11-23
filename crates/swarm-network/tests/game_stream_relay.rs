//! Locally hosted three-process-style relay acceptance.
//!
//! Player and host have NO direct application connection or manually
//! forwarded Minecraft port: they authenticate across a dedicated libp2p
//! TCP/Noise relay circuit and exchange bytes on the game substream.
//! This proves relay protocol plumbing on localhost, NOT consumer NAT,
//! CGNAT, production relay availability, real Minecraft login or quorum.

use ed25519_dalek::{Signer, SigningKey};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::{
    identify, identity,
    multiaddr::Protocol,
    noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, StreamProtocol, SwarmBuilder,
};
use rand_core::OsRng;
use std::time::Duration;
use swarm_network::{game_route::GAME_STREAM_PROTOCOL, generate_transport_key, NetworkEvent, SwarmNode};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PROTOCOL_VERSION};
use tokio::time::timeout;

#[derive(NetworkBehaviour)]
struct LocalRelayBehaviour {
    relay: relay::Behaviour,
    ping: ping::Behaviour,
    identify: identify::Behaviour,
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
    timeout(Duration::from_secs(45), async {
        let relay_key = identity::Keypair::generate_ed25519();
        let relay_peer = relay_key.public().to_peer_id();
        let mut relay_server = SwarmBuilder::with_existing_identity(relay_key)
            .with_tokio()
            .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)
            .unwrap()
            .with_behaviour(move |key| LocalRelayBehaviour {
                // The library default limits a circuit to only 128 KiB and
                // 120 seconds, unsuitable for a real Minecraft session.
                // Real deployments need an explicitly provisioned relay
                // budget; this is still bounded to 8 MiB/5 min for the test.
                relay: relay::Behaviour::new(
                    key.public().to_peer_id(),
                    relay::Config {
                        max_circuit_bytes: 8 * 1024 * 1024,
                        max_circuit_duration: Duration::from_secs(5 * 60),
                        ..Default::default()
                    },
                ),
                ping: ping::Behaviour::default(),
                identify: identify::Behaviour::new(identify::Config::new(
                    "/swarmcraft/test-relay/1".to_owned(),
                    key.public(),
                )),
            })
            .unwrap()
            .build();
        relay_server.listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap()).unwrap();
        let relay_address = loop {
            if let SwarmEvent::NewListenAddr { address, .. } = relay_server.select_next_some().await {
                break address.with(Protocol::P2p(relay_peer));
            }
        };
        // A real relay advertises a reachable multiaddr in reservation
        // responses; without an external address the reservation reply may
        // be rejected as unusable even though the TCP connection exists.
        relay_server.add_external_address(relay_address.clone());
        // Keep the relay's swarm polled; it is a separate infrastructure
        // participant with no SwarmCraft application authority or world keys.
        let relay_task = tokio::spawn(async move {
            loop {
                relay_server.select_next_some().await;
            }
        });

        let host_key = SigningKey::generate(&mut OsRng);
        let player_key = SigningKey::generate(&mut OsRng);
        let host_app = peer_id_from_public_key(&host_key.verifying_key().to_bytes());
        let player_app = peer_id_from_public_key(&player_key.verifying_key().to_bytes());
        let mut host = SwarmNode::new(generate_transport_key(), hello(&host_key, 1), host_key).unwrap();
        let mut player = SwarmNode::new(generate_transport_key(), hello(&player_key, 2), player_key).unwrap();
        let host_transport = host.local_transport_peer_id();
        let player_transport = player.local_transport_peer_id();
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

        player.dial_via_relay(relay_peer, relay_address, host_transport).unwrap();
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
        let host_task = tokio::spawn(async move { while host.next_event().await.is_ok() {} });
        let player_task = tokio::spawn(async move { while player.next_event().await.is_ok() {} });

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
        host_task.abort();
        player_task.abort();
        relay_task.abort();
    })
    .await
    .expect("bounded local relay acceptance must finish");
}
