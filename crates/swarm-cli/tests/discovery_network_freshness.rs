use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use swarm_cli::discovery::{resolve_world, search_public_worlds, DiscoverySearchInputV1, DISCOVERY_CAPABILITY};
use swarm_core::{sign_discovery_freshness_vote, sign_world_announcement, DataPaths, PeerIdentity};
use swarm_network::{generate_transport_key, DiscoveryNetworkEvent, DiscoveryNode, WireRequest, WireResponse};
use swarm_protocol::{
    DiscoveryCanonicalHeadV1, DiscoveryCompatibilityV1, DiscoveryFreshnessChallengeV1, DiscoveryMembershipProofV1,
    Hash32, MembershipPolicyV1, MembershipRecordV1, WorldAnnouncementV1, WorldGenesisV1, WorldMemberV1,
    WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
};
use tokio::{task::JoinHandle, time::timeout};

const A: [u8; 32] = [51; 32];
const B: [u8; 32] = [52; 32];
const C: [u8; 32] = [53; 32];
const X: [u8; 32] = [54; 32];

fn identity(secret: [u8; 32]) -> PeerIdentity {
    PeerIdentity::from_secret_bytes(secret)
}

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn signed_membership(
    authority: &PeerIdentity,
    world: swarm_protocol::WorldId,
    epoch: u64,
    sequence: u64,
    previous: Option<Hash32>,
    members: &[WorldMemberV1],
) -> MembershipRecordV1 {
    let mut record = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch,
        sequence,
        previous_membership_hash: previous,
        members: members.to_vec(),
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    authority.sign_membership(&mut record).unwrap();
    record
}

struct AnnouncementFixture<'a> {
    authority: &'a PeerIdentity,
    world: swarm_protocol::WorldId,
    membership: &'a MembershipRecordV1,
    epoch: u64,
    fence: u64,
    config_sequence: u64,
    config_hash: Hash32,
    canonical_head: DiscoveryCanonicalHeadV1,
    sequence: u64,
    name: &'a str,
}

fn announcement(params: AnnouncementFixture<'_>) -> WorldAnnouncementV1 {
    let AnnouncementFixture {
        authority,
        world,
        membership,
        epoch,
        fence,
        config_sequence,
        config_hash,
        canonical_head,
        sequence,
        name,
    } = params;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let mut value = WorldAnnouncementV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        presentation: WorldPresentationV1 {
            name: name.into(),
            description: "network freshness".into(),
            tags: vec!["survival".into()],
            icon_hash: None,
            approximate_region: Some("test".into()),
        },
        compatibility: DiscoveryCompatibilityV1 {
            minecraft_version: "1.21.8".into(),
            loader_id: "fabric".into(),
            loader_version: "0.17.2".into(),
            fabric_adapter_version: "0.5.0".into(),
            compatibility_fingerprint: Hash32([60; 32]),
        },
        visibility: WorldVisibilityV1::Public,
        membership_policy: MembershipPolicyV1::InviteOnly,
        config_sequence,
        config_hash,
        membership_sequence: membership.sequence,
        membership_hash: membership.record_hash().unwrap(),
        authority_epoch: epoch,
        fencing_token: fence,
        canonical_head: Some(canonical_head),
        announcement_sequence: sequence,
        issued_unix_ms: now.saturating_sub(1_000),
        expires_unix_ms: now.checked_add(60_000).unwrap(),
        announcer_peer_id: authority.peer_id(),
        announcer_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    sign_world_announcement(authority, &mut value).unwrap();
    value
}

fn challenge_matches(
    announcement: &WorldAnnouncementV1,
    proof: &DiscoveryMembershipProofV1,
    challenge: &DiscoveryFreshnessChallengeV1,
) -> bool {
    let pending = proof.pending_membership.as_ref().map(|proposal| proposal.proposal_hash().unwrap());
    challenge.protocol_version == PROTOCOL_VERSION
        && challenge.world_id == announcement.world_id
        && challenge.announcement_hash == announcement.announcement_hash().unwrap()
        && challenge.membership_sequence == announcement.membership_sequence
        && challenge.membership_hash == announcement.membership_hash
        && challenge.pending_membership_proposal_hash == pending
        && challenge.authority_peer_id == announcement.announcer_peer_id
        && challenge.authority_epoch == announcement.authority_epoch
        && challenge.fencing_token == announcement.fencing_token
        && challenge.config_sequence == announcement.config_sequence
        && challenge.config_hash == announcement.config_hash
        && challenge.canonical_head == announcement.canonical_head
}

struct PeerPlan {
    label: &'static str,
    identity: PeerIdentity,
    node: DiscoveryNode,
    announcement: Option<WorldAnnouncementV1>,
    context: Option<DiscoveryMembershipProofV1>,
    vote_state: Option<(WorldAnnouncementV1, DiscoveryMembershipProofV1)>,
    malformed_context: bool,
    delay_ms: u64,
}

async fn make_node(secret: [u8; 32]) -> (PeerIdentity, DiscoveryNode, String, String) {
    let identity = identity(secret);
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()]).unwrap();
    let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key()).unwrap();
    let transport_peer = node.local_transport_peer_id().to_string();
    node.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let address = timeout(Duration::from_secs(10), async {
        loop {
            if let DiscoveryNetworkEvent::Listening { address } = node.next_event().await.unwrap() {
                break format!("{address}/p2p/{}", node.local_transport_peer_id());
            }
        }
    })
    .await
    .expect("discovery provider should listen");
    (identity, node, address, transport_peer)
}

async fn assert_peer_tasks_healthy(
    tasks: &mut [JoinHandle<Result<(), String>>],
    lifecycle: &Arc<Mutex<Vec<String>>>,
    stage: &str,
) {
    for (index, task) in tasks.iter_mut().enumerate() {
        if task.is_finished() {
            let outcome = task.await;
            panic!(
                "provider task {index} exited during {stage}: {outcome:?}; lifecycle={:?}",
                lifecycle.lock().unwrap().clone()
            );
        }
    }
}

fn spawn_peer(
    mut plan: PeerPlan,
    order: Arc<Mutex<Vec<&'static str>>>,
    lifecycle: Arc<Mutex<Vec<String>>>,
) -> JoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        loop {
            let event = match plan.node.next_event().await {
                Ok(event) => event,
                Err(error) => {
                    let message = format!("peer {} discovery task exited: {error:#}", plan.label);
                    lifecycle.lock().unwrap().push(message.clone());
                    return Err(message);
                }
            };
            let DiscoveryNetworkEvent::InboundRequest { request, channel, .. } = event else { continue };
            match request {
                WireRequest::DiscoveryPublic { .. } => {
                    if plan.delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(plan.delay_ms)).await;
                    }
                    order.lock().unwrap().push(plan.label);
                    let values = plan.announcement.clone().into_iter().collect();
                    let _ = plan.node.respond(channel, WireResponse::DiscoveryWorlds(values));
                }
                WireRequest::DiscoveryResolve { world_id } => {
                    if plan.delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(plan.delay_ms)).await;
                    }
                    order.lock().unwrap().push(plan.label);
                    let value = plan
                        .announcement
                        .clone()
                        .filter(|announcement| announcement.world_id == world_id)
                        .map(Box::new);
                    let _ = plan.node.respond(channel, WireResponse::DiscoveryResolved(value));
                }
                WireRequest::DiscoveryFreshnessContext { announcement_hash, .. } => {
                    let response = if plan.malformed_context {
                        plan.context.clone().map(|mut proof| {
                            proof.protocol_version = PROTOCOL_VERSION + 1;
                            Box::new(proof)
                        })
                    } else {
                        plan.announcement.as_ref().and_then(|announcement| {
                            (announcement.announcement_hash().ok() == Some(announcement_hash))
                                .then(|| plan.context.clone().map(Box::new))
                                .flatten()
                        })
                    };
                    let _ = plan.node.respond(channel, WireResponse::DiscoveryFreshnessContext(response));
                }
                WireRequest::DiscoveryFreshnessVote(challenge) => {
                    let response = plan.vote_state.as_ref().and_then(|(announcement, proof)| {
                        challenge_matches(announcement, proof, &challenge)
                            .then(|| sign_discovery_freshness_vote(&plan.identity, &challenge).ok())
                            .flatten()
                            .map(Box::new)
                    });
                    let _ = plan.node.respond(channel, WireResponse::DiscoveryFreshnessVote(response));
                }
                _ => {
                    let _ = plan.node.respond(
                        channel,
                        WireResponse::Error { code: "TEST_UNSUPPORTED".into(), message: "unsupported".into() },
                    );
                }
            }
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malicious_and_stale_providers_cannot_win_browse_or_exact_resolve() {
    let a_record_id = identity(A);
    let b_record_id = identity(B);
    let c_record_id = identity(C);
    let attacker_record_id = identity(X);
    let mut members = vec![member(&a_record_id), member(&b_record_id), member(&c_record_id)];
    members.sort_by_key(|member| member.peer_id);
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "1.21.8".into(),
        fabric_loader_version: "0.17.2".into(),
        compatibility_fingerprint: Hash32([60; 32]),
        creation_nonce: [61; 32],
        creator_public_key: a_record_id.public_key(),
        initial_membership: members.iter().map(|member| member.peer_id).collect(),
    };
    let world = genesis.world_id().unwrap();
    let initial = signed_membership(&a_record_id, world, 1, 0, None, &members);
    let current = signed_membership(&b_record_id, world, 2, 1, Some(initial.record_hash().unwrap()), &members);
    let stale_proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis: genesis.clone(),
        initial_membership: initial.clone(),
        membership_certificates: Vec::new(),
        current_membership: initial.clone(),
        pending_membership: None,
    };
    let current_proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis,
        initial_membership: initial.clone(),
        membership_certificates: Vec::new(),
        current_membership: current.clone(),
        pending_membership: None,
    };
    let stale_announcement = announcement(AnnouncementFixture {
        authority: &a_record_id,
        world,
        membership: &initial,
        epoch: 1,
        fence: 1,
        config_sequence: 1,
        config_hash: Hash32([62; 32]),
        canonical_head: DiscoveryCanonicalHeadV1 {
            snapshot_number: 1,
            manifest_hash: Hash32([63; 32]),
            epoch: 1,
            sequence: 1,
        },
        sequence: 1,
        name: "stale-first",
    });
    let current_announcement = announcement(AnnouncementFixture {
        authority: &b_record_id,
        world,
        membership: &current,
        epoch: 2,
        fence: 2,
        config_sequence: 2,
        config_hash: Hash32([64; 32]),
        canonical_head: DiscoveryCanonicalHeadV1 {
            snapshot_number: 2,
            manifest_hash: Hash32([65; 32]),
            epoch: 2,
            sequence: 2,
        },
        sequence: 2,
        name: "current",
    });
    let mut attacker_announcement = current_announcement.clone();
    attacker_announcement.presentation.name = "attacker-first".into();
    attacker_announcement.announcement_sequence = 3;
    attacker_announcement.signature.clear();
    sign_world_announcement(&attacker_record_id, &mut attacker_announcement).unwrap();

    let (b_identity, mut b_node, b_address, b_transport_peer) = make_node(B).await;
    let (c_identity, mut c_node, c_address, c_transport_peer) = make_node(C).await;
    let (a_identity, mut a_node, a_address, a_transport_peer) = make_node(A).await;
    let (x_identity, mut x_node, x_address, x_transport_peer) = make_node(X).await;
    eprintln!(
        "FINAL-028 transport peers: current={b_transport_peer} voter={c_transport_peer} stale={a_transport_peer} attacker={x_transport_peer}"
    );

    b_node.add_bootstrap_address(c_address.parse().unwrap()).unwrap();
    c_node.add_bootstrap_address(b_address.parse().unwrap()).unwrap();
    a_node.add_bootstrap_address(b_address.parse().unwrap()).unwrap();
    x_node.add_bootstrap_address(b_address.parse().unwrap()).unwrap();
    let _ = b_node.bootstrap();
    let _ = c_node.bootstrap();
    let _ = a_node.bootstrap();
    let _ = x_node.bootstrap();
    for node in [&mut b_node, &mut a_node, &mut x_node] {
        node.start_providing_public_directory().unwrap();
    }
    for node in [&mut b_node, &mut c_node, &mut a_node, &mut x_node] {
        node.start_providing_world(world).unwrap();
    }

    let order = Arc::new(Mutex::new(Vec::new()));
    let lifecycle = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut tasks = vec![
        spawn_peer(
            PeerPlan {
                label: "current",
                identity: b_identity,
                node: b_node,
                announcement: Some(current_announcement.clone()),
                context: Some(current_proof.clone()),
                vote_state: Some((current_announcement.clone(), current_proof.clone())),
                malformed_context: false,
                delay_ms: 2_000,
            },
            order.clone(),
            lifecycle.clone(),
        ),
        spawn_peer(
            PeerPlan {
                label: "voter",
                identity: c_identity,
                node: c_node,
                announcement: None,
                context: None,
                vote_state: Some((current_announcement.clone(), current_proof.clone())),
                malformed_context: false,
                delay_ms: 0,
            },
            order.clone(),
            lifecycle.clone(),
        ),
        spawn_peer(
            PeerPlan {
                label: "stale",
                identity: a_identity,
                node: a_node,
                announcement: Some(stale_announcement.clone()),
                context: Some(stale_proof.clone()),
                vote_state: Some((stale_announcement, stale_proof)),
                malformed_context: false,
                delay_ms: 0,
            },
            order.clone(),
            lifecycle.clone(),
        ),
        spawn_peer(
            PeerPlan {
                label: "attacker",
                identity: x_identity,
                node: x_node,
                announcement: Some(attacker_announcement),
                context: Some(current_proof.clone()),
                vote_state: None,
                malformed_context: true,
                delay_ms: 10,
            },
            order.clone(),
            lifecycle.clone(),
        ),
    ];

    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path());
    // Dial hostile/noncanonical locators first so participation and ordering are
    // properties of the regression topology rather than transport scheduling.
    let bootstraps = vec![a_address.clone(), x_address.clone(), b_address.clone(), c_address.clone()];

    // Drive Kademlia readiness from observed provider sets rather than wall-clock sleeps.

    order.lock().unwrap().clear();
    let browse_result = search_public_worlds(&paths, DiscoverySearchInputV1::default(), &bootstraps).await;
    assert_peer_tasks_healthy(&mut tasks, &lifecycle, "public browse").await;
    let browse_order = order.lock().unwrap().clone();
    let report = browse_result.unwrap_or_else(|error| {
        panic!(
            "public browse failed: {error:#}; order={browse_order:?}; lifecycle={:?}",
            lifecycle.lock().unwrap().clone()
        )
    });
    assert_eq!(report.results.len(), 1, "only the live canonical proof may survive browse: {report:?}");
    assert_eq!(report.results[0].announcer_peer_id, b_record_id.peer_id().to_string());
    assert!(browse_order.contains(&"stale"), "stale provider must actually participate: {browse_order:?}");
    assert!(browse_order.contains(&"attacker"), "malformed attacker must actually participate: {browse_order:?}");
    assert!(browse_order.contains(&"current"), "current provider must actually participate: {browse_order:?}");
    assert!(
        browse_order.iter().position(|label| *label == "stale")
            < browse_order.iter().position(|label| *label == "current")
    );

    order.lock().unwrap().clear();
    let resolved_result = resolve_world(&paths, world, &bootstraps).await;
    assert_peer_tasks_healthy(&mut tasks, &lifecycle, "exact resolve").await;
    let resolve_order = order.lock().unwrap().clone();
    let resolved = resolved_result.unwrap_or_else(|error| {
        panic!(
            "exact resolve failed: {error:#}; order={resolve_order:?}; lifecycle={:?}",
            lifecycle.lock().unwrap().clone()
        )
    });
    assert_eq!(
        resolved.state, "found",
        "current authority should resolve after stale/attacker candidates: {resolved:?}"
    );
    let card = resolved.world.expect("current world card");
    assert_eq!(card.announcer_peer_id, b_record_id.peer_id().to_string());
    assert!(resolve_order.contains(&"stale"), "stale provider must actually participate: {resolve_order:?}");
    assert!(resolve_order.contains(&"attacker"), "malformed attacker must actually participate: {resolve_order:?}");
    assert!(resolve_order.contains(&"current"), "current provider must actually participate: {resolve_order:?}");
    assert!(
        resolve_order.iter().position(|label| *label == "stale")
            < resolve_order.iter().position(|label| *label == "current")
    );
    assert_ne!(resolve_order.first().copied(), Some("current"), "resolver must tolerate a noncanonical first response");

    for task in tasks {
        task.abort();
        match task.await {
            Err(error) if error.is_cancelled() => {}
            outcome => panic!("provider task did not end by expected test cleanup cancellation: {outcome:?}"),
        }
    }
}

fn spawn_echo_peer(mut node: DiscoveryNode, label: &'static str) -> JoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        loop {
            if let DiscoveryNetworkEvent::InboundRequest { channel, .. } =
                node.next_event().await.map_err(|error| format!("{label} discovery node failed: {error:#}"))?
            {
                node.respond(channel, WireResponse::Error { code: "TEST_OK".into(), message: label.into() })
                    .map_err(|error| format!("{label} response failed: {error:#}"))?;
            }
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicate_dials_and_peer_disconnect_keep_surviving_provider_usable() {
    let (_, closing_node, closing_address, _) = make_node([71; 32]).await;
    let (_, surviving_node, surviving_address, _) = make_node([72; 32]).await;
    let (_, mut client, _, _) = make_node([73; 32]).await;

    let closing_task = spawn_echo_peer(closing_node, "closing-provider");
    let surviving_task = spawn_echo_peer(surviving_node, "surviving-provider");

    let closing_peer = client.add_bootstrap_address(closing_address.parse().unwrap()).unwrap();
    let surviving_peer = client.add_bootstrap_address(surviving_address.parse().unwrap()).unwrap();

    for _ in 0..8 {
        client.dial_peer(closing_peer).unwrap();
        client.dial_peer(surviving_peer).unwrap();
    }

    timeout(Duration::from_secs(10), async {
        loop {
            if client.is_authenticated(&closing_peer) && client.is_authenticated(&surviving_peer) {
                break;
            }
            client.next_event().await.expect("client discovery event while authenticating providers");
        }
    })
    .await
    .expect("both providers must authenticate");

    assert_eq!(client.established_connection_count(&closing_peer), 1);
    assert_eq!(client.established_connection_count(&surviving_peer), 1);

    for _ in 0..8 {
        client.dial_peer(closing_peer).unwrap();
        client.dial_peer(surviving_peer).unwrap();
    }
    let _ = timeout(Duration::from_millis(500), async {
        loop {
            client.next_event().await.expect("client discovery event while suppressing duplicate dials");
            assert!(client.established_connection_count(&closing_peer) <= 1);
            assert!(client.established_connection_count(&surviving_peer) <= 1);
        }
    })
    .await;
    assert_eq!(client.established_connection_count(&closing_peer), 1);
    assert_eq!(client.established_connection_count(&surviving_peer), 1);

    closing_task.abort();
    let closing_outcome = closing_task.await;
    assert!(closing_outcome.is_err_and(|error| error.is_cancelled()));

    timeout(Duration::from_secs(10), async {
        loop {
            if !client.is_connected(&closing_peer) {
                break;
            }
            client.next_event().await.expect("client discovery event while closing one provider");
        }
    })
    .await
    .expect("closing provider disconnect must be observed");

    assert!(client.is_connected(&surviving_peer));
    assert!(client.is_authenticated(&surviving_peer));
    assert_eq!(client.established_connection_count(&surviving_peer), 1);

    let request_id = client
        .send_request(
            &surviving_peer,
            WireRequest::DiscoveryPublic { filter: swarm_protocol::DiscoveryFilterV1::default() },
        )
        .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            match client.next_event().await.expect("client discovery event awaiting surviving provider response") {
                DiscoveryNetworkEvent::Response {
                    transport_peer,
                    request_id: observed,
                    response: WireResponse::Error { code, .. },
                } if transport_peer == surviving_peer && observed == request_id => {
                    assert_eq!(code, "TEST_OK");
                    break;
                }
                DiscoveryNetworkEvent::OutboundFailure { transport_peer, request_id: observed, error }
                    if transport_peer == surviving_peer && observed == request_id =>
                {
                    panic!("surviving provider request failed after peer disconnect: {error}");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("surviving provider must remain usable");

    surviving_task.abort();
    let surviving_outcome = surviving_task.await;
    assert!(surviving_outcome.is_err_and(|error| error.is_cancelled()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_bidirectional_dials_converge_on_one_authenticated_connection() {
    let (_, mut left, left_address, _) = make_node([81; 32]).await;
    let (_, mut right, right_address, _) = make_node([82; 32]).await;

    let right_peer = left.add_bootstrap_address(right_address.parse().unwrap()).unwrap();
    let left_peer = right.add_bootstrap_address(left_address.parse().unwrap()).unwrap();

    // Exercise the single-flight path before either swarm is polled.
    for _ in 0..8 {
        left.dial_peer(right_peer).unwrap();
        right.dial_peer(left_peer).unwrap();
    }

    let mut left_authenticated = false;
    let mut right_authenticated = false;
    timeout(Duration::from_secs(10), async {
        while !(left_authenticated && right_authenticated) {
            tokio::select! {
                event = left.next_event() => {
                    if matches!(
                        event.expect("left discovery event during symmetric dial"),
                        DiscoveryNetworkEvent::Authenticated { transport_peer, .. } if transport_peer == right_peer
                    ) {
                        left_authenticated = true;
                    }
                }
                event = right.next_event() => {
                    if matches!(
                        event.expect("right discovery event during symmetric dial"),
                        DiscoveryNetworkEvent::Authenticated { transport_peer, .. } if transport_peer == left_peer
                    ) {
                        right_authenticated = true;
                    }
                }
            }
        }
    })
    .await
    .expect("symmetric discovery dials must converge and authenticate");

    timeout(Duration::from_secs(2), async {
        loop {
            if left.established_connection_count(&right_peer) == 1
                && right.established_connection_count(&left_peer) == 1
                && left.is_authenticated(&right_peer)
                && right.is_authenticated(&left_peer)
            {
                break;
            }
            tokio::select! {
                event = left.next_event() => {
                    event.expect("left discovery event while converging symmetric dial");
                }
                event = right.next_event() => {
                    event.expect("right discovery event while converging symmetric dial");
                }
            }
        }
    })
    .await
    .expect("symmetric discovery dials must settle to one authenticated connection per peer");

    assert_eq!(left.established_connection_count(&right_peer), 1);
    assert_eq!(right.established_connection_count(&left_peer), 1);
    assert!(left.is_authenticated(&right_peer));
    assert!(right.is_authenticated(&left_peer));
}
