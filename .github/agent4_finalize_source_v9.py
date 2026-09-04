from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing Agent 4 ordered-locator anchor in {path}: {old[:260]!r}")
    p.write_text(text.replace(old, new, 1))


cli = "crates/swarm-cli/src/discovery.rs"

replace_once(
    cli,
    '''    let bootstrap_peers = add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_public_providers();

    // Explicit bootstrap nodes are bounded untrusted locator candidates. DHT
    // provider discovery augments this set; neither source conveys authority.
    let mut providers = bootstrap_peers.clone();
    let mut requested = HashSet::new();
    let mut pending = HashSet::new();
''',
    '''    // Warm the actual browse node's direct locators before starting a DHT
    // query. This avoids racing bootstrap/query-created connections against the
    // explicit transport dials and preserves caller order for adversarial first
    // contact. Locator order and identity still grant zero authority.
    let bootstrap_order = add_explicit_locator_addresses(&mut node, bootstrap_addrs)?;
    warm_explicit_locators(&mut node, &bootstrap_order).await?;
    let bootstrap_peers = bootstrap_order.iter().copied().collect::<HashSet<_>>();

    // Explicit bootstrap nodes are bounded untrusted locator candidates. DHT
    // provider discovery augments this set; neither source conveys authority.
    let mut providers = bootstrap_peers.clone();
    let mut requested = HashSet::new();
    let mut pending = HashSet::new();
    for transport_peer in bootstrap_order.iter().copied() {
        if node.application_peer(&transport_peer).is_some() && requested.insert(transport_peer) {
            let request_id = node.send_request(
                &transport_peer,
                WireRequest::DiscoveryPublic { filter: filter.clone() },
            )?;
            pending.insert(format!("{request_id:?}"));
        }
    }
    if !bootstrap_order.is_empty() {
        node.bootstrap()?;
    }
    let query = node.find_public_providers();
''',
)

replace_once(
    cli,
    '''    let bootstrap_peers = add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_world_providers(world);
    // Exact resolve also treats explicit bootstraps only as untrusted locators.
    let mut providers = bootstrap_peers.clone();
    let mut requested = HashSet::new();
''',
    '''    // Exact resolve uses the same actual-node readiness discipline as
    // browse. Direct locators are contacted in caller order before DHT
    // augmentation, but every returned candidate still needs the canonical
    // proof and fresh Agent 1 quorum below.
    let bootstrap_order = add_explicit_locator_addresses(&mut node, bootstrap_addrs)?;
    warm_explicit_locators(&mut node, &bootstrap_order).await?;
    let bootstrap_peers = bootstrap_order.iter().copied().collect::<HashSet<_>>();
    // Exact resolve also treats explicit bootstraps only as untrusted locators.
    let mut providers = bootstrap_peers.clone();
    let mut requested = HashSet::new();
    for transport_peer in bootstrap_order.iter().copied() {
        if node.application_peer(&transport_peer).is_some() && requested.insert(transport_peer) {
            node.send_request(&transport_peer, WireRequest::DiscoveryResolve { world_id: world })?;
        }
    }
    if !bootstrap_order.is_empty() {
        node.bootstrap()?;
    }
    let query = node.find_world_providers(world);
''',
)

old_helper = '''fn add_explicit_bootstraps(node: &mut DiscoveryNode, values: &[String]) -> Result<HashSet<TransportPeerId>> {
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
'''
new_helper = '''fn add_explicit_locator_addresses(node: &mut DiscoveryNode, values: &[String]) -> Result<Vec<TransportPeerId>> {
    let mut peers = Vec::new();
    let mut seen = HashSet::new();
    for value in values.iter().map(|value| value.trim()).filter(|value| !value.is_empty()) {
        let peer = node.add_bootstrap_address(
            value.parse().with_context(|| format!("invalid discovery bootstrap address: {value}"))?,
        )?;
        if seen.insert(peer) {
            peers.push(peer);
        }
    }
    Ok(peers)
}

async fn warm_explicit_locators(node: &mut DiscoveryNode, peers: &[TransportPeerId]) -> Result<()> {
    if peers.is_empty() {
        return Ok(());
    }
    let expected = peers.iter().copied().collect::<HashSet<_>>();
    let warmup = timeout(DISCOVERY_QUERY_TIMEOUT, async {
        loop {
            if expected.iter().all(|peer| node.application_peer(peer).is_some()) {
                return Ok::<(), anyhow::Error>(());
            }
            match node.next_event().await? {
                DiscoveryNetworkEvent::Disconnected { transport_peer, .. } if expected.contains(&transport_peer) => {
                    let _ = node.dial_peer(transport_peer);
                }
                _ => {}
            }
        }
    })
    .await;
    if let Ok(result) = warmup {
        result?;
    }
    Ok(())
}

fn add_explicit_bootstraps(node: &mut DiscoveryNode, values: &[String]) -> Result<HashSet<TransportPeerId>> {
    let peers = add_explicit_locator_addresses(node, values)?;
    if !peers.is_empty() {
        node.bootstrap()?;
    }
    Ok(peers.into_iter().collect())
}
'''
replace_once(cli, old_helper, new_helper)

# Remove the two strict-clippy warnings in the permanent lifecycle regression.
test_path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
text = test_path.read_text()
text = text.replace(
    '    let mut closing_task = spawn_echo_peer(closing_node, "closing-provider");\n',
    '    let closing_task = spawn_echo_peer(closing_node, "closing-provider");\n',
)
text = text.replace(
    '    let mut surviving_task = spawn_echo_peer(surviving_node, "surviving-provider");\n',
    '    let surviving_task = spawn_echo_peer(surviving_node, "surviving-provider");\n',
)
test_path.write_text(text)

print("FINAL-028 actual browse/resolve node warms ordered explicit locators before DHT augmentation")
