use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConnectivityDiagnosticsV1 {
    pub local_addresses: Vec<String>,
    pub observed_public_address: Option<String>,
    pub nat_status: NatStatusV1,
    pub direct_connectivity: bool,
    pub relay_connectivity: bool,
    pub hole_punch: HolePunchStateV1,
    pub selected_relay: Option<String>,
    pub last_failure: Option<String>,
}

impl ConnectivityDiagnosticsV1 {
    pub fn record_local_address(&mut self, address: impl Into<String>) {
        let address = address.into();
        if !self.local_addresses.contains(&address) {
            self.local_addresses.push(address);
            self.local_addresses.sort();
        }
    }

    pub fn record_observed_address(&mut self, address: impl Into<String>) {
        self.observed_public_address = Some(address.into());
    }

    pub fn record_direct_success(&mut self) {
        self.direct_connectivity = true;
        self.last_failure = None;
    }

    pub fn record_direct_failure(&mut self, reason: impl Into<String>) {
        self.last_failure = Some(reason.into());
    }

    pub fn record_relay(&mut self, relay: impl Into<String>) {
        self.relay_connectivity = true;
        self.selected_relay = Some(relay.into());
    }

    pub fn start_hole_punch(&mut self) {
        self.hole_punch = HolePunchStateV1::Attempting;
    }

    pub fn finish_hole_punch(&mut self, result: Result<(), impl Into<String>>) {
        self.hole_punch = match result {
            Ok(()) => HolePunchStateV1::Succeeded,
            Err(reason) => HolePunchStateV1::Failed(reason.into()),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_are_deduplicated_and_stable() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_local_address("/ip4/192.168.1.2/udp/4000/quic-v1");
        diagnostics.record_local_address("/ip6/::1/udp/4000/quic-v1");
        diagnostics.record_local_address("/ip4/192.168.1.2/udp/4000/quic-v1");
        assert_eq!(diagnostics.local_addresses.len(), 2);
        assert!(diagnostics.local_addresses[0] < diagnostics.local_addresses[1]);
    }

    #[test]
    fn fallback_diagnostics_preserve_failure_reason() {
        let mut diagnostics = ConnectivityDiagnosticsV1::default();
        diagnostics.record_direct_failure("direct QUIC timeout");
        diagnostics.start_hole_punch();
        diagnostics.finish_hole_punch(Err("remote did not confirm DCUtR"));
        diagnostics.record_relay("/dns4/relay.example/tcp/443/wss");
        assert!(!diagnostics.direct_connectivity);
        assert!(diagnostics.relay_connectivity);
        assert!(matches!(diagnostics.hole_punch, HolePunchStateV1::Failed(_)));
        assert_eq!(diagnostics.last_failure.as_deref(), Some("direct QUIC timeout"));
    }
}
