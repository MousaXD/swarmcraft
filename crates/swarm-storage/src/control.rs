use crate::{
    transaction::{canonical_file_exists, durable_atomic_write, durable_remove, io_error, read_canonical_file},
    Storage, StorageError,
};
use std::{fs, io, path::PathBuf};
use swarm_protocol::{
    AuthorityLeaseGrantV1, AuthorityTransferV1, EpochRecordV1, Hash32, SleepRecordV1, TransferPhase, WorldId,
};

impl Storage {
    pub fn save_epoch_record(&self, record: &EpochRecordV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(record.world_id)?;
        record.validate_semantics()?;
        let path = self.control_path(record.world_id, "epoch.postcard");
        if canonical_file_exists(&path)? {
            let existing = self.load_epoch_record(record.world_id)?;
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
            // Preserve only the previously accepted record, never a proposed
            // successor whose canonical publication might still fail. A crash
            // after this archive but before publication cannot advance authority.
            self.archive_recovery_epoch_proof_locked(&existing)?;
            self.archive_history_locked(
                existing.world_id,
                &format!("epoch-{}.postcard", existing.record_hash()?),
                &postcard::to_allocvec(&existing)?,
            )?;
        }
        let bytes = postcard::to_allocvec(record)?;
        durable_atomic_write(&path, &bytes)
    }

    pub fn load_epoch_record(&self, world: WorldId) -> Result<EpochRecordV1, StorageError> {
        let path = self.control_path(world, "epoch.postcard");
        let bytes = read_canonical_file(&path)?;
        let record: EpochRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    /// Read exact persisted history, not a live authority grant. The caller
    /// must independently verify signatures, ancestry and transition evidence.
    pub fn load_epoch_by_hash(&self, world: WorldId, hash: Hash32) -> Result<EpochRecordV1, StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        self.load_epoch_by_hash_locked(world, hash)
    }

    pub(crate) fn load_epoch_by_hash_locked(
        &self,
        world: WorldId,
        hash: Hash32,
    ) -> Result<EpochRecordV1, StorageError> {
        let name = format!("epoch-{hash}.postcard");
        let record: EpochRecordV1 = match self.read_history_locked(world, &name)? {
            Some(bytes) => postcard::from_bytes(&bytes)?,
            None => self.load_epoch_record(world)?,
        };
        record.validate_semantics()?;
        if record.world_id != world || record.record_hash()? != hash {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(record)
    }

    pub fn reserve_recovery(&self, reservation: &AuthorityLeaseGrantV1) -> Result<bool, StorageError> {
        let _guard = self.lock_world_transaction(reservation.world_id)?;
        reservation.validate_semantics()?;
        self.reserve_recovery_locked(reservation)
    }

    pub fn save_recovery_reservation(&self, reservation: &AuthorityLeaseGrantV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(reservation.world_id)?;
        reservation.validate_semantics()?;
        if self.reserve_recovery_locked(reservation)? {
            Ok(())
        } else {
            Err(StorageError::WorldMetadataMismatch)
        }
    }

    fn reserve_recovery_locked(&self, reservation: &AuthorityLeaseGrantV1) -> Result<bool, StorageError> {
        let path = self.control_path(reservation.world_id, "recovery-reservation.postcard");
        if canonical_file_exists(&path)? {
            let existing = self.load_recovery_reservation(reservation.world_id)?;
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
        durable_atomic_write(&path, &bytes)?;
        Ok(true)
    }

    pub fn load_recovery_reservation(&self, world: WorldId) -> Result<AuthorityLeaseGrantV1, StorageError> {
        let path = self.control_path(world, "recovery-reservation.postcard");
        let bytes = read_canonical_file(&path)?;
        let reservation: AuthorityLeaseGrantV1 = postcard::from_bytes(&bytes)?;
        if reservation.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        reservation.validate_semantics()?;
        Ok(reservation)
    }

    pub fn clear_recovery_reservation(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        durable_remove(&self.control_path(world, "recovery-reservation.postcard"))?;
        Ok(())
    }

    pub fn save_transfer_record(&self, transfer: &AuthorityTransferV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(transfer.world_id)?;
        transfer.validate_semantics()?;
        let path = self.control_path(transfer.world_id, "transfer.postcard");
        // An existing canonical transfer may be at a later protocol phase.
        // Never replace it if corruption makes its phase/generation unreadable.
        if canonical_file_exists(&path)? {
            let existing = self.load_transfer_record(transfer.world_id)?;
            if existing == *transfer {
                return Ok(());
            }
            self.archive_history_locked(
                existing.world_id,
                &transfer_history_name(&existing, existing.phase),
                &postcard::to_allocvec(&existing)?,
            )?;
        }
        let bytes = postcard::to_allocvec(transfer)?;
        durable_atomic_write(&path, &bytes)
    }

    pub fn load_transfer_record(&self, world: WorldId) -> Result<AuthorityTransferV1, StorageError> {
        let path = self.control_path(world, "transfer.postcard");
        let bytes = read_canonical_file(&path)?;
        let record: AuthorityTransferV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    /// Locate a specific persisted phase of the same signed transfer, even
    /// after a later transfer replaces the live control record. Historical
    /// phases do not bypass the live source/witness/target delivery protocol.
    pub fn load_transfer_phase(
        &self,
        transcript: &AuthorityTransferV1,
        phase: TransferPhase,
    ) -> Result<AuthorityTransferV1, StorageError> {
        let world = transcript.world_id;
        let _guard = self.lock_world_transaction(world)?;
        let name = transfer_history_name(transcript, phase);
        let record: AuthorityTransferV1 = match self.read_history_locked(world, &name)? {
            Some(bytes) => postcard::from_bytes(&bytes)?,
            None => self.load_transfer_record(world)?,
        };
        record.validate_semantics()?;
        if record.world_id != world || record.phase != phase || transfer_history_name(&record, phase) != name {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(record)
    }

    pub fn save_sleep_record(&self, record: &SleepRecordV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(record.world_id)?;
        record.validate_semantics()?;
        let path = self.control_path(record.world_id, "sleep.postcard");
        if canonical_file_exists(&path)? {
            let existing = self.load_sleep_record(record.world_id)?;
            if existing == *record {
                return Ok(());
            }
            self.archive_sleep_locked(&existing)?;
        }
        durable_atomic_write(&path, &postcard::to_allocvec(record)?)
    }

    pub fn load_sleep_record(&self, world: WorldId) -> Result<SleepRecordV1, StorageError> {
        let path = self.control_path(world, "sleep.postcard");
        let bytes = read_canonical_file(&path)?;
        let record: SleepRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    pub fn clear_sleep_record(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let path = self.control_path(world, "sleep.postcard");
        if canonical_file_exists(&path)? {
            self.archive_sleep_locked(&self.load_sleep_record(world)?)?;
        }
        durable_remove(&path)?;
        Ok(())
    }

    fn archive_sleep_locked(&self, record: &SleepRecordV1) -> Result<(), StorageError> {
        self.archive_history_locked(
            record.world_id,
            &sleep_history_name(record.epoch, record.fencing_token, record.latest_snapshot_hash),
            &postcard::to_allocvec(record)?,
        )
    }

    pub fn load_sleep_boundary(
        &self,
        epoch: &EpochRecordV1,
        checkpoint: Hash32,
    ) -> Result<SleepRecordV1, StorageError> {
        let world = epoch.world_id;
        let _guard = self.lock_world_transaction(world)?;
        let name = sleep_history_name(epoch.epoch_number, epoch.fencing_token, checkpoint);
        let record: SleepRecordV1 = match self.read_history_locked(world, &name)? {
            Some(bytes) => postcard::from_bytes(&bytes)?,
            None => self.load_sleep_record(world)?,
        };
        record.validate_semantics()?;
        if record.world_id != world
            || record.epoch != epoch.epoch_number
            || record.fencing_token != epoch.fencing_token
            || record.authority_peer_id != epoch.authority_peer_id
            || record.authority_public_key != epoch.authority_public_key
            || record.latest_snapshot_hash != checkpoint
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(record)
    }

    /// Serialize immutable historical publication with the existing world
    /// transaction lock. Archive files are never inputs to live control reads.
    pub(crate) fn archive_history_locked(&self, world: WorldId, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let path = self.history_path(world, name)?;
        if canonical_file_exists(&path)? {
            if read_canonical_file(&path)? != bytes {
                return Err(StorageError::WorldMetadataMismatch);
            }
            return Ok(());
        }
        durable_atomic_write(&path, bytes)
    }

    pub(crate) fn read_history_locked(&self, world: WorldId, name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let path = self.history_path(world, name)?;
        if !canonical_file_exists(&path)? {
            return Ok(None);
        }
        Ok(Some(read_canonical_file(&path)?))
    }

    /// Store a data-only replay envelope after the caller verifies its
    /// signatures and transition evidence. It is tied to the exact already
    /// committed membership and is never read by live authority admission.
    pub fn stage_historical_replay_proof(
        &self,
        world: WorldId,
        membership_hash: Hash32,
        encoded: &[u8],
    ) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        if encoded.is_empty()
            || encoded.len() > swarm_protocol::MAX_REPLAY_PROOF_BYTES
            || canonical_file_exists(&self.control_path(world, "epoch.postcard"))?
            || self.load_membership_record(world)?.record_hash()? != membership_hash
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let path = self.control_path(world, &format!("pending-replay-{membership_hash}.postcard"));
        // Keep the first verified replay target for this membership until
        // catch-up is finished. A new save may advance the sender's head;
        // it must not replace a partially downloaded replay's commitment.
        if canonical_file_exists(&path)? {
            return if read_canonical_file(&path)? == encoded {
                Ok(())
            } else {
                Err(StorageError::WorldMetadataMismatch)
            };
        }
        durable_atomic_write(&path, encoded)
    }

    pub fn load_staged_historical_replay_proof(
        &self,
        world: WorldId,
        membership_hash: Hash32,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        if self.load_membership_record(world)?.record_hash()? != membership_hash {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let path = self.control_path(world, &format!("pending-replay-{membership_hash}.postcard"));
        if !canonical_file_exists(&path)? {
            return Ok(None);
        }
        let bytes = read_canonical_file(&path)?;
        if bytes.is_empty() || bytes.len() > swarm_protocol::MAX_REPLAY_PROOF_BYTES {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(Some(bytes))
    }

    fn history_path(&self, world: WorldId, name: &str) -> Result<PathBuf, StorageError> {
        let directory = self.control_path(world, "authority-history");
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(StorageError::SymlinkUnsupported(directory));
            }
            Ok(metadata) if !metadata.is_dir() => return Err(StorageError::WorldMetadataMismatch),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&directory, error)),
        }
        Ok(directory.join(name))
    }

    fn control_path(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
}

fn transfer_history_name(record: &AuthorityTransferV1, phase: TransferPhase) -> String {
    let phase = match phase {
        TransferPhase::Prepared => "prepared",
        TransferPhase::Accepted => "accepted",
        TransferPhase::Committed => "committed",
    };
    // Hash the immutable transition identity, not the phase's signer.
    let key = Hash32::from_domain_bytes(
        b"swarmcraft/transfer-history-key/v1\0",
        &[
            record.from_peer_id.0.as_slice(),
            record.to_peer_id.0.as_slice(),
            record.base_snapshot_hash.0.as_slice(),
            &record.next_epoch.to_le_bytes(),
            &record.next_fencing_token.to_le_bytes(),
        ]
        .concat(),
    );
    format!("transfer-{key}-{phase}.postcard")
}

fn sleep_history_name(epoch: u64, fence: u64, checkpoint: Hash32) -> String {
    format!("sleep-{epoch:020}-{fence:020}-{checkpoint}.postcard")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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
        assert!(matches!(store.save_epoch_record(&conflicting), Err(StorageError::WorldMetadataMismatch)));
        assert_eq!(store.load_epoch_record(world).unwrap(), accepted);
    }

    #[test]
    fn replaced_epochs_remain_exactly_addressable_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x81; 32]);
        let first = epoch_record(world, 2, 1, 1);
        let mut second = epoch_record(world, 3, 2, 2);
        second.previous_epoch_hash = Some(first.record_hash().unwrap());
        let mut third = epoch_record(world, 4, 3, 3);
        third.previous_epoch_hash = Some(second.record_hash().unwrap());
        store.save_epoch_record(&first).unwrap();
        store.save_epoch_record(&second).unwrap();
        store.save_epoch_record(&third).unwrap();
        drop(store);
        let store = Storage::open(temp.path()).unwrap();
        assert_eq!(store.load_epoch_record(world).unwrap(), third);
        for expected in [first, second, third] {
            assert_eq!(store.load_epoch_by_hash(world, expected.record_hash().unwrap()).unwrap(), expected);
        }
        assert!(store.load_epoch_by_hash(world, Hash32([0x88; 32])).is_err());
    }

    #[test]
    fn corrupt_epoch_archive_cannot_be_used_or_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x82; 32]);
        let first = epoch_record(world, 2, 1, 1);
        let next = epoch_record(world, 3, 2, 2);
        store.save_epoch_record(&first).unwrap();
        let hash = first.record_hash().unwrap();
        let path = store.history_path(world, &format!("epoch-{hash}.postcard")).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"corrupt archive").unwrap();
        assert!(store.load_epoch_by_hash(world, hash).is_err());
        assert!(store.save_epoch_record(&next).is_err());
        assert_eq!(store.load_epoch_record(world).unwrap(), first);
        assert_eq!(fs::read(path).unwrap(), b"corrupt archive");
    }

    #[test]
    fn transfer_phases_survive_a_subsequent_transfer_without_becoming_live() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x83; 32]);
        let prepared = AuthorityTransferV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            from_peer_id: PeerId([2; 32]),
            to_peer_id: PeerId([3; 32]),
            base_snapshot_hash: Hash32([4; 32]),
            next_epoch: 2,
            next_fencing_token: 2,
            phase: TransferPhase::Prepared,
            signer_peer_id: PeerId([2; 32]),
            signer_public_key: [2; 32],
            signature: vec![2; 64],
        };
        let accepted = AuthorityTransferV1 {
            phase: TransferPhase::Accepted,
            signer_peer_id: PeerId([3; 32]),
            signer_public_key: [3; 32],
            signature: vec![3; 64],
            ..prepared.clone()
        };
        let committed = AuthorityTransferV1 { phase: TransferPhase::Committed, ..prepared.clone() };
        let later = AuthorityTransferV1 { next_epoch: 3, next_fencing_token: 3, ..prepared.clone() };
        for record in [&prepared, &accepted, &committed, &later] {
            store.save_transfer_record(record).unwrap();
        }
        drop(store);
        let store = Storage::open(temp.path()).unwrap();
        assert_eq!(store.load_transfer_record(world).unwrap(), later);
        for record in [prepared, accepted, committed] {
            assert_eq!(store.load_transfer_phase(&record, record.phase).unwrap(), record);
        }
        assert!(store.load_transfer_phase(&later, TransferPhase::Committed).is_err());
    }

    #[test]
    fn cleared_sleep_boundary_is_history_only() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x84; 32]);
        let epoch = epoch_record(world, 2, 1, 1);
        store.save_epoch_record(&epoch).unwrap();
        let boundary = SleepRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            latest_snapshot_hash: Hash32([3; 32]),
            epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
            authority_peer_id: epoch.authority_peer_id,
            authority_public_key: epoch.authority_public_key,
            signature: vec![2; 64],
        };
        store.save_sleep_record(&boundary).unwrap();
        store.clear_sleep_record(world).unwrap();
        assert!(store.load_sleep_record(world).is_err());
        assert_eq!(store.load_sleep_boundary(&epoch, boundary.latest_snapshot_hash).unwrap(), boundary);
        assert!(store.load_sleep_boundary(&epoch, Hash32([9; 32])).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_history_namespace_refuses_publication() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path().join("store")).unwrap();
        let world = WorldId([0x85; 32]);
        let first = epoch_record(world, 2, 1, 1);
        store.save_epoch_record(&first).unwrap();
        let directory = store.control_path(world, "authority-history");
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, &directory).unwrap();
        assert!(matches!(
            store.save_epoch_record(&epoch_record(world, 3, 2, 2)),
            Err(StorageError::SymlinkUnsupported(path)) if path == directory
        ));
        assert_eq!(store.load_epoch_record(world).unwrap(), first);
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_epoch_record_cannot_be_reinitialized() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x35; 32]);
        let path = store.world_dir(world).join("metadata/epoch.postcard");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let missing = path.with_extension("missing");
        symlink(&missing, &path).unwrap();

        assert!(matches!(
            store.save_epoch_record(&epoch_record(world, 2, 8, 12)),
            Err(StorageError::SymlinkUnsupported(actual)) if actual == path
        ));
        assert_eq!(fs::read_link(&path).unwrap(), missing);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_recovery_reservation_cannot_be_reinitialized() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x36; 32]);
        let path = store.world_dir(world).join("metadata/recovery-reservation.postcard");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let missing = path.with_extension("missing");
        symlink(&missing, &path).unwrap();

        assert!(matches!(
            store.reserve_recovery(&reservation(world, 2, 8, 12)),
            Err(StorageError::SymlinkUnsupported(actual)) if actual == path
        ));
        assert_eq!(fs::read_link(&path).unwrap(), missing);
    }

    #[test]
    fn corrupt_epoch_record_cannot_be_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x34; 32]);
        let path = store.world_dir(world).join("metadata").join("epoch.postcard");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"corrupt").unwrap();

        let replacement = epoch_record(world, 2, 8, 12);
        assert!(store.save_epoch_record(&replacement).is_err());
        assert_eq!(fs::read(path).unwrap(), b"corrupt");
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
        assert!(matches!(store.save_recovery_reservation(&stale), Err(StorageError::WorldMetadataMismatch)));
        assert_eq!(store.load_recovery_reservation(world).unwrap().authority_peer_id, accepted.authority_peer_id);
    }

    #[test]
    fn corrupt_recovery_reservation_cannot_be_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([0x57; 32]);
        let path = store.world_dir(world).join("metadata").join("recovery-reservation.postcard");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"corrupt").unwrap();

        let replacement = reservation(world, 6, 9, 13);
        assert!(store.save_recovery_reservation(&replacement).is_err());
        assert_eq!(fs::read(path).unwrap(), b"corrupt");
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
