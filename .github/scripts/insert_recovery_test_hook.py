from pathlib import Path

path = Path("crates/swarm-network/src/node.rs")
text = path.read_text()
old = '''                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    self.authenticated.remove(&peer_id);
                    return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });
                }
'''
new = '''                SwarmEvent::ConnectionClosed { peer_id, num_established, .. } => {
                    // A peer can have multiple libp2p connections at once, especially
                    // during reconnects. Closing an older connection must not erase
                    // authentication established by a newer live connection.
                    if num_established == 0 {
                        self.authenticated.remove(&peer_id);
                        return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });
                    }
                    debug!(transport_peer = %peer_id, remaining_connections = num_established, "peer connection closed; keeping authentication for remaining connection");
                }
'''
if old in text:
    path.write_text(text.replace(old, new, 1))
elif new not in text:
    raise SystemExit("ConnectionClosed block not found")
