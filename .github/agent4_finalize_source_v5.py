from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing Agent 4 locator-resilience anchor in {path}: {old[:180]!r}")
    p.write_text(text.replace(old, new, 1))


# Explicit bootstrap peers are untrusted locators too. Keep them in the bounded
# candidate set so one partial Kademlia provider result or one failed provider
# cannot hide other directly configured viable locators. All announcement,
# membership-proof, and freshness-quorum material remains cryptographically
# verified exactly as before.
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''use swarm_network::{
    generate_transport_key, load_or_create_transport_key, DiscoveryNetworkEvent, DiscoveryNode, WireRequest,
    WireResponse, MAX_DISCOVERY_RESULTS,
};
''',
    '''use swarm_network::{
    generate_transport_key, load_or_create_transport_key, DiscoveryNetworkEvent, DiscoveryNode, TransportPeerId,
    WireRequest, WireResponse, MAX_DISCOVERY_RESULTS,
};
''',
)

replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''async fn prove_candidate_freshness(
    node: &mut DiscoveryNode,
    verifier: &PeerIdentity,
    announcement: &WorldAnnouncementV1,
) -> Result<bool> {
''',
    '''async fn prove_candidate_freshness(
    node: &mut DiscoveryNode,
    verifier: &PeerIdentity,
    announcement: &WorldAnnouncementV1,
    locator_peers: &HashSet<TransportPeerId>,
) -> Result<bool> {
''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''    let mut providers = HashSet::new();
    let mut applications = HashMap::new();
    let mut context_requested = HashSet::new();
''',
    '''    let mut providers = locator_peers.clone();
    let mut applications = HashMap::new();
    let mut context_requested = HashSet::new();
''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''    let mut votes = Vec::<DiscoveryFreshnessVoteV1>::new();
    let mut replay = DiscoveryFreshnessReplayGuard::default();

    let run = timeout(DISCOVERY_FRESHNESS_TIMEOUT, async {
''',
    '''    let mut votes = Vec::<DiscoveryFreshnessVoteV1>::new();
    let mut replay = DiscoveryFreshnessReplayGuard::default();

    // Reuse already-authenticated explicit locators from browse/resolve while
    // still accepting additional DHT-discovered world providers. Locator
    // identity never grants authority; proof and quorum verification below do.
    for transport_peer in providers.iter().copied().collect::<Vec<_>>() {
        let _ = node.dial_peer(transport_peer);
        if let Some(application_peer) = node.application_peer(&transport_peer) {
            applications.insert(transport_peer, application_peer);
        }
    }
    for (transport_peer, application_peer) in applications.clone() {
        if application_peer == announcement.announcer_peer_id && context_requested.insert(transport_peer) {
            node.send_request(
                &transport_peer,
                WireRequest::DiscoveryFreshnessContext {
                    world_id: announcement.world_id,
                    announcement_hash,
                    verifier_peer_id: verifier.peer_id(),
                    nonce,
                    issued_unix_ms,
                    expires_unix_ms,
                },
            )?;
        }
    }

    let run = timeout(DISCOVERY_FRESHNESS_TIMEOUT, async {
''',
)

replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''    add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_public_providers();

    let mut providers = HashSet::new();
''',
    '''    let bootstrap_peers = add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_public_providers();

    // Explicit bootstrap nodes are bounded untrusted locator candidates. DHT
    // provider discovery augments this set; neither source conveys authority.
    let mut providers = bootstrap_peers.clone();
''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''        match prove_candidate_freshness(&mut node, &identity, &candidate).await {
''',
    '''        match prove_candidate_freshness(&mut node, &identity, &candidate, &bootstrap_peers).await {
''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''    add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_world_providers(world);
    let mut providers = HashSet::new();
''',
    '''    let bootstrap_peers = add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_world_providers(world);
    // Exact resolve also treats explicit bootstraps only as untrusted locators.
    let mut providers = bootstrap_peers.clone();
''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''        match prove_candidate_freshness(&mut node, &identity, &candidate).await {
''',
    '''        match prove_candidate_freshness(&mut node, &identity, &candidate, &bootstrap_peers).await {
''',
)

replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''fn add_explicit_bootstraps(node: &mut DiscoveryNode, values: &[String]) -> Result<()> {
    let mut any = false;
    for value in values.iter().map(|value| value.trim()).filter(|value| !value.is_empty()) {
        node.add_bootstrap_address(
            value.parse().with_context(|| format!("invalid discovery bootstrap address: {value}"))?,
        )?;
        any = true;
    }
    if any {
        node.bootstrap()?;
    }
    Ok(())
}
''',
    '''fn add_explicit_bootstraps(node: &mut DiscoveryNode, values: &[String]) -> Result<HashSet<TransportPeerId>> {
    let mut peers = HashSet::new();
    for value in values.iter().map(|value| value.trim()).filter(|value| !value.is_empty()) {
        let peer = node.add_bootstrap_address(
            value.parse().with_context(|| format!("invalid discovery bootstrap address: {value}"))?,
        )?;
        peers.insert(peer);
    }
    if !peers.is_empty() {
        node.bootstrap()?;
    }
    Ok(peers)
}
''',
)

# Tighten the live topology probe: a qualifying readiness observation must see
# the whole expected provider set in one Kademlia query, and every expected
# provider must have completed application authentication on the probe node.
test_path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
test_text = test_path.read_text()
old_helper = r'''async fn wait_for_provider_topology(
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
'''
new_helper = r'''async fn wait_for_provider_topology(
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
    let mut authenticated = HashSet::new();
    let mut last_observed = HashSet::new();
    timeout(Duration::from_secs(20), async {
        loop {
            let query = match world {
                Some(world) => node.find_world_providers(world),
                None => node.find_public_providers(),
            };
            let mut observed_this_query = HashSet::new();
            loop {
                match node.next_event().await.expect("topology probe discovery event") {
                    DiscoveryNetworkEvent::Authenticated { transport_peer, .. } => {
                        let peer = transport_peer.to_string();
                        if expected.contains(&peer) {
                            authenticated.insert(peer);
                        }
                    }
                    DiscoveryNetworkEvent::ProvidersFound { query_id, providers } if query_id == query => {
                        observed_this_query.extend(providers.into_iter().map(|peer| peer.to_string()));
                        if expected.is_subset(&observed_this_query) && expected.is_subset(&authenticated) {
                            return;
                        }
                    }
                    DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query => {
                        if expected.is_subset(&observed_this_query) && expected.is_subset(&authenticated) {
                            return;
                        }
                        break;
                    }
                    DiscoveryNetworkEvent::ProvidersFailed { query_id, .. } if query_id == query => break,
                    _ => {}
                }
            }
            last_observed = observed_this_query;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "{label} topology did not converge in one provider query; expected={expected:?}, observed={last_observed:?}, authenticated={authenticated:?}"
        )
    });
}
'''
if old_helper in test_text:
    test_text = test_text.replace(old_helper, new_helper, 1)
elif new_helper not in test_text:
    raise SystemExit("missing topology helper anchor")

browse_health_anchor = '''    )
    .await;

    order.lock().unwrap().clear();
    let browse_result = search_public_worlds'''
browse_health_replacement = '''    )
    .await;
    assert_peer_tasks_healthy(&mut tasks, &lifecycle, "browse topology readiness").await;

    order.lock().unwrap().clear();
    let browse_result = search_public_worlds'''
if browse_health_anchor in test_text:
    test_text = test_text.replace(browse_health_anchor, browse_health_replacement, 1)
elif browse_health_replacement not in test_text:
    raise SystemExit("missing browse topology-health anchor")

resolve_health_anchor = '''    )
    .await;
    order.lock().unwrap().clear();
    let resolved_result = resolve_world'''
resolve_health_replacement = '''    )
    .await;
    assert_peer_tasks_healthy(&mut tasks, &lifecycle, "resolve topology readiness").await;
    order.lock().unwrap().clear();
    let resolved_result = resolve_world'''
if resolve_health_anchor in test_text:
    test_text = test_text.replace(resolve_health_anchor, resolve_health_replacement, 1)
elif resolve_health_replacement not in test_text:
    raise SystemExit("missing resolve topology-health anchor")

test_path.write_text(test_text)
print("FINAL-028 explicit locators remain available across partial DHT/provider failures; topology readiness is single-query + authenticated")
