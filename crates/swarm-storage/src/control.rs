use crate::{Storage, StorageError};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use swarm_protocol::{AuthorityLeaseGrantV1, AuthorityTransferV1, EpochRecordV1, SleepRecordV1, WorldId};

impl Storage {
    pub fn save_epoch_record(&self, record: &EpochRecordV1) -> Result<(), StorageError> {
        record.validate_semantics()?;
        let bytes = postcard::to_allocvec(record)?;
        atomic_write(&self.control_path(record.world_id, "epoch.postcard"), &bytes)
    }

    pub fn load_epoch_record(&self, world: WorldId) -> Result<EpochRecordV1, StorageError> {
        let path = self.control_path(world, "epoch.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let record: EpochRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    pub fn reserve_recovery(&self, reservation: &AuthorityLeaseGrantV1) -> Result<bool, StorageError> {
        reservation.validate_semantics()?;
        if let Ok(existing) = self.load_recovery_reservation(reservation.world_id) {
            let existing_generation = (existing.epoch, existing.fencing_token);
            let requested_generation = (reservation.epoch, reservation.fencing_token);
            if requested_generation < existing_generation {
                return Ok(false);
            }
            if requested_generation == existing_generation {
                return Ok(existing.authority_peer_id == reservation.authority_peer_id
                    && existing.authority_public_key == reservation.authority_public_key);
            }
        }
        self.save_recovery_reservation(reservation)?;
        Ok(true)
    }

    pub fn save_recovery_reservation(&self, reservation: &AuthorityLeaseGrantV1) -> Result<(), StorageError> {
        reservation.validate_semantics()?;
        let bytes = postcard::to_allocvec(reservation)?;
        atomic_write(&self.control_path(reservation.world_id, "recovery-reservation.postcard"), &bytes)
    }

    pub fn load_recovery_reservation(&self, world: WorldId) -> Result<AuthorityLeaseGrantV1, StorageError> {
        let path = self.control_path(world, "recovery-reservation.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let reservation: AuthorityLeaseGrantV1 = postcard::from_bytes(&bytes)?;
        if reservation.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        reservation.validate_semantics()?;
        Ok(reservation)
    }

    pub fn clear_recovery_reservation(&self, world: WorldId) -> Result<(), StorageError> {
        remove_if_present(&self.control_path(world, "recovery-reservation.postcard"))
    }

    pub fn save_transfer_record(&self, transfer: &AuthorityTransferV1) -> Result<(), StorageError> {
        transfer.validate_semantics()?;
        let bytes = postcard::to_allocvec(transfer)?;
        atomic_write(&self.control_path(transfer.world_id, "transfer.postcard"), &bytes)
    }

    pub fn load_transfer_record(&self, world: WorldId) -> Result<AuthorityTransferV1, StorageError> {
        let path = self.control_path(world, "transfer.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let record: AuthorityTransferV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    pub fn save_sleep_record(&self, record: &SleepRecordV1) -> Result<(), StorageError> {
        record.validate_semantics()?;
        let bytes = postcard::to_allocvec(record)?;
        atomic_write(&self.control_path(record.world_id, "sleep.postcard"), &bytes)
    }

    pub fn load_sleep_record(&self, world: WorldId) -> Result<SleepRecordV1, StorageError> {
        let path = self.control_path(world, "sleep.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let record: SleepRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    pub fn clear_sleep_record(&self, world: WorldId) -> Result<(), StorageError> {
        remove_if_present(&self.control_path(world, "sleep.postcard"))
    }

    fn control_path(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
}

fn remove_if_present(path: &Path) -> Result<(), StorageError> {
    if path.exists() {
        fs::remove_file(path).map_err(|error| io_error(path, error))?;
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
    use swarm_protocol::{EpochMode, Hash32, PeerId, PROTOCOL_VERSION};

    fn reservation(world: WorldId, peer: u8, epoch: u64, fencing_token: u64) -> AuthorityLeaseGrantV1 {
        AuthorityLeaseGrantV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch,
            fencing_token,
            lease_duration_ms: 5_000,
            authority_peer_id: PeerId([peer; 32]),
            authority_public_key: [peer; 32],
            nonce: [3; 32],
            signature: vec![2; 64],
        }
    }

    #[test]
    fn epoch_record_round_trip_preserves_fencing_generation() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([3; 32]);
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        let record = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: 4,
            previous_epoch_hash: None,
            base_state_hash: Hash32([9; 32]),
            authority_peer_id: PeerId([2; 32]),
            authority_public_key: [7; 32],
            mode: EpochMode::Quorum,
            fencing_token: 11,
            reason: "test".into(),
            signature: vec![1; 64],
        };
        store.save_epoch_record(&record).unwrap();
        let loaded = store.load_epoch_record(world).unwrap();
        assert_eq!(loaded.epoch_number, 4);
        assert_eq!(loaded.fencing_token, 11);
    }

    #[test]
    fn unsupported_control_record_version_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([8; 32]);
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        let record = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION + 1,
            world_id: world,
            epoch_number: 1,
            previous_epoch_hash: None,
            base_state_hash: Hash32([9; 32]),
            authority_peer_id: PeerId([2; 32]),
            authority_public_key: [2; 32],
            mode: EpochMode::Quorum,
            fencing_token: 1,
            reason: "unsupported".into(),
            signature: vec![1; 64],
        };
        assert!(store.save_epoch_record(&record).is_err());
        assert!(store.load_epoch_record(world).is_err());
    }

    #[test]
    fn recovery_reservation_round_trip_is_durable() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([4; 32]);
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        let value = reservation(world, 6, 8, 12);
        assert!(store.reserve_recovery(&value).unwrap());
        assert_eq!(store.load_recovery_reservation(world).unwrap(), value);
        store.clear_recovery_reservation(world).unwrap();
        assert!(store.load_recovery_reservation(world).is_err());
    }

    #[test]
    fn same_generation_cannot_be_reserved_for_two_authorities() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([5; 32]);
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        let first = reservation(world, 6, 8, 12);
        let conflicting = reservation(world, 7, 8, 12);
        assert!(store.reserve_recovery(&first).unwrap());
        assert!(!store.reserve_recovery(&conflicting).unwrap());
        assert_eq!(store.load_recovery_reservation(world).unwrap().authority_peer_id, first.authority_peer_id);
        assert!(store.reserve_recovery(&first).unwrap());
    }

    #[test]
    fn newer_generation_can_replace_old_reservation_but_older_cannot() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([6; 32]);
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        let old = reservation(world, 6, 8, 12);
        let new = reservation(world, 7, 9, 13);
        assert!(store.reserve_recovery(&old).unwrap());
        assert!(store.reserve_recovery(&new).unwrap());
        assert!(!store.reserve_recovery(&old).unwrap());
        assert_eq!(store.load_recovery_reservation(world).unwrap().authority_peer_id, new.authority_peer_id);
    }
}
