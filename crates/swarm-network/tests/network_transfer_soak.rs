use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::{env, path::Path, time::Duration};
use swarm_network::{
    load_or_create_transport_key, BlobResumeV1, NetworkEvent, SwarmNode, TransportPeerId, WireRequest, WireResponse,
    MAX_BLOB_CHUNK,
};
use swarm_protocol::{peer_id_from_public_key, BlobEncoding, Hash32, PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tempfile::tempdir;
use tokio::time::timeout;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

fn signed_hello(key: &SigningKey, nonce: u8) -> PeerHelloV1 {
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["snapshot-replication-v1".into()],
        nonce: [nonce; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
    hello
}

fn synthetic_chunk(offset: u64, len: usize) -> Vec<u8> {
    let mut data = vec![0_u8; len];
    for (index, block) in data.chunks_mut(8).enumerate() {
        let absolute = offset.wrapping_add((index as u64) * 8);
        let word = absolute.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left((absolute as u32) & 31).to_le_bytes();
        block.copy_from_slice(&word[..block.len()]);
    }
    data
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

async fn listen_address(node: &mut SwarmNode) -> libp2p::Multiaddr {
    timeout(EVENT_TIMEOUT, async {
        loop {
            if let NetworkEvent::Listening { address } = node.next_event().await.unwrap() {
                break address;
            }
        }
    })
    .await
    .expect("receiver should start listening")
}

async fn wait_for_authentication(
    receiver: &mut SwarmNode,
    sender: &mut SwarmNode,
    receiver_app: PeerId,
    sender_app: PeerId,
) {
    let mut receiver_saw_sender = false;
    let mut sender_saw_receiver = false;
    timeout(EVENT_TIMEOUT, async {
        while !(receiver_saw_sender && sender_saw_receiver) {
            tokio::select! {
                event = receiver.next_event() => {
                    if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                        receiver_saw_sender |= application_peer == sender_app;
                    }
                }
                event = sender.next_event() => {
                    if let NetworkEvent::Authenticated { application_peer, .. } = event.unwrap() {
                        sender_saw_receiver |= application_peer == receiver_app;
                    }
                }
            }
        }
    })
    .await
    .expect("peers should authenticate after connect/reconnect");
}

struct TransferIdentity {
    world: WorldId,
    hash: Hash32,
}

async fn query_resume_offset(
    sender: &mut SwarmNode,
    receiver: &mut SwarmNode,
    receiver_transport: TransportPeerId,
    transfer: &TransferIdentity,
    committed_offset: u64,
) -> u64 {
    let request_id = sender
        .send_request(
            &receiver_transport,
            WireRequest::MissingBlobs { world_id: transfer.world, snapshot_number: 1, hashes: vec![transfer.hash] },
        )
        .unwrap();

    timeout(EVENT_TIMEOUT, async {
        loop {
            tokio::select! {
                event = receiver.next_event() => {
                    match event.unwrap() {
                        NetworkEvent::InboundRequest {
                            request: WireRequest::MissingBlobs { world_id, hashes, .. },
                            channel,
                            ..
                        } => {
                            assert_eq!(world_id, transfer.world);
                            assert_eq!(hashes, vec![transfer.hash]);
                            receiver
                                .respond(
                                    channel,
                                    WireResponse::MissingBlobs(vec![BlobResumeV1 {
                                        hash: transfer.hash,
                                        offset: committed_offset,
                                    }]),
                                )
                                .unwrap();
                        }
                        NetworkEvent::OutboundFailure { error, .. } => panic!("receiver outbound failure: {error}"),
                        _ => {}
                    }
                }
                event = sender.next_event() => {
                    match event.unwrap() {
                        NetworkEvent::Response {
                            request_id: observed,
                            response: WireResponse::MissingBlobs(resume),
                            ..
                        } if observed == request_id => {
                            let entry = resume.into_iter().find(|entry| entry.hash == transfer.hash).expect("resume entry");
                            break entry.offset;
                        }
                        NetworkEvent::OutboundFailure { request_id: observed, error, .. } if observed == request_id => {
                            panic!("resume negotiation failed: {error}");
                        }
                        _ => {}
                    }
                }
            }
        }
    })
    .await
    .expect("resume negotiation should complete")
}

async fn send_chunk(
    sender: &mut SwarmNode,
    receiver: &mut SwarmNode,
    receiver_transport: TransportPeerId,
    transfer: &TransferIdentity,
    committed_offset: &mut u64,
    total_bytes: u64,
    len: usize,
) {
    let offset = *committed_offset;
    let data = synthetic_chunk(offset, len);
    let next_offset = offset + len as u64;
    let request_id = sender
        .send_request(
            &receiver_transport,
            WireRequest::BlobChunk {
                world_id: transfer.world,
                hash: transfer.hash,
                encoding: BlobEncoding::Raw,
                offset,
                data,
                finished: next_offset == total_bytes,
            },
        )
        .unwrap();

    timeout(EVENT_TIMEOUT, async {
        loop {
            tokio::select! {
                event = receiver.next_event() => {
                    match event.unwrap() {
                        NetworkEvent::InboundRequest {
                            request: WireRequest::BlobChunk {
                                world_id,
                                hash,
                                encoding,
                                offset: observed_offset,
                                data,
                                finished,
                            },
                            channel,
                            ..
                        } => {
                            assert_eq!(world_id, transfer.world);
                            assert_eq!(hash, transfer.hash);
                            assert_eq!(encoding, BlobEncoding::Raw);
                            assert_eq!(observed_offset, *committed_offset);
                            assert_eq!(data, synthetic_chunk(observed_offset, data.len()));
                            let observed_next = observed_offset + data.len() as u64;
                            assert_eq!(finished, observed_next == total_bytes);
                            *committed_offset = observed_next;
                            receiver
                                .respond(
                                    channel,
                                    WireResponse::BlobChunkAccepted {
                                        hash: transfer.hash,
                                        next_offset: observed_next,
                                    },
                                )
                                .unwrap();
                        }
                        NetworkEvent::OutboundFailure { error, .. } => panic!("receiver outbound failure: {error}"),
                        _ => {}
                    }
                }
                event = sender.next_event() => {
                    match event.unwrap() {
                        NetworkEvent::Response {
                            request_id: observed,
                            response: WireResponse::BlobChunkAccepted { hash, next_offset: acknowledged },
                            ..
                        } if observed == request_id => {
                            assert_eq!(hash, transfer.hash);
                            assert_eq!(acknowledged, next_offset);
                            assert_eq!(*committed_offset, next_offset);
                            break;
                        }
                        NetworkEvent::OutboundFailure { request_id: observed, error, .. } if observed == request_id => {
                            panic!("blob chunk failed: {error}");
                        }
                        _ => {}
                    }
                }
            }
        }
    })
    .await
    .expect("blob chunk should be acknowledged");
}

async fn send_chunk_then_lose_ack(
    sender: &mut SwarmNode,
    receiver: &mut SwarmNode,
    receiver_transport: TransportPeerId,
    transfer: &TransferIdentity,
    committed_offset: &mut u64,
    total_bytes: u64,
    len: usize,
) {
    let offset = *committed_offset;
    let data = synthetic_chunk(offset, len);
    let next_offset = offset + len as u64;
    sender
        .send_request(
            &receiver_transport,
            WireRequest::BlobChunk {
                world_id: transfer.world,
                hash: transfer.hash,
                encoding: BlobEncoding::Raw,
                offset,
                data,
                finished: next_offset == total_bytes,
            },
        )
        .unwrap();

    timeout(EVENT_TIMEOUT, async {
        loop {
            tokio::select! {
                event = receiver.next_event() => {
                    if let NetworkEvent::InboundRequest {
                        request: WireRequest::BlobChunk {
                            world_id,
                            hash,
                            encoding,
                            offset: observed_offset,
                            data,
                            finished,
                        },
                        channel,
                        ..
                    } = event.unwrap()
                    {
                        assert_eq!(world_id, transfer.world);
                        assert_eq!(hash, transfer.hash);
                        assert_eq!(encoding, BlobEncoding::Raw);
                        assert_eq!(observed_offset, *committed_offset);
                        assert_eq!(data, synthetic_chunk(observed_offset, data.len()));
                        let observed_next = observed_offset + data.len() as u64;
                        assert_eq!(finished, observed_next == total_bytes);
                        *committed_offset = observed_next;
                        receiver
                            .respond(
                                channel,
                                WireResponse::BlobChunkAccepted {
                                    hash: transfer.hash,
                                    next_offset: observed_next,
                                },
                            )
                            .unwrap();
                        break;
                    }
                }
                event = sender.next_event() => {
                    if let NetworkEvent::OutboundFailure { error, .. } = event.unwrap() {
                        panic!("blob chunk failed before receiver committed it: {error}");
                    }
                }
            }
        }
    })
    .await
    .expect("receiver should commit the pre-disconnect chunk");

    // Deliberately do not poll the sender for the response. The sender is dropped by
    // the caller, so the receiver has committed data whose acknowledgement is lost.
    assert_eq!(*committed_offset, next_offset);
}

async fn new_sender(
    transport_key_path: &Path,
    app_key: &SigningKey,
    nonce: u8,
    listen: &libp2p::Multiaddr,
) -> SwarmNode {
    let transport_key = load_or_create_transport_key(transport_key_path).unwrap();
    let mut sender = SwarmNode::new(transport_key, signed_hello(app_key, nonce)).unwrap();
    sender.dial(listen.clone()).unwrap();
    sender
}

async fn run_interrupted_transfer(total_bytes: u64, restart_every: u64, chunk_bytes: usize) {
    assert!(total_bytes > restart_every);
    assert!(restart_every >= chunk_bytes as u64);
    assert!((1..=MAX_BLOB_CHUNK).contains(&chunk_bytes));

    let temp = tempdir().unwrap();
    let sender_transport_path = temp.path().join("sender/transport.key");
    let receiver_transport_path = temp.path().join("receiver/transport.key");
    let sender_app_key = SigningKey::generate(&mut OsRng);
    let receiver_app_key = SigningKey::generate(&mut OsRng);
    let sender_app = signed_hello(&sender_app_key, 1).peer_id;
    let receiver_hello = signed_hello(&receiver_app_key, 2);
    let receiver_app = receiver_hello.peer_id;

    let receiver_key = load_or_create_transport_key(&receiver_transport_path).unwrap();
    let receiver_transport = receiver_key.public().to_peer_id();
    let mut receiver = SwarmNode::new(receiver_key, receiver_hello).unwrap();
    receiver.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let listen = listen_address(&mut receiver).await;

    let transfer = TransferIdentity { world: WorldId([0x53; 32]), hash: Hash32([0xA7; 32]) };
    let mut sender = new_sender(&sender_transport_path, &sender_app_key, 3, &listen).await;
    let sender_transport = sender.local_transport_peer_id();
    wait_for_authentication(&mut receiver, &mut sender, receiver_app, sender_app).await;
    assert_eq!(receiver.application_peer(&sender_transport), Some(sender_app));

    let mut committed_offset = 0_u64;
    let mut next_restart = restart_every;
    let mut restarts = 0_u64;

    while committed_offset < total_bytes {
        let segment_end = next_restart.min(total_bytes);
        while committed_offset < segment_end {
            let remaining = segment_end - committed_offset;
            let len = remaining.min(chunk_bytes as u64) as usize;
            let at_forced_disconnect = committed_offset + len as u64 == segment_end && segment_end < total_bytes;

            if at_forced_disconnect {
                send_chunk_then_lose_ack(
                    &mut sender,
                    &mut receiver,
                    receiver_transport,
                    &transfer,
                    &mut committed_offset,
                    total_bytes,
                    len,
                )
                .await;
                drop(sender);
                restarts += 1;

                sender =
                    new_sender(&sender_transport_path, &sender_app_key, 3_u8.wrapping_add(restarts as u8), &listen)
                        .await;
                assert_eq!(sender.local_transport_peer_id(), sender_transport);
                wait_for_authentication(&mut receiver, &mut sender, receiver_app, sender_app).await;
                assert_eq!(receiver.application_peer(&sender_transport), Some(sender_app));

                let resume =
                    query_resume_offset(&mut sender, &mut receiver, receiver_transport, &transfer, committed_offset)
                        .await;
                assert_eq!(
                    resume, committed_offset,
                    "reconnect must resume after receiver-committed data even when its ack was lost"
                );
                next_restart = committed_offset.saturating_add(restart_every);
                break;
            }

            send_chunk(
                &mut sender,
                &mut receiver,
                receiver_transport,
                &transfer,
                &mut committed_offset,
                total_bytes,
                len,
            )
            .await;
        }
    }

    assert_eq!(committed_offset, total_bytes);
    assert!(restarts >= 1);
    println!(
        "network transfer soak complete: bytes={total_bytes} chunk_bytes={chunk_bytes} restarts={restarts} transport_peer={sender_transport}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "run explicitly in the impaired-network CI gate"]
async fn interrupted_quic_transfer_resumes_after_lost_ack() {
    run_interrupted_transfer(64 * MIB, 16 * MIB, MAX_BLOB_CHUNK).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "multi-gigabyte soak; run from the scheduled/manual Network Soak workflow"]
async fn multi_gib_interrupted_quic_transfer_soak() {
    let gib = env_u64("SWARMCRAFT_NETWORK_SOAK_GIB", 2);
    let restart_mib = env_u64("SWARMCRAFT_NETWORK_SOAK_RESTART_MIB", 256);
    assert!((1..=16).contains(&gib), "SWARMCRAFT_NETWORK_SOAK_GIB must be between 1 and 16");
    assert!((8..=4096).contains(&restart_mib), "SWARMCRAFT_NETWORK_SOAK_RESTART_MIB must be between 8 and 4096");

    run_interrupted_transfer(gib * GIB, restart_mib * MIB, MAX_BLOB_CHUNK).await;
}
