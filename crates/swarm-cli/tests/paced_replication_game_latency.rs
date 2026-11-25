//! Actual encoded storage blob, authenticated QUIC and concurrent game
//! substream. The test driver invokes the production pacer directly; it is
//! not a real Minecraft session or a full daemon admission test.
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::StreamProtocol;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use swarm_cli::replication_pacing::{ChunkPlan, ReplicationPacer};
use swarm_core::PeerIdentity;
use swarm_network::{
    game_route::GAME_STREAM_PROTOCOL, generate_transport_key, NetworkEvent, SwarmNode, WireRequest, WireResponse,
    MAX_BLOB_CHUNK,
};
use swarm_protocol::WorldId;
use swarm_storage::Storage;
use tokio::{
    sync::oneshot,
    time::{interval, sleep, timeout, MissedTickBehavior},
};

#[tokio::test]
async fn paced_blob_transfer_preserves_game_echo_latency() {
    timeout(Duration::from_secs(45), async {
        let temp = tempfile::tempdir().unwrap();
        let source = Storage::open(temp.path().join("source")).unwrap();
        let replica = Storage::open(temp.path().join("replica")).unwrap();
        let world = WorldId([9; 32]);
        let mut state = 0x1234_5678_u32;
        let bytes: Vec<u8> = (0..128 * 1024).map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        }).collect();
        let descriptor = source.put_blob(world, &bytes).unwrap();
        let encoded_size = descriptor.encoded_size;
        assert!(encoded_size >= 120 * 1024);
        let make_node = |seed| {
            let identity = PeerIdentity::from_secret_bytes([seed; 32]);
            let hello = identity.signed_peer_hello(vec!["snapshot-replication-v1".into()]).unwrap();
            let app = hello.peer_id;
            let node = SwarmNode::new(generate_transport_key(), hello, identity.network_signing_key()).unwrap();
            (node, app)
        };
        let (mut host, host_app) = make_node(23);
        let (mut player, player_app) = make_node(24);
        let host_peer = host.local_transport_peer_id();
        let player_peer = player.local_transport_peer_id();
        host.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
        let address = loop {
            if let NetworkEvent::Listening { address } = host.next_event().await.unwrap() { break address; }
        };
        player.dial(address).unwrap();
        let (mut host_ready, mut player_ready) = (false, false);
        while !host_ready || !player_ready {
            tokio::select! {
                event = host.next_event() => if let NetworkEvent::Authenticated { application_peer, transport_peer } = event.unwrap() {
                    host_ready |= application_peer == player_app && transport_peer == player_peer;
                },
                event = player.next_event() => if let NetworkEvent::Authenticated { application_peer, transport_peer } = event.unwrap() {
                    player_ready |= application_peer == host_app && transport_peer == host_peer;
                },
            }
        }

        let mut host_control = host.game_stream_control();
        let mut player_control = player.game_stream_control();
        let mut incoming = host_control.accept(StreamProtocol::new(GAME_STREAM_PROTOCOL)).unwrap();
        let progress = Arc::new(AtomicU64::new(0));
        let (begin, mut begin_rx) = oneshot::channel::<()>();
        let (done, done_rx) = oneshot::channel::<Duration>();
        let host_descriptor = descriptor.clone();
        let host_progress = Arc::clone(&progress);
        let host_driver = tokio::spawn(async move {
            let mut pacer = ReplicationPacer::new(32).unwrap();
            let mut ticks = interval(Duration::from_millis(20));
            ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut started = None;
            let mut outstanding = None;
            let mut done = Some(done);
            let mut offset = 0u64;
            loop {
                tokio::select! {
                    start = &mut begin_rx, if started.is_none() => {
                        start.unwrap();
                        started = Some(Instant::now());
                    }
                    _ = ticks.tick(), if started.is_some() && offset < encoded_size && outstanding.is_none() => {
                        let now = Instant::now();
                        if let ChunkPlan::Ready { max_bytes, paced } = pacer.plan(now, true, MAX_BLOB_CHUNK) {
                            let (data, finished) = source.read_encoded_blob_chunk(world, &host_descriptor, offset, max_bytes).unwrap();
                            let count = data.len();
                            let end = offset + count as u64;
                            let request = host.send_request(&player_peer, WireRequest::BlobChunk {
                                world_id: world, hash: host_descriptor.hash, encoding: host_descriptor.encoding,
                                offset, data, finished,
                            }).unwrap();
                            pacer.sent(now, count, paced);
                            outstanding = Some((request, end, now, paced));
                        }
                    }
                    event = host.next_event() => match event.unwrap() {
                        NetworkEvent::Response { transport_peer, request_id, response: WireResponse::BlobChunkAccepted { hash, next_offset } } => {
                            let (expected, end, sent, paced) = outstanding.take().expect("exact in-flight blob");
                            assert_eq!((transport_peer, request_id, hash, next_offset), (player_peer, expected, host_descriptor.hash, end));
                            pacer.acknowledge(sent.elapsed(), paced);
                            offset = end;
                            host_progress.store(offset, Ordering::Release);
                            if offset == encoded_size {
                                done.take().unwrap().send(started.unwrap().elapsed()).unwrap();
                            }
                        }
                        NetworkEvent::OutboundFailure { error, .. } => panic!("blob transport failure: {error}"),
                        _ => {}
                    }
                }
            }
        });
        let replica_descriptor = descriptor.clone();
        let player_driver = tokio::spawn(async move {
            loop {
                if let NetworkEvent::InboundRequest { transport_peer, request: WireRequest::BlobChunk { world_id, hash, encoding, offset, data, finished }, channel } = player.next_event().await.unwrap() {
                    assert_eq!(transport_peer, host_peer);
                    assert_eq!((world_id, hash, encoding), (world, replica_descriptor.hash, replica_descriptor.encoding));
                    let next_offset = replica.receive_blob_chunk(world, &replica_descriptor, offset, &data, finished).unwrap();
                    if finished { assert_eq!(replica.read_blob(world, &replica_descriptor).unwrap(), bytes); }
                    player.respond(channel, WireResponse::BlobChunkAccepted { hash, next_offset }).unwrap();
                }
            }
        });
        let echo = tokio::spawn(async move {
            let (peer, mut stream) = incoming.next().await.unwrap();
            assert_eq!(peer, player_peer);
            let mut packet = [0; 64];
            while stream.read_exact(&mut packet).await.is_ok() {
                stream.write_all(&packet).await.unwrap();
                stream.flush().await.unwrap();
            }
        });
        let mut game = player_control.open_stream(host_peer, StreamProtocol::new(GAME_STREAM_PROTOCOL)).await.unwrap();
        let mut packet = [0x7b; 64];
        let mut received = [0; 64];
        game.write_all(&packet).await.unwrap();
        game.flush().await.unwrap();
        game.read_exact(&mut received).await.unwrap();
        assert_eq!(packet, received);
        begin.send(()).unwrap();
        let mut samples = Vec::new();
        while progress.load(Ordering::Acquire) < encoded_size {
            packet[0] = samples.len() as u8;
            let start = Instant::now();
            timeout(Duration::from_secs(2), async {
                game.write_all(&packet).await.unwrap();
                game.flush().await.unwrap();
                game.read_exact(&mut received).await.unwrap();
            }).await.expect("game echo stalled during blob transfer");
            assert_eq!(packet, received);
            samples.push(start.elapsed());
            sleep(Duration::from_millis(100)).await;
        }
        let elapsed = done_rx.await.unwrap();
        assert!(samples.len() >= 12, "insufficient overlapping samples: {}", samples.len());
        samples.sort_unstable();
        let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
        println!("paced replication: {encoded_size} encoded bytes in {elapsed:?}; {} overlapping echo RTT samples, p95={p95:?}", samples.len());
        assert!(p95 <= Duration::from_millis(400), "game echo p95 exceeded 400ms: {p95:?}");
        assert!(elapsed >= Duration::from_secs(3), "blob pacing was bypassed");
        assert!(elapsed <= Duration::from_secs(30), "replication failed to make bounded progress");
        drop(game);
        echo.abort();
        host_driver.abort();
        player_driver.abort();
    }).await.expect("paced transfer with concurrent echo must terminate");
}
