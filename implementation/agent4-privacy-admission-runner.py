from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text()


def write(path: str, text: str) -> None:
    Path(path).write_text(text)


def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one target, found {count}: {old[:120]!r}")
    write(path, text.replace(old, new, 1))


admission = r'''use libp2p::PeerId as TransportPeerId;
use std::{collections::HashMap, time::{Duration, Instant}};

pub(crate) const MAX_APPLICATION_CONNECTIONS: usize = 64;
pub(crate) const REQUEST_WINDOW: Duration = Duration::from_secs(10);
pub(crate) const MAX_UNAUTHENTICATED_REQUESTS_PER_PEER: u32 = 8;
pub(crate) const MAX_AUTHENTICATED_REQUESTS_PER_PEER: u32 = 128;
pub(crate) const MAX_GLOBAL_UNAUTHENTICATED_REQUESTS: u32 = 256;
pub(crate) const MAX_GLOBAL_AUTHENTICATED_REQUESTS: u32 = 4096;

#[derive(Debug, Clone)]
struct WindowCounter {
    started: Instant,
    count: u32,
}

impl WindowCounter {
    fn new(now: Instant) -> Self {
        Self { started: now, count: 0 }
    }

    fn admit(&mut self, now: Instant, limit: u32) -> bool {
        if now.saturating_duration_since(self.started) >= REQUEST_WINDOW {
            self.started = now;
            self.count = 0;
        }
        if self.count >= limit {
            return false;
        }
        self.count += 1;
        true
    }
}

#[derive(Debug, Clone)]
struct PeerBudget {
    unauthenticated: WindowCounter,
    authenticated: WindowCounter,
}

impl PeerBudget {
    fn new(now: Instant) -> Self {
        Self { unauthenticated: WindowCounter::new(now), authenticated: WindowCounter::new(now) }
    }
}

#[derive(Debug)]
pub(crate) struct AdmissionController {
    peers: HashMap<TransportPeerId, PeerBudget>,
    global_unauthenticated: WindowCounter,
    global_authenticated: WindowCounter,
}

impl AdmissionController {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            peers: HashMap::new(),
            global_unauthenticated: WindowCounter::new(now),
            global_authenticated: WindowCounter::new(now),
        }
    }

    pub(crate) fn admit_request(&mut self, peer: TransportPeerId, authenticated: bool, now: Instant) -> bool {
        let peer_budget = self.peers.entry(peer).or_insert_with(|| PeerBudget::new(now));
        let peer_allowed = if authenticated {
            peer_budget.authenticated.admit(now, MAX_AUTHENTICATED_REQUESTS_PER_PEER)
        } else {
            peer_budget.unauthenticated.admit(now, MAX_UNAUTHENTICATED_REQUESTS_PER_PEER)
        };
        if !peer_allowed {
            return false;
        }
        if authenticated {
            self.global_authenticated.admit(now, MAX_GLOBAL_AUTHENTICATED_REQUESTS)
        } else {
            self.global_unauthenticated.admit(now, MAX_GLOBAL_UNAUTHENTICATED_REQUESTS)
        }
    }

    pub(crate) fn forget_peer(&mut self, peer: TransportPeerId) {
        self.peers.remove(&peer);
    }
}

pub(crate) fn application_connection_allowed(active_application_connections: usize, replacing_peer: bool) -> bool {
    replacing_peer || active_application_connections < MAX_APPLICATION_CONNECTIONS
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    fn peer() -> TransportPeerId {
        Keypair::generate_ed25519().public().to_peer_id()
    }

    #[test]
    fn unauthenticated_and_authenticated_budgets_are_separate() {
        let now = Instant::now();
        let peer = peer();
        let mut admission = AdmissionController::new();
        for _ in 0..MAX_UNAUTHENTICATED_REQUESTS_PER_PEER {
            assert!(admission.admit_request(peer, false, now));
        }
        assert!(!admission.admit_request(peer, false, now));
        assert!(admission.admit_request(peer, true, now));
    }

    #[test]
    fn request_budget_recovers_after_window() {
        let now = Instant::now();
        let peer = peer();
        let mut admission = AdmissionController::new();
        for _ in 0..MAX_UNAUTHENTICATED_REQUESTS_PER_PEER {
            assert!(admission.admit_request(peer, false, now));
        }
        assert!(admission.admit_request(peer, false, now + REQUEST_WINDOW));
    }

    #[test]
    fn application_connection_cap_allows_replacement_but_not_new_overflow() {
        assert!(application_connection_allowed(MAX_APPLICATION_CONNECTIONS - 1, false));
        assert!(!application_connection_allowed(MAX_APPLICATION_CONNECTIONS, false));
        assert!(application_connection_allowed(MAX_APPLICATION_CONNECTIONS, true));
    }
}
'''
write("crates/swarm-network/src/admission.rs", admission)

replace_once(
    "crates/swarm-network/src/lib.rs",
    "mod diagnostics;\n",
    "mod admission;\nmod diagnostics;\n",
)

replace_once(
    "crates/swarm-network/src/node.rs",
    "use crate::{\n    build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, wire::WireRequest, wire::WireResponse,\n",
    "use crate::{\n    admission::{application_connection_allowed, AdmissionController},\n    build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, wire::WireRequest, wire::WireResponse,\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "    time::Duration,\n",
    "    time::{Duration, Instant},\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "    diagnostics: ConnectivityDiagnosticsV1,\n",
    "    diagnostics: ConnectivityDiagnosticsV1,\n    admission: AdmissionController,\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "            diagnostics: ConnectivityDiagnosticsV1::default(),\n",
    "            diagnostics: ConnectivityDiagnosticsV1::default(),\n            admission: AdmissionController::new(),\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "                    self.connection_paths.insert(connection_id, path_kind);\n                    self.refresh_connectivity_paths();\n",
    "                    let application_path = matches!(\n                        path_kind,\n                        ConnectionPathKind::DirectApplication | ConnectionPathKind::RelayedApplication\n                    );\n                    let active_application_connections = self\n                        .connection_paths\n                        .values()\n                        .filter(|path| matches!(path, ConnectionPathKind::DirectApplication | ConnectionPathKind::RelayedApplication))\n                        .count();\n                    if application_path\n                        && !application_connection_allowed(\n                            active_application_connections,\n                            self.active_connections.contains_key(&peer_id),\n                        )\n                    {\n                        warn!(transport_peer = %peer_id, %connection_id, \"application connection admission limit reached\");\n                        let _ = self.swarm.close_connection(connection_id);\n                        continue;\n                    }\n                    self.connection_paths.insert(connection_id, path_kind);\n                    self.refresh_connectivity_paths();\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "                        self.pending_challenges.remove(&peer_id);\n                        return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });\n",
    "                        self.pending_challenges.remove(&peer_id);\n                        self.admission.forget_peer(peer_id);\n                        return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });\n",
)
replace_once(
    "crates/swarm-network/src/node.rs",
    "                                request_response::Message::Request { request, channel, .. } => {\n                                    if let Err(error) = request.validate_limits() {\n",
    "                                request_response::Message::Request { request, channel, .. } => {\n                                    let authenticated_request = self.authenticated.get(&peer).is_some_and(\n                                        |(_, authenticated_connection)| *authenticated_connection == connection_id,\n                                    );\n                                    if !self.admission.admit_request(peer, authenticated_request, Instant::now()) {\n                                        let _ = self.respond(\n                                            channel,\n                                            WireResponse::Error {\n                                                code: \"RATE_LIMITED\".into(),\n                                                message: if authenticated_request {\n                                                    \"authenticated request budget exceeded; retry after the admission window\".into()\n                                                } else {\n                                                    \"pre-authentication request budget exceeded; reconnect later\".into()\n                                                },\n                                            },\n                                        );\n                                        continue;\n                                    }\n                                    if let Err(error) = request.validate_limits() {\n",
)

replace_once(
    "crates/swarm-network/src/discovery.rs",
    "use std::{collections::HashMap, env, time::Duration};\n",
    "use std::{collections::HashMap, env, time::{Duration, Instant}};\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "use crate::{\n    build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, WireRequest, WireResponse, BOOTSTRAP_ENV,\n};\n",
    "use crate::{\n    admission::{application_connection_allowed, AdmissionController},\n    build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, WireRequest, WireResponse, BOOTSTRAP_ENV,\n};\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "const FRIEND_KEY_PREFIX: &[u8] = b\"swarmcraft/discovery/friend/v1\\0\";\n",
    "const FRIEND_KEY_PREFIX: &[u8] = b\"swarmcraft/discovery/friend/v2\\0\";\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    connection_counts: HashMap<TransportPeerId, usize>,\n",
    "    connection_counts: HashMap<TransportPeerId, usize>,\n    admission: AdmissionController,\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "            connection_counts: HashMap::new(),\n",
    "            connection_counts: HashMap::new(),\n            admission: AdmissionController::new(),\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    pub fn start_providing_friend_presence(&mut self, peer: PeerId) -> Result<kad::QueryId> {\n        self.swarm\n            .behaviour_mut()\n            .kad\n            .start_providing(friend_presence_key(peer))\n            .context(\"failed to publish friend presence provider\")\n    }\n",
    "    pub fn start_providing_friend_presence(&mut self, peer: PeerId, requester: PeerId) -> Result<kad::QueryId> {\n        self.swarm\n            .behaviour_mut()\n            .kad\n            .start_providing(friend_presence_key(peer, requester))\n            .context(\"failed to publish friend presence provider\")\n    }\n\n    pub fn stop_providing_friend_presence(&mut self, peer: PeerId, requester: PeerId) {\n        self.swarm.behaviour_mut().kad.stop_providing(&friend_presence_key(peer, requester));\n    }\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "    pub fn find_friend_providers(&mut self, peer: PeerId) -> kad::QueryId {\n        self.swarm.behaviour_mut().kad.get_providers(friend_presence_key(peer))\n    }\n",
    "    pub fn find_friend_providers(&mut self, peer: PeerId, requester: PeerId) -> kad::QueryId {\n        self.swarm.behaviour_mut().kad.get_providers(friend_presence_key(peer, requester))\n    }\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "                SwarmEvent::ConnectionEstablished { peer_id, connection_id, num_established, .. } => {\n                    self.connection_counts.insert(peer_id, num_established.get() as usize);\n",
    "                SwarmEvent::ConnectionEstablished { peer_id, connection_id, num_established, .. } => {\n                    if !application_connection_allowed(\n                        self.active_connections.len(),\n                        self.active_connections.contains_key(&peer_id),\n                    ) {\n                        warn!(transport_peer = %peer_id, %connection_id, \"discovery connection admission limit reached\");\n                        let _ = self.swarm.close_connection(connection_id);\n                        continue;\n                    }\n                    self.connection_counts.insert(peer_id, num_established.get() as usize);\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "                        self.pending_challenges.remove(&peer_id);\n                        return Ok(DiscoveryNetworkEvent::Disconnected { transport_peer: peer_id, application_peer });\n",
    "                        self.pending_challenges.remove(&peer_id);\n                        self.admission.forget_peer(peer_id);\n                        return Ok(DiscoveryNetworkEvent::Disconnected { transport_peer: peer_id, application_peer });\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "                        request_response::Message::Request { request, channel, .. } => {\n                            if let Err(error) = request.validate_limits() {\n",
    "                        request_response::Message::Request { request, channel, .. } => {\n                            let authenticated_request = self.authenticated.get(&peer).is_some_and(\n                                |(_, authenticated_connection)| *authenticated_connection == connection_id,\n                            );\n                            if !self.admission.admit_request(peer, authenticated_request, Instant::now()) {\n                                let _ = self.respond(\n                                    channel,\n                                    WireResponse::Error {\n                                        code: \"RATE_LIMITED\".into(),\n                                        message: if authenticated_request {\n                                            \"authenticated discovery request budget exceeded\".into()\n                                        } else {\n                                            \"pre-authentication discovery request budget exceeded\".into()\n                                        },\n                                    },\n                                );\n                                continue;\n                            }\n                            if let Err(error) = request.validate_limits() {\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "pub fn friend_presence_key(peer: PeerId) -> kad::RecordKey {\n    let mut bytes = Vec::with_capacity(FRIEND_KEY_PREFIX.len() + 32);\n    bytes.extend_from_slice(FRIEND_KEY_PREFIX);\n    bytes.extend_from_slice(&peer.0);\n    kad::RecordKey::new(&bytes)\n}\n",
    "pub fn friend_presence_key(peer: PeerId, requester: PeerId) -> kad::RecordKey {\n    let mut bytes = Vec::with_capacity(FRIEND_KEY_PREFIX.len() + 64);\n    bytes.extend_from_slice(FRIEND_KEY_PREFIX);\n    bytes.extend_from_slice(&peer.0);\n    bytes.extend_from_slice(&requester.0);\n    kad::RecordKey::new(&bytes)\n}\n",
)
replace_once(
    "crates/swarm-network/src/discovery.rs",
    "        let friend = friend_presence_key(PeerId([1; 32]));\n        assert_ne!(world_a, world_b);\n        assert_ne!(world_a, friend);\n",
    "        let friend = friend_presence_key(PeerId([1; 32]), PeerId([3; 32]));\n        let other_requester = friend_presence_key(PeerId([1; 32]), PeerId([4; 32]));\n        assert_ne!(world_a, world_b);\n        assert_ne!(world_a, friend);\n        assert_ne!(friend, other_requester);\n",
)

replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "    sequences: HashMap<WorldId, u64>,\n",
    "    sequences: HashMap<WorldId, u64>,\n    presence_requesters: HashSet<PeerId>,\n",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "    node.listen(listen.parse().context(\"invalid discovery listen multiaddress\")?)?;\n    let _ = node.start_providing_friend_presence(identity.peer_id())?;\n\n    let mut published = PublishedDiscoveryState::default();\n",
    "    node.listen(listen.parse().context(\"invalid discovery listen multiaddress\")?)?;\n\n    let mut published = PublishedDiscoveryState::default();\n",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "    refresh_publications(&storage, &identity, &mut node, &mut published)?;\n\n    info!(peer = %identity.peer_id(), \"discovery service starting\");\n",
    "    refresh_publications(&storage, &identity, &mut node, &mut published)?;\n    refresh_presence_publications(&paths, &identity, &mut node, &mut published)?;\n\n    info!(peer = %identity.peer_id(), \"discovery service starting\");\n",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "                if let Err(error) = refresh_publications(&storage, &identity, &mut node, &mut published) {\n                    warn!(%error, \"discovery publication refresh failed\");\n                }\n",
    "                if let Err(error) = refresh_publications(&storage, &identity, &mut node, &mut published) {\n                    warn!(%error, \"discovery publication refresh failed\");\n                }\n                if let Err(error) = refresh_presence_publications(&paths, &identity, &mut node, &mut published) {\n                    warn!(%error, \"friend presence publication refresh failed\");\n                }\n",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "                        if let Err(error) = handle_discovery_request(\n                            &identity,\n",
    "                        if let Err(error) = handle_discovery_request(\n                            &paths,\n                            &identity,\n",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "fn handle_discovery_request(\n    identity: &PeerIdentity,\n",
    "fn handle_discovery_request(\n    paths: &DataPaths,\n    identity: &PeerIdentity,\n",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "        WireRequest::FriendPresence { expected_peer_id, requester_peer_id, nonce } => {\n            if application_peer != requester_peer_id {\n                bail!(\"presence requester does not match authenticated peer identity\");\n            }\n            if expected_peer_id != identity.peer_id() {\n",
    "        WireRequest::FriendPresence { expected_peer_id, requester_peer_id, nonce } => {\n            if application_peer != requester_peer_id {\n                bail!(\"presence requester does not match authenticated peer identity\");\n            }\n            if !accepted_friend_peers(paths)?.contains(&requester_peer_id) {\n                node.respond(channel, WireResponse::FriendPresence(None))?;\n                return Ok(());\n            }\n            if expected_peer_id != identity.peer_id() {\n",
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "    let query = node.find_friend_providers(peer);\n",
    "    let query = node.find_friend_providers(peer, requester);\n",
)
marker = "fn shared_worlds(storage: &Storage, local: PeerId, friend: PeerId, friend_key: [u8; 32]) -> Result<Vec<SharedWorldV1>> {\n"
insert = r'''fn accepted_friend_peers(paths: &DataPaths) -> Result<HashSet<PeerId>> {
    load_friend_store(paths)?
        .friends
        .into_iter()
        .map(|friend| PeerId::from_str(&friend.peer_id).context("stored friend peer ID is invalid"))
        .collect()
}

fn refresh_presence_publications(
    paths: &DataPaths,
    identity: &PeerIdentity,
    node: &mut DiscoveryNode,
    state: &mut PublishedDiscoveryState,
) -> Result<()> {
    let next = accepted_friend_peers(paths)?;
    for requester in state.presence_requesters.difference(&next).copied().collect::<Vec<_>>() {
        node.stop_providing_friend_presence(identity.peer_id(), requester);
    }
    for requester in next.difference(&state.presence_requesters).copied().collect::<Vec<_>>() {
        let _ = node.start_providing_friend_presence(identity.peer_id(), requester)?;
    }
    state.presence_requesters = next;
    Ok(())
}

'''
replace_once("crates/swarm-cli/src/discovery.rs", marker, insert + marker)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    "        assert_eq!(list_friends(&paths).unwrap().len(), 2);\n",
    "        assert_eq!(list_friends(&paths).unwrap().len(), 2);\n        let accepted = accepted_friend_peers(&paths).unwrap();\n        assert_eq!(accepted.len(), 2);\n        assert!(accepted.contains(&a.peer_id()));\n        assert!(accepted.contains(&b.peer_id()));\n",
)

replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "    AuthorityLeaseGrantV1, BlobDescriptor, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, PeerId,\n",
    "    peer_id_from_public_key, AuthorityLeaseGrantV1, BlobDescriptor, EpochMode, EpochRecordV1, Hash32,\n    MembershipRecordV1, PeerId,\n",
)
replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "        let Ok(descriptor) = storage.load_world_descriptor(metadata.world_id) else { continue };\n        if descriptor.member(application_peer).is_none() {\n            continue;\n        }\n",
    "        let Ok(descriptor) = storage.load_world_descriptor(metadata.world_id) else { continue };\n        let Some(remote_member) = descriptor.member(application_peer) else { continue };\n        if remote_member.banned || peer_id_from_public_key(&remote_member.public_key) != application_peer {\n            continue;\n        }\n",
)

# Old failed validation diagnostics were only a temporary runner artifact.
Path("implementation/agent4-handshake-failure.log").unlink(missing_ok=True)
