use anyhow::{anyhow, Context, Result};
use std::{
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::{verify_invite_signature, DataPaths};
use swarm_network::{
    invite_connectivity_from_snapshot, validate_invite_addresses, InviteConnectivityV1,
    CONNECTIVITY_DIAGNOSTICS_JSON_ENV, DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE,
};
use swarm_protocol::{InviteV1, PROTOCOL_VERSION};

const PREFIX: &str = "scinvite:";
pub const MAX_INVITE_TOKEN_CHARS: usize = 32 * 1024;
pub const MAX_INVITE_LIFETIME_MINUTES: u64 = 24 * 60;
pub const MAX_INVITE_LIFETIME_MS: u64 = MAX_INVITE_LIFETIME_MINUTES * 60_000;

pub fn encode(invite: &InviteV1) -> Result<String> {
    validate_invite_addresses(&invite.bootstrap_addrs).context("invite connection hints are invalid")?;
    let bytes = postcard::to_allocvec(invite).context("cannot encode invite")?;
    let token = format!("{PREFIX}{}", hex::encode(bytes));
    if token.chars().count() > MAX_INVITE_TOKEN_CHARS {
        return Err(anyhow!("invite token exceeds the maximum supported size"));
    }
    Ok(token)
}

pub fn decode(value: &str) -> Result<InviteV1> {
    if value.chars().count() > MAX_INVITE_TOKEN_CHARS {
        return Err(anyhow!("invite token exceeds the maximum supported size"));
    }
    let encoded = value.strip_prefix(PREFIX).ok_or_else(|| anyhow!("invite must start with {PREFIX}"))?;
    let bytes = hex::decode(encoded).context("invite payload is not valid hex")?;
    let invite: InviteV1 = postcard::from_bytes(&bytes).context("invite payload is invalid")?;
    if invite.protocol_version != PROTOCOL_VERSION {
        return Err(anyhow!("invite protocol version {} is unsupported", invite.protocol_version));
    }
    if invite.genesis.world_id()? != invite.world_id {
        return Err(anyhow!("invite world ID does not match its genesis record"));
    }
    validate_invite_addresses(&invite.bootstrap_addrs).context("invite connection hints are invalid")?;
    verify_invite_signature(&invite).context("invite signature is invalid")?;
    validate_invite_expiry(invite.expires_unix_ms, unix_time_ms()?)?;
    Ok(invite)
}

/// Validate the bearer-token exposure window. Invitations are intentionally
/// reusable by multiple joining identities until expiry, but only while the
/// signer remains the world's current authority. There is no hidden single-use
/// nonce state: the signed nonce provides token uniqueness, not consumption.
pub fn validate_invite_expiry(expires_unix_ms: u64, now_unix_ms: u64) -> Result<()> {
    if expires_unix_ms <= now_unix_ms {
        return Err(anyhow!("invite expired"));
    }
    let latest_allowed = now_unix_ms
        .checked_add(MAX_INVITE_LIFETIME_MS)
        .ok_or_else(|| anyhow!("invite lifetime overflows the timestamp range"))?;
    if expires_unix_ms > latest_allowed {
        return Err(anyhow!("invite expiry exceeds the maximum {MAX_INVITE_LIFETIME_MINUTES}-minute bearer lifetime"));
    }
    Ok(())
}

pub fn expiry_from_minutes(now_unix_ms: u64, expires_minutes: u64) -> Result<u64> {
    if expires_minutes == 0 || expires_minutes > MAX_INVITE_LIFETIME_MINUTES {
        return Err(anyhow!("invite lifetime must be between 1 and {MAX_INVITE_LIFETIME_MINUTES} minutes"));
    }
    let lifetime_ms =
        expires_minutes.checked_mul(60_000).ok_or_else(|| anyhow!("invite lifetime overflows milliseconds"))?;
    now_unix_ms.checked_add(lifetime_ms).ok_or_else(|| anyhow!("invite expiry overflows the timestamp range"))
}

pub fn configure_connectivity_snapshot(paths: &DataPaths) {
    if env::var_os(CONNECTIVITY_DIAGNOSTICS_JSON_ENV).is_none() {
        env::set_var(CONNECTIVITY_DIAGNOSTICS_JSON_ENV, default_connectivity_snapshot(paths));
    }
}

pub fn current_invite_connectivity(paths: &DataPaths) -> Result<InviteConnectivityV1> {
    let snapshot = env::var_os(CONNECTIVITY_DIAGNOSTICS_JSON_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_connectivity_snapshot(paths));
    invite_connectivity_from_snapshot(&snapshot).with_context(|| {
        format!(
            "automatic invite connectivity is unavailable from {}; keep the SwarmCraft networking daemon running",
            snapshot.display()
        )
    })
}

pub fn resolve_bootstrap_addrs(paths: &DataPaths, manual: Vec<String>) -> Result<Vec<String>> {
    if !manual.is_empty() {
        return validate_invite_addresses(&manual).context("manual bootstrap override is invalid");
    }
    let connectivity = current_invite_connectivity(paths)?;
    if connectivity.shareable_addresses.is_empty() {
        return Err(anyhow!(
            "this device currently has no proven reachable direct, relay, or local-network invite path. {} Remediation: keep networking running and configure a relay or restore a reachable listener. Advanced users may provide a validated --bootstrap override.",
            connectivity.diagnostic_reason
        ));
    }
    Ok(connectivity.shareable_addresses)
}

fn default_connectivity_snapshot(paths: &DataPaths) -> PathBuf {
    paths.root.join(DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE)
}

pub fn unix_time_ms() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH).context("system clock is before Unix epoch")?.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_core::{random_nonce, PeerIdentity};
    use swarm_network::{ConnectivityDiagnosticsV1, InviteReachabilityV1, NatStatusV1, MAX_INVITE_ADDRESSES};
    use swarm_protocol::{Hash32, PeerId, WorldGenesisV1};

    fn signed_invite(expires_unix_ms: u64, bootstrap_addrs: Vec<String>) -> InviteV1 {
        let identity = PeerIdentity::generate();
        let mut initial_membership = vec![identity.peer_id(), PeerId([8; 32])];
        initial_membership.sort();
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([4; 32]),
            creation_nonce: random_nonce(),
            creator_public_key: identity.public_key(),
            initial_membership,
        };
        let mut invite = InviteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: genesis.world_id().unwrap(),
            display_name: "Test world".into(),
            genesis,
            inviter_peer_id: identity.peer_id(),
            inviter_public_key: identity.public_key(),
            bootstrap_addrs,
            expires_unix_ms,
            nonce: random_nonce(),
            signature: Vec::new(),
        };
        identity.sign_invite(&mut invite).unwrap();
        invite
    }

    #[test]
    fn signed_invite_round_trip() {
        let invite = signed_invite(unix_time_ms().unwrap() + 60_000, vec!["/ip4/1.1.1.1/udp/4001/quic-v1".into()]);
        assert_eq!(decode(&encode(&invite).unwrap()).unwrap(), invite);
    }

    #[test]
    fn invite_is_explicitly_reusable_until_expiry() {
        let invite = signed_invite(unix_time_ms().unwrap() + 60_000, vec!["/ip4/1.1.1.1/udp/4001/quic-v1".into()]);
        let encoded = encode(&invite).unwrap();
        assert_eq!(decode(&encoded).unwrap(), invite);
        assert_eq!(decode(&encoded).unwrap(), invite);
    }

    #[test]
    fn invite_lifetime_is_bounded_and_checked() {
        let now = unix_time_ms().unwrap();
        assert!(expiry_from_minutes(now, 0).is_err());
        assert!(expiry_from_minutes(now, MAX_INVITE_LIFETIME_MINUTES + 1).is_err());
        assert_eq!(expiry_from_minutes(now, MAX_INVITE_LIFETIME_MINUTES).unwrap(), now + MAX_INVITE_LIFETIME_MS);
        let overlong =
            signed_invite(now + MAX_INVITE_LIFETIME_MS + 60_000, vec!["/ip4/1.1.1.1/udp/4001/quic-v1".into()]);
        assert!(decode(&encode(&overlong).unwrap()).unwrap_err().to_string().contains("maximum"));
    }

    #[test]
    fn expired_invite_is_rejected() {
        let invite = signed_invite(1, vec!["/ip4/1.1.1.1/udp/4001/quic-v1".into()]);
        let encoded = encode(&invite).unwrap();
        assert!(decode(&encoded).unwrap_err().to_string().contains("expired"));
    }

    #[test]
    fn loopback_hint_is_rejected_even_when_signed() {
        let invite = signed_invite(unix_time_ms().unwrap() + 60_000, vec!["/ip4/127.0.0.1/udp/4001/quic-v1".into()]);
        assert!(encode(&invite).unwrap_err().to_string().contains("connection hints"));
    }

    #[test]
    fn too_many_hints_are_rejected() {
        let addresses =
            (0..=MAX_INVITE_ADDRESSES).map(|index| format!("/ip4/10.0.0.{}/udp/4001/quic-v1", index + 1)).collect();
        let invite = signed_invite(unix_time_ms().unwrap() + 60_000, addresses);
        assert!(encode(&invite).is_err());
    }

    #[test]
    fn automatic_resolution_reads_current_backend_connectivity_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        std::fs::create_dir_all(&paths.root).unwrap();
        let snapshot = default_connectivity_snapshot(&paths);
        let diagnostics = ConnectivityDiagnosticsV1 {
            observed_public_address: Some("/ip4/1.1.1.1/udp/41000/quic-v1".into()),
            nat_status: NatStatusV1::Public,
            ..Default::default()
        };
        std::fs::write(&snapshot, serde_json::to_vec(&diagnostics).unwrap()).unwrap();

        let connectivity = invite_connectivity_from_snapshot(&snapshot).unwrap();
        assert_eq!(connectivity.reachability, InviteReachabilityV1::PublicDirect);
        assert_eq!(resolve_bootstrap_addrs(&paths, Vec::new()).unwrap(), vec!["/ip4/1.1.1.1/udp/41000/quic-v1"]);
    }

    #[test]
    fn no_reachable_path_returns_remediation_instead_of_false_green() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        std::fs::create_dir_all(&paths.root).unwrap();
        let snapshot = default_connectivity_snapshot(&paths);
        let diagnostics = ConnectivityDiagnosticsV1 {
            local_addresses: vec!["/ip4/127.0.0.1/udp/4000/quic-v1".into()],
            nat_status: NatStatusV1::Private,
            ..Default::default()
        };
        std::fs::write(&snapshot, serde_json::to_vec(&diagnostics).unwrap()).unwrap();

        let error = resolve_bootstrap_addrs(&paths, Vec::new()).unwrap_err().to_string();
        assert!(error.contains("no proven reachable"));
        assert!(error.contains("Remediation"));
    }

    #[test]
    fn malformed_manual_override_is_rejected_before_signing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        assert!(resolve_bootstrap_addrs(&paths, vec!["not-a-multiaddr".into()]).is_err());
    }
}
