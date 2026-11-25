//! Real libp2p stream integration, not a Minecraft gameplay certification.
//! Validates that the dedicated game protocol works over an authenticated
//! encrypted QUIC or TCP/Noise connection without using snapshot
//! request-response framing.

use ed25519_dalek::{Signer, SigningKey};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::StreamProtocol;
use rand_core::OsRng;
use std::time::{Duration, Instant};
use swarm_network::{
    game_route::GAME_STREAM_PROTOCOL, generate_transport_key, NetworkEvent, SwarmNode, WireRequest, WireResponse,
};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PROTOCOL_VERSION};
use tokio::time::timeout;

fn hello(key: &SigningKey, nonce: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut value = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".to_owned()],
        nonce: [nonce; 32],
        signature: Vec::new(),
    };
    value.signature = key.sign(&value.signing_bytes().unwrap()).to_bytes().to_vec();
    value
}

async fn authenticated_game_substream_echoes_bytes_without_snapshot_protocol(
    listen_address: &str,
    transport_name: &str,
) {
    let key_a = SigningKey::generate(&mut OsRng);
    let key_b = SigningKey::generate(&mut OsRng);
    let application_a = peer_id_from_public_key(&key_a.verifying_key().to_bytes());
    let application_b = peer_id_from_public_key(&key_b.verifying_key().to_bytes());
    let mut node_a = SwarmNode::new(generate_transport_key(), hello(&key_a, 1), key_a).unwrap();
    let mut node_b = SwarmNode::new(generate_transport_key(), hello(&key_b, 2), key_b).unwrap();
    let transport_a = node_a.local_transport_peer_id();
    let transport_b = node_b.local_transport_peer_id();
    node_a.listen(listen_address.parse().unwrap()).unwrap();
    let listen = timeout(Duration::from_secs(10), async {
        loop {
            if let NetworkEvent::Listening { address } = node_a.next_event().await.unwrap() {
                break address;
            }
        }
    })
    .await
    .expect("authenticated transport listener must bind");
    node_b.dial(listen).unwrap();

    let mut a_authenticated = false;
    let mut b_authenticated = false;
    timeout(Duration::from_secs(15), async {
        while !(a_authenticated && b_authenticated) {
            tokio::select! {
                a = node_a.next_event() => {
                    if let NetworkEvent::Authenticated { transport_peer, application_peer } = a.unwrap() {
                        a_authenticated |= transport_peer == transport_b && application_peer == application_b;
                    }
                }
                b = node_b.next_event() => {
                    if let NetworkEvent::Authenticated { transport_peer, application_peer } = b.unwrap() {
                        b_authenticated |= transport_peer == transport_a && application_peer == application_a;
                    }
                }
            }
        }
    })
    .await
    .expect("both application identities must be authenticated");

    assert!(node_a.connectivity_diagnostics().direct_connectivity);
    assert!(node_b.connectivity_diagnostics().direct_connectivity);
    assert!(!node_a.connectivity_diagnostics().relay_connectivity);
    assert!(!node_b.connectivity_diagnostics().relay_connectivity);

    // Independently exercise the ordinary authenticated control RPC before
    // opening the game stream; both must work on the selected direct path.
    let request = node_b.send_request(&transport_a, WireRequest::Ping { nonce: 29 }).unwrap();
    timeout(Duration::from_secs(8), async {
        loop {
            tokio::select! {
                event = node_a.next_event() => {
                    if let NetworkEvent::InboundRequest { transport_peer, request: WireRequest::Ping { nonce }, channel } = event.unwrap() {
                        assert_eq!(transport_peer, transport_b);
                        node_a.respond(channel, WireResponse::Pong { nonce }).unwrap();
                    }
                }
                event = node_b.next_event() => {
                    if let NetworkEvent::Response { transport_peer, request_id, response: WireResponse::Pong { nonce: 29 } } = event.unwrap() {
                        assert_eq!(transport_peer, transport_a);
                        assert_eq!(request_id, request);
                        break;
                    }
                }
            }
        }
    }).await.expect("authenticated control RPC must make progress");
    let registry_a = node_a.game_authenticated_peers();
    let registry_b = node_b.game_authenticated_peers();
    assert_eq!(registry_a.read().unwrap().get(&transport_b), Some(&application_b));
    assert_eq!(registry_b.read().unwrap().get(&transport_a), Some(&application_a));
    let mut control_a = node_a.game_stream_control();
    let mut control_b = node_b.game_stream_control();
    let mut incoming = control_a.accept(StreamProtocol::new(GAME_STREAM_PROTOCOL)).unwrap();

    let driver_a = tokio::spawn(async move {
        loop {
            node_a.next_event().await.unwrap();
        }
    });
    let driver_b = tokio::spawn(async move {
        loop {
            node_b.next_event().await.unwrap();
        }
    });
    let start = Instant::now();
    timeout(Duration::from_secs(25), async {
        let send = async {
            let mut stream = control_b
                .open_stream(transport_a, StreamProtocol::new(GAME_STREAM_PROTOCOL))
                .await
                .expect("dedicated Minecraft game stream must negotiate");
            for sequence in 0..256 {
                let bytes = [sequence as u8; 4096];
                stream.write_all(&bytes).await.unwrap();
                stream.flush().await.unwrap();
                let mut echo = [0; 4096];
                stream.read_exact(&mut echo).await.unwrap();
                assert_eq!(echo, bytes);
            }
        };
        let receive = async {
            let (sender, mut stream) = incoming.next().await.expect("must receive game stream");
            assert_eq!(sender, transport_b);
            assert_eq!(registry_a.read().unwrap().get(&sender), Some(&application_b));
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
    .unwrap_or_else(|_| panic!("{transport_name} game substream must make progress"));
    eprintln!(
        "capacity transport={transport_name} acknowledged_bytes=1048576 elapsed_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    driver_a.abort();
    driver_b.abort();
}

#[tokio::test]
async fn authenticated_game_substream_echoes_bytes_over_quic_without_snapshot_protocol() {
    authenticated_game_substream_echoes_bytes_without_snapshot_protocol("/ip4/127.0.0.1/udp/0/quic-v1", "QUIC").await;
}

#[tokio::test]
async fn authenticated_game_substream_echoes_bytes_over_tcp_noise_without_snapshot_protocol() {
    authenticated_game_substream_echoes_bytes_without_snapshot_protocol("/ip4/127.0.0.1/tcp/0", "TCP/Noise").await;
}

#[tokio::test]
async fn authenticated_game_substream_echoes_bytes_over_ipv6_quic() {
    // Explicitly report unavailable IPv6 instead of falling back to IPv4.
    // Linux CI is required to exercise this path; other OS runners may have
    // IPv6 disabled by policy.
    if let Err(error) = std::net::UdpSocket::bind("[::1]:0") {
        #[cfg(target_os = "linux")]
        panic!("IPv6 loopback is required on Linux: {error}");
        #[cfg(not(target_os = "linux"))]
        {
            eprintln!("IPv6 loopback not available on this runner: {error}");
            return;
        }
    }
    authenticated_game_substream_echoes_bytes_without_snapshot_protocol("/ip6/::1/udp/0/quic-v1", "IPv6 QUIC").await;
}
