use crate::{Storage, StorageError};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use swarm_protocol::{peer_id_from_public_key, RecoveryBallotV1, RecoveryVoteV1, SoloBranchV1, WorldConfigV1, WorldId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableRecoveryPromiseV1 {
    pub ballot: RecoveryBallotV1,
    pub vote: RecoveryVoteV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPromiseResult {
    Accepted,
    Idempotent,
    Rejected { highest_round: u64 },
}

impl Storage {
    pub fn promise_recovery_ballot(
        &self,
        ballot: &RecoveryBallotV1,
        vote: &RecoveryVoteV1,
    ) -> Result<RecoveryPromiseResult, StorageError> {
        ballot.validate_semantics()?;
        vote.validate_semantics()?;
        if !vote.matches_ballot(ballot)?
            || peer_id_from_public_key(&ballot.candidate_public_key) != ballot.candidate_peer_id
            || peer_id_from_public_key(&vote.voter_public_key) != vote.voter_peer_id
        {
            return Ok(RecoveryPromiseResult::Rejected {
                highest_round: self.load_recovery_promise(ballot.world_id).map_or(0, |value| value.ballot.round),
            });
        }

        if let Ok(existing) = self.load_recovery_promise(ballot.world_id) {
            if ballot.round < existing.ballot.round {
                return Ok(RecoveryPromiseResult::Rejected { highest_round: existing.ballot.round });
            }
            if ballot.round == existing.ballot.round {
                if ballot.ballot_hash()? == existing.ballot.ballot_hash()? {
                    return Ok(RecoveryPromiseResult::Idempotent);
                }
                return Ok(RecoveryPromiseResult::Rejected { highest_round: existing.ballot.round });
            }
            if !same_recovery_base(&existing.ballot, ballot) {
                return Ok(RecoveryPromiseResult::Rejected { highest_round: existing.ballot.round });
            }
        }

        let promise = DurableRecoveryPromiseV1 { ballot: ballot.clone(), vote: vote.clone() };
        let bytes = postcard::to_allocvec(&promise)?;
        atomic_write(&self.control_path_v2(ballot.world_id, "recovery-promise.postcard"), &bytes)?;
        Ok(RecoveryPromiseResult::Accepted)
    }

    pub fn load_recovery_promise(&self, world: WorldId) -> Result<DurableRecoveryPromiseV1, StorageError> {
        let path = self.control_path_v2(world, "recovery-promise.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let promise: DurableRecoveryPromiseV1 = postcard::from_bytes(&bytes)?;
        promise.ballot.validate_semantics()?;
        promise.vote.validate_semantics()?;
        if promise.ballot.world_id != world
            || promise.vote.world_id != world
            || !promise.vote.matches_ballot(&promise.ballot)?
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(promise)
    }

    /// Clear only after the accepted canonical epoch has advanced beyond this
    /// promise. Callers must not use this as a timeout mechanism.
    pub fn clear_recovery_promise_after_epoch_advance(
        &self,
        world: WorldId,
        accepted_epoch: u64,
    ) -> Result<bool, StorageError> {
        let Ok(promise) = self.load_recovery_promise(world) else {
            return Ok(false);
        };
        if accepted_epoch < promise.ballot.target_epoch {
            return Ok(false);
        }
        remove_if_present(&self.control_path_v2(world, "recovery-promise.postcard"))?;
        Ok(true)
    }

    /// WorldConfig authority transition rule:
    ///
    /// * exact canonical duplicates are idempotent and never mutate history;
    /// * before the first accepted epoch, a new config must be signed by the
    ///   bootstrap membership authority;
    /// * once an epoch exists, every new config generation must be signed by
    ///   the authority peer/key of that currently accepted epoch;
    /// * sequence and previous hash must extend the accepted config exactly.
    pub fn save_world_config(&self, config: &WorldConfigV1) -> Result<(), StorageError> {
        let mut config = config.clone();
        config.normalize_canonical();
        config.validate_semantics()?;
        if config.world_id != self.load_world(config.world_id)?.world_id {
            return Err(StorageError::WorldMetadataMismatch);
        }

        if let Ok(existing) = self.load_world_config(config.world_id) {
            if config == existing {
                return Ok(());
            }
            if config.sequence <= existing.sequence {
                return Err(StorageError::WorldMetadataMismatch);
            }
            let expected_sequence = next_world_config_sequence(existing.sequence)?;
            if config.sequence != expected_sequence || config.previous_config_hash != Some(existing.config_hash()?) {
                return Err(StorageError::WorldMetadataMismatch);
            }
        } else if config.sequence != 1 || config.previous_config_hash.is_some() {
            return Err(StorageError::WorldMetadataMismatch);
        }

        if let Ok(epoch) = self.load_epoch_record(config.world_id) {
            if config.authority_peer_id != epoch.authority_peer_id
                || config.authority_public_key != epoch.authority_public_key
            {
                return Err(StorageError::WorldMetadataMismatch);
            }
        } else {
            let membership = self.load_membership_record(config.world_id)?;
            if config.authority_peer_id != membership.authority_peer_id
                || config.authority_public_key != membership.authority_public_key
            {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }

        let bytes = postcard::to_allocvec(&config)?;
        atomic_write(&self.control_path_v2(config.world_id, "world-config.postcard"), &bytes)
    }

    pub fn load_world_config(&self, world: WorldId) -> Result<WorldConfigV1, StorageError> {
        let path = self.control_path_v2(world, "world-config.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let config: WorldConfigV1 = postcard::from_bytes(&bytes)?;
        if config.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        config.validate_semantics()?;
        Ok(config)
    }

    pub fn save_solo_branch(&self, branch: &SoloBranchV1) -> Result<(), StorageError> {
        branch.validate_semantics()?;
        let bytes = postcard::to_allocvec(branch)?;
        atomic_write(&self.control_path_v2(branch.world_id, "solo-branch.postcard"), &bytes)
    }

    pub fn load_solo_branch(&self, world: WorldId) -> Result<SoloBranchV1, StorageError> {
        let path = self.control_path_v2(world, "solo-branch.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let branch: SoloBranchV1 = postcard::from_bytes(&bytes)?;
        if branch.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        branch.validate_semantics()?;
        Ok(branch)
    }

    pub fn preserve_solo_conflict(&self, branch: &SoloBranchV1) -> Result<PathBuf, StorageError> {
        branch.validate_semantics()?;
        let hash = branch.branch_hash()?;
        let path = self
            .world_dir(branch.world_id)
            .join("recovery")
            .join("solo-conflicts")
            .join(format!("{}.postcard", hash.to_hex()));
        if !path.exists() {
            atomic_write(&path, &postcard::to_allocvec(branch)?)?;
        }
        Ok(path)
    }

    pub fn list_solo_conflicts(&self, world: WorldId) -> Result<Vec<SoloBranchV1>, StorageError> {
        let dir = self.world_dir(world).join("recovery").join("solo-conflicts");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut branches = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|error| io_error(&dir, error))? {
            let entry = entry.map_err(|error| io_error(&dir, error))?;
            if !entry.file_type().map_err(|error| io_error(entry.path(), error))?.is_file() {
                continue;
            }
            let bytes = fs::read(entry.path()).map_err(|error| io_error(entry.path(), error))?;
            let branch: SoloBranchV1 = postcard::from_bytes(&bytes)?;
            if branch.world_id == world {
                branch.validate_semantics()?;
                branches.push(branch);
            }
        }
        branches.sort_by_key(|branch| (branch.head_epoch, branch.head_sequence, branch.head_snapshot_hash));
        Ok(branches)
    }

    pub fn set_background_seeding(&self, world: WorldId, enabled: bool) -> Result<(), StorageError> {
        self.load_world(world)?;
        atomic_write(&self.control_path_v2(world, "background-seeding"), if enabled { b"1\n" } else { b"0\n" })
    }

    pub fn background_seeding_enabled(&self, world: WorldId) -> Result<bool, StorageError> {
        let path = self.control_path_v2(world, "background-seeding");
        if !path.exists() {
            return Ok(false);
        }
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        Ok(bytes == b"1\n")
    }

    fn control_path_v2(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
}

fn next_world_config_sequence(sequence: u64) -> Result<u64, StorageError> {
    sequence.checked_add(1).ok_or(StorageError::WorldMetadataMismatch)
}

fn same_recovery_base(a: &RecoveryBallotV1, b: &RecoveryBallotV1) -> bool {
    a.world_id == b.world_id
        && a.base_epoch == b.base_epoch
        && a.base_fencing_token == b.base_fencing_token
        && a.target_epoch == b.target_epoch
        && a.target_fencing_token == b.target_fencing_token
        && a.candidate_peer_id == b.candidate_peer_id
        && a.candidate_public_key == b.candidate_public_key
        && a.base_snapshot_hash == b.base_snapshot_hash
        && a.base_state_hash == b.base_state_hash
        && a.membership_hash == b.membership_hash
}

fn remove_if_present(path: &Path) -> Result<(), StorageError> {
    if path.exists() {
        fs::remove_file(path).map_err(|error| io_error(path, error))?;
        if let Some(parent) = path.parent() {
            sync_parent(parent)?;
        }
    }
    Ok(())
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    let tmp = path.with_extension("tmp");
    let mut file =
        OpenOptions::new().create(true).truncate(true).write(true).open(&tmp).map_err(|error| io_error(&tmp, error))?;
    file.write_all(bytes).map_err(|error| io_error(&tmp, error))?;
    file.sync_all().map_err(|error| io_error(&tmp, error))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|error| io_error(path, error))?;
    sync_parent(parent)
}

fn sync_parent(parent: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        fs::File::open(parent).and_then(|file| file.sync_all()).map_err(|error| io_error(parent, error))?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldMetadataV1;
    use swarm_protocol::{
        AuthorityPolicyV1, EpochMode, EpochRecordV1, Hash32, MembershipPolicyV1, MembershipRecordV1, PeerId,
        RuntimeCompatibilityManifestV1, WorldGenesisV1, WorldMemberV1, WorldPresentationV1, WorldVisibilityV1,
        PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
    };

    fn test_world() -> (WorldGenesisV1, WorldId) {
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([9; 32]),
            creation_nonce: [8; 32],
            creator_public_key: [7; 32],
            initial_membership: vec![PeerId([6; 32])],
        };
        let world = genesis.world_id().unwrap();
        (genesis, world)
    }

    fn ballot(world: WorldId, candidate: u8, round: u64) -> RecoveryBallotV1 {
        let candidate_public_key = [candidate; 32];
        RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            base_epoch: 4,
            base_fencing_token: 8,
            target_epoch: 5,
            target_fencing_token: 9,
            round,
            candidate_peer_id: peer_id_from_public_key(&candidate_public_key),
            candidate_public_key,
            base_snapshot_hash: Hash32([3; 32]),
            base_state_hash: Hash32([4; 32]),
            membership_hash: Hash32([5; 32]),
            signature: Vec::new(),
        }
    }

    fn vote(ballot: &RecoveryBallotV1, voter: u8) -> RecoveryVoteV1 {
        let voter_public_key = [voter; 32];
        RecoveryVoteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: ballot.world_id,
            ballot_hash: ballot.ballot_hash().unwrap(),
            base_epoch: ballot.base_epoch,
            target_epoch: ballot.target_epoch,
            round: ballot.round,
            candidate_peer_id: ballot.candidate_peer_id,
            voter_peer_id: peer_id_from_public_key(&voter_public_key),
            voter_public_key,
            signature: Vec::new(),
        }
    }

    fn bootstrap_membership(world: WorldId) -> MembershipRecordV1 {
        MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: 0,
            sequence: 0,
            previous_membership_hash: None,
            members: vec![WorldMemberV1 {
                peer_id: PeerId([6; 32]),
                public_key: [6; 32],
                authority_eligible: true,
                banned: false,
            }],
            authority_peer_id: PeerId([6; 32]),
            authority_public_key: [6; 32],
            signature: Vec::new(),
        }
    }

    fn world_config(world: WorldId, authority: u8, sequence: u64, previous: Option<Hash32>) -> WorldConfigV1 {
        WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            sequence,
            previous_config_hash: previous,
            compatibility: RuntimeCompatibilityManifestV1 {
                minecraft_version: "1.21.8".into(),
                loader_id: "fabric".into(),
                loader_version: "0.17.2".into(),
                swarmcraft_protocol_version: PROTOCOL_VERSION,
                fabric_adapter_version: "0.2.0".into(),
                required_server_mods: Vec::new(),
                required_client_mods: Vec::new(),
                datapacks: Vec::new(),
            },
            visibility: WorldVisibilityV1::Private,
            authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 3 },
            membership_policy: MembershipPolicyV1::InviteOnly,
            presentation: WorldPresentationV1 {
                name: "test".into(),
                description: String::new(),
                tags: Vec::new(),
                icon_hash: None,
                approximate_region: None,
            },
            authority_peer_id: PeerId([authority; 32]),
            authority_public_key: [authority; 32],
            signature: vec![authority; 64],
        }
    }

    #[test]
    fn recovery_promise_survives_restart_and_preserves_the_accepted_value() {
        let temp = tempfile::tempdir().unwrap();
        let (_, world) = test_world();
        let store = Storage::open(temp.path()).unwrap();
        let bob = ballot(world, 2, 1);
        assert_eq!(store.promise_recovery_ballot(&bob, &vote(&bob, 6)).unwrap(), RecoveryPromiseResult::Accepted);
        drop(store);

        let store = Storage::open(temp.path()).unwrap();
        let charlie = ballot(world, 3, 2);
        assert_eq!(
            store.promise_recovery_ballot(&charlie, &vote(&charlie, 6)).unwrap(),
            RecoveryPromiseResult::Rejected { highest_round: 1 }
        );
        let bob_round_two = ballot(world, 2, 2);
        assert_eq!(
            store.promise_recovery_ballot(&bob_round_two, &vote(&bob_round_two, 6)).unwrap(),
            RecoveryPromiseResult::Accepted
        );
    }

    #[test]
    fn recovery_promise_is_not_cleared_until_epoch_advances() {
        let temp = tempfile::tempdir().unwrap();
        let (_, world) = test_world();
        let store = Storage::open(temp.path()).unwrap();
        let bob = ballot(world, 2, 1);
        store.promise_recovery_ballot(&bob, &vote(&bob, 6)).unwrap();
        assert!(!store.clear_recovery_promise_after_epoch_advance(world, 4).unwrap());
        assert!(store.load_recovery_promise(world).is_ok());
        assert!(store.clear_recovery_promise_after_epoch_advance(world, 5).unwrap());
        assert!(store.load_recovery_promise(world).is_err());
    }

    #[test]
    fn world_config_sequence_exhaustion_fails_closed() {
        assert_eq!(next_world_config_sequence(u64::MAX - 1).unwrap(), u64::MAX);
        assert!(next_world_config_sequence(u64::MAX).is_err());
    }

    #[test]
    fn world_config_authority_tracks_the_current_accepted_epoch() {
        let temp = tempfile::tempdir().unwrap();
        let (genesis, world) = test_world();
        let store = Storage::open(temp.path()).unwrap();
        store
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "test".into(),
                world_id: world,
                genesis,
            })
            .unwrap();
        store.save_membership_record(&bootstrap_membership(world)).unwrap();

        let first = world_config(world, 6, 1, None);
        store.save_world_config(&first).unwrap();
        assert_eq!(store.load_world_config(world).unwrap(), first);

        store
            .save_epoch_record(&EpochRecordV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                epoch_number: 1,
                previous_epoch_hash: None,
                base_state_hash: Hash32([8; 32]),
                authority_peer_id: PeerId([7; 32]),
                authority_public_key: [7; 32],
                mode: EpochMode::Recovery,
                fencing_token: 1,
                reason: "authority transition".into(),
                signature: vec![7; 64],
            })
            .unwrap();

        // Replaying the exact accepted value is a no-op even after authority changes.
        store.save_world_config(&first).unwrap();

        let previous = first.config_hash().unwrap();
        let stale_authority = world_config(world, 6, 2, Some(previous));
        assert!(store.save_world_config(&stale_authority).is_err());
        assert_eq!(store.load_world_config(world).unwrap(), first);

        let current_authority = world_config(world, 7, 2, Some(previous));
        store.save_world_config(&current_authority).unwrap();
        assert_eq!(store.load_world_config(world).unwrap(), current_authority);
    }

    #[test]
    fn world_config_and_background_seeding_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let (genesis, world) = test_world();
        let store = Storage::open(temp.path()).unwrap();
        store
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "test".into(),
                world_id: world,
                genesis,
            })
            .unwrap();
        store.save_membership_record(&bootstrap_membership(world)).unwrap();
        let config = world_config(world, 6, 1, None);
        store.save_world_config(&config).unwrap();
        assert_eq!(store.load_world_config(world).unwrap(), config);
        assert!(!store.background_seeding_enabled(world).unwrap());
        store.set_background_seeding(world, true).unwrap();
        assert!(store.background_seeding_enabled(world).unwrap());
    }
}
