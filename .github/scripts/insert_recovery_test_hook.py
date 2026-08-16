from pathlib import Path

path = Path("crates/swarm-network/src/node.rs")
text = path.read_text()

text = text.replace(
    '    swarm::{dial_opts::DialOpts, NetworkBehaviour, SwarmEvent},\n',
    '    swarm::{dial_opts::DialOpts, ConnectionId, NetworkBehaviour, SwarmEvent},\n',
    1,
)

old_field = '''    authenticated: HashMap<TransportPeerId, PeerId>,
    diagnostics: ConnectivityDiagnosticsV1,
'''
new_field = '''    authenticated: HashMap<TransportPeerId, PeerId>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    diagnostics: ConnectivityDiagnosticsV1,
'''
if old_field in text:
    text = text.replace(old_field, new_field, 1)
elif new_field not in text:
    raise SystemExit("SwarmNode fields anchor not found")

old_init = '''            local_hello,
            authenticated: HashMap::new(),
            diagnostics: ConnectivityDiagnosticsV1::default(),
'''
new_init = '''            local_hello,
            authenticated: HashMap::new(),
            active_connections: HashMap::new(),
            diagnostics: ConnectivityDiagnosticsV1::default(),
'''
if old_init in text:
    text = text.replace(old_init, new_init, 1)
elif new_init not in text:
    raise SystemExit("SwarmNode init anchor not found")

old_events = '''                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                    let endpoint_debug = format!("{endpoint:?}");
                    if endpoint_debug.contains("p2p-circuit") || endpoint_debug.contains("P2pCircuit") {
                        self.diagnostics.relay_connectivity = true;
                    } else {
                        self.diagnostics.record_direct_success();
                    }
                    debug!(transport_peer = %peer_id, "peer connected");
                    self.swarm
                        .behaviour_mut()
                        .request_response
                        .send_request(&peer_id, WireRequest::Hello(self.local_hello.clone()));
                    return Ok(NetworkEvent::Connected { transport_peer: peer_id });
                }
                SwarmEvent::ConnectionClosed { peer_id, num_established, .. } => {
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
new_events = '''                SwarmEvent::ConnectionEstablished { peer_id, connection_id, endpoint, num_established, .. } => {
                    let endpoint_debug = format!("{endpoint:?}");
                    if endpoint_debug.contains("p2p-circuit") || endpoint_debug.contains("P2pCircuit") {
                        self.diagnostics.relay_connectivity = true;
                    } else {
                        self.diagnostics.record_direct_success();
                    }
                    debug!(transport_peer = %peer_id, %connection_id, %num_established, "peer connected");

                    // request-response chooses a connection by peer ID, not by connection
                    // ID. After a hard peer restart libp2p can briefly retain the dead
                    // connection while the replacement is already established. Sending
                    // PeerHello immediately can therefore land on the dead connection and
                    // leave the replacement waiting forever for authentication. Make the
                    // newest connection canonical, close the superseded connection, and
                    // send Hello after that close is observed so request-response has only
                    // the live route left.
                    let previous = self.active_connections.insert(peer_id, connection_id);
                    let defer_hello = previous
                        .filter(|previous| *previous != connection_id && num_established > 1)
                        .is_some_and(|previous| self.swarm.close_connection(previous));
                    if !defer_hello {
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_request(&peer_id, WireRequest::Hello(self.local_hello.clone()));
                    }
                    return Ok(NetworkEvent::Connected { transport_peer: peer_id });
                }
                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    // A peer can have multiple libp2p connections at once, especially
                    // during reconnects. Closing an older connection must not erase
                    // authentication established by a newer live connection.
                    if num_established == 0 {
                        self.active_connections.remove(&peer_id);
                        self.authenticated.remove(&peer_id);
                        return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });
                    }

                    if self.active_connections.get(&peer_id).is_some_and(|active| *active != connection_id) {
                        // The superseded connection is gone. Re-send the signed hello now
                        // that request-response can route it over the replacement.
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_request(&peer_id, WireRequest::Hello(self.local_hello.clone()));
                    }
                    debug!(transport_peer = %peer_id, %connection_id, remaining_connections = num_established, "peer connection closed; keeping authentication for remaining connection");
                }
'''
if old_events in text:
    text = text.replace(old_events, new_events, 1)
elif new_events not in text:
    raise SystemExit("connection event block not found")

path.write_text(text)
