from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing Agent 4 lifecycle-repair anchor in {path}: {old[:220]!r}")
    p.write_text(text.replace(old, new, 1))


network = "crates/swarm-network/src/discovery.rs"

replace_once(
    network,
    '''use std::{
    collections::HashMap,
    env,
    time::{Duration, Instant},
};
''',
    '''use std::{
    collections::{HashMap, HashSet},
    env,
    time::{Duration, Instant},
};
''',
)

replace_once(
    network,
    '''    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32], Instant)>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    connection_counts: HashMap<TransportPeerId, usize>,
    admission: AdmissionController,
''',
    '''    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32], Instant)>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    established_connections: HashMap<TransportPeerId, HashSet<ConnectionId>>,
    connection_counts: HashMap<TransportPeerId, usize>,
    pending_dials: HashSet<TransportPeerId>,
    admission: AdmissionController,
''',
)

replace_once(
    network,
    '''            pending_challenges: HashMap::new(),
            active_connections: HashMap::new(),
            connection_counts: HashMap::new(),
            admission: AdmissionController::new(),
''',
    '''            pending_challenges: HashMap::new(),
            active_connections: HashMap::new(),
            established_connections: HashMap::new(),
            connection_counts: HashMap::new(),
            pending_dials: HashSet::new(),
            admission: AdmissionController::new(),
''',
)

replace_once(
    network,
    '''    pub fn add_bootstrap_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let peer = transport_peer_from_address(&address)
            .ok_or_else(|| anyhow!("bootstrap address must contain /p2p/<peer-id>"))?;
        self.add_peer_address(peer, address.clone());
        if !self.swarm.is_connected(&peer) {
            self.swarm
                .dial(DialOpts::peer_id(peer).addresses(vec![address]).build())
                .context("failed to dial discovery bootstrap peer")?;
        }
        Ok(peer)
    }
''',
    '''    pub fn add_bootstrap_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let peer = transport_peer_from_address(&address)
            .ok_or_else(|| anyhow!("bootstrap address must contain /p2p/<peer-id>"))?;
        self.add_peer_address(peer, address.clone());
        if !self.swarm.is_connected(&peer) && self.pending_dials.insert(peer) {
            if let Err(error) = self.swarm.dial(DialOpts::peer_id(peer).addresses(vec![address]).build()) {
                self.pending_dials.remove(&peer);
                return Err(error).context("failed to dial discovery bootstrap peer");
            }
        }
        Ok(peer)
    }
''',
)

replace_once(
    network,
    '''    pub fn dial_peer(&mut self, peer: TransportPeerId) -> Result<()> {
        if !self.swarm.is_connected(&peer) {
            self.swarm.dial(DialOpts::peer_id(peer).build()).context("failed to dial discovery provider")?;
        }
        Ok(())
    }
''',
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
)

old_established = '''                SwarmEvent::ConnectionEstablished { peer_id, connection_id, num_established, .. } => {
                    if !application_connection_allowed(
                        self.active_connections.len(),
                        self.active_connections.contains_key(&peer_id),
                    ) {
                        warn!(transport_peer = %peer_id, %connection_id, "discovery connection admission limit reached");
                        let _ = self.swarm.close_connection(connection_id);
                        continue;
                    }
                    self.connection_counts.insert(peer_id, num_established.get() as usize);
                    self.authenticated.remove(&peer_id);
                    self.pending_challenges.remove(&peer_id);
                    let previous = self.active_connections.insert(peer_id, connection_id);
                    let defer_challenge = previous
                        .filter(|previous| *previous != connection_id && num_established.get() > 1)
                        .is_some_and(|previous| self.swarm.close_connection(previous));
                    if !defer_challenge {
                        self.issue_auth_challenge(peer_id, connection_id)?;
                    }
                }
'''
new_established = '''                SwarmEvent::ConnectionEstablished { peer_id, connection_id, num_established, .. } => {
                    self.pending_dials.remove(&peer_id);
                    let established = self.established_connections.entry(peer_id).or_default();
                    established.insert(connection_id);
                    self.connection_counts.insert(peer_id, established.len());
                    debug_assert_eq!(established.len(), num_established.get() as usize);

                    if !application_connection_allowed(
                        self.active_connections.len(),
                        self.active_connections.contains_key(&peer_id),
                    ) {
                        warn!(transport_peer = %peer_id, %connection_id, "discovery connection admission limit reached");
                        let _ = self.swarm.close_connection(connection_id);
                        continue;
                    }

                    if let Some(active) = self.active_connections.get(&peer_id).copied() {
                        if active != connection_id && established.contains(&active) {
                            // Discovery intentionally has one active application connection per
                            // transport peer. A simultaneous transport connection is redundant:
                            // preserve the healthy connection-bound auth state and close only the
                            // newcomer instead of making the newest connection implicitly win.
                            debug!(
                                transport_peer = %peer_id,
                                active_connection = %active,
                                duplicate_connection = %connection_id,
                                "closing redundant discovery connection while preserving active authentication"
                            );
                            let _ = self.swarm.close_connection(connection_id);
                            continue;
                        }
                    }

                    self.active_connections.insert(peer_id, connection_id);
                    self.ensure_auth_challenge(peer_id, connection_id)?;
                }
'''
replace_once(network, old_established, new_established)

old_closed = '''                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    self.connection_counts.insert(peer_id, num_established as usize);
                    if self
                        .authenticated
                        .get(&peer_id)
                        .is_some_and(|(_, authenticated_connection)| *authenticated_connection == connection_id)
                    {
                        self.authenticated.remove(&peer_id);
                    }
                    if self
                        .pending_challenges
                        .get(&peer_id)
                        .is_some_and(|(challenge_connection, _, _)| *challenge_connection == connection_id)
                    {
                        self.pending_challenges.remove(&peer_id);
                    }
                    if num_established == 0 {
                        self.active_connections.remove(&peer_id);
                        self.connection_counts.remove(&peer_id);
                        let application_peer = self.authenticated.remove(&peer_id).map(|(peer, _)| peer);
                        self.pending_challenges.remove(&peer_id);
                        self.admission.forget_peer(peer_id);
                        return Ok(DiscoveryNetworkEvent::Disconnected { transport_peer: peer_id, application_peer });
                    }
                    if let Some(active) =
                        self.active_connections.get(&peer_id).copied().filter(|active| *active != connection_id)
                    {
                        self.issue_auth_challenge(peer_id, active)?;
                    }
                }
'''
new_closed = '''                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    let established = self.established_connections.entry(peer_id).or_default();
                    established.remove(&connection_id);
                    self.connection_counts.insert(peer_id, established.len());
                    debug_assert_eq!(established.len(), num_established as usize);

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

                    if established.is_empty() {
                        self.established_connections.remove(&peer_id);
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

                    let active_closed = self.active_connections.get(&peer_id).copied() == Some(connection_id);
                    let active_is_live = self
                        .active_connections
                        .get(&peer_id)
                        .is_some_and(|active| established.contains(active));
                    if active_closed || !active_is_live {
                        let survivor = *established.iter().next().expect("nonempty established connection set");
                        self.active_connections.insert(peer_id, survivor);
                        self.ensure_auth_challenge(peer_id, survivor)?;
                    }
                }
'''
replace_once(network, old_closed, new_closed)

replace_once(
    network,
    '''                    for (peer, address) in peers {
                        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
                        if !self.swarm.is_connected(&peer) {
                            let _ = self.swarm.dial(DialOpts::peer_id(peer).addresses(vec![address]).build());
                        }
                    }
''',
    '''                    for (peer, address) in peers {
                        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
                        if !self.swarm.is_connected(&peer) && self.pending_dials.insert(peer) {
                            if self.swarm.dial(DialOpts::peer_id(peer).addresses(vec![address]).build()).is_err() {
                                self.pending_dials.remove(&peer);
                            }
                        }
                    }
''',
)

replace_once(
    network,
    '''                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    debug!(transport_peer = ?peer_id, %error, "discovery outgoing connection failed");
                }
                other => debug!(event = ?other, "discovery network event"),
''',
    '''                SwarmEvent::Dialing { peer_id: Some(peer_id), .. } => {
                    self.pending_dials.insert(peer_id);
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    if let Some(peer_id) = peer_id {
                        self.pending_dials.remove(&peer_id);
                    }
                    debug!(transport_peer = ?peer_id, %error, "discovery outgoing connection failed");
                }
                other => debug!(event = ?other, "discovery network event"),
''',
)

replace_once(
    network,
    '''    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
''',
    '''    fn ensure_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        if self
            .authenticated
            .get(&peer)
            .is_some_and(|(_, authenticated_connection)| *authenticated_connection == connection_id)
            || self
                .pending_challenges
                .get(&peer)
                .is_some_and(|(challenge_connection, _, _)| *challenge_connection == connection_id)
        {
            return Ok(());
        }
        self.issue_auth_challenge(peer, connection_id)
    }

    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
''',
)

# Freshness collection must reuse an authenticated locator before considering
# another transport dial. Locator identity still has zero authority.
cli = "crates/swarm-cli/src/discovery.rs"
replace_once(
    cli,
    '''    for transport_peer in providers.iter().copied().collect::<Vec<_>>() {
        let _ = node.dial_peer(transport_peer);
        if let Some(application_peer) = node.application_peer(&transport_peer) {
            applications.insert(transport_peer, application_peer);
        }
    }
''',
    '''    for transport_peer in providers.iter().copied().collect::<Vec<_>>() {
        if let Some(application_peer) = node.application_peer(&transport_peer) {
            applications.insert(transport_peer, application_peer);
        } else {
            let _ = node.dial_peer(transport_peer);
        }
    }
''',
)

# Remove the throwaway topology probe. It created and destroyed an independent
# Swarm immediately before the real browse/resolve rounds. Readiness now belongs
# to the actual search/resolve nodes, which already seed all explicit locators
# into their provider sets and wait for those real connections/requests.
test = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
text = test.read_text()
helper_start = text.find("async fn wait_for_provider_topology(")
if helper_start >= 0:
    helper_end = text.find("async fn assert_peer_tasks_healthy(", helper_start)
    if helper_end < 0:
        raise SystemExit("missing topology helper end anchor")
    text = text[:helper_start] + text[helper_end:]

for label in ["public browse", "world freshness", "exact resolve"]:
    needle = f'''    wait_for_provider_topology(\n        "{label}",'''
    start = text.find(needle)
    if start >= 0:
        end = text.find("    .await;\n", start)
        if end < 0:
            raise SystemExit(f"missing {label} topology await anchor")
        text = text[:start] + text[end + len("    .await;\n"):]

text = text.replace(
    '    assert_peer_tasks_healthy(&mut tasks, &lifecycle, "browse topology readiness").await;\n',
    '',
)
text = text.replace(
    '    assert_peer_tasks_healthy(&mut tasks, &lifecycle, "resolve topology readiness").await;\n',
    '',
)
# HashSet was introduced only for the deleted topology helper.
text = text.replace(
    '''use std::{\n    collections::HashSet,\n    sync::{Arc, Mutex},\n''',
    '''use std::{\n    sync::{Arc, Mutex},\n''',
)
test.write_text(text)

print("FINAL-028 connection lifecycle repaired: single-flight dials, stable active auth, no throwaway readiness probe")
