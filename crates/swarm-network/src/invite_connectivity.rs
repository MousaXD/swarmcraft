use crate::{ConnectivityDiagnosticsV1, NatStatusV1};
use libp2p::{multiaddr::Protocol, Multiaddr};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    net::{Ipv4Addr, Ipv6Addr},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

pub const MAX_INVITE_ADDRESSES: usize = 8;
pub const MAX_INVITE_ADDRESS_CHARS: usize = 512;
pub const MAX_CONNECTIVITY_SNAPSHOT_BYTES: u64 = 64 * 1024;
pub const DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE: &str = "connectivity-diagnostics-v1.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteReachabilityV1 {
    PublicDirect,
    Relay,
    LocalNetwork,
    Unavailable,
}

impl InviteReachabilityV1 {
    pub const fn internet_usable(self) -> bool {
        matches!(self, Self::PublicDirect | Self::Relay)
    }

    pub const fn machine_name(self) -> &'static str {
        match self {
            Self::PublicDirect => "public_direct",
            Self::Relay => "relay",
            Self::LocalNetwork => "local_network",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteConnectivityV1 {
    pub shareable_addresses: Vec<String>,
    pub reachability: InviteReachabilityV1,
    pub observed_unix_ms: u64,
    pub relay_available: bool,
    pub internet_usable: bool,
    pub diagnostic_reason: String,
}

impl InviteConnectivityV1 {
    pub fn from_diagnostics(diagnostics: &ConnectivityDiagnosticsV1, observed_unix_ms: u64) -> Self {
        let mut public_direct = Vec::new();
        let mut relay = Vec::new();
        let mut local = Vec::new();

        // A historical observed address is not enough. AutoNAT must still say
        // Public at the time of this snapshot before it can be advertised as a
        // public direct invite path.
        if diagnostics.nat_status == NatStatusV1::Public {
            if let Some(address) = diagnostics.observed_public_address.as_deref() {
                if let Ok(validated) = validate_address(address) {
                    if validated.kind == AddressKind::PublicDirect {
                        public_direct.push(validated.canonical);
                    }
                }
            }
        }

        // NewListenAddr / ExpiredListenAddr maintain local_addresses as the
        // current listener set. A relay reservation is represented by a full
        // /p2p-circuit/... destination address. Merely having a relay configured
        // or an infrastructure connection is intentionally not enough.
        for address in &diagnostics.local_addresses {
            let Ok(validated) = validate_address(address) else { continue };
            match validated.kind {
                AddressKind::Relay => relay.push(validated.canonical),
                AddressKind::PublicDirect if diagnostics.nat_status == NatStatusV1::Public => {
                    public_direct.push(validated.canonical)
                }
                AddressKind::LocalDirect => local.push(validated.canonical),
                AddressKind::PublicDirect => {}
            }
        }

        stable_dedup(&mut public_direct);
        stable_dedup(&mut relay);
        stable_dedup(&mut local);

        let relay_available = !relay.is_empty();
        let reachability = if !public_direct.is_empty() {
            InviteReachabilityV1::PublicDirect
        } else if relay_available {
            InviteReachabilityV1::Relay
        } else if !local.is_empty() {
            InviteReachabilityV1::LocalNetwork
        } else {
            InviteReachabilityV1::Unavailable
        };

        let mut shareable_addresses = Vec::new();
        for address in public_direct.into_iter().chain(relay).chain(local) {
            if shareable_addresses.len() == MAX_INVITE_ADDRESSES {
                break;
            }
            if !shareable_addresses.contains(&address) {
                shareable_addresses.push(address);
            }
        }

        let diagnostic_reason = match reachability {
            InviteReachabilityV1::PublicDirect if relay_available => {
                "AutoNAT currently proves a public direct address; an active relay reservation is also included as fallback."
                    .to_owned()
            }
            InviteReachabilityV1::PublicDirect => {
                "AutoNAT currently proves a public direct address for this device.".to_owned()
            }
            InviteReachabilityV1::Relay => {
                "No public direct address is currently proven, but an active relay reservation provides an inbound circuit path."
                    .to_owned()
            }
            InviteReachabilityV1::LocalNetwork => {
                "Only private local-network listener addresses are currently shareable. The invite can be useful on the same network, but internet reachability is not proven; configure a relay or restore public AutoNAT reachability for remote friends."
                    .to_owned()
            }
            InviteReachabilityV1::Unavailable => {
                "This device currently has no proven public direct address, active relay reservation, or usable local-network listener. Keep networking running and configure a relay or restore reachable listening addresses."
                    .to_owned()
            }
        };

        Self {
            shareable_addresses,
            reachability,
            observed_unix_ms,
            relay_available,
            internet_usable: reachability.internet_usable(),
            diagnostic_reason,
        }
    }
}

#[derive(Debug, Error)]
pub enum InviteConnectivityError {
    #[error("invite contains {actual} connection hints; at most {maximum} are allowed")]
    TooManyAddresses { actual: usize, maximum: usize },
    #[error("invite connection hint is empty")]
    EmptyAddress,
    #[error("invite connection hint is too long ({actual} characters; maximum {maximum})")]
    AddressTooLong { actual: usize, maximum: usize },
    #[error("invite connection hint is not a valid libp2p multiaddress: {0}")]
    InvalidAddress(String),
    #[error("invite connection hint is not remotely usable: {0}")]
    UnusableAddress(String),
    #[error("connectivity diagnostics snapshot is unavailable: {0}")]
    SnapshotUnavailable(String),
    #[error("connectivity diagnostics snapshot is too large ({actual} bytes; maximum {maximum})")]
    SnapshotTooLarge { actual: u64, maximum: u64 },
    #[error("connectivity diagnostics snapshot is invalid: {0}")]
    SnapshotInvalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressKind {
    PublicDirect,
    LocalDirect,
    Relay,
}

#[derive(Debug)]
struct ValidatedAddress {
    canonical: String,
    kind: AddressKind,
}

/// Validate, canonicalize, deduplicate, and bound signed invite connection hints.
///
/// These are dial candidates only. Successfully dialing one never grants world
/// membership or authority; the signed PeerHello and canonical membership flow
/// remain independent authorization gates.
pub fn validate_invite_addresses(addresses: &[String]) -> Result<Vec<String>, InviteConnectivityError> {
    if addresses.len() > MAX_INVITE_ADDRESSES {
        return Err(InviteConnectivityError::TooManyAddresses {
            actual: addresses.len(),
            maximum: MAX_INVITE_ADDRESSES,
        });
    }

    let mut canonical = Vec::with_capacity(addresses.len());
    for address in addresses {
        let validated = validate_address(address)?;
        if !canonical.contains(&validated.canonical) {
            canonical.push(validated.canonical);
        }
    }
    Ok(canonical)
}

pub fn invite_connectivity_from_snapshot(path: &Path) -> Result<InviteConnectivityV1, InviteConnectivityError> {
    let metadata = fs::metadata(path).map_err(|error| InviteConnectivityError::SnapshotUnavailable(error.to_string()))?;
    if metadata.len() > MAX_CONNECTIVITY_SNAPSHOT_BYTES {
        return Err(InviteConnectivityError::SnapshotTooLarge {
            actual: metadata.len(),
            maximum: MAX_CONNECTIVITY_SNAPSHOT_BYTES,
        });
    }
    let bytes = fs::read(path).map_err(|error| InviteConnectivityError::SnapshotUnavailable(error.to_string()))?;
    let diagnostics: ConnectivityDiagnosticsV1 =
        serde_json::from_slice(&bytes).map_err(|error| InviteConnectivityError::SnapshotInvalid(error.to_string()))?;
    let observed_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_else(unix_time_ms);
    Ok(InviteConnectivityV1::from_diagnostics(&diagnostics, observed_unix_ms))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn stable_dedup(addresses: &mut Vec<String>) {
    addresses.sort();
    let mut seen = HashSet::new();
    addresses.retain(|address| seen.insert(address.clone()));
}

fn validate_address(value: &str) -> Result<ValidatedAddress, InviteConnectivityError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(InviteConnectivityError::EmptyAddress);
    }
    let char_count = value.chars().count();
    if char_count > MAX_INVITE_ADDRESS_CHARS {
        return Err(InviteConnectivityError::AddressTooLong {
            actual: char_count,
            maximum: MAX_INVITE_ADDRESS_CHARS,
        });
    }

    let address: Multiaddr = value
        .parse()
        .map_err(|_| InviteConnectivityError::InvalidAddress(value.to_owned()))?;

    let mut has_host = false;
    let mut has_tcp = false;
    let mut has_udp = false;
    let mut has_quic = false;
    let mut saw_circuit = false;
    let mut peer_before_circuit = false;
    let mut peer_after_circuit = false;
    let mut local_scope = false;

    for protocol in address.iter() {
        match protocol {
            Protocol::Ip4(ip) => {
                has_host = true;
                match ipv4_scope(ip)? {
                    IpScope::Public => {}
                    IpScope::Local => local_scope = true,
                }
            }
            Protocol::Ip6(ip) => {
                has_host = true;
                match ipv6_scope(ip)? {
                    IpScope::Public => {}
                    IpScope::Local => local_scope = true,
                }
            }
            Protocol::Dns(name) | Protocol::Dns4(name) | Protocol::Dns6(name) => {
                has_host = true;
                let name = name.as_ref();
                if name.eq_ignore_ascii_case("localhost") || name.to_ascii_lowercase().ends_with(".local") {
                    return Err(InviteConnectivityError::UnusableAddress(value.to_owned()));
                }
            }
            Protocol::Tcp(_) => has_tcp = true,
            Protocol::Udp(_) => has_udp = true,
            Protocol::QuicV1 => has_quic = true,
            Protocol::P2pCircuit => saw_circuit = true,
            Protocol::P2p(_) if saw_circuit => peer_after_circuit = true,
            Protocol::P2p(_) => peer_before_circuit = true,
            _ => {}
        }
    }

    let transport_usable = has_tcp || (has_udp && has_quic);
    if !has_host || !transport_usable {
        return Err(InviteConnectivityError::UnusableAddress(value.to_owned()));
    }

    let kind = if saw_circuit {
        // A concrete relay hint must name both the relay peer and destination
        // peer. /p2p-circuit alone is only a transport intent, not a remotely
        // dialable address for this inviter.
        if !peer_before_circuit || !peer_after_circuit {
            return Err(InviteConnectivityError::UnusableAddress(value.to_owned()));
        }
        AddressKind::Relay
    } else if local_scope {
        AddressKind::LocalDirect
    } else {
        AddressKind::PublicDirect
    };

    Ok(ValidatedAddress { canonical: address.to_string(), kind })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IpScope {
    Public,
    Local,
}

fn ipv4_scope(ip: Ipv4Addr) -> Result<IpScope, InviteConnectivityError> {
    let octets = ip.octets();
    if ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || octets[0] == 0
        || octets[0] >= 240
        || (octets[0] == 198 && matches!(octets[1], 18 | 19))
    {
        return Err(InviteConnectivityError::UnusableAddress(ip.to_string()));
    }

    if ip.is_private() || (octets[0] == 100 && (64..=127).contains(&octets[1])) {
        Ok(IpScope::Local)
    } else {
        Ok(IpScope::Public)
    }
}

fn ipv6_scope(ip: Ipv6Addr) -> Result<IpScope, InviteConnectivityError> {
    let segments = ip.segments();
    if ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unicast_link_local()
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
    {
        return Err(InviteConnectivityError::UnusableAddress(ip.to_string()));
    }

    if ip.is_unique_local() {
        Ok(IpScope::Local)
    } else {
        Ok(IpScope::Public)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectivityStateV1, NatStatusV1};
    use libp2p::identity::Keypair;

    fn peer() -> String {
        Keypair::generate_ed25519().public().to_peer_id().to_string()
    }

    fn relay_address() -> String {
        format!(
            "/dns4/relay.example/tcp/443/p2p/{}/p2p-circuit/p2p/{}",
            peer(),
            peer()
        )
    }

    #[test]
    fn public_autonat_address_is_preferred_for_direct_invites() {
        let diagnostics = ConnectivityDiagnosticsV1 {
            observed_public_address: Some("/ip4/1.1.1.1/udp/41000/quic-v1".into()),
            nat_status: NatStatusV1::Public,
            state: ConnectivityStateV1::NatStatusUnknown,
            ..Default::default()
        };
        let connectivity = InviteConnectivityV1::from_diagnostics(&diagnostics, 7);
        assert_eq!(connectivity.reachability, InviteReachabilityV1::PublicDirect);
        assert!(connectivity.internet_usable);
        assert_eq!(connectivity.shareable_addresses, vec!["/ip4/1.1.1.1/udp/41000/quic-v1"]);
    }

    #[test]
    fn active_relay_listener_is_shareable_even_without_relay_application_traffic() {
        let relay = relay_address();
        let diagnostics = ConnectivityDiagnosticsV1 {
            local_addresses: vec![relay.clone()],
            nat_status: NatStatusV1::Private,
            relay_connectivity: false,
            selected_relay: Some("/dns4/relay.example/tcp/443".into()),
            ..Default::default()
        };
        let connectivity = InviteConnectivityV1::from_diagnostics(&diagnostics, 8);
        assert_eq!(connectivity.reachability, InviteReachabilityV1::Relay);
        assert!(connectivity.relay_available);
        assert_eq!(connectivity.shareable_addresses, vec![relay]);
    }

    #[test]
    fn multiple_addresses_are_deterministic_and_bounded() {
        let relay = relay_address();
        let diagnostics = ConnectivityDiagnosticsV1 {
            local_addresses: vec![
                "/ip4/10.10.0.7/udp/4000/quic-v1".into(),
                relay.clone(),
                "/ip4/8.8.8.8/udp/4000/quic-v1".into(),
            ],
            observed_public_address: Some("/ip4/1.1.1.1/udp/4000/quic-v1".into()),
            nat_status: NatStatusV1::Public,
            ..Default::default()
        };
        let connectivity = InviteConnectivityV1::from_diagnostics(&diagnostics, 9);
        assert_eq!(connectivity.reachability, InviteReachabilityV1::PublicDirect);
        assert_eq!(connectivity.shareable_addresses.len(), 4);
        assert_eq!(connectivity.shareable_addresses[0], "/ip4/1.1.1.1/udp/4000/quic-v1");
        assert_eq!(connectivity.shareable_addresses[1], "/ip4/8.8.8.8/udp/4000/quic-v1");
        assert_eq!(connectivity.shareable_addresses[2], relay);
        assert_eq!(connectivity.shareable_addresses[3], "/ip4/10.10.0.7/udp/4000/quic-v1");
    }

    #[test]
    fn loopback_and_link_local_addresses_are_filtered() {
        let diagnostics = ConnectivityDiagnosticsV1 {
            local_addresses: vec![
                "/ip4/127.0.0.1/udp/4000/quic-v1".into(),
                "/ip4/169.254.4.2/udp/4000/quic-v1".into(),
                "/ip6/::1/udp/4000/quic-v1".into(),
            ],
            ..Default::default()
        };
        let connectivity = InviteConnectivityV1::from_diagnostics(&diagnostics, 10);
        assert_eq!(connectivity.reachability, InviteReachabilityV1::Unavailable);
        assert!(connectivity.shareable_addresses.is_empty());
    }

    #[test]
    fn stale_public_observation_is_ignored_after_nat_is_no_longer_public() {
        let diagnostics = ConnectivityDiagnosticsV1 {
            observed_public_address: Some("/ip4/1.1.1.1/udp/4000/quic-v1".into()),
            nat_status: NatStatusV1::Private,
            direct_connectivity: false,
            ..Default::default()
        };
        let connectivity = InviteConnectivityV1::from_diagnostics(&diagnostics, 11);
        assert_eq!(connectivity.reachability, InviteReachabilityV1::Unavailable);
        assert!(connectivity.shareable_addresses.is_empty());
    }

    #[test]
    fn configured_relay_without_current_circuit_listener_is_not_advertised() {
        let diagnostics = ConnectivityDiagnosticsV1 {
            nat_status: NatStatusV1::Private,
            selected_relay: Some(format!("/dns4/relay.example/tcp/443/p2p/{}", peer())),
            relay_configured: 1,
            relay_connectivity: false,
            ..Default::default()
        };
        let connectivity = InviteConnectivityV1::from_diagnostics(&diagnostics, 12);
        assert!(!connectivity.relay_available);
        assert_eq!(connectivity.reachability, InviteReachabilityV1::Unavailable);
    }

    #[test]
    fn bootstrap_infrastructure_never_becomes_an_inviter_endpoint() {
        let diagnostics = ConnectivityDiagnosticsV1 {
            bootstrap_configured: 1,
            bootstrap_connectivity: true,
            ..Default::default()
        };
        let connectivity = InviteConnectivityV1::from_diagnostics(&diagnostics, 13);
        assert_eq!(connectivity.reachability, InviteReachabilityV1::Unavailable);
        assert!(connectivity.shareable_addresses.is_empty());
    }

    #[test]
    fn private_lan_listener_is_explicitly_local_only() {
        let diagnostics = ConnectivityDiagnosticsV1 {
            local_addresses: vec!["/ip4/192.168.50.7/udp/4000/quic-v1".into()],
            ..Default::default()
        };
        let connectivity = InviteConnectivityV1::from_diagnostics(&diagnostics, 14);
        assert_eq!(connectivity.reachability, InviteReachabilityV1::LocalNetwork);
        assert!(!connectivity.internet_usable);
        assert_eq!(connectivity.shareable_addresses.len(), 1);
    }

    #[test]
    fn malformed_and_oversized_addresses_are_rejected() {
        assert!(matches!(
            validate_invite_addresses(&["not-a-multiaddr".into()]),
            Err(InviteConnectivityError::InvalidAddress(_))
        ));
        assert!(matches!(
            validate_invite_addresses(&[format!("/dns4/{}/tcp/443", "a".repeat(MAX_INVITE_ADDRESS_CHARS))]),
            Err(InviteConnectivityError::AddressTooLong { .. })
        ));
        let too_many = (0..=MAX_INVITE_ADDRESSES)
            .map(|index| format!("/ip4/10.0.0.{}/udp/4000/quic-v1", index + 1))
            .collect::<Vec<_>>();
        assert!(matches!(
            validate_invite_addresses(&too_many),
            Err(InviteConnectivityError::TooManyAddresses { .. })
        ));
    }

    #[test]
    fn relay_hint_requires_concrete_relay_and_destination_peers() {
        assert!(matches!(
            validate_invite_addresses(&["/ip4/1.1.1.1/tcp/4001/p2p-circuit".into()]),
            Err(InviteConnectivityError::UnusableAddress(_))
        ));
    }
}
