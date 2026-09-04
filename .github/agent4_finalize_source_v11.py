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

print("Agent 4 final structural lint repairs and deterministic hostile-first fixture applied")
