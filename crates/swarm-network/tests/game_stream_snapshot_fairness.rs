//! Exercise game-data backpressure while the independent snapshot protocol
//! continues to make progress over the SAME authenticated encrypted QUIC
//! connection. Synthetic byte traffic, NOT real Minecraft gameplay or
//! network throughput certification.

use ed25519_dalek::{Signer, SigningKey};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::StreamProtocol;
use rand_core::OsRng;
use std::time::Duration;
use swarm_network::{
    game_route::GAME_STREAM_PROTOCOL, generate_transport_key, NetworkEvent, SwarmNode, WireRequest, WireResponse,
};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PROTOCOL_VERSION};
use tokio::{
    sync::{mpsc, oneshot},
    time::{sleep, timeout},
};

const CHUNKS: u64 = 128;
const CHUNK_BYTES: usize = 8192;
const PING_NONCE: u64 = 0x6A6A;

fn signed_hello(key: &SigningKey, nonce: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".to_owned()],
        nonce: [nonce; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

#[tokio::test]
async fn snapshot_ping_progresses_while_authenticated_game_bytes_face_backpressure() {
    timeout(Duration::from_secs(25), async {
        let key_host = SigningKey::generate(&mut OsRng);
        let key_player = SigningKey::generate(&mut OsRng);
        let app_host = peer_id_from_public_key(&key_host.verifying_key().to_bytes());
        let app_player = peer_id_from_public_key(&key_player.verifying_key().to_bytes());
        let mut host = SwarmNode::new(generate_transport_key(), signed_hello(&key_host, 1), key_host).unwrap();
        let mut player = SwarmNode::new(generate_transport_key(), signed_hello(&key_player, 2), key_player).unwrap();
        let host_transport = host.local_transport_peer_id();
        let player_transport = player.local_transport_peer_id();
        host.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
        let address = loop {
            if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() {
                break address;
            }
        };
        player.dial(address).unwrap();
        let mut host_ready = false;
        let mut player_ready = false;
        timeout(Duration::from_secs(12), async {
            while !host_ready || !player_ready {
                tokio::select! {
                    event = host.next_event() => {
                        if let NetworkEvent::Authenticated { transport_peer, application_peer } = event.unwrap() {
                            host_ready |= transport_peer == player_transport && application_peer == app_player;
                        }
                    }
                    event = player.next_event() => {
                        if let NetworkEvent::Authenticated { transport_peer, application_peer } = event.unwrap() {
                            player_ready |= transport_peer == host_transport && application_peer == app_host;
                        }
                    }
                }
            }
        })
        .await
        .expect("both peers must complete application auth first");

        let mut host_control = host.game_stream_control();
        let mut player_control = player.game_stream_control();
        let mut incoming = host_control.accept(StreamProtocol::new(GAME_STREAM_PROTOCOL)).unwrap();
        let (ping_start, mut ping_commands) = mpsc::channel::<()>(1);
        let (ping_done, ping_response) = oneshot::channel::<()>();
        let host_driver = tokio::spawn(async move {
            loop {
                if let NetworkEvent::InboundRequest {
                    request: WireRequest::Ping { nonce: PING_NONCE }, channel, ..
                } = host.next_event().await.unwrap()
                {
                    host.respond(channel, WireResponse::Pong { nonce: PING_NONCE }).unwrap();
                }
            }
        });
        let player_driver = tokio::spawn(async move {
            let mut outstanding = None;
            let mut done = Some(ping_done);
            loop {
                tokio::select! {
                    command = ping_commands.recv(), if outstanding.is_none() => {
                        if command.is_some() {
                            outstanding = Some(
                                player.send_request(&host_transport, WireRequest::Ping { nonce: PING_NONCE }).unwrap()
                            );
                        }
                    }
                    event = player.next_event() => {
                        if let NetworkEvent::Response {
                            request_id,
                            response: WireResponse::Pong { nonce: PING_NONCE },
                            ..
                        } = event.unwrap() {
                            if outstanding == Some(request_id) {
                                if let Some(tx) = done.take() { let _ = tx.send(()); }
                            }
                        }
                    }
                }
            }
        });

        let writer = tokio::spawn(async move {
            let mut stream = player_control
                .open_stream(host_transport, StreamProtocol::new(GAME_STREAM_PROTOCOL))
                .await
                .expect("separate game stream must negotiate");
            let mut packet = [0x42_u8; CHUNK_BYTES];
            for sequence in 0..CHUNKS {
                packet[..8].copy_from_slice(&sequence.to_le_bytes());
                stream.write_all(&packet).await.unwrap();
            }
            stream.flush().await.unwrap();
            let mut complete = [0u8; 8];
            stream.read_exact(&mut complete).await.unwrap();
            assert_eq!(&complete, b"complete");
        });
        let receiver = tokio::spawn(async move {
            let (remote, mut stream) = incoming.next().await.expect("host must accept game stream");
            assert_eq!(remote, player_transport);
            let mut packet = [0u8; CHUNK_BYTES];
            for sequence in 0..CHUNKS {
                stream.read_exact(&mut packet).await.unwrap();
                assert_eq!(&packet[..8], &sequence.to_le_bytes());
                assert!(packet[8..].iter().all(|byte| *byte == 0x42));
                // Slow game consumption to exercise transport backpressure.
                sleep(Duration::from_millis(6)).await;
            }
            stream.write_all(b"complete").await.unwrap();
            stream.flush().await.unwrap();
        });

        sleep(Duration::from_millis(50)).await;
        assert!(!receiver.is_finished(), "game transfer must remain active during snapshot check");
        ping_start.send(()).await.unwrap();
        timeout(Duration::from_secs(3), ping_response)
            .await
            .expect("snapshot control must progress despite game backpressure")
            .expect("snapshot response driver must not crash");
        assert!(!receiver.is_finished(), "snapshot check should finish while game still has backpressure");

        timeout(Duration::from_secs(12), async {
            writer.await.unwrap();
            receiver.await.unwrap();
        })
        .await
        .expect("game traffic must eventually complete without corruption");
        host_driver.abort();
        player_driver.abort();
    })
    .await
    .expect("authenticated control and game streams must not deadlock");
}
