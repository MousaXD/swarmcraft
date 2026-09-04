from pathlib import Path


path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
text = path.read_text()

old_import = '''use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
'''
new_import = '''use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
'''
if old_import in text:
    text = text.replace(old_import, new_import, 1)
elif new_import not in text:
    raise SystemExit("missing network-test std import anchor")

old_make_signature = "async fn make_node(secret: [u8; 32]) -> (PeerIdentity, DiscoveryNode, String) {\n"
new_make_signature = "async fn make_node(secret: [u8; 32]) -> (PeerIdentity, DiscoveryNode, String, String) {\n"
if old_make_signature in text:
    text = text.replace(old_make_signature, new_make_signature, 1)
elif new_make_signature not in text:
    raise SystemExit("missing make_node signature anchor")

old_node_create = '''    let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key()).unwrap();
    node.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
'''
new_node_create = '''    let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key()).unwrap();
    let transport_peer = node.local_transport_peer_id().to_string();
    node.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
'''
if old_node_create in text:
    text = text.replace(old_node_create, new_node_create, 1)
elif new_node_create not in text:
    raise SystemExit("missing make_node construction anchor")

old_make_return = "    (identity, node, address)\n}\n\nfn spawn_peer"
new_make_return = "    (identity, node, address, transport_peer)\n}\n\nfn spawn_peer"
if old_make_return in text:
    text = text.replace(old_make_return, new_make_return, 1)
elif new_make_return not in text:
    raise SystemExit("missing make_node return anchor")

for label in ["b", "c", "a", "x"]:
    old = f"    let ({label}_identity, mut {label}_node, {label}_address) = make_node({label.upper()}).await;\n"
    new = (
        f"    let ({label}_identity, mut {label}_node, {label}_address, {label}_transport_peer) = "
        f"make_node({label.upper()}).await;\n"
    )
    if old in text:
        text = text.replace(old, new, 1)
    elif new not in text:
        raise SystemExit(f"missing {label} make_node call anchor")

spawn_old = '''fn spawn_peer(mut plan: PeerPlan, order: Arc<Mutex<Vec<&'static str>>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok(event) = plan.node.next_event().await else { continue };
'''
spawn_new = '''fn spawn_peer(
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
'''
if spawn_old in text:
    text = text.replace(spawn_old, spawn_new, 1)
elif spawn_new not in text:
    raise SystemExit("missing spawn_peer lifecycle anchor")

helper_marker = "fn spawn_peer(\n"
helpers = r'''async fn wait_for_provider_topology(
    label: &str,
    bootstrap_addrs: &[String],
    expected_transport_peers: &[String],
    world: Option<swarm_protocol::WorldId>,
) {
    let identity = PeerIdentity::from_secret_bytes([240; 32]);
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()]).unwrap();
    let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key()).unwrap();
    node.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    for address in bootstrap_addrs {
        node.add_bootstrap_address(address.parse().unwrap()).unwrap();
    }
    let _ = node.bootstrap();

    let expected = expected_transport_peers.iter().cloned().collect::<HashSet<_>>();
    let mut observed = HashSet::new();
    timeout(Duration::from_secs(20), async {
        loop {
            let query = match world {
                Some(world) => node.find_world_providers(world),
                None => node.find_public_providers(),
            };
            loop {
                match node.next_event().await.expect("topology probe discovery event") {
                    DiscoveryNetworkEvent::ProvidersFound { query_id, providers } if query_id == query => {
                        observed.extend(providers.into_iter().map(|peer| peer.to_string()));
                        if expected.is_subset(&observed) {
                            return;
                        }
                    }
                    DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query => break,
                    DiscoveryNetworkEvent::ProvidersFailed { query_id, .. } if query_id == query => break,
                    _ => {}
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "{label} topology did not converge to expected providers; expected={expected:?}, observed={observed:?}"
        )
    });
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

'''
if helpers.strip() not in text:
    marker_index = text.find(helper_marker)
    if marker_index < 0:
        raise SystemExit("missing spawn_peer insertion marker")
    text = text[:marker_index] + helpers + text[marker_index:]

order_old = "    let order = Arc::new(Mutex::new(Vec::new()));\n    let tasks = vec![\n"
order_new = (
    "    let order = Arc::new(Mutex::new(Vec::new()));\n"
    "    let lifecycle = Arc::new(Mutex::new(Vec::<String>::new()));\n"
    "    let mut tasks = vec![\n"
)
if order_old in text:
    text = text.replace(order_old, order_new, 1)
elif order_new not in text:
    raise SystemExit("missing task vector anchor")

spawn_call_old = "        }, order.clone()),\n"
spawn_call_new = "        }, order.clone(), lifecycle.clone()),\n"
count = text.count(spawn_call_old)
if count:
    text = text.replace(spawn_call_old, spawn_call_new)
elif text.count(spawn_call_new) != 4:
    raise SystemExit("missing spawn_peer call-site anchors")

browse_start = '''    tokio::time::sleep(Duration::from_secs(3)).await;
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path());
    let bootstraps = vec![b_address.clone(), c_address.clone(), a_address.clone(), x_address.clone()];
    let report = search_public_worlds(&paths, DiscoverySearchInputV1::default(), &bootstraps).await.unwrap();
    assert_eq!(report.results.len(), 1, "only the live canonical proof may survive browse: {report:?}");
    assert_eq!(report.results[0].announcer_peer_id, b_record_id.peer_id().to_string());
    let browse_order = order.lock().unwrap().clone();
    assert!(browse_order.contains(&"stale"));
    assert!(browse_order.contains(&"attacker"));
    assert!(browse_order.iter().position(|label| *label == "stale") < browse_order.iter().position(|label| *label == "current"));

'''
browse_replacement = '''    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path());
    let bootstraps = vec![b_address.clone(), c_address.clone(), a_address.clone(), x_address.clone()];

    // Drive Kademlia readiness from observed provider sets rather than wall-clock sleeps.
    wait_for_provider_topology(
        "public browse",
        &bootstraps,
        &[b_transport_peer.clone(), a_transport_peer.clone(), x_transport_peer.clone()],
        None,
    )
    .await;
    wait_for_provider_topology(
        "world freshness",
        &bootstraps,
        &[
            b_transport_peer.clone(),
            c_transport_peer.clone(),
            a_transport_peer.clone(),
            x_transport_peer.clone(),
        ],
        Some(world),
    )
    .await;

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
    assert!(browse_order.iter().position(|label| *label == "stale") < browse_order.iter().position(|label| *label == "current"));

'''
if browse_start in text:
    text = text.replace(browse_start, browse_replacement, 1)
elif browse_replacement not in text:
    raise SystemExit("missing browse adversarial-round anchor")

resolve_start = '''    order.lock().unwrap().clear();
    let resolved = resolve_world(&paths, world, &bootstraps).await.unwrap();
    assert_eq!(resolved.state, "found", "current authority should resolve after stale/attacker candidates: {resolved:?}");
    let card = resolved.world.expect("current world card");
    assert_eq!(card.announcer_peer_id, b_record_id.peer_id().to_string());
    let resolve_order = order.lock().unwrap().clone();
    assert!(resolve_order.contains(&"stale"));
    assert!(resolve_order.contains(&"attacker"));
    assert_ne!(resolve_order.first().copied(), Some("current"), "resolver must tolerate a noncanonical first response");

'''
resolve_replacement = '''    wait_for_provider_topology(
        "exact resolve",
        &bootstraps,
        &[
            b_transport_peer.clone(),
            c_transport_peer.clone(),
            a_transport_peer.clone(),
            x_transport_peer.clone(),
        ],
        Some(world),
    )
    .await;
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
    assert_eq!(resolved.state, "found", "current authority should resolve after stale/attacker candidates: {resolved:?}");
    let card = resolved.world.expect("current world card");
    assert_eq!(card.announcer_peer_id, b_record_id.peer_id().to_string());
    assert!(resolve_order.contains(&"stale"), "stale provider must actually participate: {resolve_order:?}");
    assert!(resolve_order.contains(&"attacker"), "malformed attacker must actually participate: {resolve_order:?}");
    assert!(resolve_order.contains(&"current"), "current provider must actually participate: {resolve_order:?}");
    assert!(resolve_order.iter().position(|label| *label == "stale") < resolve_order.iter().position(|label| *label == "current"));
    assert_ne!(resolve_order.first().copied(), Some("current"), "resolver must tolerate a noncanonical first response");

'''
if resolve_start in text:
    text = text.replace(resolve_start, resolve_replacement, 1)
elif resolve_replacement not in text:
    raise SystemExit("missing exact-resolve adversarial-round anchor")

cleanup_old = '''    for task in tasks {
        task.abort();
    }
'''
cleanup_new = '''    for task in tasks {
        task.abort();
        match task.await {
            Err(error) if error.is_cancelled() => {}
            outcome => panic!("provider task did not end by expected test cleanup cancellation: {outcome:?}"),
        }
    }
'''
if cleanup_old in text:
    text = text.replace(cleanup_old, cleanup_new, 1)
elif cleanup_new not in text:
    raise SystemExit("missing provider task cleanup anchor")

path.write_text(text)
print("FINAL-028 network regression uses observed topology readiness and surfaces peer task lifecycle failures")
