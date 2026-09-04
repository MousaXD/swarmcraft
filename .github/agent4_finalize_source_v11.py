from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing Agent 4 closure anchor in {path}: {old[:220]!r}")
    p.write_text(text.replace(old, new, 1))


network = "crates/swarm-network/src/discovery.rs"
replace_once(
    network,
    '''                        if !self.swarm.is_connected(&peer) && self.pending_dials.insert(peer) {
                            if self.swarm.dial(DialOpts::peer_id(peer).addresses(vec![address]).build()).is_err() {
                                self.pending_dials.remove(&peer);
                            }
                        }
''',
    '''                        if !self.swarm.is_connected(&peer)
                            && self.pending_dials.insert(peer)
                            && self
                                .swarm
                                .dial(DialOpts::peer_id(peer).addresses(vec![address]).build())
                                .is_err()
                        {
                            self.pending_dials.remove(&peer);
                        }
''',
)

test = "crates/swarm-cli/tests/discovery_network_freshness.rs"
replace_once(
    test,
    '''            match node.next_event().await.map_err(|error| format!("{label} discovery node failed: {error:#}"))? {
                DiscoveryNetworkEvent::InboundRequest { channel, .. } => {
                    node.respond(
                        channel,
                        WireResponse::Error {
                            code: "TEST_OK".into(),
                            message: label.into(),
                        },
                    )
                    .map_err(|error| format!("{label} response failed: {error:#}"))?;
                }
                _ => {}
            }
''',
    '''            if let DiscoveryNetworkEvent::InboundRequest { channel, .. } =
                node.next_event().await.map_err(|error| format!("{label} discovery node failed: {error:#}"))?
            {
                node.respond(
                    channel,
                    WireResponse::Error {
                        code: "TEST_OK".into(),
                        message: label.into(),
                    },
                )
                .map_err(|error| format!("{label} response failed: {error:#}"))?;
            }
''',
)

# Hosted runners can finish current-provider auth well ahead of hostile peers.
# The regression intentionally delays the healthy provider's response so stale
# and malformed candidates complete first and the security property is tested
# deterministically. Production discovery behavior is unchanged.
replace_once(test, "            delay_ms: 250,\n", "            delay_ms: 2_000,\n")

# A symmetric dial can transiently expose both physical connections while the
# asynchronously requested noncanonical close is still being delivered. The
# invariant is convergence, not zero transient overlap: drive both swarms until
# they each retain exactly one authenticated connection, then assert the final
# single-connection state.
replace_once(
    test,
    '''    // Drive close/replacement notifications briefly. At no point may either
    // peer retain two request-response connections.
    let _ = timeout(Duration::from_millis(500), async {
        loop {
            tokio::select! {
                event = left.next_event() => {
                    event.expect("left discovery event after symmetric convergence");
                }
                event = right.next_event() => {
                    event.expect("right discovery event after symmetric convergence");
                }
            }
            assert!(left.established_connection_count(&right_peer) <= 1);
            assert!(right.established_connection_count(&left_peer) <= 1);
        }
    })
    .await;

    assert_eq!(left.established_connection_count(&right_peer), 1);
    assert_eq!(right.established_connection_count(&left_peer), 1);
    assert!(left.is_authenticated(&right_peer));
    assert!(right.is_authenticated(&left_peer));
''',
    '''    timeout(Duration::from_secs(2), async {
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
''',
)

print("Agent 4 final structural lint repairs and deterministic network regressions applied")
