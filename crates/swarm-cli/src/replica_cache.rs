//! Ordinary replication scheduling only. Neither the policy nor scheduler
//! grants permissions, establishes a recovery receipt or deletes history.
use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use swarm_protocol::WorldId;
use swarm_storage::{replica_policy::ReplicaPolicy, Storage};

pub fn utc_minute() -> Result<u16> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).context("UTC clock before Unix epoch")?.as_secs();
    Ok(((seconds / 60) % 1440) as u16)
}

pub fn parse_utc_minute(value: &str) -> Result<u16> {
    let (hour, minute) = value.split_once(':').context("expected UTC HH:MM")?;
    if hour.len() != 2 || minute.len() != 2 || !value.bytes().all(|b| b.is_ascii_digit() || b == b':') {
        bail!("expected UTC HH:MM");
    }
    let (h, m) = (hour.parse::<u16>()?, minute.parse::<u16>()?);
    if h >= 24 || m >= 60 {
        bail!("invalid UTC HH:MM");
    }
    Ok(h * 60 + m)
}

pub fn current_policy(storage: &Storage, world: WorldId) -> Result<ReplicaPolicy> {
    Ok(storage.world_replica_policy(world)?)
}

pub fn receive_allowed(policy: ReplicaPolicy) -> Result<bool> {
    Ok(policy.follow && (policy.utc_window.is_none() || policy.window_open(utc_minute()?)))
}

pub fn send_allowed(policy: ReplicaPolicy) -> Result<bool> {
    Ok(policy.utc_window.is_none() || policy.window_open(utc_minute()?))
}

/// Never-served worlds go first. Hot worlds may move ahead by at most
/// 500 ms of wait time, so sustained hot traffic cannot starve older work.
pub fn scheduling_key(last_scheduled: Option<Instant>, hot: bool) -> Option<Instant> {
    last_scheduled.map(|last| if hot { last.checked_sub(Duration::from_millis(500)).unwrap_or(last) } else { last })
}

#[derive(Debug, Default)]
pub struct PayloadBudget {
    next: HashMap<WorldId, Instant>,
}

impl PayloadBudget {
    /// Charge requested receive bytes conservatively, actual sent upload
    /// bytes, across all sources/recipients for this world. No idle credit.
    /// The caller reserves immediately in the single daemon event loop.
    pub fn allowance(&mut self, world: WorldId, policy: ReplicaPolicy, now: Instant, limit: usize) -> usize {
        self.next.retain(|_, deadline| *deadline > now);
        let Some(rate) = policy.max_kib_per_second else { return limit };
        if self.next.contains_key(&world) {
            return 0;
        }
        // At most a quarter second's payload per dispatch.
        limit.min(rate as usize * 256)
    }

    pub fn charge(&mut self, world: WorldId, policy: ReplicaPolicy, now: Instant, bytes: usize) {
        if let Some(rate) = policy.max_kib_per_second.filter(|_| bytes != 0) {
            let nanos = (bytes as u64 * 1_000_000_000).div_ceil(u64::from(rate) * 1024);
            self.next.insert(world, now + Duration::from_nanos(nanos));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_shared_budget_does_not_accumulate_credit_or_reset_on_policy_change() {
        let mut budget = PayloadBudget::default();
        let a = WorldId([1; 32]);
        let b = WorldId([2; 32]);
        let now = Instant::now();
        let policy = ReplicaPolicy { max_kib_per_second: Some(32), ..Default::default() };
        assert_eq!(budget.allowance(a, policy, now, 256 * 1024), 8192);
        budget.charge(a, policy, now, 8192);
        assert_eq!(budget.allowance(a, policy, now + Duration::from_millis(249), 8192), 0);
        assert_eq!(budget.allowance(b, policy, now, 8192), 8192);
        let lower = ReplicaPolicy { max_kib_per_second: Some(4), ..policy };
        assert_eq!(budget.allowance(a, lower, now + Duration::from_millis(249), 8192), 0);
        assert_eq!(budget.allowance(a, lower, now + Duration::from_millis(250), 8192), 1024);
        assert_eq!(budget.allowance(a, lower, now + Duration::from_secs(3600), 8192), 1024);
    }

    #[test]
    fn utc_parse_is_strict() {
        assert_eq!(parse_utc_minute("23:59").unwrap(), 1439);
        for value in ["24:00", "12:60", "2:00", "00:0", "+1:00", "00:00:00"] {
            assert!(parse_utc_minute(value).is_err());
        }
    }

    #[test]
    fn hot_preference_is_bounded_and_never_served_is_first() {
        let now = Instant::now();
        let normal = scheduling_key(Some(now), false);
        assert!(scheduling_key(None, false) < scheduling_key(Some(now), true));
        assert!(scheduling_key(Some(now + Duration::from_millis(200)), true) < normal);
        assert!(scheduling_key(Some(now + Duration::from_millis(501)), true) > normal);
    }
}
