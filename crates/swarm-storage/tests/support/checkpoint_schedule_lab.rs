//! Model only. The runtime has no verified online frozen-copy provider yet.
//! A normal Fabric SAVE_COMPLETE notification does not satisfy immutable_view.
use super::{Hash32, Pin};
use std::time::Duration;

type Tick = u64; // monotonic milliseconds, not wall-clock timestamps

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Generation {
    epoch: u64,
    fence: u64,
}

#[derive(Clone, Copy, Debug)]
struct Gates {
    generation: Generation,
    immutable_view: bool,
    storage_available: bool,
    conflict_free: bool,
}

#[derive(Clone, Copy, Debug)]
struct Policy {
    min_ms: Tick,
    target_ms: Tick,
    dirty_threshold: u64,
    retry_ms: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    Clean,
    NotDue,
    RetryBackoff,
    Busy,
    UnsafeSource,
    StoragePaused,
    StaleGeneration,
    Explicit,
    Deadline,
    DirtyThreshold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Attempt {
    id: u64,
    generation: Generation,
    captured_at: Tick,
    dirty_total: u64,
    explicit_revision: u64,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    attempt: Attempt,
    pin: Option<Pin>,
}

struct Planner {
    policy: Policy,
    generation: Generation,
    verified: Pin,
    verified_at: Tick,
    dirty_total: u64,
    verified_dirty: u64,
    explicit_revision: u64,
    verified_explicit: u64,
    last_attempt: Tick,
    retry_until: Tick,
    serial: u64,
    pending: Option<Pending>,
}

impl Planner {
    fn new(policy: Policy, generation: Generation, pin: Pin, now: Tick) -> Self {
        assert!(policy.min_ms > 0 && policy.min_ms <= policy.target_ms);
        Self {
            policy,
            generation,
            verified: pin,
            verified_at: now,
            dirty_total: 0,
            verified_dirty: 0,
            explicit_revision: 0,
            verified_explicit: 0,
            last_attempt: now,
            retry_until: now,
            serial: 0,
            pending: None,
        }
    }

    fn change(&mut self, bytes: u64) {
        self.dirty_total = self.dirty_total.checked_add(bytes).expect("model dirty counter");
    }

    fn request(&mut self) {
        self.explicit_revision = self.explicit_revision.checked_add(1).expect("model explicit counter");
    }

    fn dirty(&self) -> u64 {
        self.dirty_total - self.verified_dirty
    }

    fn age(&self, now: Tick) -> Tick {
        if self.dirty() == 0 {
            0
        } else {
            now.saturating_sub(self.verified_at)
        }
    }

    fn plan(&self, now: Tick, gates: Gates) -> Decision {
        if gates.generation != self.generation || !gates.conflict_free || now < self.last_attempt {
            return Decision::StaleGeneration;
        }
        if self.pending.is_some() {
            return Decision::Busy;
        }
        let explicit = self.explicit_revision != self.verified_explicit;
        if self.dirty() == 0 && !explicit {
            return Decision::Clean;
        }
        if now < self.retry_until {
            return Decision::RetryBackoff;
        }
        let deadline = self.age(now) >= self.policy.target_ms;
        let threshold =
            self.dirty() >= self.policy.dirty_threshold && now.saturating_sub(self.last_attempt) >= self.policy.min_ms;
        if !explicit && !deadline && !threshold {
            return Decision::NotDue;
        }
        if !gates.immutable_view {
            return Decision::UnsafeSource;
        }
        if !gates.storage_available {
            return Decision::StoragePaused;
        }
        if explicit {
            Decision::Explicit
        } else if deadline {
            Decision::Deadline
        } else {
            Decision::DirtyThreshold
        }
    }

    fn begin(&mut self, now: Tick, gates: Gates) -> Option<Attempt> {
        if !matches!(self.plan(now, gates), Decision::Explicit | Decision::Deadline | Decision::DirtyThreshold) {
            return None;
        }
        self.serial = self.serial.checked_add(1).expect("model attempt counter");
        self.last_attempt = now;
        let attempt = Attempt {
            id: self.serial,
            generation: self.generation,
            captured_at: now,
            dirty_total: self.dirty_total,
            explicit_revision: self.explicit_revision,
        };
        self.pending = Some(Pending { attempt, pin: None });
        Some(attempt)
    }

    fn captured(&mut self, attempt: Attempt, pin: Pin, generation: Generation) -> bool {
        let Some(pending) = &mut self.pending else { return false };
        if pending.attempt != attempt
            || pending.pin.is_some()
            || generation != self.generation
            || pin.number <= self.verified.number
            || pin.hash == Hash32::default()
        {
            return false;
        }
        pending.pin = Some(pin);
        true
    }

    fn receipt(&mut self, attempt: Attempt, pin: Pin, generation: Generation, independently_verified: bool) -> bool {
        // This boolean is a model input, not an authentication mechanism.
        // Production must prove signed membership, exact manifest, all
        // payloads and the intended recipient independently.
        if !independently_verified
            || generation != self.generation
            || !self.pending.is_some_and(|pending| pending.attempt == attempt && pending.pin == Some(pin))
        {
            return false;
        }
        self.verified = pin;
        self.verified_at = attempt.captured_at;
        self.verified_dirty = attempt.dirty_total;
        self.verified_explicit = attempt.explicit_revision;
        self.pending = None;
        true
    }

    fn failed(&mut self, attempt: Attempt, now: Tick) -> bool {
        if now < attempt.captured_at || !self.pending.is_some_and(|pending| pending.attempt == attempt) {
            return false;
        }
        self.pending = None;
        self.retry_until = now.saturating_add(self.policy.retry_ms);
        // No verified receipt: do not advance the recoverable point or
        // discard any dirty revision, even if capture completed locally.
        true
    }
}

#[derive(Debug, Default)]
struct Metrics {
    attempts: u64,
    verified: u64,
    estimated_transfer_bytes: u64,
    max_dirty_age_ms: u64,
    over_target_ms: u64,
    unsafe_ticks: u64,
    pressure_ticks: u64,
}

fn fixture_pin(number: u64) -> Pin {
    Pin { number, hash: Hash32::from_domain_bytes(b"schedule-model-only", &number.to_le_bytes()) }
}

fn simulate(policy: Policy, completion: Duration, transfer_bytes: u64, interrupted: bool) -> Metrics {
    let generation = Generation { epoch: 1, fence: 1 };
    let mut planner = Planner::new(policy, generation, fixture_pin(1), 0);
    let mut metrics = Metrics::default();
    let mut active: Option<(Attempt, Pin, Tick, bool)> = None;
    let completion_ms = u64::try_from(completion.as_millis()).unwrap_or(u64::MAX).max(1);
    let mut next_pin = 2;
    // Model: 10 minutes, 32 KiB of estimated new dirty sector bytes per
    // ten seconds, a safe immutable-view opportunity every 30 seconds.
    // The benchmark measured per-checkpoint sparse costs, not this trace;
    // applying those costs repeatedly is a scheduling sensitivity model.
    for now in (0..=600_000).step_by(1000) {
        if now > 0 && now % 10_000 == 0 {
            planner.change(32 * 1024);
        }
        if let Some((attempt, pin, finish, lose_receipt)) = active {
            if now >= finish {
                if lose_receipt {
                    assert!(planner.failed(attempt, now));
                } else {
                    assert!(planner.captured(attempt, pin, generation));
                    assert!(planner.receipt(attempt, pin, generation, true));
                    metrics.verified += 1;
                }
                active = None;
            }
        }
        let gates = Gates {
            generation,
            conflict_free: true,
            immutable_view: now % 30_000 == 0 && !(interrupted && (240_000..360_000).contains(&now)),
            storage_available: !(interrupted && (420_000..450_000).contains(&now)),
        };
        match planner.plan(now, gates) {
            Decision::UnsafeSource => metrics.unsafe_ticks += 1,
            Decision::StoragePaused => metrics.pressure_ticks += 1,
            _ => {}
        }
        if let Some(attempt) = planner.begin(now, gates) {
            let lose_receipt = interrupted && metrics.attempts == 1;
            metrics.attempts += 1;
            // Charge the full representative cost on each attempt, including
            // one still in flight at the end and any lost receipt. This is
            // an estimated budget, not observed traffic in this model.
            metrics.estimated_transfer_bytes += transfer_bytes;
            let pin = fixture_pin(next_pin);
            next_pin += 1;
            active = Some((attempt, pin, now.saturating_add(completion_ms), lose_receipt));
        }
        let age = planner.age(now);
        metrics.max_dirty_age_ms = metrics.max_dirty_age_ms.max(age);
        if age > policy.target_ms {
            metrics.over_target_ms += 1000;
        }
    }
    metrics
}

fn evaluate(v1_cost: (Duration, u64), block_cost: (Duration, u64)) -> (Metrics, Metrics, Metrics) {
    let baseline = Policy { min_ms: 120_000, target_ms: 120_000, dirty_threshold: u64::MAX, retry_ms: 5_000 };
    let incremental = Policy { min_ms: 30_000, target_ms: 60_000, dirty_threshold: 128 * 1024, retry_ms: 5_000 };
    let fixed = simulate(baseline, v1_cost.0, v1_cost.1, false);
    let adaptive = simulate(incremental, block_cost.0, block_cost.1, false);
    let interrupted = simulate(incremental, block_cost.0, block_cost.1, true);
    (fixed, adaptive, interrupted)
}

pub(super) fn report(v1_cost: (Duration, u64), block_cost: (Duration, u64)) {
    let (fixed, adaptive, interrupted) = evaluate(v1_cost, block_cost);
    eprintln!("schedule_model baseline_fixed={fixed:?}");
    eprintln!("schedule_model incremental_adaptive={adaptive:?}");
    eprintln!("schedule_model incremental_interrupted={interrupted:?}");
    // Measured timings can vary on a loaded machine. Do not make a timing
    // ratio or this sensitivity model a flaky correctness gate.
}

#[test]
fn model_compares_fixed_and_adaptive_policies_and_reports_blocked_deadlines() {
    // Explicit model inputs near the representative fixture costs, not
    // measurements. Actual benchmark costs are passed separately to report.
    let (fixed, adaptive, interrupted) =
        evaluate((Duration::from_secs(2), 25_200_000), (Duration::from_millis(500), 1_580_000));
    assert_eq!((fixed.attempts, fixed.verified), (5, 4));
    assert_eq!((adaptive.attempts, adaptive.verified), (10, 9));
    assert!(adaptive.max_dirty_age_ms < fixed.max_dirty_age_ms);
    assert!(adaptive.estimated_transfer_bytes < fixed.estimated_transfer_bytes);
    assert!(interrupted.max_dirty_age_ms > 60_000);
    assert!(interrupted.unsafe_ticks > 0);
    assert!(interrupted.pressure_ticks > 0);
    assert!(interrupted.over_target_ms > 0);
}

#[test]
fn model_never_treats_local_capture_or_stale_receipt_as_recoverable() {
    let generation = Generation { epoch: 1, fence: 1 };
    let newer = Generation { epoch: 2, fence: 2 };
    let gates = Gates { generation, immutable_view: true, storage_available: true, conflict_free: true };
    let policy = Policy { min_ms: 30, target_ms: 60, dirty_threshold: 100, retry_ms: 10 };
    let mut planner = Planner::new(policy, generation, fixture_pin(1), 0);
    planner.change(100);
    assert_eq!(planner.plan(29, gates), Decision::NotDue);
    assert_eq!(planner.plan(30, Gates { immutable_view: false, ..gates }), Decision::UnsafeSource);
    assert_eq!(planner.plan(30, Gates { storage_available: false, ..gates }), Decision::StoragePaused);
    assert_eq!(planner.plan(30, Gates { generation: newer, ..gates }), Decision::StaleGeneration);
    assert_eq!(planner.plan(30, Gates { conflict_free: false, ..gates }), Decision::StaleGeneration);
    let attempt = planner.begin(30, gates).unwrap();
    assert!(planner.begin(60, gates).is_none());
    let pin = fixture_pin(2);
    assert!(planner.captured(attempt, pin, generation));
    assert_eq!(planner.verified, fixture_pin(1));
    assert_eq!(planner.age(60), 60);
    planner.change(12);
    planner.request();
    assert!(!planner.receipt(attempt, fixture_pin(3), generation, true));
    assert!(!planner.receipt(attempt, pin, newer, true));
    assert!(!planner.receipt(attempt, pin, generation, false));
    assert!(planner.receipt(attempt, pin, generation, true));
    assert_eq!(planner.verified, pin);
    assert_eq!(planner.dirty(), 12, "edits after the capture cannot be lost");
    assert_eq!(planner.age(61), 31, "use captured time, not ACK arrival time");
    assert_eq!(planner.plan(61, gates), Decision::Explicit, "coalesce request made during prior capture");
    let second = planner.begin(61, gates).unwrap();
    assert!(!planner.receipt(attempt, pin, generation, true));
    assert!(planner.captured(second, fixture_pin(3), generation));
    assert!(planner.failed(second, 62));
    assert_eq!(planner.plan(69, gates), Decision::RetryBackoff);
    assert_eq!(planner.plan(72, gates), Decision::Explicit);
    assert_eq!(planner.verified, pin);
    assert_eq!(planner.dirty(), 12);
    assert!(!planner.receipt(second, fixture_pin(3), generation, true));
    let third = planner.begin(72, gates).unwrap();
    assert!(planner.captured(third, fixture_pin(4), generation));
    assert!(planner.receipt(third, fixture_pin(4), generation, true));
    assert_eq!(planner.plan(90, gates), Decision::Clean);
    assert_eq!(planner.age(90), 0);
}
