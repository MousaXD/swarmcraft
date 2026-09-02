from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text()


def write(path: str, text: str) -> None:
    Path(path).write_text(text)


def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one target, found {count}: {old[:140]!r}")
    write(path, text.replace(old, new, 1))


# Centralize transport-level limits so both swarms reject excess work before
# protocol handshakes allocate long-lived application state.
replace_once(
    "crates/swarm-network/src/admission.rs",
    "pub(crate) const MAX_APPLICATION_CONNECTIONS: usize = 64;\npub(crate) const REQUEST_WINDOW: Duration = Duration::from_secs(10);\n",
    "pub(crate) const MAX_APPLICATION_CONNECTIONS: usize = 64;\n"
    "pub(crate) const MAX_PENDING_INCOMING_CONNECTIONS: u32 = 32;\n"
    "pub(crate) const MAX_PENDING_OUTGOING_CONNECTIONS: u32 = 32;\n"
    "pub(crate) const MAX_ESTABLISHED_INCOMING_CONNECTIONS: u32 = 72;\n"
    "pub(crate) const MAX_ESTABLISHED_CONNECTIONS: u32 = 96;\n"
    "pub(crate) const MAX_ESTABLISHED_CONNECTIONS_PER_PEER: u32 = 2;\n"
    "pub(crate) const MAX_DISCOVERY_PENDING_INCOMING_CONNECTIONS: u32 = 24;\n"
    "pub(crate) const MAX_DISCOVERY_PENDING_OUTGOING_CONNECTIONS: u32 = 24;\n"
    "pub(crate) const MAX_DISCOVERY_ESTABLISHED_INCOMING_CONNECTIONS: u32 = 48;\n"
    "pub(crate) const MAX_DISCOVERY_ESTABLISHED_CONNECTIONS: u32 = 64;\n"
    "pub(crate) const AUTH_CHALLENGE_TIMEOUT: Duration = Duration::from_secs(10);\n"
    "pub(crate) const REQUEST_WINDOW: Duration = Duration::from_secs(10);\n",
)
replace_once(
    "crates/swarm-network/src/admission.rs",
    "pub(crate) fn application_connection_allowed(active_application_connections: usize, replacing_peer: bool) -> bool {\n    replacing_peer || active_application_connections < MAX_APPLICATION_CONNECTIONS\n}\n",
    '''pub(crate) fn application_connection_allowed(active_application_connections: usize, replacing_peer: bool) -> bool {
    replacing_peer || active_application_connections < MAX_APPLICATION_CONNECTIONS
}

pub(crate) fn primary_connection_limits() -> libp2p::connection_limits::ConnectionLimits {
    libp2p::connection_limits::ConnectionLimits::default()
        .with_max_pending_incoming(Some(MAX_PENDING_INCOMING_CONNECTIONS))
        .with_max_pending_outgoing(Some(MAX_PENDING_OUTGOING_CONNECTIONS))
        .with_max_established_incoming(Some(MAX_ESTABLISHED_INCOMING_CONNECTIONS))
        .with_max_established(Some(MAX_ESTABLISHED_CONNECTIONS))
        .with_max_established_per_peer(Some(MAX_ESTABLISHED_CONNECTIONS_PER_PEER))
}

pub(crate) fn discovery_connection_limits() -> libp2p::connection_limits::ConnectionLimits {
    libp2p::connection_limits::ConnectionLimits::default()
        .with_max_pending_incoming(Some(MAX_DISCOVERY_PENDING_INCOMING_CONNECTIONS))
        .with_max_pending_outgoing(Some(MAX_DISCOVERY_PENDING_OUTGOING_CONNECTIONS))
        .with_max_established_incoming(Some(MAX_DISCOVERY_ESTABLISHED_INCOMING_CONNECTIONS))
        .with_max_established(Some(MAX_DISCOVERY_ESTABLISHED_CONNECTIONS))
        .with_max_established_per_peer(Some(MAX_ESTABLISHED_CONNECTIONS_PER_PEER))
}

pub(crate) fn auth_challenge_expired(issued_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(issued_at) >= AUTH_CHALLENGE_TIMEOUT
}
''',
)
replace_once(
    "crates/swarm-network/src/admission.rs",
    "    fn application_connection_cap_allows_replacement_but_not_new_overflow() {\n        assert!(application_connection_allowed(MAX_APPLICATION_CONNECTIONS - 1, false));\n        assert!(!application_connection_allowed(MAX_APPLICATION_CONNECTIONS, false));\n        assert!(application_connection_allowed(MAX_APPLICATION_CONNECTIONS, true));\n    }\n",
    '''    fn application_connection_cap_allows_replacement_but_not_new_overflow() {
        assert!(application_connection_allowed(MAX_APPLICATION_CONNECTIONS - 1, false));
        assert!(!application_connection_allowed(MAX_APPLICATION_CONNECTIONS, false));
        assert!(application_connection_allowed(MAX_APPLICATION_CONNECTIONS, true));
    }

    #[test]
    fn transport_caps_bound_handshake_and_connection_state() {
        assert!(MAX_PENDING_INCOMING_CONNECTIONS < MAX_ESTABLISHED_CONNECTIONS);
        assert!(MAX_ESTABLISHED_INCOMING_CONNECTIONS <= MAX_ESTABLISHED_CONNECTIONS);
        assert!(MAX_APPLICATION_CONNECTIONS <= MAX_ESTABLISHED_CONNECTIONS as usize);
        assert!(MAX_DISCOVERY_PENDING_INCOMING_CONNECTIONS < MAX_DISCOVERY_ESTABLISHED_CONNECTIONS);
        assert!(MAX_DISCOVERY_ESTABLISHED_INCOMING_CONNECTIONS <= MAX_DISCOVERY_ESTABLISHED_CONNECTIONS);
        assert!(MAX_ESTABLISHED_CONNECTIONS_PER_PEER >= 2);
    }

    #[test]
    fn silent_authentication_challenge_expires() {
        let issued = Instant::now();
        assert!(!auth_challenge_expired(issued, issued));
        assert!(auth_challenge_expired(issued, issued + AUTH_CHALLENGE_TIMEOUT));
    }
''',
)

# Primary swarm: compose libp2p connection limits into the behaviour and expire
# silent challenge holders so connection slots cannot be parked forever.
replace_once(
    "crates/swarm-network/src/node.rs",
    "    admission::{application_connection_allowed, AdmissionController},\n",
    "    admission::{\n        application_connection_allowed, auth_challenge_expired, primary_connection_limits, AdmissionController,\n        AUTH_CHALLENGE_TIMEOUT,\n    },\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "    autonat, dcutr, identify,\n",
    "    autonat, connection_limits, dcutr, identify,\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "    auto_nat: autonat::Behaviour,\n",
    "    auto_nat: autonat::Behaviour,\n    limits: connection_limits::Behaviour,\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32])>,\n",
    "    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32], Instant)>,\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "                auto_nat,\n            })?\n",
    "                auto_nat,\n                limits: connection_limits::Behaviour::new(primary_connection_limits()),\n            })?\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "    pub async fn next_event(&mut self) -> Result<NetworkEvent> {\n        loop {\n            match self.swarm.select_next_some().await {\n",
    '''    pub async fn next_event(&mut self) -> Result<NetworkEvent> {
        loop {
            self.expire_stale_auth_challenges();
            let event = match tokio::time::timeout(AUTH_CHALLENGE_TIMEOUT, self.swarm.select_next_some()).await {
                Ok(event) => event,
                Err(_) => continue,
            };
            self.expire_stale_auth_challenges();
            match event {
''',
)
replace_once(
    "crates/swarm-network/src/node.rs",
    ".is_some_and(|(challenge_connection, _)| *challenge_connection == connection_id)\n",
    ".is_some_and(|(challenge_connection, _, _)| *challenge_connection == connection_id)\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "                                    let Some((challenge_connection, expected_challenge)) = expected else {\n",
    "                                    let Some((challenge_connection, expected_challenge, _issued_at)) = expected else {\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "        self.pending_challenges.insert(peer, (connection_id, challenge));\n",
    "        self.pending_challenges.insert(peer, (connection_id, challenge, Instant::now()));\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "    fn refresh_connectivity_paths(&mut self) {\n",
    '''    fn expire_stale_auth_challenges(&mut self) {
        let now = Instant::now();
        let stale = self
            .pending_challenges
            .iter()
            .filter_map(|(peer, (connection_id, _, issued_at))| {
                auth_challenge_expired(*issued_at, now).then_some((*peer, *connection_id))
            })
            .collect::<Vec<_>>();
        for (peer, connection_id) in stale {
            self.pending_challenges.remove(&peer);
            self.authenticated.remove(&peer);
            if self.active_connections.get(&peer).is_some_and(|active| *active == connection_id) {
                self.active_connections.remove(&peer);
            }
            let _ = self.swarm.close_connection(connection_id);
            warn!(transport_peer = %peer, %connection_id, "authentication challenge expired; closing silent connection");
        }
    }

    fn refresh_connectivity_paths(&mut self) {
''',
)

# Discovery swarm gets smaller transport budgets and the same challenge expiry.
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    identify,\n",
    "    connection_limits, identify,\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    admission::{application_connection_allowed, AdmissionController},\n",
    "    admission::{\n        application_connection_allowed, auth_challenge_expired, discovery_connection_limits, AdmissionController,\n        AUTH_CHALLENGE_TIMEOUT,\n    },\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    kad: kad::Behaviour<MemoryStore>,\n",
    "    kad: kad::Behaviour<MemoryStore>,\n    limits: connection_limits::Behaviour,\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32])>,\n",
    "    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32], Instant)>,\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "                kad,\n            })?\n",
    "                kad,\n                limits: connection_limits::Behaviour::new(discovery_connection_limits()),\n            })?\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    pub async fn next_event(&mut self) -> Result<DiscoveryNetworkEvent> {\n        loop {\n            match self.swarm.select_next_some().await {\n",
    '''    pub async fn next_event(&mut self) -> Result<DiscoveryNetworkEvent> {
        loop {
            self.expire_stale_auth_challenges();
            let event = match tokio::time::timeout(AUTH_CHALLENGE_TIMEOUT, self.swarm.select_next_some()).await {
                Ok(event) => event,
                Err(_) => continue,
            };
            self.expire_stale_auth_challenges();
            match event {
''',
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    ".is_some_and(|(challenge_connection, _)| *challenge_connection == connection_id)\n",
    ".is_some_and(|(challenge_connection, _, _)| *challenge_connection == connection_id)\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "                                    let Some((challenge_connection, expected_challenge)) =\n                                        self.pending_challenges.remove(&peer)\n",
    "                                    let Some((challenge_connection, expected_challenge, _issued_at)) =\n                                        self.pending_challenges.remove(&peer)\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "        self.pending_challenges.insert(peer, (connection_id, challenge));\n",
    "        self.pending_challenges.insert(peer, (connection_id, challenge, Instant::now()));\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    }\n}\n\npub fn public_directory_key()",
    '''    }

    fn expire_stale_auth_challenges(&mut self) {
        let now = Instant::now();
        let stale = self
            .pending_challenges
            .iter()
            .filter_map(|(peer, (connection_id, _, issued_at))| {
                auth_challenge_expired(*issued_at, now).then_some((*peer, *connection_id))
            })
            .collect::<Vec<_>>();
        for (peer, connection_id) in stale {
            self.pending_challenges.remove(&peer);
            self.authenticated.remove(&peer);
            if self.active_connections.get(&peer).is_some_and(|active| *active == connection_id) {
                self.active_connections.remove(&peer);
            }
            let _ = self.swarm.close_connection(connection_id);
            warn!(transport_peer = %peer, %connection_id, "discovery authentication challenge expired; closing silent connection");
        }
    }
}

pub fn public_directory_key()''',
)
