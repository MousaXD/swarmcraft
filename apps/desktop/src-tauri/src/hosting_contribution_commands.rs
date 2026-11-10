//! Desktop-side input and backend handoff contract for explicit hosting contribution.
//!
//! This module is staged behind `#[cfg(test)]` in main.rs until Agent 00
//! approves a base containing Agent 03's real `hosting_budget` API.
//! It does not register Tauri commands or emulate persisted consent.
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
pub fn validated_get<B: HostingContributionBackend>(backend: &B, world: &str) -> Result<B::Readiness, String> {
    backend.get(validate_world_id(world)?)
}

/// Prepare a typed, validated mutation. The backend, NOT Desktop or the
/// renderer, must perform durable atomic updates, revision invalidation,
/// admission locking and safe live-authority withdrawal.
pub fn validated_set<B: HostingContributionBackend>(
    backend: &B,
    world: &str,
    heap_mib: u64,
) -> Result<B::Consent, String> {
    let world = validate_world_id(world)?;
    validate_heap_mib(heap_mib)?;
    backend.set(world, heap_mib)
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
    fn world_validation_precedes_heap_and_backend_mutation() {
        let spy = BoundarySpy::default();
        assert!(validated_set(&spy, "../world", u64::MAX).is_err());
        assert_eq!(spy.calls.load(Ordering::SeqCst), 0);
    }
}
