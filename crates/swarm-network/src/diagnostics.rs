use serde::{Deserialize, Serialize};

pub const MAX_CONNECTIVITY_FAILURES: usize = 8;
const MAX_FAILURE_DETAIL_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NatStatusV1 {
    #[default]
    Unknown,
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HolePunchStateV1 {
    #[default]
    Idle,
    Attempting,
    Succeeded,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConnectivityStateV1 {
    #[default]
    NatStatusUnknown,
    DirectReachable,
    HolePunched,
    RelayConnected,
    RelayRequired,
    PrivateUnreachable,
    BootstrapUnavailable,
    NoViablePath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectivityIssueKindV1 {
    DirectDialFailed,
    RequestFailed,
    HolePunchFailed,
    RelayUnavailable,
    BootstrapUnavailable,
    InvalidAddress,
    NoViablePath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectivityIssueV1 {
    pub kind: ConnectivityIssueKindV1,
    pub peer: Option<String>,
    pub address: Option<String>,
    pub detail: String,
}

/// Serializable connectivity contract for CLI/Desktop consumers.
///
/// `state` describes the currently owned application path, while `nat_status`
/// independently describes AutoNAT reachability. Infrastructure connections are
/// tracked separately and never make an application path look direct/relayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ConnectivityDiagnosticsV1 {
    pub local_addresses: Vec<String>,
    pub observed_public_address: Option<String>,
    pub nat_status: NatStatusV1,
    pub direct_connectivity: bool,
    pub relay_connectivity: bool,
    pub hole_punch: HolePunchStateV1,
    pub selected_relay: Option<String>,
    pub last_failure: Option<String>,
    pub state: ConnectivityStateV1,
    pub ipv4_available: bool,
    pub ipv6_available: bool,
    pub bootstrap_configured: usize,
    pub bootstrap_connectivity: bool,
    pub relay_configured: usize,
    pub recent_failures: Vec<ConnectivityIssueV1>,
    #[serde(skip)]
    active_direct_paths: usize,
    #[serde(skip)]
    active_relay_paths: usize,
    #[serde(skip)]
    active_bootstrap_paths: usize,
}

impl ConnectivityDiagnosticsV1 {
    pub fn record_local_address(&mut self, address: impl Into<String>) {
        let address = address.into();
        self.record_address_family(&address);
        if !self.local_addresses.contains(&address) {
            self.local_addresses.push(address);
            self.local_addresses.sort();
        }
    }

    pub fn record_observed_address(&mut self, address: impl Into<String>) {
        let address = address.into();
        self.record_address_family(&address);
        self.observed_public_address = Some(address);
    }

    pub fn record_nat_status(&mut self, status: NatStatusV1) {
        self.nat_status = status;
        self.refresh_state();
    }

    pub fn record_direct_path_up(&mut self) {
        self.active_direct_paths = self.active_direct_paths.saturating_add(1);
        self.direct_connectivity = self.active_direct_paths > 0;
        self.last_failure = None;
        self.refresh_state();
    }

    pub fn record_direct_path_down(&mut self) {
        self.active_direct_paths = self.active_direct_paths.saturating_sub(1);
        self.direct_connectivity = self.active_direct_paths > 0;
        if !self.direct_connectivity && matches!(self.hole_punch, HolePunchStateV1::Succeeded) {
            self.hole_punch = HolePunchStateV1::Idle;
        }
        self.refresh_state();
    }

    pub fn record_direct_success(&mut self) {
        self.record_direct_path_up();
    }

    pub fn record_direct_failure(&mut self, reason: impl Into<String>) {
        self.record_issue(ConnectivityIssueV1 {
            kind: ConnectivityIssueKindV1::DirectDialFailed,
            peer: None,
            address: None,
            detail: reason.into(),
        });
    }

    pub fn record_bootstrap_configured(&mut self, count: usize) {
        self.bootstrap_configured = count;
        self.refresh_state();
    }

    pub fn record_bootstrap_path_up(&mut self) {
        self.active_bootstrap_paths = self.active_bootstrap_paths.saturating_add(1);
        self.bootstrap_connectivity = self.active_bootstrap_paths > 0;
        self.refresh_state();
    }

    pub fn record_bootstrap_path_down(&mut self) {
        self.active_bootstrap_paths = self.active_bootstrap_paths.saturating_sub(1);
        self.bootstrap_connectivity = self.active_bootstrap_paths > 0;
        self.refresh_state();
    }

    pub fn record_bootstrap_connected(&mut self) {
        self.record_bootstrap_path_up();
    }

    pub fn record_relay_configured(&mut self, relay: impl Into<String>, count: usize) {
        self.selected_relay = Some(relay.into());
        self.relay_configured = count;
        self.refresh_state();
    }

    pub fn record_relay_path_up(&mut self) {
        self.active_relay_paths = self.active_relay_paths.saturating_add(1);
        self.relay_connectivity = self.active_relay_paths > 0;
        self.refresh_state();
    }

    pub fn record_relay_path_down(&mut self) {
        self.active_relay_paths = self.active_relay_paths.saturating_sub(1);
        self.relay_connectivity = self.active_relay_paths > 0;
        self.refresh_state();
    }

    pub fn record_relay_connected(&mut self) {
        self.record_relay_path_up();
    }

    pub fn record_relay(&mut self, relay: impl Into<String>) {
        self.selected_relay = Some(relay.into());
        self.record_relay_path_up();
    }

    pub fn start_hole_punch(&mut self) {
        self.hole_punch = HolePunchStateV1::Attempting;
        self.refresh_state();
    }

    pub fn finish_hole_punch(&mut self, result: Result<(), impl Into<String>>) {
        match result {
            Ok(()) => {
                self.hole_punch = HolePunchStateV1::Succeeded;
                self.last_failure = None;
            }
            Err(reason) => {
                let detail = truncate_detail(reason.into());
                self.hole_punch = HolePunchStateV1::Failed(detail.clone());
                self.record_issue(ConnectivityIssueV1 {
                    kind: ConnectivityIssueKindV1::HolePunchFailed,
                    peer: None,
                    address: None,
                    detail,
                });
                return;
            }
        }
        self.refresh_state();
    }

    pub fn record_issue(&mut self, mut issue: ConnectivityIssueV1) {
        issue.detail = truncate_detail(issue.detail);
        self.last_failure = Some(issue.detail.clone());
        self.recent_failures.push(issue);
        if self.recent_failures.len() > MAX_CONNECTIVITY_FAILURES {
            let excess = self.recent_failures.len() - MAX_CONNECTIVITY_FAILURES;
            self.recent_failures.drain(0..excess);
        }
        self.refresh_state();
    }

    pub fn record_no_viable_path(&mut self, detail: impl Into<String>) {
        self.record_issue(ConnectivityIssueV1 {
            kind: ConnectivityIssueKindV1::NoViablePath,
            peer: None,
            address: None,
            detail: detail.into(),
        });
    }

    fn record_address_family(&mut self, address: &str) {
        self.ipv4_available |= address.contains("/ip4/");
        self.ipv6_available |= address.contains("/ip6/");
    }

    fn refresh_state(&mut self) {
        let last_issue = self.recent_failures.last().map(|issue| issue.kind);
        self.state = if self.direct_connectivity && matches!(self.hole_punch, HolePunchStateV1::Succeeded) {
            ConnectivityStateV1::HolePunched
        } else if self.direct_connectivity {
            ConnectivityStateV1::DirectReachable
        } else if self.relay_connectivity {
            ConnectivityStateV1::RelayConnected
        } else if last_issue == Some(ConnectivityIssueKindV1::NoViablePath) {
            ConnectivityStateV1::NoViablePath
        } else if self.relay_configured > 0
            && (self.nat_status == NatStatusV1::Private
                || matches!(
                    last_issue,
                    Some(ConnectivityIssueKindV1::DirectDialFailed | ConnectivityIssueKindV1::HolePunchFailed)
                ))
        {
            ConnectivityStateV1::RelayRequired
        } else if last_issue == Some(ConnectivityIssueKindV1::BootstrapUnavailable) {
            ConnectivityStateV1::BootstrapUnavailable
        } else if self.nat_status == NatStatusV1::Private {
            ConnectivityStateV1::PrivateUnreachable
        } else {
            // AutoNAT Public is intentionally represented in `nat_status`, not as
            // an active DirectReachable application connection.
            ConnectivityStateV1::NatStatusUnknown
        };
    }
}

fn truncate_detail(detail: String) -> String {
    detail.chars().take(MAX_FAILURE_DETAIL_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_are_deduplicated_stable_and_report_ip_families() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_local_address("/ip4/192.168.1.2/udp/4000/quic-v1");
        diagnostics.record_local_address("/ip6/::1/udp/4000/quic-v1");
        diagnostics.record_local_address("/ip4/192.168.1.2/udp/4000/quic-v1");
        assert_eq!(diagnostics.local_addresses.len(), 2);
        assert!(diagnostics.local_addresses[0] < diagnostics.local_addresses[1]);
        assert!(diagnostics.ipv4_available);
        assert!(diagnostics.ipv6_available);
    }

    #[test]
    fn direct_path_state_tracks_current_ownership() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_direct_path_up();
        diagnostics.record_direct_path_up();
        assert_eq!(diagnostics.state, ConnectivityStateV1::DirectReachable);
        diagnostics.record_direct_path_down();
        assert_eq!(diagnostics.state, ConnectivityStateV1::DirectReachable);
        diagnostics.record_direct_path_down();
        assert!(!diagnostics.direct_connectivity);
        assert_ne!(diagnostics.state, ConnectivityStateV1::DirectReachable);
    }

    #[test]
    fn relay_path_state_tracks_current_ownership() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_relay_path_up();
        diagnostics.record_relay_path_up();
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayConnected);
        diagnostics.record_relay_path_down();
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayConnected);
        diagnostics.record_relay_path_down();
        assert!(!diagnostics.relay_connectivity);
        assert_ne!(diagnostics.state, ConnectivityStateV1::RelayConnected);
    }

    #[test]
    fn infrastructure_paths_do_not_create_application_reachability() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_bootstrap_configured(1);
        diagnostics.record_bootstrap_path_up();
        assert!(diagnostics.bootstrap_connectivity);
        assert!(!diagnostics.direct_connectivity);
        assert!(!diagnostics.relay_connectivity);
        assert_eq!(diagnostics.state, ConnectivityStateV1::NatStatusUnknown);
        diagnostics.record_bootstrap_path_down();
        assert!(!diagnostics.bootstrap_connectivity);
    }

    #[test]
    fn fallback_diagnostics_preserve_path_failures() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_direct_failure("direct QUIC timeout");
        diagnostics.start_hole_punch();
        diagnostics.finish_hole_punch(Err("remote did not confirm DCUtR"));
        diagnostics.record_relay("/dns4/relay.example/tcp/443/wss");
        assert!(diagnostics.relay_connectivity);
        assert!(matches!(diagnostics.hole_punch, HolePunchStateV1::Failed(_)));
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayConnected);
        assert_eq!(diagnostics.recent_failures.len(), 2);
        assert_eq!(diagnostics.last_failure.as_deref(), Some("remote did not confirm DCUtR"));
    }

    #[test]
    fn private_nat_distinguishes_relay_required_from_unreachable() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_nat_status(NatStatusV1::Private);
        assert_eq!(diagnostics.state, ConnectivityStateV1::PrivateUnreachable);

        diagnostics.record_relay_configured("/dns4/relay.example/tcp/443/p2p/relay", 1);
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayRequired);

        diagnostics.record_relay_path_up();
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayConnected);
    }

    #[test]
    fn direct_failure_with_configured_relay_requires_fallback_even_before_nat_classification() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_relay_configured("/dns4/relay.example/tcp/443/p2p/relay", 1);
        diagnostics.record_direct_failure("direct address timed out");
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayRequired);
    }

    #[test]
    fn public_autonat_result_does_not_invent_an_active_direct_path() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_nat_status(NatStatusV1::Public);
        assert_eq!(diagnostics.nat_status, NatStatusV1::Public);
        assert!(!diagnostics.direct_connectivity);
        assert_ne!(diagnostics.state, ConnectivityStateV1::DirectReachable);
    }

    #[test]
    fn hole_punch_success_is_distinct_while_direct_path_is_live() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_direct_path_up();
        assert_eq!(diagnostics.state, ConnectivityStateV1::DirectReachable);

        diagnostics.start_hole_punch();
        diagnostics.finish_hole_punch(Ok::<(), String>(()));
        assert_eq!(diagnostics.state, ConnectivityStateV1::HolePunched);
        diagnostics.record_direct_path_down();
        assert_ne!(diagnostics.state, ConnectivityStateV1::HolePunched);
    }

    #[test]
    fn request_failures_have_their_own_typed_classification() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_issue(ConnectivityIssueV1 {
            kind: ConnectivityIssueKindV1::RequestFailed,
            peer: Some("peer-a".into()),
            address: None,
            detail: "request timed out".into(),
        });
        assert_eq!(diagnostics.recent_failures.last().map(|issue| issue.kind), Some(ConnectivityIssueKindV1::RequestFailed));
    }

    #[test]
    fn failure_history_is_bounded() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        for index in 0..(MAX_CONNECTIVITY_FAILURES + 5) {
            diagnostics.record_issue(ConnectivityIssueV1 {
                kind: ConnectivityIssueKindV1::DirectDialFailed,
                peer: Some(format!("peer-{index}")),
                address: None,
                detail: format!("failure-{index}"),
            });
        }

        assert_eq!(diagnostics.recent_failures.len(), MAX_CONNECTIVITY_FAILURES);
        assert_eq!(diagnostics.recent_failures[0].peer.as_deref(), Some("peer-5"));
    }

    #[test]
    fn explicit_terminal_failures_have_actionable_states() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_issue(ConnectivityIssueV1 {
            kind: ConnectivityIssueKindV1::BootstrapUnavailable,
            peer: None,
            address: Some("/dns4/bootstrap.example/udp/4001/quic-v1".into()),
            detail: "bootstrap dial failed".into(),
        });
        assert_eq!(diagnostics.state, ConnectivityStateV1::BootstrapUnavailable);

        diagnostics.record_no_viable_path("direct and relay paths both failed");
        assert_eq!(diagnostics.state, ConnectivityStateV1::NoViablePath);
    }
}
