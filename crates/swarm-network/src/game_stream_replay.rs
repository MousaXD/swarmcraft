//! Durable one-use admission of signed Minecraft game-stream challenges.
//!
//! The game route verifier persists the accepted generation, but an in-memory
//! stream nonce cache would forget which signed challenge had already opened
//! a Minecraft stream if the host gateway process restarted. Preserve that
//! one-use invariant with a small, private, per-world ledger.
//!
//! This ledger is NEVER proof of authority. The caller MUST first verify the
//! route signature, current quorum, requester membership, fencing, hosting
//! consent and fresh Fabric readiness, and MUST repeat those checks while a
//! stream is active. The ledger only prevents replay within the signed
//! response's validity window, including across process restarts.

use crate::game_route::{
    ensure_private_floor_directory, private_floor_options, reject_symlink, replace_floor_atomically, sync_directory,
    MAX_ROUTE_RESPONSE_AGE_MS,
};
use fs2::FileExt;
use rand_core::{OsRng, RngCore};
use serde::{
    de::{self, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use swarm_protocol::WorldId;
use thiserror::Error;

pub const MAX_DURABLE_GAME_REPLAYS: usize = 256;
const REPLAY_LEDGER_VERSION: u8 = 1;
const MAX_LEDGER_BYTES: u64 = 128 + (MAX_DURABLE_GAME_REPLAYS as u64 * 48);
const TIMESTAMP_TOLERANCE_MS: u64 = 2_000;

#[derive(Debug, Error)]
pub enum GameStreamReplayError {
    #[error("signed game-stream challenge already consumed")]
    Replayed,
    #[error("game stream replay ledger is at capacity")]
    Capacity,
    #[error("malformed challenge or unsafe/corrupt replay ledger")]
    InvalidState,
    #[error("durable game-stream replay storage failed: {0}")]
    Storage(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct DurableGameStreamReplayLedger {
    directory: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct ReplayEntry {
    challenge_nonce: [u8; 32],
    signed_response_expiry_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct ReplayRecord {
    version: u8,
    world_id: WorldId,
    #[serde(deserialize_with = "deserialize_bounded_entries")]
    entries: Vec<ReplayEntry>,
}

/// A malicious/corrupt postcard length prefix must never allocate a giant
/// vector before the outer byte limit or entry-count validation can run.
fn deserialize_bounded_entries<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<ReplayEntry>, D::Error> {
    struct BoundedVisitor;
    impl<'de> Visitor<'de> for BoundedVisitor {
        type Value = Vec<ReplayEntry>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {MAX_DURABLE_GAME_REPLAYS} signed game-stream challenges")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let count = seq.size_hint().unwrap_or(0);
            if count > MAX_DURABLE_GAME_REPLAYS {
                return Err(de::Error::custom("oversized replay entry count"));
            }
            let mut entries = Vec::with_capacity(count);
            while let Some(entry) = seq.next_element()? {
                if entries.len() >= MAX_DURABLE_GAME_REPLAYS {
                    return Err(de::Error::custom("too many signed game-stream challenges"));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_seq(BoundedVisitor)
}

impl DurableGameStreamReplayLedger {
    /// Use a PRIVATE durable user-data folder, never an ephemeral process
    /// directory, imported world save or configured untrusted path.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self { directory: directory.into() }
    }

    /// Consume the signed challenge nonce at most once for its exact world.
    ///
    /// Atomically persist the consumed nonce *before* forwarding any game
    /// bytes. A crash after ledger persistence may deny a legitimate retry,
    /// but must never admit a duplicate stream.
    pub fn consume(
        &self,
        world_id: WorldId,
        challenge_nonce: [u8; 32],
        signed_response_expiry_ms: u64,
        now_unix_ms: u64,
    ) -> Result<(), GameStreamReplayError> {
        if world_id == WorldId::default()
            || challenge_nonce == [0; 32]
            || signed_response_expiry_ms <= now_unix_ms
            || signed_response_expiry_ms
                > now_unix_ms.saturating_add(MAX_ROUTE_RESPONSE_AGE_MS + TIMESTAMP_TOLERANCE_MS)
        {
            return Err(GameStreamReplayError::InvalidState);
        }
        ensure_private_floor_directory(&self.directory).map_err(|_| GameStreamReplayError::InvalidState)?;
        let basename = world_id.to_hex();
        let lock_path = self.directory.join(format!("{basename}.replay.lock"));
        reject_symlink(&lock_path).map_err(|_| GameStreamReplayError::InvalidState)?;
        let lock = private_floor_options().read(true).write(true).create(true).truncate(false).open(&lock_path)?;
        lock.lock_exclusive()?;

        let outcome = self.consume_locked(world_id, &basename, challenge_nonce, signed_response_expiry_ms, now_unix_ms);
        FileExt::unlock(&lock)?;
        outcome
    }

    fn consume_locked(
        &self,
        world_id: WorldId,
        basename: &str,
        nonce: [u8; 32],
        expires_ms: u64,
        now_ms: u64,
    ) -> Result<(), GameStreamReplayError> {
        let path = self.directory.join(format!("{basename}.replay"));
        reject_symlink(&path).map_err(|_| GameStreamReplayError::InvalidState)?;
        let mut record = if path.exists() {
            Self::read_record(&path, world_id)?
        } else {
            ReplayRecord { version: REPLAY_LEDGER_VERSION, world_id, entries: Vec::new() }
        };
        // Old challenges cannot be replayed once expired because the signed
        // response verifier also rejects them. Prune only those entries.
        record.entries.retain(|entry| entry.signed_response_expiry_ms > now_ms);
        if record.entries.iter().any(|entry| entry.challenge_nonce == nonce) {
            return Err(GameStreamReplayError::Replayed);
        }
        if record.entries.len() >= MAX_DURABLE_GAME_REPLAYS {
            return Err(GameStreamReplayError::Capacity);
        }
        record.entries.push(ReplayEntry { challenge_nonce: nonce, signed_response_expiry_ms: expires_ms });
        let bytes = postcard::to_allocvec(&record).map_err(|_| GameStreamReplayError::InvalidState)?;
        if bytes.len() as u64 > MAX_LEDGER_BYTES {
            return Err(GameStreamReplayError::Capacity);
        }

        let mut randomness = [0; 8];
        OsRng.fill_bytes(&mut randomness);
        let tmp = self.directory.join(format!(".{basename}-{}.replay-tmp", u64::from_be_bytes(randomness)));
        let mut file = private_floor_options().write(true).create_new(true).open(&tmp)?;
        let committed = (|| -> Result<(), GameStreamReplayError> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            replace_floor_atomically(&tmp, &path)?;
            sync_directory(&self.directory).map_err(|_| GameStreamReplayError::InvalidState)?;
            Ok(())
        })();
        if committed.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        committed
    }

    fn read_record(path: &Path, world_id: WorldId) -> Result<ReplayRecord, GameStreamReplayError> {
        let mut bytes = Vec::new();
        File::open(path)?.take(MAX_LEDGER_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_LEDGER_BYTES {
            return Err(GameStreamReplayError::InvalidState);
        }
        let (record, trailing): (ReplayRecord, &[u8]) =
            postcard::take_from_bytes(&bytes).map_err(|_| GameStreamReplayError::InvalidState)?;
        if !trailing.is_empty()
            || record.version != REPLAY_LEDGER_VERSION
            || record.world_id != world_id
            || record.entries.len() > MAX_DURABLE_GAME_REPLAYS
            || record
                .entries
                .iter()
                .any(|entry| entry.challenge_nonce == [0; 32] || entry.signed_response_expiry_ms == 0)
        {
            return Err(GameStreamReplayError::InvalidState);
        }
        let mut seen = std::collections::HashSet::new();
        if record.entries.iter().any(|entry| !seen.insert(entry.challenge_nonce)) {
            return Err(GameStreamReplayError::InvalidState);
        }
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn secured() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        dir
    }

    #[test]
    fn signed_challenge_cannot_reopen_after_host_process_restart() {
        let dir = secured();
        let path = dir.path().join("game-replay");
        let world = WorldId([1; 32]);
        let nonce = [7; 32];
        let now = 1_000_000;
        DurableGameStreamReplayLedger::new(&path).consume(world, nonce, now + 2_000, now).unwrap();
        let restarted = DurableGameStreamReplayLedger::new(path);
        assert!(matches!(
            restarted.consume(world, nonce, now + 2_000, now + 150),
            Err(GameStreamReplayError::Replayed)
        ));
        assert!(restarted.consume(world, [8; 32], now + 2_500, now + 150).is_ok());
        assert!(restarted.consume(WorldId([2; 32]), nonce, now + 2_000, now + 150).is_ok());
    }

    #[test]
    fn corrupted_world_or_noncanonical_ledger_never_resets_replay_protection() {
        let dir = secured();
        let path = dir.path().join("game-replay");
        let ledger = DurableGameStreamReplayLedger::new(&path);
        let world = WorldId([1; 32]);
        ledger.consume(world, [3; 32], 1050, 1000).unwrap();
        let record = path.join(format!("{}.replay", world.to_hex()));
        let old = std::fs::read(&record).unwrap();
        std::fs::write(&record, b"invalid-ledger").unwrap();
        assert!(matches!(ledger.consume(world, [4; 32], 1050, 1000), Err(GameStreamReplayError::InvalidState)));
        std::fs::write(&record, [old, vec![0xaa]].concat()).unwrap();
        assert!(matches!(ledger.consume(world, [4; 32], 1050, 1000), Err(GameStreamReplayError::InvalidState)));
        std::fs::write(&record, vec![0; MAX_LEDGER_BYTES as usize + 5]).unwrap();
        assert!(matches!(ledger.consume(world, [4; 32], 1050, 1000), Err(GameStreamReplayError::InvalidState)));
    }

    #[test]
    fn two_contending_process_style_receivers_cannot_consume_the_same_nonce() {
        let dir = secured();
        let ledger = Arc::new(DurableGameStreamReplayLedger::new(dir.path().join("game-replay")));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let ledger = Arc::clone(&ledger);
            workers.push(std::thread::spawn(move || ledger.consume(WorldId([3; 32]), [13; 32], 1_050, 1_000)));
        }
        let results = workers.into_iter().map(|thread| thread.join().unwrap()).collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| matches!(result, Err(GameStreamReplayError::Replayed))).count(), 7);
    }

    #[test]
    fn invalid_or_expired_signed_nonce_and_untrusted_parent_fail_closed() {
        let dir = secured();
        let ledger = DurableGameStreamReplayLedger::new(dir.path().join("game-replay"));
        assert!(matches!(
            ledger.consume(WorldId::default(), [2; 32], 1_010, 1_000),
            Err(GameStreamReplayError::InvalidState)
        ));
        assert!(matches!(
            ledger.consume(WorldId([2; 32]), [0; 32], 1_010, 1_000),
            Err(GameStreamReplayError::InvalidState)
        ));
        assert!(matches!(
            ledger.consume(WorldId([2; 32]), [2; 32], 999, 1_000),
            Err(GameStreamReplayError::InvalidState)
        ));
        assert!(matches!(
            ledger.consume(WorldId([2; 32]), [2; 32], 10_000, 1_000),
            Err(GameStreamReplayError::InvalidState)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let unsafe_parent = dir.path().join("shared");
            std::fs::create_dir(&unsafe_parent).unwrap();
            std::fs::set_permissions(&unsafe_parent, std::fs::Permissions::from_mode(0o770)).unwrap();
            let ledger = DurableGameStreamReplayLedger::new(unsafe_parent.join("game-replay"));
            assert!(matches!(
                ledger.consume(WorldId([1; 32]), [3; 32], 1_500, 1_000),
                Err(GameStreamReplayError::InvalidState)
            ));
        }
    }

    #[test]
    fn oversized_postcard_vector_length_is_rejected_without_allocating_elements() {
        let dir = secured();
        let ledger = DurableGameStreamReplayLedger::new(dir.path().join("game-replay"));
        let world = WorldId([23; 32]);
        ledger.consume(world, [1; 32], 1_500, 1_000).unwrap();
        let path = ledger.directory.join(format!("{}.replay", world.to_hex()));
        // One version byte, exact world ID, then a deliberately huge Vec
        // length in postcard's little-endian varint encoding.
        let mut poisoned = vec![REPLAY_LEDGER_VERSION];
        poisoned.extend_from_slice(&world.0);
        poisoned.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        std::fs::write(path, poisoned).unwrap();
        assert!(matches!(ledger.consume(world, [2; 32], 1_500, 1_000), Err(GameStreamReplayError::InvalidState)));
    }
}
