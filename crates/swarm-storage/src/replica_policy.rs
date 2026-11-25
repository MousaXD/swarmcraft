//! Device-local placement and ordinary replication preferences, never signed
//! world configuration, authority, hosting consent or a retention policy.
use crate::{
    transaction::{canonical_file_exists, durable_atomic_write, io_error, read_canonical_file},
    Storage, StorageError,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use swarm_protocol::WorldId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UtcWindow {
    pub start_minute: u16,
    pub end_minute: u16,
}

impl UtcWindow {
    pub fn contains(self, minute: u16) -> bool {
        if self.start_minute < self.end_minute {
            (self.start_minute..self.end_minute).contains(&minute)
        } else {
            minute >= self.start_minute || minute < self.end_minute
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicaPolicy {
    pub follow: bool,
    /// Give this world's ordinary pulls a bounded scheduling preference.
    #[serde(default)]
    pub hot: bool,
    pub utc_window: Option<UtcWindow>,
    /// Aggregate ordinary encoded payload request/upload budget per world.
    pub max_kib_per_second: Option<u32>,
}

impl Default for ReplicaPolicy {
    fn default() -> Self {
        Self { follow: true, hot: false, utc_window: None, max_kib_per_second: None }
    }
}

impl ReplicaPolicy {
    pub fn validate(self) -> Result<Self, StorageError> {
        if self
            .utc_window
            .is_some_and(|w| w.start_minute >= 1440 || w.end_minute >= 1440 || w.start_minute == w.end_minute)
        {
            return Err(StorageError::InvalidReplicaPolicy("UTC window needs distinct endpoints in 00:00..23:59"));
        }
        if self.max_kib_per_second.is_some_and(|rate| !(4..=65_536).contains(&rate)) {
            return Err(StorageError::InvalidReplicaPolicy("rate must be 4..65536 KiB/s"));
        }
        Ok(self)
    }

    pub fn window_open(self, minute: u16) -> bool {
        minute < 1440 && self.utc_window.is_none_or(|w| w.contains(minute))
    }

    /// Never negotiate legacy push when an inbound limit must be enforced
    /// before the remote peer sends payload bytes.
    pub fn requires_controlled_pull(self) -> bool {
        self.utc_window.is_some() || self.max_kib_per_second.is_some()
    }
}

impl Storage {
    fn replica_policy_path(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("metadata/local-replica-policy.json")
    }

    pub fn world_replica_policy(&self, world: WorldId) -> Result<ReplicaPolicy, StorageError> {
        let path = self.replica_policy_path(world);
        if !canonical_file_exists(&path)? {
            return Ok(ReplicaPolicy::default());
        }
        if fs::metadata(&path).map_err(|error| io_error(&path, error))?.len() > 4096 {
            return Err(StorageError::InvalidReplicaPolicy("policy exceeds size limit"));
        }
        serde_json::from_slice::<ReplicaPolicy>(&read_canonical_file(&path)?)?.validate()
    }

    pub fn set_world_replica_policy(&self, world: WorldId, policy: ReplicaPolicy) -> Result<(), StorageError> {
        let policy = policy.validate()?;
        self.load_world(world)?;
        let _lock = self.lock_world_transaction(world)?;
        let path = self.replica_policy_path(world);
        canonical_file_exists(&path)?;
        durable_atomic_write(&path, &serde_json::to_vec(&policy)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_window_boundaries_and_invalid_values() {
        let day = UtcWindow { start_minute: 60, end_minute: 120 };
        assert!(!day.contains(59));
        assert!(day.contains(60));
        assert!(day.contains(119));
        assert!(!day.contains(120));
        let night = UtcWindow { start_minute: 1380, end_minute: 120 };
        for (minute, expected) in [(1379, false), (1380, true), (1439, true), (0, true), (119, true), (120, false)] {
            assert_eq!(night.contains(minute), expected);
        }
        for bad in [UtcWindow { start_minute: 0, end_minute: 1440 }, UtcWindow { start_minute: 1, end_minute: 1 }] {
            assert!(ReplicaPolicy { utc_window: Some(bad), ..Default::default() }.validate().is_err());
        }
        for rate in [0, 3, 65_537, u32::MAX] {
            assert!(ReplicaPolicy { max_kib_per_second: Some(rate), ..Default::default() }.validate().is_err());
        }
        assert!(!ReplicaPolicy::default().requires_controlled_pull());
    }
}
