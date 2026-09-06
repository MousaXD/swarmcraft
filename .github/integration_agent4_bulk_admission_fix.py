from pathlib import Path

admission = Path('crates/swarm-network/src/admission.rs')
admission.write_text('''use libp2p::PeerId as TransportPeerId;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub(crate) const MAX_APPLICATION_CONNECTIONS: usize = 64;
pub(crate) const MAX_PENDING_INCOMING_CONNECTIONS: u32 = 32;
pub(crate) const MAX_PENDING_OUTGOING_CONNECTIONS: u32 = 32;
pub(crate) const MAX_ESTABLISHED_INCOMING_CONNECTIONS: u32 = 72;
pub(crate) const MAX_ESTABLISHED_CONNECTIONS: u32 = 96;
pub(crate) const MAX_ESTABLISHED_CONNECTIONS_PER_PEER: u32 = 2;
pub(crate) const MAX_DISCOVERY_PENDING_INCOMING_CONNECTIONS: u32 = 24;
pub(crate) const MAX_DISCOVERY_PENDING_OUTGOING_CONNECTIONS: u32 = 24;
pub(crate) const MAX_DISCOVERY_ESTABLISHED_INCOMING_CONNECTIONS: u32 = 48;
pub(crate) const MAX_DISCOVERY_ESTABLISHED_CONNECTIONS: u32 = 64;
pub(crate) const AUTH_CHALLENGE_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const REQUEST_WINDOW: Duration = Duration::from_secs(10);
pub(crate) const MAX_UNAUTHENTICATED_REQUESTS_PER_PEER: u32 = 8;
pub(crate) const MAX_AUTHENTICATED_REQUESTS_PER_PEER: u32 = 128;
pub(crate) const MAX_AUTHENTICATED_BULK_REQUESTS_PER_PEER: u32 = 8_192;
pub(crate) const MAX_GLOBAL_UNAUTHENTICATED_REQUESTS: u32 = 256;
pub(crate) const MAX_GLOBAL_AUTHENTICATED_REQUESTS: u32 = 4_096;
pub(crate) const MAX_GLOBAL_AUTHENTICATED_BULK_REQUESTS: u32 = 32_768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestClass {
    Unauthenticated,
    AuthenticatedControl,
    AuthenticatedBulk,
}

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
    authenticated_control: WindowCounter,
    authenticated_bulk: WindowCounter,
}

impl PeerBudget {
    fn new(now: Instant) -> Self {
        Self {
            unauthenticated: WindowCounter::new(now),
            authenticated_control: WindowCounter::new(now),
            authenticated_bulk: WindowCounter::new(now),
        }
    }
}

#[derive(Debug)]
pub(crate) struct AdmissionController {
    peers: HashMap<TransportPeerId, PeerBudget>,
    global_unauthenticated: WindowCounter,
    global_authenticated_control: WindowCounter,
    global_authenticated_bulk: WindowCounter,
}

impl AdmissionController {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            peers: HashMap::new(),
            global_unauthenticated: WindowCounter::new(now),
            global_authenticated_control: WindowCounter::new(now),
            global_authenticated_bulk: WindowCounter::new(now),
        }
    }

    pub(crate) fn admit_request(&mut self, peer: TransportPeerId, class: RequestClass, now: Instant) -> bool {
        let peer_budget = self.peers.entry(peer).or_insert_with(|| PeerBudget::new(now));
        let peer_allowed = match class {
            RequestClass::Unauthenticated => {
                peer_budget.unauthenticated.admit(now, MAX_UNAUTHENTICATED_REQUESTS_PER_PEER)
            }
            RequestClass::AuthenticatedControl => {
                peer_budget.authenticated_control.admit(now, MAX_AUTHENTICATED_REQUESTS_PER_PEER)
            }
            RequestClass::AuthenticatedBulk => {
                peer_budget.authenticated_bulk.admit(now, MAX_AUTHENTICATED_BULK_REQUESTS_PER_PEER)
            }
        };
        if !peer_allowed {
            return false;
        }
        match class {
            RequestClass::Unauthenticated => {
                self.global_unauthenticated.admit(now, MAX_GLOBAL_UNAUTHENTICATED_REQUESTS)
            }
            RequestClass::AuthenticatedControl => {
                self.global_authenticated_control.admit(now, MAX_GLOBAL_AUTHENTICATED_REQUESTS)
            }
            RequestClass::AuthenticatedBulk => {
                self.global_authenticated_bulk.admit(now, MAX_GLOBAL_AUTHENTICATED_BULK_REQUESTS)
            }
        }
    }

    pub(crate) fn forget_peer(&mut self, peer: TransportPeerId) {
        self.peers.remove(&peer);
    }
}

pub(crate) fn application_connection_allowed(active_application_connections: usize, replacing_peer: bool) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    fn peer() -> TransportPeerId {
        Keypair::generate_ed25519().public().to_peer_id()
    }

    #[test]
    fn unauthenticated_control_and_bulk_budgets_are_separate() {
        let now = Instant::now();
        let peer = peer();
        let mut admission = AdmissionController::new();
        for _ in 0..MAX_UNAUTHENTICATED_REQUESTS_PER_PEER {
            assert!(admission.admit_request(peer, RequestClass::Unauthenticated, now));
        }
        assert!(!admission.admit_request(peer, RequestClass::Unauthenticated, now));
        assert!(admission.admit_request(peer, RequestClass::AuthenticatedControl, now));
        assert!(admission.admit_request(peer, RequestClass::AuthenticatedBulk, now));
    }

    #[test]
    fn authenticated_bulk_budget_is_bounded_without_consuming_control_budget() {
        let now = Instant::now();
        let peer = peer();
        let mut admission = AdmissionController::new();
        for _ in 0..MAX_AUTHENTICATED_BULK_REQUESTS_PER_PEER {
            assert!(admission.admit_request(peer, RequestClass::AuthenticatedBulk, now));
        }
        assert!(!admission.admit_request(peer, RequestClass::AuthenticatedBulk, now));
        assert!(admission.admit_request(peer, RequestClass::AuthenticatedControl, now));
    }

    #[test]
    fn request_budget_recovers_after_window() {
        let now = Instant::now();
        let peer = peer();
        let mut admission = AdmissionController::new();
        for _ in 0..MAX_UNAUTHENTICATED_REQUESTS_PER_PEER {
            assert!(admission.admit_request(peer, RequestClass::Unauthenticated, now));
        }
        assert!(admission.admit_request(peer, RequestClass::Unauthenticated, now + REQUEST_WINDOW));
    }

    #[test]
    fn application_connection_cap_allows_replacement_but_not_new_overflow() {
        assert!(application_connection_allowed(MAX_APPLICATION_CONNECTIONS - 1, false));
        assert!(!application_connection_allowed(MAX_APPLICATION_CONNECTIONS, false));
        assert!(application_connection_allowed(MAX_APPLICATION_CONNECTIONS, true));
    }

    const _: () = {
        assert!(MAX_PENDING_INCOMING_CONNECTIONS < MAX_ESTABLISHED_CONNECTIONS);
        assert!(MAX_ESTABLISHED_INCOMING_CONNECTIONS <= MAX_ESTABLISHED_CONNECTIONS);
        assert!(MAX_APPLICATION_CONNECTIONS <= MAX_ESTABLISHED_CONNECTIONS as usize);
        assert!(MAX_DISCOVERY_PENDING_INCOMING_CONNECTIONS < MAX_DISCOVERY_ESTABLISHED_CONNECTIONS);
        assert!(MAX_DISCOVERY_ESTABLISHED_INCOMING_CONNECTIONS <= MAX_DISCOVERY_ESTABLISHED_CONNECTIONS);
        assert!(MAX_ESTABLISHED_CONNECTIONS_PER_PEER >= 2);
        assert!(MAX_AUTHENTICATED_BULK_REQUESTS_PER_PEER > MAX_AUTHENTICATED_REQUESTS_PER_PEER);
        assert!(MAX_GLOBAL_AUTHENTICATED_BULK_REQUESTS >= MAX_AUTHENTICATED_BULK_REQUESTS_PER_PEER);
    };

    #[test]
    fn transport_limit_behaviours_are_constructible() {
        let _ = primary_connection_limits();
        let _ = discovery_connection_limits();
    }

    #[test]
    fn silent_authentication_challenge_expires() {
        let issued = Instant::now();
        assert!(!auth_challenge_expired(issued, issued));
        assert!(auth_challenge_expired(issued, issued + AUTH_CHALLENGE_TIMEOUT));
    }
}
''')

node = Path('crates/swarm-network/src/node.rs')
text = node.read_text()
old_import = '''        application_connection_allowed, auth_challenge_expired, primary_connection_limits, AdmissionController,
        AUTH_CHALLENGE_TIMEOUT,
'''
new_import = '''        application_connection_allowed, auth_challenge_expired, primary_connection_limits, AdmissionController,
        RequestClass, AUTH_CHALLENGE_TIMEOUT,
'''
if old_import not in text:
    raise SystemExit('node admission import anchor missing')
text = text.replace(old_import, new_import, 1)
old_block = '''                                    let authenticated_request =
                                        self.authenticated.get(&peer).is_some_and(|(_, authenticated_connection)| {
                                            *authenticated_connection == connection_id
                                        });
                                    if !self.admission.admit_request(peer, authenticated_request, Instant::now()) {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: "RATE_LIMITED".into(),
                                                message: if authenticated_request {
                                                    "authenticated request budget exceeded; retry after the admission window".into()
                                                } else {
                                                    "pre-authentication request budget exceeded; reconnect later".into()
                                                },
                                            },
                                        );
                                        continue;
                                    }
'''
new_block = '''                                    let authenticated_request =
                                        self.authenticated.get(&peer).is_some_and(|(_, authenticated_connection)| {
                                            *authenticated_connection == connection_id
                                        });
                                    let request_class = if !authenticated_request {
                                        RequestClass::Unauthenticated
                                    } else if matches!(&request, WireRequest::BlobChunk { .. }) {
                                        RequestClass::AuthenticatedBulk
                                    } else {
                                        RequestClass::AuthenticatedControl
                                    };
                                    if !self.admission.admit_request(peer, request_class, Instant::now()) {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: "RATE_LIMITED".into(),
                                                message: match request_class {
                                                    RequestClass::Unauthenticated => {
                                                        "pre-authentication request budget exceeded; reconnect later".into()
                                                    }
                                                    RequestClass::AuthenticatedControl => {
                                                        "authenticated control request budget exceeded; retry after the admission window".into()
                                                    }
                                                    RequestClass::AuthenticatedBulk => {
                                                        "authenticated bulk-transfer request budget exceeded; retry after the admission window".into()
                                                    }
                                                },
                                            },
                                        );
                                        continue;
                                    }
'''
if old_block not in text:
    raise SystemExit('node admission block anchor missing')
node.write_text(text.replace(old_block, new_block, 1))

soak = Path('crates/swarm-network/tests/network_transfer_soak.rs')
text = soak.read_text()
old_response = '''                        NetworkEvent::OutboundFailure { request_id: observed, error, .. } if observed == request_id => {
                            panic!("blob chunk failed: {error}");
                        }
                        _ => {}
'''
new_response = '''                        NetworkEvent::Response {
                            request_id: observed,
                            response: WireResponse::Error { code, message },
                            ..
                        } if observed == request_id => {
                            panic!("blob chunk rejected: {code}: {message}");
                        }
                        NetworkEvent::OutboundFailure { request_id: observed, error, .. } if observed == request_id => {
                            panic!("blob chunk failed: {error}");
                        }
                        _ => {}
'''
if old_response not in text:
    raise SystemExit('soak response anchor missing')
soak.write_text(text.replace(old_response, new_response, 1))
