//! Read-only Desktop boundary for an independently verified, current game route.
//!
//! The CLI/daemon must obtain this report from fresh canonical quorum,
//! membership, hosting-consent and authenticated Fabric IPC evidence, plus a
//! newly verified signed route. This decoder does NOT create that evidence.
//! A persisted migration Ready phase or configured game_endpoint is never a
//! substitute. The CLI subcommand may be absent until daemon integration;
//! that error is deliberately surfaced instead of offering a guessed address.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use swarm_protocol::WorldId;

const MAX_PROOF_AGE_MS: u64 = 5_000;
const MAX_PROOF_LIFETIME_MS: u64 = 15_000;
const CLOCK_SKEW_MS: u64 = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameRouteStatusReply {
    pub world_id: String,
    /// verified, waiting_for_quorum, syncing_checkpoint, restoring_runtime,
    /// verifying_route, blocked or unavailable.
    pub state: String,
    pub quorum_verified: bool,
    pub hosting_consent_verified: bool,
    pub server_ready: bool,
    pub route_verified: bool,
    pub local_gateway: Option<String>,
    pub authority_peer_id: Option<String>,
    pub epoch: Option<u64>,
    pub fencing_token: Option<u64>,
    pub snapshot_hash: Option<String>,
    pub verified_at_unix_ms: Option<u64>,
    pub valid_until_unix_ms: Option<u64>,
}

fn valid_snapshot_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_authority_peer_id(value: &str) -> bool {
    value.strip_prefix("scpeer:").is_some_and(|hex| valid_snapshot_hash(hex))
}

pub fn decode_game_route_status(
    output: &str,
    world: WorldId,
    now_unix_ms: u64,
) -> Result<GameRouteStatusReply, String> {
    let mut report: GameRouteStatusReply =
        serde_json::from_str(output).map_err(|_| "Invalid native game-route status response".to_owned())?;
    if report.world_id != world.to_string() {
        return Err("Game-route status world identity mismatch".into());
    }
    if !matches!(
        report.state.as_str(),
        "verified"
            | "waiting_for_quorum"
            | "syncing_checkpoint"
            | "restoring_runtime"
            | "verifying_route"
            | "blocked"
            | "unavailable"
    ) {
        return Err("Unknown game-route status state".into());
    }

    if report.state != "verified" {
        if report.route_verified {
            return Err("Unverified route state claimed game-route authorization".into());
        }
        // A partial/missing proof cannot give the WebView a usable endpoint,
        // positive safety checkpoint or stale identity from a prior generation.
        report.quorum_verified = false;
        report.hosting_consent_verified = false;
        report.server_ready = false;
        report.local_gateway = None;
        report.authority_peer_id = None;
        report.epoch = None;
        report.fencing_token = None;
        report.snapshot_hash = None;
        report.verified_at_unix_ms = None;
        report.valid_until_unix_ms = None;
        return Ok(report);
    }

    if !(report.quorum_verified && report.hosting_consent_verified && report.server_ready && report.route_verified) {
        return Err("Game route is not backed by all fresh authorization checks".into());
    }
    let address = report.local_gateway.as_deref().ok_or("Verified route has no local gateway")?;
    let parsed: SocketAddr = address.parse().map_err(|_| "Verified route has invalid gateway address".to_owned())?;
    if parsed.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) || parsed.port() == 0 || address != parsed.to_string() {
        return Err("Verified gateway must bind the exact 127.0.0.1 loopback address".into());
    }
    if !report.authority_peer_id.as_deref().is_some_and(valid_authority_peer_id)
        || report.epoch.is_none()
        || report.fencing_token.is_none()
        || !report.snapshot_hash.as_deref().is_some_and(valid_snapshot_hash)
    {
        return Err("Verified game route lacks bound authority generation or checkpoint".into());
    }
    let verified_at = report.verified_at_unix_ms.ok_or("Verified route lacks proof time")?;
    let valid_until = report.valid_until_unix_ms.ok_or("Verified route lacks expiry")?;
    if verified_at > now_unix_ms.saturating_add(CLOCK_SKEW_MS)
        || now_unix_ms.saturating_sub(verified_at) > MAX_PROOF_AGE_MS
        || valid_until <= now_unix_ms
        || valid_until <= verified_at
        || valid_until.saturating_sub(verified_at) > MAX_PROOF_LIFETIME_MS
    {
        return Err("Game-route proof is expired, stale or implausibly dated".into());
    }
    Ok(report)
}

pub fn decode_current_game_route_status(output: &str, world: WorldId) -> Result<GameRouteStatusReply, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock cannot validate game-route proof".to_owned())?
        .as_millis();
    let now = u64::try_from(now).map_err(|_| "System clock overflow".to_owned())?;
    decode_game_route_status(output, world, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    const WORLD: &str = "scworld:1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER: &str = "scworld:2222222222222222222222222222222222222222222222222222222222222";
    const NOW: u64 = 2_000_000;

    fn verified() -> serde_json::Value {
        serde_json::json!({
            "world_id": WORLD,
            "state": "verified",
            "quorum_verified": true,
            "hosting_consent_verified": true,
            "server_ready": true,
            "route_verified": true,
            "local_gateway": "127.0.0.1:25570",
            "authority_peer_id": format!("scpeer:{}", "c".repeat(64)),
            "epoch": 3,
            "fencing_token": 12,
            "snapshot_hash": "a".repeat(64),
            "verified_at_unix_ms": NOW - 500,
            "valid_until_unix_ms": NOW + 2_000
        })
    }

    fn decode(raw: &serde_json::Value) -> Result<GameRouteStatusReply, String> {
        decode_game_route_status(&raw.to_string(), WorldId::from_str(WORLD).unwrap(), NOW)
    }

    #[test]
    fn only_fresh_world_bound_loopback_route_is_exposed() {
        assert_eq!(decode(&verified()).unwrap().local_gateway.as_deref(), Some("127.0.0.1:25570"));
        for (field, replacement) in [
            ("world_id", serde_json::json!(OTHER)),
            ("local_gateway", serde_json::json!("192.168.1.2:25570")),
            ("authority_peer_id", serde_json::json!("")),
            ("authority_peer_id", serde_json::json!("not-a-canonical-peer")),
            ("snapshot_hash", serde_json::json!("not-a-hash")),
            ("verified_at_unix_ms", serde_json::json!(NOW - 7_000)),
            ("valid_until_unix_ms", serde_json::json!(NOW - 1)),
            ("quorum_verified", serde_json::json!(false)),
            ("hosting_consent_verified", serde_json::json!(false)),
            ("server_ready", serde_json::json!(false)),
        ] {
            let mut altered = verified();
            altered[field] = replacement;
            assert!(decode(&altered).is_err(), "accepted invalid {field}");
        }
    }

    #[test]
    fn an_unready_native_report_cannot_leak_old_route() {
        let mut old = verified();
        old["state"] = serde_json::json!("waiting_for_quorum");
        old["route_verified"] = serde_json::json!(false);
        let report = decode(&old).unwrap();
        assert!(report.local_gateway.is_none());
        assert!(!report.quorum_verified);
        assert!(!report.server_ready);
        assert!(report.snapshot_hash.is_none());
        old["route_verified"] = serde_json::json!(true);
        assert!(decode(&old).is_err());
    }

    #[test]
    fn unknown_or_extra_fields_are_refused_instead_of_misread() {
        let mut value = verified();
        value["untrusted_endpoint"] = serde_json::json!("evil.example:25565");
        assert!(decode(&value).is_err());
        value = verified();
        value["state"] = serde_json::json!("old_ready");
        assert!(decode(&value).is_err());
    }
}
