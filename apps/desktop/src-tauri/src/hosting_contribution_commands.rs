//! Desktop hosting IPC: real CLI-backed Rust consent, never renderer-only preference.
//! Signed authority permission is NOT implied by local readiness.
use std::str::FromStr;
use swarm_protocol::WorldId;

/// These match Agent 03's current `hosting_budget` input limits.
/// The integrated command must use the backend's exported constants instead.
pub const MIN_HEAP_MIB: u64 = 512;
pub const MAX_HEAP_MIB: u64 = 65_536;

/// The sole authority for reads and writes must be the local Rust hosting API,
/// not a renderer setting or an unauthenticated remote daemon. The future
/// adapter MUST verify the world exists locally and the consent is bound to
/// this device's current cryptographic identity. It must preserve backend
/// errors and must not convert a failed write to a successful response.
///
/// Associated output types intentionally avoid shadow copies of Agent 03's
/// unmerged `ContributionReadiness` and `HostingConsentV1` types.
#[cfg(test)]
pub trait HostingContributionBackend {
    type Readiness: serde::Serialize;
    type Consent: serde::Serialize;

    fn get(&self, world: WorldId) -> Result<Self::Readiness, String>;
    fn set(&self, world: WorldId, heap_mib: u64) -> Result<Self::Consent, String>;
}

/// Parse either the canonical `scworld:<64 hex>` identifier or its 64-hex
/// compatibility form, using the shared protocol parser. No path handling,
/// coercion, or whitespace trimming is allowed at this untrusted IPC boundary.
pub fn validate_world_id(world: &str) -> Result<WorldId, String> {
    if world.is_empty() || world != world.trim() {
        return Err("A valid world ID is required".into());
    }
    WorldId::from_str(world).map_err(|_| "Invalid world ID".into())
}

/// Zero means explicit withdrawal/player-only; any nonzero selection must
/// meet the Rust backend's heap bounds. Physical/available memory is checked
/// again by Agent 03 on write and on actual host admission.
pub fn validate_heap_mib(heap_mib: u64) -> Result<(), String> {
    if heap_mib == 0 || (MIN_HEAP_MIB..=MAX_HEAP_MIB).contains(&heap_mib) {
        Ok(())
    } else {
        Err(format!("Server heap must be 0 (player-only) or between {MIN_HEAP_MIB} and {MAX_HEAP_MIB} MiB"))
    }
}

/// Prepare a typed, validated read call. Registration as a real Tauri command
/// is intentionally deferred until `HostingContributionBackend` is bound to
/// the imported Rust `hosting_budget::contribution_readiness` implementation.
#[cfg(test)]
pub fn validated_get<B: HostingContributionBackend>(backend: &B, world: &str) -> Result<B::Readiness, String> {
    backend.get(validate_world_id(world)?)
}

/// Prepare a typed, validated mutation. The backend, NOT Desktop or the
/// renderer, must perform durable atomic updates, revision invalidation,
/// admission locking and safe live-authority withdrawal.
#[cfg(test)]
pub fn validated_set<B: HostingContributionBackend>(
    backend: &B,
    world: &str,
    heap_mib: u64,
) -> Result<B::Consent, String> {
    let world = validate_world_id(world)?;
    validate_heap_mib(heap_mib)?;
    backend.set(world, heap_mib)
}

/// Public-only response from the real packaged Rust hosting CLI. Unknown
/// fields (including peer identity or private paths) fail closed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostingContributionReadinessReply {
    pub world_id: String,
    pub consented: bool,
    pub selected_heap_mib: u64,
    pub physical_mib: Option<u64>,
    pub available_mib: Option<u64>,
    pub estimated_native_overhead_mib: u64,
    pub client_headroom_mib: u64,
    pub os_headroom_mib: u64,
    pub daemon_headroom_mib: u64,
    pub may_host_now: bool,
    pub reason: String,
    pub hard_process_memory_cap_guaranteed: bool,
    pub consent_revision: Option<String>,
}

/// Check identity and consistency on the response from the Rust sidecar.
/// Local readiness does not constitute a signed world authority grant.
pub fn decode_readiness(output: &str, world: WorldId) -> Result<HostingContributionReadinessReply, String> {
    let report: HostingContributionReadinessReply =
        serde_json::from_str(output).map_err(|_| "Invalid local hosting readiness response".to_string())?;
    if report.world_id != world.to_string() {
        return Err("Local hosting readiness world identity mismatch".into());
    }
    validate_heap_mib(report.selected_heap_mib)?;
    if report.consented != (report.selected_heap_mib != 0) {
        return Err("Local hosting consent and heap do not agree".into());
    }
    if report.may_host_now && !report.consented {
        return Err("Player-only device cannot advertise hosting eligibility".into());
    }
    if report.hard_process_memory_cap_guaranteed {
        return Err("Whole-process JVM memory limit cannot be guaranteed".into());
    }
    if report.consented && report.consent_revision.is_none() {
        return Err("Persisted hosting consent revision is absent".into());
    }
    if let Some(revision) = report.consent_revision.as_deref() {
        if revision.is_empty() || revision.parse::<u64>().is_err() {
            return Err("Hosting consent revision must be a lossless decimal string".into());
        }
    }
    Ok(report)
}

/// Safe, bounded result of a successful persisted native consent mutation.
/// The production adapter constructs this only from Agent 03's validated
/// HostingConsentV1 after set_contribution returns Ok. Never serialize that
/// record directly into the webview: it also contains the local peer identity.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HostingContributionConsentReply {
    pub world_id: String,
    pub enabled: bool,
    pub heap_mib: u64,
    /// Revision is a decimal string; Rust's u64 nanosecond-scale revisions
    /// exceed JavaScript's Number.MAX_SAFE_INTEGER.
    pub revision: String,
}

#[cfg(test)]
impl HostingContributionConsentReply {
    pub fn from_persisted(world: WorldId, enabled: bool, heap_mib: u64, revision: u64) -> Self {
        Self { world_id: world.to_string(), enabled, heap_mib, revision: revision.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    const WORLD: &str = "scworld:1111111111111111111111111111111111111111111111111111111111111111";

    /// Test-only boundary spy. This is NOT a production adapter, a persistence
    /// implementation, or evidence of live-host revocation correctness.
    #[derive(Default)]
    struct BoundarySpy {
        calls: AtomicUsize,
        selected: Mutex<Option<u64>>,
        error: Mutex<Option<String>>,
    }

    impl HostingContributionBackend for BoundarySpy {
        type Readiness = u64;
        type Consent = u64;

        fn get(&self, _: WorldId) -> Result<Self::Readiness, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.error.lock().unwrap().clone() {
                return Err(error);
            }
            Ok(self.selected.lock().unwrap().unwrap_or(0))
        }

        fn set(&self, _: WorldId, heap_mib: u64) -> Result<Self::Consent, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.error.lock().unwrap().clone() {
                return Err(error);
            }
            *self.selected.lock().unwrap() = Some(heap_mib);
            Ok(heap_mib)
        }
    }

    #[test]
    fn canonical_and_legacy_hex_world_ids_resolve_to_the_same_identity() {
        assert_eq!(validate_world_id(WORLD).unwrap(), validate_world_id(&WORLD[8..]).unwrap());
        assert_eq!(validate_world_id(WORLD).unwrap().to_string(), WORLD);
    }

    #[test]
    fn malformed_world_and_path_injection_are_rejected_before_backend_access() {
        let spy = BoundarySpy::default();
        for world in
            ["", " ", " ./world", "../escape", "world/abc", "scworld:zz", "scworld:", "a\n--java", "scworld:111"]
        {
            assert!(validated_get(&spy, world).is_err(), "unexpectedly accepted: {world:?}");
            assert!(validated_set(&spy, world, 512).is_err(), "unexpectedly accepted: {world:?}");
        }
        assert_eq!(spy.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn default_zero_withdrawal_and_heap_bounds_are_distinct() {
        for allowed in [0, 512, 1024, 4096, 65_536] {
            assert!(validate_heap_mib(allowed).is_ok(), "rejected {allowed}");
        }
        for denied in [1, 511, 65_537, u64::MAX] {
            assert!(validate_heap_mib(denied).is_err(), "accepted {denied}");
        }
    }

    #[test]
    fn invalid_heap_never_reaches_the_backend() {
        let spy = BoundarySpy::default();
        for heap_mib in [1, 511, 65_537, u64::MAX] {
            assert!(validated_set(&spy, WORLD, heap_mib).is_err());
        }
        assert_eq!(spy.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn zero_is_forwarded_as_explicit_revocation_without_implicit_consent() {
        let spy = BoundarySpy::default();
        assert_eq!(validated_get(&spy, WORLD).unwrap(), 0);
        assert_eq!(validated_set(&spy, WORLD, 4096).unwrap(), 4096);
        assert_eq!(validated_get(&spy, WORLD).unwrap(), 4096);
        assert_eq!(validated_set(&spy, WORLD, 0).unwrap(), 0);
        assert_eq!(validated_get(&spy, WORLD).unwrap(), 0);
    }

    #[test]
    fn backend_failure_is_not_rewritten_as_a_success() {
        let spy = BoundarySpy::default();
        *spy.error.lock().unwrap() = Some("insufficient hosting memory".into());
        assert_eq!(validated_set(&spy, WORLD, 4096).unwrap_err(), "insufficient hosting memory");
        assert_eq!(validated_get(&spy, WORLD).unwrap_err(), "insufficient hosting memory");
        assert_eq!(*spy.selected.lock().unwrap(), None);
    }

    #[test]
    fn persisted_consent_reply_has_only_public_fields_and_lossless_revision() {
        let world = validate_world_id(WORLD).unwrap();
        let reply = HostingContributionConsentReply::from_persisted(world, false, 0, u64::MAX);
        assert_eq!(
            serde_json::to_value(reply).unwrap(),
            serde_json::json!({
                "world_id": WORLD,
                "enabled": false,
                "heap_mib": 0,
                "revision": u64::MAX.to_string(),
            })
        );
    }

    #[test]
    fn world_validation_precedes_heap_and_backend_mutation() {
        let spy = BoundarySpy::default();
        assert!(validated_set(&spy, "../world", u64::MAX).is_err());
        assert_eq!(spy.calls.load(Ordering::SeqCst), 0);
    }

    fn sample_readiness() -> String {
        serde_json::json!({
            "world_id": WORLD, "consented": false, "selected_heap_mib": 0,
            "physical_mib": 8192, "available_mib": 4096,
            "estimated_native_overhead_mib": 0,
            "client_headroom_mib": 1024, "os_headroom_mib": 1024,
            "daemon_headroom_mib": 256, "may_host_now": false,
            "reason": "Player-only; no Minecraft hosting consent",
            "hard_process_memory_cap_guaranteed": false,
            "consent_revision": null
        })
        .to_string()
    }

    #[test]
    fn real_readiness_schema_has_no_private_identity() {
        let world = validate_world_id(WORLD).unwrap();
        let report = decode_readiness(&sample_readiness(), world).unwrap();
        let public = serde_json::to_value(report).unwrap();
        assert_eq!(public["selected_heap_mib"], 0);
        assert_eq!(public["may_host_now"], false);
        assert!(public.get("peer_id").is_none());
        assert!(public.get("private_key").is_none());
    }

    #[test]
    fn malformed_or_inconsistent_real_readiness_is_rejected() {
        let world = validate_world_id(WORLD).unwrap();
        let original: serde_json::Value = serde_json::from_str(&sample_readiness()).unwrap();
        for (field, value) in [
            ("world_id", serde_json::json!("scworld:2222222222222222222222222222222222222222222222222222222222222222")),
            ("consented", serde_json::json!(true)),
            ("may_host_now", serde_json::json!(true)),
            ("hard_process_memory_cap_guaranteed", serde_json::json!(true)),
            ("consent_revision", serde_json::json!("invalid-revision")),
            ("peer_id", serde_json::json!("must-not-leak")),
        ] {
            let mut mutated = original.clone();
            mutated[field] = value;
            assert!(decode_readiness(&mutated.to_string(), world).is_err(), "accepted inconsistent {field}");
        }
        let mut opted_in: serde_json::Value = serde_json::from_str(&sample_readiness()).unwrap();
        opted_in["consented"] = serde_json::json!(true);
        opted_in["selected_heap_mib"] = serde_json::json!(512);
        opted_in["may_host_now"] = serde_json::json!(true);
        assert!(decode_readiness(&opted_in.to_string(), world).is_err());
        opted_in["consent_revision"] = serde_json::json!("18446744073709551615");
        assert!(decode_readiness(&opted_in.to_string(), world).is_ok());
    }
}
