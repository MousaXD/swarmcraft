from pathlib import Path


path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
text = path.read_text()

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
browse_replacement = '''    tokio::time::sleep(Duration::from_secs(3)).await;
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path());
    let bootstraps = vec![b_address.clone(), c_address.clone(), a_address.clone(), x_address.clone()];

    // Kademlia provider publication is asynchronous. Do not mistake a partially
    // converged provider table for a freshness-security result. A qualifying
    // round must actually exercise stale + malformed attacker + current peers,
    // and it must still select only the current fresh-quorum announcement.
    let mut browse_report = None;
    let mut browse_order = Vec::new();
    for _ in 0..3 {
        order.lock().unwrap().clear();
        let report = search_public_worlds(&paths, DiscoverySearchInputV1::default(), &bootstraps).await.unwrap();
        let observed = order.lock().unwrap().clone();
        let all_adversaries_exercised = observed.contains(&"stale")
            && observed.contains(&"attacker")
            && observed.contains(&"current");
        browse_report = Some(report);
        browse_order = observed;
        if all_adversaries_exercised && browse_report.as_ref().is_some_and(|report| report.results.len() == 1) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let report = browse_report.expect("browse attempt");
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
resolve_replacement = '''    let mut resolved_report = None;
    let mut resolve_order = Vec::new();
    for _ in 0..3 {
        order.lock().unwrap().clear();
        let resolved = resolve_world(&paths, world, &bootstraps).await.unwrap();
        let observed = order.lock().unwrap().clone();
        let adversary_first = observed.contains(&"stale")
            && observed.contains(&"attacker")
            && observed.contains(&"current")
            && observed.first().copied() != Some("current");
        let found = resolved.state == "found";
        resolved_report = Some(resolved);
        resolve_order = observed;
        if adversary_first && found {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let resolved = resolved_report.expect("resolve attempt");
    assert_eq!(resolved.state, "found", "current authority should resolve after stale/attacker candidates: {resolved:?}");
    let card = resolved.world.expect("current world card");
    assert_eq!(card.announcer_peer_id, b_record_id.peer_id().to_string());
    assert!(resolve_order.contains(&"stale"), "stale provider must actually participate: {resolve_order:?}");
    assert!(resolve_order.contains(&"attacker"), "malformed attacker must actually participate: {resolve_order:?}");
    assert!(resolve_order.contains(&"current"), "current provider must actually participate: {resolve_order:?}");
    assert_ne!(resolve_order.first().copied(), Some("current"), "resolver must tolerate a noncanonical first response");

'''
if resolve_start in text:
    text = text.replace(resolve_start, resolve_replacement, 1)
elif resolve_replacement not in text:
    raise SystemExit("missing exact-resolve adversarial-round anchor")

path.write_text(text)
print("FINAL-028 network regression waits for fully participating adversarial DHT rounds")
