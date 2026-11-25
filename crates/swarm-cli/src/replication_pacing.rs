//! Local outbound blob pacing. Never used as membership, checkpoint,
//! route, quorum or authority evidence.
use anyhow::{bail, Context, Result};
use std::time::{Duration, Instant};

pub const PLAY_RATE_ENV: &str = "SWARMCRAFT_REPLICATION_PLAY_KIBPS";
const DEFAULT_PLAY_KIBPS: u64 = 64;
const MIN_PLAY_KIBPS: u64 = 4;
const MAX_PLAY_KIBPS: u64 = 128;
const MAX_PLAY_CHUNK: usize = 16 * 1024;
const MIN_SEND_INTERVAL: Duration = Duration::from_millis(125);
const SLOW_ACK: Duration = Duration::from_millis(500);
const FAST_ACK: Duration = Duration::from_millis(150);
const FAST_ACKS_TO_INCREASE: u8 = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum ChunkPlan {
    Ready { max_bytes: usize, paced: bool },
    Deferred(Instant),
}

#[derive(Debug)]
pub struct ReplicationPacer {
    ceiling: u64,
    rate: u64,
    next_send: Option<Instant>,
    active: bool,
    fast_acks: u8,
}

impl Default for ReplicationPacer {
    fn default() -> Self {
        Self::new(DEFAULT_PLAY_KIBPS).expect("default pacing rate is valid")
    }
}

impl ReplicationPacer {
    pub fn new(play_kibps: u64) -> Result<Self> {
        if !(MIN_PLAY_KIBPS..=MAX_PLAY_KIBPS).contains(&play_kibps) {
            bail!("replication play rate must be 4-128 KiB/s");
        }
        let ceiling = play_kibps * 1024;
        Ok(Self { ceiling, rate: ceiling, next_send: None, active: false, fast_acks: 0 })
    }

    pub fn from_environment() -> Result<Self> {
        let rate = match std::env::var(PLAY_RATE_ENV) {
            Ok(value) => value.parse::<u64>().context("invalid replication play rate")?,
            Err(std::env::VarError::NotPresent) => DEFAULT_PLAY_KIBPS,
            Err(error) => return Err(error).context("invalid replication play rate environment"),
        };
        Self::new(rate)
    }

    pub fn plan(&mut self, now: Instant, active_game: bool, normal_limit: usize) -> ChunkPlan {
        if active_game != self.active {
            self.active = active_game;
            self.rate = self.ceiling;
            self.fast_acks = 0;
            self.next_send = None;
        }
        if !active_game {
            return ChunkPlan::Ready { max_bytes: normal_limit, paced: false };
        }
        if let Some(next) = self.next_send {
            if next > now {
                return ChunkPlan::Deferred(next);
            }
        }
        let quantum = (self.rate / 4) as usize;
        ChunkPlan::Ready { max_bytes: normal_limit.min(MAX_PLAY_CHUNK).min(quantum), paced: true }
    }

    pub fn sent(&mut self, now: Instant, bytes: usize, paced: bool) {
        if !paced {
            return;
        }
        // No idle-credit accumulation or catch-up burst. Account for the
        // actual encoded blob bytes, not estimates of total wire traffic.
        let interval = Duration::from_secs_f64(bytes as f64 / self.rate as f64).max(MIN_SEND_INTERVAL);
        self.next_send = Some(now + interval);
    }

    pub fn acknowledge(&mut self, elapsed: Duration, paced: bool) {
        if !paced || !self.active {
            return;
        }
        // A slow blob ACK includes network and receiver storage delay.
        // Treat it conservatively as pressure, never as Minecraft latency.
        if elapsed > SLOW_ACK {
            self.rate = (self.rate / 2).max(MIN_PLAY_KIBPS * 1024);
            self.fast_acks = 0;
        } else if elapsed <= FAST_ACK {
            self.fast_acks += 1;
            if self.fast_acks >= FAST_ACKS_TO_INCREASE {
                self.rate = (self.rate + 4 * 1024).min(self.ceiling);
                self.fast_acks = 0;
            }
        } else {
            self.fast_acks = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_mode_caps_aggregate_burst_and_idle_restores_normal_chunk() {
        let now = Instant::now();
        let mut p = ReplicationPacer::default();
        assert_eq!(p.plan(now, false, 256 * 1024), ChunkPlan::Ready { max_bytes: 256 * 1024, paced: false });
        assert_eq!(p.plan(now, true, 256 * 1024), ChunkPlan::Ready { max_bytes: 16 * 1024, paced: true });
        p.sent(now, 16 * 1024, true);
        assert_eq!(
            p.plan(now + Duration::from_millis(249), true, 256 * 1024),
            ChunkPlan::Deferred(now + Duration::from_millis(250))
        );
        // All peer/world keys share the same next-send deadline.
        assert_eq!(p.plan(now, true, 256 * 1024), ChunkPlan::Deferred(now + Duration::from_millis(250)));
        let later = now + Duration::from_secs(20);
        assert!(matches!(p.plan(later, true, 256 * 1024), ChunkPlan::Ready { .. }));
        p.sent(later, 1, true);
        assert_eq!(p.plan(later, true, 256 * 1024), ChunkPlan::Deferred(later + MIN_SEND_INTERVAL));
        assert_eq!(p.plan(later, false, 256 * 1024), ChunkPlan::Ready { max_bytes: 256 * 1024, paced: false });
    }

    #[test]
    fn slow_feedback_reduces_rate_but_not_progress_and_fast_feedback_recovers() {
        let now = Instant::now();
        let mut p = ReplicationPacer::default();
        p.plan(now, true, 256 * 1024);
        for _ in 0..10 {
            p.acknowledge(Duration::from_secs(2), true);
        }
        assert_eq!(p.rate, MIN_PLAY_KIBPS * 1024);
        assert_eq!(p.plan(now, true, 256 * 1024), ChunkPlan::Ready { max_bytes: 1024, paced: true });
        p.sent(now, 1024, true);
        let later = now + Duration::from_millis(250);
        assert!(matches!(p.plan(later, true, 256 * 1024), ChunkPlan::Ready { .. }));
        for _ in 0..128 {
            p.acknowledge(Duration::from_millis(20), true);
        }
        assert_eq!(p.rate, p.ceiling);
        p.acknowledge(Duration::from_secs(2), false);
        assert_eq!(p.rate, p.ceiling);
    }

    #[test]
    fn invalid_budgets_are_rejected_and_never_disable_replication() {
        assert!(ReplicationPacer::new(0).is_err());
        assert!(ReplicationPacer::new(129).is_err());
        assert!(ReplicationPacer::new(4).is_ok());
    }
}
