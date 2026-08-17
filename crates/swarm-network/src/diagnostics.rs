use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

pub const MAX_CONNECTIVITY_FAILURES: usize = 8;
pub const CONNECTIVITY_DIAGNOSTICS_SNAPSHOT_FILE: &str = "connectivity-diagnostics-v1.postcard";
pub const CONNECTIVITY_DIAGNOSTICS_JSON_ENV: &str = "SWARMCRAFT_CONNECTIVITY_DIAGNOSTICS_JSON";
const MAX_FAILURE_DETAIL_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NatStatusV1 {
    #[default]
    Unknown,
    Public,
    Private,
}

impl NatStatusV1 {
    pub const fn machine_name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HolePunchStateV1 {
    #[default]
    Idle,
    Attempting,
    Succeeded,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
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

impl ConnectivityStateV1 {
    pub const fn machine_name(self) -> &'static str {
        match self {
            Self::NatStatusUnknown => "nat_status_unknown",
            Self::DirectReachable => "direct_reachable",
            Self::HolePunched => "hole_punched",
            Self::RelayConnected => "relay_connected",
            Self::RelayRequired => "relay_required",
            Self::PrivateUnreachable => "private_unreachable",
            Self::BootstrapUnavailable => "bootstrap_unavailable",
            Self::NoViablePath => "no_viable_path",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
}

impl ConnectivityDiagnosticsV1 {
    pub fn record_local_address(&mut self, address: impl Into<String>) {
        let address = address.into();
        if !self.local_addresses.contains(&address) {
            self.local_addresses.push(address);
            self.local_addresses.sort();
        }
        self.refresh_address_families();
        self.persist_json_snapshot();
    }

    pub fn remove_local_address(&mut self, address: &str) {
        self.local_addresses.retain(|candidate| candidate != address);
        self.refresh_address_families();
        self.persist_json_snapshot();
    }

    pub fn record_observed_address(&mut self, address: impl Into<String>) {
        self.observed_public_address = Some(address.into());
        self.refresh_address_families();
        self.persist_json_snapshot();
    }

    pub fn record_nat_status(&mut self, status: NatStatusV1) {
        self.nat_status = status;
        self.refresh_state();
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

    pub fn record_relay_configured(&mut self, relay: impl Into<String>, count: usize) {
        self.selected_relay = Some(relay.into());
        self.relay_configured = count;
        self.refresh_state();
    }

    /// Replace current path state from exact live connection ownership.
    ///
    /// Direct and relay counts are established application paths. Bootstrap
    /// connectivity is infrastructure only and never implies gameplay reachability.
    pub fn record_active_paths(&mut self, direct_paths: usize, relay_paths: usize, bootstrap_paths: usize) {
        let had_direct_path = self.direct_connectivity;
        self.direct_connectivity = direct_paths > 0;
        self.relay_connectivity = relay_paths > 0;
        self.bootstrap_connectivity = bootstrap_paths > 0;
        if had_direct_path && !self.direct_connectivity && matches!(self.hole_punch, HolePunchStateV1::Succeeded) {
            self.hole_punch = HolePunchStateV1::Idle;
        }
        self.refresh_state();
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
                self.refresh_state();
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
            }
        }
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

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    fn refresh_address_families(&mut self) {
        self.ipv4_available = self.local_addresses.iter().any(|address| address.contains("/ip4/"))
            || self.observed_public_address.as_deref().is_some_and(|address| address.contains("/ip4/"));
        self.ipv6_available = self.local_addresses.iter().any(|address| address.contains("/ip6/"))
            || self.observed_public_address.as_deref().is_some_and(|address| address.contains("/ip6/"));
    }

    fn refresh_state(&mut self) {
        let last_path_issue = self.recent_failures.iter().rev().find_map(|issue| match issue.kind {
            ConnectivityIssueKindV1::RequestFailed => None,
            kind => Some(kind),
        });
        self.state = if self.direct_connectivity && matches!(self.hole_punch, HolePunchStateV1::Succeeded) {
            ConnectivityStateV1::HolePunched
        } else if self.direct_connectivity {
            ConnectivityStateV1::DirectReachable
        } else if self.relay_connectivity {
            ConnectivityStateV1::RelayConnected
        } else if last_path_issue == Some(ConnectivityIssueKindV1::NoViablePath) {
            ConnectivityStateV1::NoViablePath
        } else if self.relay_configured > 0
            && (self.nat_status == NatStatusV1::Private
                || matches!(
                    last_path_issue,
                    Some(ConnectivityIssueKindV1::DirectDialFailed | ConnectivityIssueKindV1::HolePunchFailed)
                ))
        {
            ConnectivityStateV1::RelayRequired
        } else if last_path_issue == Some(ConnectivityIssueKindV1::BootstrapUnavailable) {
            ConnectivityStateV1::BootstrapUnavailable
        } else if self.nat_status == NatStatusV1::Private {
            ConnectivityStateV1::PrivateUnreachable
        } else {
            // AutoNAT public reachability is a NAT fact, not ownership of a
            // current application connection. The UI can display nat_status
            // independently without resurrecting stale DirectReachable state.
            ConnectivityStateV1::NatStatusUnknown
        };
        self.persist_json_snapshot();
    }

    fn persist_json_snapshot(&self) {
        let Some(path) = env::var_os(CONNECTIVITY_DIAGNOSTICS_JSON_ENV).map(PathBuf::from) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = fs::write(path, bytes);
        }
    }
}

fn truncate_detail(detail: String) -> String {
    detail.chars().take(MAX_FAILURE_DETAIL_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

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

        diagnostics.remove_local_address("/ip6/::1/udp/4000/quic-v1");
        assert!(!diagnostics.ipv6_available);
    }

    #[test]
    fn private_nat_distinguishes_relay_required_from_connected() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_nat_status(NatStatusV1::Private);
        assert_eq!(diagnostics.state, ConnectivityStateV1::PrivateUnreachable);

        diagnostics.record_relay_configured("/dns4/relay.example/tcp/443/p2p/relay", 1);
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayRequired);

        diagnostics.record_active_paths(0, 1, 0);
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayConnected);
    }

    #[test]
    fn direct_path_lifecycle_clears_last_path_without_public_autonat() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_active_paths(1, 0, 0);
        assert_eq!(diagnostics.state, ConnectivityStateV1::DirectReachable);

        diagnostics.record_active_paths(0, 0, 0);
        assert!(!diagnostics.direct_connectivity);
        assert_ne!(diagnostics.state, ConnectivityStateV1::DirectReachable);
    }

    #[test]
    fn public_autonat_does_not_invent_an_active_direct_application_path() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_nat_status(NatStatusV1::Public);
        diagnostics.record_active_paths(1, 0, 0);
        diagnostics.record_active_paths(0, 0, 0);
        assert_eq!(diagnostics.nat_status, NatStatusV1::Public);
        assert!(!diagnostics.direct_connectivity);
        assert_ne!(diagnostics.state, ConnectivityStateV1::DirectReachable);
    }

    #[test]
    fn multiple_direct_paths_survive_single_close() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_active_paths(2, 0, 0);
        diagnostics.record_active_paths(1, 0, 0);
        assert!(diagnostics.direct_connectivity);
        assert_eq!(diagnostics.state, ConnectivityStateV1::DirectReachable);

        diagnostics.record_active_paths(0, 0, 0);
        assert!(!diagnostics.direct_connectivity);
        assert_ne!(diagnostics.state, ConnectivityStateV1::DirectReachable);
    }

    #[test]
    fn relay_path_lifecycle_clears_last_application_circuit() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_relay_configured("/dns4/relay.example/tcp/443/p2p/relay", 1);
        diagnostics.record_active_paths(0, 1, 0);
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayConnected);

        diagnostics.record_active_paths(0, 0, 0);
        assert!(!diagnostics.relay_connectivity);
        assert_ne!(diagnostics.state, ConnectivityStateV1::RelayConnected);
    }

    #[test]
    fn bootstrap_only_connectivity_does_not_imply_direct_reachability() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_bootstrap_configured(1);
        diagnostics.record_active_paths(0, 0, 1);
        assert!(diagnostics.bootstrap_connectivity);
        assert!(!diagnostics.direct_connectivity);
        assert_ne!(diagnostics.state, ConnectivityStateV1::DirectReachable);
    }

    #[test]
    fn request_failure_is_not_a_dial_failure_or_relay_trigger() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_relay_configured("/dns4/relay.example/tcp/443/p2p/relay", 1);
        diagnostics.record_issue(ConnectivityIssueV1 {
            kind: ConnectivityIssueKindV1::RequestFailed,
            peer: Some("peer-a".into()),
            address: None,
            detail: "request timed out on an established route".into(),
        });
        assert_eq!(diagnostics.recent_failures.last().unwrap().kind, ConnectivityIssueKindV1::RequestFailed);
        assert_ne!(diagnostics.state, ConnectivityStateV1::RelayRequired);
    }

    #[test]
    fn request_failure_does_not_overwrite_real_transport_fallback_state() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_relay_configured("/dns4/relay.example/tcp/443/p2p/relay", 1);
        diagnostics.record_direct_failure("transport dial failed");
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayRequired);
        diagnostics.record_issue(ConnectivityIssueV1 {
            kind: ConnectivityIssueKindV1::RequestFailed,
            peer: None,
            address: None,
            detail: "later request failed".into(),
        });
        assert_eq!(diagnostics.state, ConnectivityStateV1::RelayRequired);
    }

    #[test]
    fn hole_punch_success_requires_a_current_direct_path() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.start_hole_punch();
        diagnostics.finish_hole_punch(Ok::<(), String>(()));
        assert_ne!(diagnostics.state, ConnectivityStateV1::HolePunched);

        diagnostics.record_active_paths(1, 0, 0);
        assert_eq!(diagnostics.state, ConnectivityStateV1::HolePunched);

        diagnostics.record_active_paths(0, 0, 0);
        assert_ne!(diagnostics.state, ConnectivityStateV1::HolePunched);
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
    fn json_roundtrip_uses_stable_snake_case_machine_values() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_nat_status(NatStatusV1::Public);
        diagnostics.record_local_address("/ip4/127.0.0.1/udp/4000/quic-v1");
        diagnostics.record_issue(ConnectivityIssueV1 {
            kind: ConnectivityIssueKindV1::RequestFailed,
            peer: Some("peer-a".into()),
            address: None,
            detail: "request timed out".into(),
        });

        let json = diagnostics.to_json().unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["state"], "nat_status_unknown");
        assert_eq!(value["nat_status"], "public");
        assert_eq!(value["recent_failures"][0]["kind"], "request_failed");
        assert_eq!(value["ipv4_available"], true);
        let decoded: ConnectivityDiagnosticsV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, diagnostics);

        let stable_states = [
            (ConnectivityStateV1::NatStatusUnknown, "nat_status_unknown"),
            (ConnectivityStateV1::DirectReachable, "direct_reachable"),
            (ConnectivityStateV1::HolePunched, "hole_punched"),
            (ConnectivityStateV1::RelayConnected, "relay_connected"),
            (ConnectivityStateV1::RelayRequired, "relay_required"),
            (ConnectivityStateV1::PrivateUnreachable, "private_unreachable"),
            (ConnectivityStateV1::BootstrapUnavailable, "bootstrap_unavailable"),
            (ConnectivityStateV1::NoViablePath, "no_viable_path"),
        ];
        for (state, expected) in stable_states {
            assert_eq!(state.machine_name(), expected);
            assert_eq!(serde_json::to_string(&state).unwrap(), format!("\"{expected}\""));
        }
    }
}
