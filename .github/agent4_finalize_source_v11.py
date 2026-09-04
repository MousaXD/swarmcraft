from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing Agent 4 clippy closure anchor in {path}: {old[:220]!r}")
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

print("Agent 4 final clippy-only collapsible-if repair applied without semantic change")
