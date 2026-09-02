use crate::{
    transaction::{durable_atomic_write, durable_remove},
    Storage, StorageError,
};
use std::{fs, path::PathBuf};
use swarm_protocol::{AuthorityLeaseGrantV1, AuthorityTransferV1, EpochRecordV1, SleepRecordV1, WorldId};

impl Storage {
    pub fn save_epoch_record(&self, record: &EpochRecordV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(record.world_id)?;
        if let Ok(existing) = self.load_epoch_record(record.world_id) {
            let existing_generation = (existing.epoch_number, existing.fencing_token);
            let requested_generation = (record.epoch_number, record.fencing_token);
            if requested_generation < existing_generation {
                return Err(StorageError::WorldMetadataMismatch);
            }
            if requested_generation == existing_generation {
                if existing == *record {
                    return Ok(());
                }
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        let bytes = postcard::to_allocvec(record)?;
        durable_atomic_write(&self.control_path(record.world_id, "epoch.postcard"), &bytes)
    }

    pub fn load_epoch_record(&self, world: WorldId) -> Result<EpochRecordV1, StorageError> {
        let path = self.control_path(world, "epoch.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let record: EpochRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(record)
    }

    pub fn reserve_recovery(&self, reservation: &AuthorityLeaseGrantV1) -> Result<bool, StorageError> {
        let _guard = self.lock_world_transaction(reservation.world_id)?;
        self.reserve_recovery_locked(reservation)
    }

    pub fn save_recovery_reservation(&self, reservation: &AuthorityLeaseGrantV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(reservation.world_id)?;
        if self.reserve_recovery_locked(reservation)? {
            Ok(())
        } else {
            Err(StorageError::WorldMetadataMismatch)
        }
    }

    fn reserve_recovery_locked(&self, reservation: &AuthorityLeaseGrantV1) -> Result<bool, StorageError> {
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
        let bytes = postcard::to_allocvec(reservation)?;
        durable_atomic_write(&self.control_path(reservation.world_id, "recovery-reservation.postcard"), &bytes)?;
        Ok(true)
    }

    pub fn load_recovery_reservation(&self, world: WorldId) -> Result<AuthorityLeaseGrantV1, StorageError> {
        let path = self.control_path(world, "recovery-reservation.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let reservation: AuthorityLeaseGrantV1 = postcard::from_bytes(&bytes)?;
        if reservation.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(reservation)
    }

    pub fn clear_recovery_reservation(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        durable_remove(&self.control_path(world, "recovery-reservation.postcard"))?;
        Ok(())
    }

    pub fn save_transfer_record(&self, transfer: &AuthorityTransferV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(transfer.world_id)?;
        let bytes = postcard::to_allocvec(transfer)?;
        durable_atomic_write(&self.control_path(transfer.world_id, "transfer.postcard"), &bytes)
    }

    pub fn load_transfer_record(&self, world: WorldId) -> Result<AuthorityTransferV1, StorageError> {
        let path = self.control_path(world, "transfer.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let record: AuthorityTransferV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(record)
    }

    pub fn save_sleep_record(&self, record: &SleepRecordV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(record.world_id)?;
        let bytes = postcard::to_allocvec(record)?;
        durable_atomic_write(&self.control_path(record.world_id, "sleep.postcard"), &bytes)
    }

    pub fn load_sleep_record(&self, world: WorldId) -> Result<SleepRecordV1, StorageError> {
        let path = self.control_path(world, "sleep.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let record: SleepRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(record)
    }

    pub fn clear_sleep_record(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        durable_remove(&self.control_path(world, "sleep.postcard"))?;
        Ok(())
    }

    fn control_path(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
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

    fn epoch_record(world: WorldId, peer: u8, epoch: u64, fencing_token: u64) -> EpochRecordV1 {
        EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: epoch,
            previous_epoch_hash: None,
            base_state_hash: Hash32([9; 32]),
            authority_peer_id: PeerId([peer; 32]),
            authority_public_key: [peer; 32],
            mode: EpochMode::Quorum,
            fencing_token,
            reason: "test".into(),
            signature: vec![peer; 64],
        }
    }

    #[test]
    fn epoch_record_round_trip_preserves_fencing_generation() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([3; 32]);
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        let record = epoch_record(world, 2, 4, 11);
        store.save_epoch_record(&record).unwrap();
        let loaded = store.load_epoch_record(world).unwrap();
        assert_eq!(loaded.epoch_number, 4);
        assert_eq!(loaded.fencing_token, 11);
    }

    #[test]
    fn epoch_record_cannot_regress_or_equivocate_same_generation() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x33; 32]);
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        let accepted = epoch_record(world, 2, 8, 12);
        store.save_epoch_record(&accepted).unwrap();
        store.save_epoch_record(&accepted).unwrap();

        let stale = epoch_record(world, 3, 7, 11);
        assert!(matches!(store.save_epoch_record(&stale), Err(StorageError::WorldMetadataMismatch)));
        let conflicting = epoch_record(world, 3, 8, 12);
        assert!(matches!(
            store.save_epoch_record(&conflicting),
            Err(StorageError::WorldMetadataMismatch)
        ));
        assert_eq!(store.load_epoch_record(world).unwrap(), accepted);
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
    fn direct_reservation_save_cannot_bypass_generation_checks() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x56; 32]);
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        let accepted = reservation(world, 6, 9, 13);
        let stale = reservation(world, 7, 8, 12);
        store.save_recovery_reservation(&accepted).unwrap();
        assert!(matches!(
            store.save_recovery_reservation(&stale),
            Err(StorageError::WorldMetadataMismatch)
        ));
        assert_eq!(store.load_recovery_reservation(world).unwrap().authority_peer_id, accepted.authority_peer_id);
    }

    #[test]
    fn concurrent_same_generation_reservations_have_one_winner() {
        let temp = tempfile::tempdir().unwrap();
        let a = Storage::open(temp.path()).unwrap();
        let b = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x55; 32]);
        fs::create_dir_all(a.world_dir(world).join("metadata")).unwrap();
        let left = reservation(world, 6, 8, 12);
        let right = reservation(world, 7, 8, 12);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let left_barrier = barrier.clone();
        let right_barrier = barrier.clone();
        let left_thread = std::thread::spawn(move || {
            left_barrier.wait();
            a.reserve_recovery(&left).unwrap()
        });
        let right_thread = std::thread::spawn(move || {
            right_barrier.wait();
            b.reserve_recovery(&right).unwrap()
        });
        barrier.wait();
        let results = [left_thread.join().unwrap(), right_thread.join().unwrap()];
        assert_eq!(results.into_iter().filter(|accepted| *accepted).count(), 1);
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
