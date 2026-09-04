from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing Agent 4 canonical-connection anchor in {path}: {old[:240]!r}")
    p.write_text(text.replace(old, new, 1))


network = Path("crates/swarm-network/src/discovery.rs")
text = network.read_text()

replace_once(
    str(network),
    '''    established_connections: HashMap<TransportPeerId, HashSet<ConnectionId>>,
    connection_counts: HashMap<TransportPeerId, usize>,
    pending_dials: HashSet<TransportPeerId>,
''',
    '''    established_connections: HashMap<TransportPeerId, HashSet<ConnectionId>>,
    connection_directions: HashMap<TransportPeerId, HashMap<ConnectionId, bool>>,
    connection_counts: HashMap<TransportPeerId, usize>,
    pending_dials: HashSet<TransportPeerId>,
''',
)
replace_once(
    str(network),
    '''            established_connections: HashMap::new(),
            connection_counts: HashMap::new(),
            pending_dials: HashSet::new(),
''',
    '''            established_connections: HashMap::new(),
            connection_directions: HashMap::new(),
            connection_counts: HashMap::new(),
            pending_dials: HashSet::new(),
''',
)

# Make every explicit/provider dial attempt observable and single-flight.
replace_once(
    str(network),
    '''    pub fn dial_peer(&mut self, peer: TransportPeerId) -> Result<()> {
        if self.swarm.is_connected(&peer) || !self.pending_dials.insert(peer) {
            return Ok(());
        }
        if let Err(error) = self.swarm.dial(DialOpts::peer_id(peer).build()) {
            self.pending_dials.remove(&peer);
            return Err(error).context("failed to dial discovery provider");
        }
        Ok(())
    }
''',
    '''    pub fn dial_peer(&mut self, peer: TransportPeerId) -> Result<()> {
        if self.swarm.is_connected(&peer) {
            debug!(transport_peer = %peer, "discovery dial suppressed; live connection already exists");
            return Ok(());
        }
        if !self.pending_dials.insert(peer) {
            debug!(transport_peer = %peer, "discovery dial suppressed; outbound dial already pending");
            return Ok(());
        }
        debug!(transport_peer = %peer, "discovery outbound provider dial attempt");
        if let Err(error) = self.swarm.dial(DialOpts::peer_id(peer).build()) {
            self.pending_dials.remove(&peer);
            return Err(error).context("failed to dial discovery provider");
        }
        Ok(())
    }
''',
)

# Replace the two connection lifecycle arms as one unit.  The deterministic
# rule is shared by both peers: the lexicographically smaller transport PeerId
# keeps the dialer side, and the larger PeerId keeps the listener side.  Thus
# simultaneous A->B and B->A connections cannot make A and B preserve opposite
# physical connections. Authentication remains bound to the chosen ConnectionId.
text = network.read_text()
start = text.index("                SwarmEvent::ConnectionEstablished {")
end = text.index("                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Mdns", start)
new_lifecycle = r'''                SwarmEvent::ConnectionEstablished {
                    peer_id,
                    connection_id,
                    endpoint,
                    num_established,
                    ..
                } => {
                    self.pending_dials.remove(&peer_id);
                    let connection_is_dialer = endpoint.is_dialer();
                    let prefer_dialer = *self.swarm.local_peer_id() < peer_id;
                    self.connection_directions
                        .entry(peer_id)
                        .or_default()
                        .insert(connection_id, connection_is_dialer);
                    let established_count = {
                        let established = self.established_connections.entry(peer_id).or_default();
                        established.insert(connection_id);
                        established.len()
                    };
                    self.connection_counts.insert(peer_id, established_count);
                    debug_assert_eq!(established_count, num_established.get() as usize);
                    debug!(
                        transport_peer = %peer_id,
                        %connection_id,
                        connection_is_dialer,
                        prefer_dialer,
                        tracked_established = established_count,
                        reported_established = num_established.get(),
                        active_connection = ?self.active_connections.get(&peer_id),
                        "discovery connection established"
                    );

                    if !application_connection_allowed(
                        self.active_connections.len(),
                        self.active_connections.contains_key(&peer_id),
                    ) {
                        warn!(transport_peer = %peer_id, %connection_id, "discovery connection admission limit reached");
                        let _ = self.swarm.close_connection(connection_id);
                        continue;
                    }

                    if let Some(active) = self.active_connections.get(&peer_id).copied() {
                        let active_is_live = self
                            .established_connections
                            .get(&peer_id)
                            .is_some_and(|connections| connections.contains(&active));
                        if active != connection_id && active_is_live {
                            let active_is_dialer = self
                                .connection_directions
                                .get(&peer_id)
                                .and_then(|connections| connections.get(&active))
                                .copied()
                                .unwrap_or(false);
                            let active_is_preferred = active_is_dialer == prefer_dialer;
                            let newcomer_is_preferred = connection_is_dialer == prefer_dialer;

                            if newcomer_is_preferred && !active_is_preferred {
                                debug!(
                                    transport_peer = %peer_id,
                                    old_connection = %active,
                                    new_connection = %connection_id,
                                    old_is_dialer = active_is_dialer,
                                    new_is_dialer = connection_is_dialer,
                                    "switching discovery peer to deterministic canonical connection"
                                );
                                if self
                                    .authenticated
                                    .get(&peer_id)
                                    .is_some_and(|(_, authenticated_connection)| *authenticated_connection == active)
                                {
                                    self.authenticated.remove(&peer_id);
                                }
                                if self
                                    .pending_challenges
                                    .get(&peer_id)
                                    .is_some_and(|(challenge_connection, _, _)| *challenge_connection == active)
                                {
                                    self.pending_challenges.remove(&peer_id);
                                }
                                self.active_connections.insert(peer_id, connection_id);
                                // Defer the new application challenge until the old physical
                                // connection has closed and request-response sees one connection.
                                let _ = self.swarm.close_connection(active);
                            } else {
                                debug!(
                                    transport_peer = %peer_id,
                                    active_connection = %active,
                                    duplicate_connection = %connection_id,
                                    active_is_dialer,
                                    duplicate_is_dialer = connection_is_dialer,
                                    "closing noncanonical duplicate discovery connection"
                                );
                                let _ = self.swarm.close_connection(connection_id);
                            }
                            continue;
                        }
                    }

                    self.active_connections.insert(peer_id, connection_id);
                    if established_count == 1 {
                        self.ensure_auth_challenge(peer_id, connection_id)?;
                    }
                }
                SwarmEvent::ConnectionClosed {
                    peer_id,
                    connection_id,
                    num_established,
                    ..
                } => {
                    let established_count = {
                        let established = self.established_connections.entry(peer_id).or_default();
                        established.remove(&connection_id);
                        established.len()
                    };
                    if let Some(directions) = self.connection_directions.get_mut(&peer_id) {
                        directions.remove(&connection_id);
                        if directions.is_empty() {
                            self.connection_directions.remove(&peer_id);
                        }
                    }
                    self.connection_counts.insert(peer_id, established_count);
                    debug_assert_eq!(established_count, num_established as usize);
                    debug!(
                        transport_peer = %peer_id,
                        %connection_id,
                        tracked_remaining = established_count,
                        remaining_established = num_established,
                        active_connection = ?self.active_connections.get(&peer_id),
                        another_connection_live = established_count > 0,
                        "discovery connection closed"
                    );

                    let closing_application_peer = self
                        .authenticated
                        .get(&peer_id)
                        .filter(|(_, authenticated_connection)| *authenticated_connection == connection_id)
                        .map(|(peer, _)| *peer);
                    if closing_application_peer.is_some() {
                        self.authenticated.remove(&peer_id);
                    }
                    if self
                        .pending_challenges
                        .get(&peer_id)
                        .is_some_and(|(challenge_connection, _, _)| *challenge_connection == connection_id)
                    {
                        self.pending_challenges.remove(&peer_id);
                    }

                    if established_count == 0 {
                        self.established_connections.remove(&peer_id);
                        self.connection_directions.remove(&peer_id);
                        self.active_connections.remove(&peer_id);
                        self.connection_counts.remove(&peer_id);
                        self.pending_dials.remove(&peer_id);
                        let application_peer = self.authenticated.remove(&peer_id).map(|(peer, _)| peer);
                        self.pending_challenges.remove(&peer_id);
                        self.admission.forget_peer(peer_id);
                        return Ok(DiscoveryNetworkEvent::Disconnected {
                            transport_peer: peer_id,
                            application_peer: application_peer.or(closing_application_peer),
                        });
                    }

                    let active_is_live = self
                        .active_connections
                        .get(&peer_id)
                        .is_some_and(|active| {
                            self.established_connections
                                .get(&peer_id)
                                .is_some_and(|connections| connections.contains(active))
                        });
                    if !active_is_live {
                        let prefer_dialer = *self.swarm.local_peer_id() < peer_id;
                        let survivor = self
                            .established_connections
                            .get(&peer_id)
                            .and_then(|connections| {
                                connections
                                    .iter()
                                    .copied()
                                    .find(|candidate| {
                                        self.connection_directions
                                            .get(&peer_id)
                                            .and_then(|directions| directions.get(candidate))
                                            .is_some_and(|is_dialer| *is_dialer == prefer_dialer)
                                    })
                                    .or_else(|| connections.iter().copied().next())
                            })
                            .expect("nonempty established discovery connection set");
                        self.active_connections.insert(peer_id, survivor);
                    }

                    // The request-response behaviour must see exactly one physical
                    // connection before connection-bound application auth resumes.
                    if established_count == 1 {
                        if let Some(active) = self.active_connections.get(&peer_id).copied() {
                            self.ensure_auth_challenge(peer_id, active)?;
                        }
                    }
                }
'''
text = text[:start] + new_lifecycle + text[end:]
network.write_text(text)

# Add a permanent symmetric-dial regression. It deliberately starts outbound
# dials from both peers before either event loop is driven. Both peers must
# converge on one physical connection and authenticate it without task death.
test_path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
test = test_path.read_text()
regression = r'''

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

    // Drive close/replacement notifications briefly. At no point may either
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
}
'''
if "simultaneous_bidirectional_dials_converge_on_one_authenticated_connection" not in test:
    test += regression
    test_path.write_text(test)

print("FINAL-028 duplicate discovery connections now converge by deterministic transport direction; symmetric-dial regression added")
