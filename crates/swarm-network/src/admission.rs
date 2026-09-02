use libp2p::PeerId as TransportPeerId;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

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
