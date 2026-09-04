from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing Agent 4 lifecycle probe anchor in {path}: {old[:180]!r}")
    p.write_text(text.replace(old, new, 1))


path = "crates/swarm-network/src/discovery.rs"

replace_once(
    path,
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
        let connected = self.swarm.is_connected(&peer);
        debug!(
            local_transport_peer = %self.swarm.local_peer_id(),
            transport_peer = %peer,
            connected,
            active_connection = ?self.active_connections.get(&peer),
            authenticated_connection = ?self.authenticated.get(&peer).map(|(_, connection)| *connection),
            tracked_established = ?self.connection_counts.get(&peer),
            address = %address,
            "discovery lifecycle: explicit bootstrap evaluated"
        );
        if !connected {
            debug!(
                local_transport_peer = %self.swarm.local_peer_id(),
                transport_peer = %peer,
                address = %address,
                "discovery lifecycle: outbound dial attempt (explicit bootstrap)"
            );
            self.swarm
                .dial(DialOpts::peer_id(peer).addresses(vec![address]).build())
                .context("failed to dial discovery bootstrap peer")?;
        }
        Ok(peer)
    }
''',
)

replace_once(
    path,
    '''    pub fn dial_peer(&mut self, peer: TransportPeerId) -> Result<()> {
        if !self.swarm.is_connected(&peer) {
            self.swarm.dial(DialOpts::peer_id(peer).build()).context("failed to dial discovery provider")?;
        }
        Ok(())
    }
''',
    '''    pub fn dial_peer(&mut self, peer: TransportPeerId) -> Result<()> {
        let connected = self.swarm.is_connected(&peer);
        debug!(
            local_transport_peer = %self.swarm.local_peer_id(),
            transport_peer = %peer,
            connected,
            active_connection = ?self.active_connections.get(&peer),
            authenticated_connection = ?self.authenticated.get(&peer).map(|(_, connection)| *connection),
            tracked_established = ?self.connection_counts.get(&peer),
            "discovery lifecycle: provider dial evaluated"
        );
        if !connected {
            debug!(
                local_transport_peer = %self.swarm.local_peer_id(),
                transport_peer = %peer,
                "discovery lifecycle: outbound dial attempt (provider)"
            );
            self.swarm.dial(DialOpts::peer_id(peer).build()).context("failed to dial discovery provider")?;
        }
        Ok(())
    }
''',
)

replace_once(
    path,
    '''    pub fn send_request(
        &mut self,
        peer: &TransportPeerId,
        request: WireRequest,
    ) -> Result<request_response::OutboundRequestId> {
        request.validate_limits()?;
        Ok(self.swarm.behaviour_mut().request_response.send_request(peer, request))
    }
''',
    '''    pub fn send_request(
        &mut self,
        peer: &TransportPeerId,
        request: WireRequest,
    ) -> Result<request_response::OutboundRequestId> {
        request.validate_limits()?;
        debug!(
            local_transport_peer = %self.swarm.local_peer_id(),
            transport_peer = %peer,
            active_connection = ?self.active_connections.get(peer),
            authenticated_connection = ?self.authenticated.get(peer).map(|(_, connection)| *connection),
            tracked_established = ?self.connection_counts.get(peer),
            request = ?request,
            "discovery lifecycle: outbound request queued"
        );
        Ok(self.swarm.behaviour_mut().request_response.send_request(peer, request))
    }
''',
)

replace_once(
    path,
    '''                SwarmEvent::ConnectionEstablished { peer_id, connection_id, num_established, .. } => {
                    if !application_connection_allowed(
''',
    '''                SwarmEvent::ConnectionEstablished { peer_id, connection_id, num_established, .. } => {
                    debug!(
                        local_transport_peer = %self.swarm.local_peer_id(),
                        transport_peer = %peer_id,
                        %connection_id,
                        remaining_after_establish = num_established.get(),
                        previous_active_connection = ?self.active_connections.get(&peer_id),
                        previous_authenticated_connection = ?self.authenticated.get(&peer_id).map(|(_, connection)| *connection),
                        previous_tracked_established = ?self.connection_counts.get(&peer_id),
                        "discovery lifecycle: connection established"
                    );
                    if !application_connection_allowed(
''',
)

replace_once(
    path,
    '''                    let defer_challenge = previous
                        .filter(|previous| *previous != connection_id && num_established.get() > 1)
                        .is_some_and(|previous| self.swarm.close_connection(previous));
                    if !defer_challenge {
''',
    '''                    let defer_challenge = previous
                        .filter(|previous| *previous != connection_id && num_established.get() > 1)
                        .is_some_and(|previous| {
                            debug!(
                                local_transport_peer = %self.swarm.local_peer_id(),
                                transport_peer = %peer_id,
                                closing_connection = %previous,
                                replacement_connection = %connection_id,
                                established = num_established.get(),
                                "discovery lifecycle: closing previous connection for replacement"
                            );
                            self.swarm.close_connection(previous)
                        });
                    if !defer_challenge {
''',
)

replace_once(
    path,
    '''                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    self.connection_counts.insert(peer_id, num_established as usize);
''',
    '''                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    debug!(
                        local_transport_peer = %self.swarm.local_peer_id(),
                        transport_peer = %peer_id,
                        %connection_id,
                        remaining_established = num_established,
                        active_connection_before_close = ?self.active_connections.get(&peer_id),
                        authenticated_connection_before_close = ?self.authenticated.get(&peer_id).map(|(_, connection)| *connection),
                        tracked_established_before_close = ?self.connection_counts.get(&peer_id),
                        "discovery lifecycle: connection closed"
                    );
                    self.connection_counts.insert(peer_id, num_established as usize);
''',
)

replace_once(
    path,
    '''                        request_response::Message::Request { request, channel, .. } => {
                            let authenticated_request =
''',
    '''                        request_response::Message::Request { request, channel, .. } => {
                            debug!(
                                local_transport_peer = %self.swarm.local_peer_id(),
                                transport_peer = %peer,
                                %connection_id,
                                request = ?request,
                                active_connection = ?self.active_connections.get(&peer),
                                authenticated_connection = ?self.authenticated.get(&peer).map(|(_, connection)| *connection),
                                "discovery lifecycle: inbound request on connection"
                            );
                            let authenticated_request =
''',
)

replace_once(
    path,
    '''                        request_response::Message::Response { request_id, response } => {
                            if let Err(error) = response.validate_limits() {
''',
    '''                        request_response::Message::Response { request_id, response } => {
                            debug!(
                                local_transport_peer = %self.swarm.local_peer_id(),
                                transport_peer = %peer,
                                %connection_id,
                                ?request_id,
                                response = ?response,
                                active_connection = ?self.active_connections.get(&peer),
                                authenticated_connection = ?self.authenticated.get(&peer).map(|(_, connection)| *connection),
                                "discovery lifecycle: inbound response on connection"
                            );
                            if let Err(error) = response.validate_limits() {
''',
)

replace_once(
    path,
    '''    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        let mut challenge = [0_u8; 32];
''',
    '''    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        debug!(
            local_transport_peer = %self.swarm.local_peer_id(),
            transport_peer = %peer,
            %connection_id,
            tracked_established = ?self.connection_counts.get(&peer),
            "discovery lifecycle: issuing connection-bound authentication challenge"
        );
        let mut challenge = [0_u8; 32];
''',
)

# Turn tracing on in the permanent network fixture for this diagnostic run.
test_path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
test_text = test_path.read_text()
old_test = '''async fn malicious_and_stale_providers_cannot_win_browse_or_exact_resolve() {
    let a_record_id = identity(A);
'''
new_test = '''async fn malicious_and_stale_providers_cannot_win_browse_or_exact_resolve() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let a_record_id = identity(A);
'''
if old_test in test_text:
    test_text = test_text.replace(old_test, new_test, 1)
elif new_test not in test_text:
    raise SystemExit("missing adversarial test tracing anchor")
test_path.write_text(test_text)

print("Agent 4 connection lifecycle probe instrumented without changing connection policy")
