use crate::{
    transaction::{canonical_file_exists, durable_atomic_write, durable_remove, io_error, read_canonical_file},
    Storage, StorageError,
};
use std::io;
use swarm_protocol::{EpochMode, EpochRecordV1, Hash32, RecoveryCertificateV1, WorldId};

const CERTIFICATE: &str = "recovery-certificate.postcard";
const PENDING_CERTIFICATE: &str = "recovery-certificate-pending.postcard";

impl Storage {
    pub fn save_recovery_certificate(&self, certificate: &RecoveryCertificateV1) -> Result<(), StorageError> {
        let world = certificate.ballot.world_id;
        let _guard = self.lock_world_transaction(world)?;
        self.load_world(world)?;
        validate_stored_certificate(certificate, world)?;
        let metadata = self.world_dir(world).join("metadata");
        let accepted_path = metadata.join(CERTIFICATE);
        let pending_path = metadata.join(PENDING_CERTIFICATE);
        let certificates = self.load_recovery_certificates(world)?;
        let proposed_generation = certificate_generation(certificate);
        let accepted = optional_epoch(self, world)?;
        let mut accepted_proof = None;
        if let Some(epoch) = &accepted {
            let accepted_generation = (epoch.epoch_number, epoch.fencing_token);
            if proposed_generation < accepted_generation {
                // An old certificate must never replace the current recovery
                // proof after this world has advanced its accepted generation.
                return Err(StorageError::WorldMetadataMismatch);
            }
            if epoch.mode == EpochMode::Recovery {
                accepted_proof = certificate_for_epoch(&certificates, epoch)?;
            }
            if proposed_generation == accepted_generation {
                if !matches_recovery_epoch(certificate, epoch) {
                    return Err(StorageError::WorldMetadataMismatch);
                }
                if let Some(previous) = accepted_proof {
                    if previous.ballot.ballot_hash()? != certificate.ballot.ballot_hash()? {
                        return Err(StorageError::WorldMetadataMismatch);
                    }
                    // Retain the original proof and any future pending certificate.
                    return Ok(());
                }
                return durable_atomic_write(&accepted_path, &postcard::to_allocvec(certificate)?);
            }
            if epoch.mode == EpochMode::Recovery && accepted_proof.is_none() {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        self.ensure_no_device_retirement_locked(world)?;
        if let Some(previous) = latest_certificate(&certificates) {
            let prior_generation = certificate_generation(previous);
            if proposed_generation < prior_generation {
                return Err(StorageError::WorldMetadataMismatch);
            }
            if proposed_generation == prior_generation {
                // Before an epoch is accepted, a strictly higher ballot round
                // may supersede a failed candidate's earlier certificate.
                // Never roll back to an older round or replace a same-round
                // ballot with conflicting durable canonical evidence.
                if certificate.ballot.round < previous.ballot.round
                    || (certificate.ballot.round == previous.ballot.round
                        && certificate.ballot.ballot_hash()? != previous.ballot.ballot_hash()?)
                {
                    return Err(StorageError::WorldMetadataMismatch);
                }
            }
        }
        let path = if let Some(proof) = accepted_proof {
            // A prior promotion may have made the pending proof authoritative.
            // Persist it separately before reusing the pending slot.
            if certificates[0].as_ref() != Some(proof) {
                durable_atomic_write(&accepted_path, &postcard::to_allocvec(proof)?)?;
            }
            pending_path
        } else if certificates[1].is_some() {
            pending_path
        } else {
            accepted_path
        };
        durable_atomic_write(&path, &postcard::to_allocvec(certificate)?)
    }

    pub fn load_recovery_certificate(&self, world: WorldId) -> Result<RecoveryCertificateV1, StorageError> {
        let epoch = optional_epoch(self, world)?;
        let certificates = self.load_recovery_certificates(world)?;
        if let Some(epoch) = epoch.filter(|epoch| epoch.mode == EpochMode::Recovery) {
            return certificate_for_epoch(&certificates, &epoch)?.cloned().ok_or(StorageError::WorldMetadataMismatch);
        }
        latest_certificate(&certificates).cloned().ok_or_else(|| {
            io_error(self.world_dir(world).join("metadata").join(CERTIFICATE), io::ErrorKind::NotFound.into())
        })
    }

    /// Called under the world lock before replacing the accepted epoch.
    /// Pending certificates for unsuccessful candidates are not archived as
    /// accepted proofs. Legacy missing proofs remain missing, not invented.
    pub(crate) fn archive_recovery_epoch_proof_locked(&self, epoch: &EpochRecordV1) -> Result<(), StorageError> {
        if epoch.mode != EpochMode::Recovery {
            return Ok(());
        }
        let certificates = self.load_recovery_certificates(epoch.world_id)?;
        if let Some(proof) = certificate_for_epoch(&certificates, epoch)? {
            self.archive_history_locked(
                epoch.world_id,
                &format!("recovery-{}.postcard", epoch.record_hash()?),
                &postcard::to_allocvec(proof)?,
            )?;
        }
        Ok(())
    }

    /// Exact proof for a persisted epoch, not the newest pending ballot.
    /// Historical proofs still require full membership and quorum validation
    /// by the caller; this function cannot grant recovery authority.
    pub fn load_recovery_certificate_for_epoch(
        &self,
        world: WorldId,
        epoch_hash: Hash32,
    ) -> Result<RecoveryCertificateV1, StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let epoch = self.load_epoch_by_hash_locked(world, epoch_hash)?;
        if epoch.mode != EpochMode::Recovery {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let name = format!("recovery-{epoch_hash}.postcard");
        let proof: RecoveryCertificateV1 = match self.read_history_locked(world, &name)? {
            Some(bytes) => postcard::from_bytes(&bytes)?,
            None => {
                if self.load_epoch_record(world)? != epoch {
                    return Err(StorageError::WorldMetadataMismatch);
                }
                let certificates = self.load_recovery_certificates(world)?;
                certificate_for_epoch(&certificates, &epoch)?.cloned().ok_or(StorageError::WorldMetadataMismatch)?
            }
        };
        validate_stored_certificate(&proof, world)?;
        if !matches_recovery_epoch(&proof, &epoch) {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(proof)
    }

    pub(crate) fn load_recovery_certificates(
        &self,
        world: WorldId,
    ) -> Result<[Option<RecoveryCertificateV1>; 2], StorageError> {
        let metadata = self.world_dir(world).join("metadata");
        let read = |name| -> Result<_, StorageError> {
            let path = metadata.join(name);
            if !canonical_file_exists(&path)? {
                return Ok(None);
            }
            let certificate = postcard::from_bytes(&read_canonical_file(&path)?)?;
            validate_stored_certificate(&certificate, world)?;
            Ok(Some(certificate))
        };
        Ok([read(CERTIFICATE)?, read(PENDING_CERTIFICATE)?])
    }

    pub fn clear_recovery_certificate(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let metadata = self.world_dir(world).join("metadata");
        durable_remove(&metadata.join(PENDING_CERTIFICATE))?;
        durable_remove(&metadata.join(CERTIFICATE))?;
        Ok(())
    }
}

fn optional_epoch(storage: &Storage, world: WorldId) -> Result<Option<EpochRecordV1>, StorageError> {
    match storage.load_epoch_record(world) {
        Ok(epoch) => Ok(Some(epoch)),
        Err(StorageError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn certificate_generation(certificate: &RecoveryCertificateV1) -> (u64, u64) {
    (certificate.ballot.target_epoch, certificate.ballot.target_fencing_token)
}

fn latest_certificate(certificates: &[Option<RecoveryCertificateV1>]) -> Option<&RecoveryCertificateV1> {
    certificates
        .iter()
        .flatten()
        .max_by_key(|certificate| (certificate_generation(certificate), certificate.ballot.round))
}

fn matches_recovery_epoch(certificate: &RecoveryCertificateV1, epoch: &EpochRecordV1) -> bool {
    epoch.mode == EpochMode::Recovery
        && certificate_generation(certificate) == (epoch.epoch_number, epoch.fencing_token)
        && epoch.authority_peer_id == certificate.ballot.candidate_peer_id
        && epoch.authority_public_key == certificate.ballot.candidate_public_key
        && epoch.base_state_hash == certificate.ballot.base_state_hash
}

fn certificate_for_epoch<'a>(
    certificates: &'a [Option<RecoveryCertificateV1>],
    epoch: &EpochRecordV1,
) -> Result<Option<&'a RecoveryCertificateV1>, StorageError> {
    let mut matching = certificates
        .iter()
        .flatten()
        .filter(|proof| certificate_generation(proof) == (epoch.epoch_number, epoch.fencing_token));
    let Some(proof) = matching.next() else { return Ok(None) };
    if !matches_recovery_epoch(proof, epoch) || matching.any(|other| other != proof) {
        return Err(StorageError::WorldMetadataMismatch);
    }
    Ok(Some(proof))
}

fn validate_stored_certificate(certificate: &RecoveryCertificateV1, world: WorldId) -> Result<(), StorageError> {
    if certificate.ballot.world_id != world {
        return Err(StorageError::WorldMetadataMismatch);
    }
    certificate.ballot.validate_semantics()?;
    for vote in &certificate.votes {
        vote.validate_semantics()?;
        if vote.world_id != world || !vote.matches_ballot(&certificate.ballot)? {
            return Err(StorageError::WorldMetadataMismatch);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldMetadataV1;
    use std::fs;
    use swarm_protocol::{
        peer_id_from_public_key, Hash32, RecoveryBallotV1, WorldGenesisV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
    };

    fn fixture() -> (tempfile::TempDir, Storage, WorldId) {
        let temp = tempfile::tempdir().unwrap();
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([9; 32]),
            creation_nonce: [8; 32],
            creator_public_key: [7; 32],
            initial_membership: vec![peer_id_from_public_key(&[6; 32])],
        };
        let world = genesis.world_id().unwrap();
        let storage = Storage::open(temp.path()).unwrap();
        storage
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "certificate-integrity".into(),
                world_id: world,
                genesis,
            })
            .unwrap();
        (temp, storage, world)
    }

    // These fixtures exercise the canonical storage boundary, not quorum or
    // cryptographic validation (both belong to the caller).
    fn certificate(world: WorldId) -> RecoveryCertificateV1 {
        RecoveryCertificateV1 {
            ballot: RecoveryBallotV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                base_epoch: 1,
                base_fencing_token: 1,
                target_epoch: 2,
                target_fencing_token: 2,
                round: 1,
                candidate_peer_id: peer_id_from_public_key(&[9; 32]),
                candidate_public_key: [9; 32],
                base_snapshot_hash: Hash32([3; 32]),
                base_state_hash: Hash32([4; 32]),
                membership_hash: Hash32([5; 32]),
                signature: Vec::new(),
            },
            votes: Vec::new(),
        }
    }

    #[test]
    fn corrupt_durable_certificate_must_not_be_overwritten() {
        let (_temp, storage, world) = fixture();
        let certificate = certificate(world);
        storage.save_recovery_certificate(&certificate).unwrap();
        let path = storage.world_dir(world).join("metadata/recovery-certificate.postcard");
        let corrupted = b"unreadable durable recovery certificate";
        fs::write(&path, corrupted).unwrap();

        assert!(storage.save_recovery_certificate(&certificate).is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupted);

        fs::write(&path, postcard::to_allocvec(&certificate).unwrap()).unwrap();
        assert_eq!(storage.load_recovery_certificate(world).unwrap(), certificate);
        storage.save_recovery_certificate(&certificate).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_certificate_must_not_be_read_or_replaced() {
        use std::os::unix::fs::symlink;

        let (temp, storage, world) = fixture();
        let certificate = certificate(world);
        let path = storage.world_dir(world).join("metadata/recovery-certificate.postcard");
        let external = temp.path().join("outside-canonical-certificate.postcard");
        let original = postcard::to_allocvec(&certificate).unwrap();
        fs::write(&external, &original).unwrap();
        symlink(&external, &path).unwrap();

        assert!(storage.load_recovery_certificate(world).is_err());
        assert!(storage.save_recovery_certificate(&certificate).is_err());
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&external).unwrap(), original);
    }

    #[test]
    fn equal_round_conflicting_unaccepted_certificate_cannot_replace_durable_history() {
        let (_temp, storage, world) = fixture();
        let original = certificate(world);
        storage.save_recovery_certificate(&original).unwrap();
        let path = storage.world_dir(world).join("metadata/recovery-certificate.postcard");
        let prior_bytes = fs::read(&path).unwrap();

        let mut conflict = original.clone();
        conflict.ballot.base_snapshot_hash = Hash32([99; 32]);
        assert!(storage.save_recovery_certificate(&conflict).is_err());
        assert_eq!(fs::read(&path).unwrap(), prior_bytes);
    }

    #[test]
    fn semantically_invalid_durable_certificate_is_not_readable_or_replaceable() {
        let (_temp, storage, world) = fixture();
        let valid = certificate(world);
        storage.save_recovery_certificate(&valid).unwrap();
        let mut invalid = valid.clone();
        invalid.ballot.protocol_version = 0;
        let path = storage.world_dir(world).join("metadata/recovery-certificate.postcard");
        let invalid_bytes = postcard::to_allocvec(&invalid).unwrap();
        fs::write(&path, &invalid_bytes).unwrap();

        assert!(storage.load_recovery_certificate(world).is_err());
        assert!(storage.save_recovery_certificate(&valid).is_err());
        assert_eq!(fs::read(&path).unwrap(), invalid_bytes);
    }

    #[test]
    fn corrupt_accepted_epoch_cannot_be_ignored_during_certificate_replacement() {
        let (_temp, storage, world) = fixture();
        let original = certificate(world);
        storage.save_recovery_certificate(&original).unwrap();
        let certificate_path = storage.world_dir(world).join("metadata/recovery-certificate.postcard");
        let prior = fs::read(&certificate_path).unwrap();
        let epoch_path = storage.world_dir(world).join("metadata/epoch.postcard");
        fs::write(&epoch_path, b"invalid-durable-accepted-epoch").unwrap();

        let mut newer = original;
        newer.ballot.round += 1;
        assert!(storage.save_recovery_certificate(&newer).is_err());
        assert_eq!(fs::read(&certificate_path).unwrap(), prior);
        assert_eq!(fs::read(&epoch_path).unwrap(), b"invalid-durable-accepted-epoch");
    }

    #[test]
    fn competing_same_round_certificates_cannot_both_overwrite_durable_history() {
        use std::sync::{Arc, Barrier};

        let (_temp, storage, world) = fixture();
        let root = storage.root().to_path_buf();
        let barrier = Arc::new(Barrier::new(2));
        let accepted = std::thread::scope(|scope| {
            let handles = [10_u8, 11_u8]
                .into_iter()
                .map(|byte| {
                    let barrier = Arc::clone(&barrier);
                    let root = &root;
                    scope.spawn(move || {
                        let storage = Storage::open(root).unwrap();
                        let mut candidate = certificate(world);
                        candidate.ballot.base_snapshot_hash = Hash32([byte; 32]);
                        barrier.wait();
                        storage.save_recovery_certificate(&candidate).is_ok()
                    })
                })
                .collect::<Vec<_>>();
            handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
        });

        assert_eq!(accepted.iter().filter(|value| **value).count(), 1);
        assert!(storage.load_recovery_certificate(world).is_ok());
    }

    #[test]
    fn interrupted_next_epoch_promotion_does_not_erase_current_accepted_certificate() {
        let (temp, storage, world) = fixture();
        let current = certificate(world);
        storage.save_recovery_certificate(&current).unwrap();
        storage.save_epoch_record(&accepted_epoch_for(&current)).unwrap();

        let mut next = current.clone();
        next.ballot.base_epoch = current.ballot.target_epoch;
        next.ballot.base_fencing_token = current.ballot.target_fencing_token;
        next.ballot.target_epoch += 1;
        next.ballot.target_fencing_token += 1;
        next.ballot.round += 1;
        next.ballot.base_snapshot_hash = Hash32([6; 32]);

        // Restart after persisting the successor's proof but before saving its epoch.
        storage.save_recovery_certificate(&next).unwrap();
        drop(storage);

        let reopened = Storage::open(temp.path()).unwrap();
        assert_eq!(reopened.load_epoch_record(world).unwrap().epoch_number, current.ballot.target_epoch);
        // The authority at the still-accepted generation must retain its
        // original durable quorum evidence until promotion commits.
        assert_eq!(reopened.load_recovery_certificate(world).unwrap(), current);
        assert_eq!(reopened.load_recovery_certificates(world).unwrap(), [Some(current.clone()), Some(next.clone())]);
        reopened.save_recovery_certificate(&current).unwrap();
        reopened.save_epoch_record(&accepted_epoch_for(&next)).unwrap();
        drop(reopened);

        let reopened = Storage::open(temp.path()).unwrap();
        assert_eq!(reopened.load_recovery_certificate(world).unwrap(), next);
        let mut third = next.clone();
        third.ballot.base_epoch += 1;
        third.ballot.base_fencing_token += 1;
        third.ballot.target_epoch += 1;
        third.ballot.target_fencing_token += 1;
        reopened.save_recovery_certificate(&third).unwrap();
        assert_eq!(reopened.load_recovery_certificate(world).unwrap(), next);
        assert_eq!(
            reopened
                .load_recovery_certificate_for_epoch(world, accepted_epoch_for(&current).record_hash().unwrap())
                .unwrap(),
            current
        );
        assert_eq!(
            reopened
                .load_recovery_certificate_for_epoch(world, accepted_epoch_for(&next).record_hash().unwrap())
                .unwrap(),
            next
        );
        assert!(reopened
            .load_recovery_certificate_for_epoch(world, accepted_epoch_for(&third).record_hash().unwrap())
            .is_err());
        assert_eq!(reopened.load_recovery_certificates(world).unwrap(), [Some(next), Some(third)]);
    }

    #[test]
    fn retention_keeps_accepted_and_pending_recovery_snapshot_roots() {
        use crate::{retention::RetentionPolicy, SnapshotContext};

        let (_temp, storage, world) = fixture();
        let source = tempfile::tempdir().unwrap();
        let mut current = certificate(world);
        let mut previous = None;
        let mut hashes = Vec::new();
        for number in 1..=3 {
            fs::write(source.path().join("level.dat"), number.to_string()).unwrap();
            let mut snapshot = storage
                .snapshot_directory(
                    source.path(),
                    SnapshotContext {
                        world,
                        snapshot_number: number,
                        epoch: 1,
                        sequence: number,
                        previous_snapshot_hash: previous,
                        authority_peer_id: current.ballot.candidate_peer_id,
                        authority_public_key: current.ballot.candidate_public_key,
                    },
                )
                .unwrap();
            snapshot.signature = vec![0; 64];
            storage.commit_snapshot(&snapshot).unwrap();
            let hash = snapshot.manifest_hash().unwrap();
            hashes.push(hash);
            previous = Some(hash);
        }
        current.ballot.base_snapshot_hash = hashes[0];
        storage.save_recovery_certificate(&current).unwrap();
        storage.save_epoch_record(&accepted_epoch_for(&current)).unwrap();
        let mut pending = current;
        pending.ballot.base_epoch += 1;
        pending.ballot.base_fencing_token += 1;
        pending.ballot.target_epoch += 1;
        pending.ballot.target_fencing_token += 1;
        pending.ballot.base_snapshot_hash = hashes[1];
        storage.save_recovery_certificate(&pending).unwrap();

        let report = storage
            .apply_retention(world, &RetentionPolicy { keep_latest: 1, protected_snapshots: Default::default() })
            .unwrap();
        assert_eq!(report.retained_snapshots, vec![1, 2, 3]);
        for number in 1..=3 {
            storage.verify_snapshot(&storage.load_snapshot(world, number).unwrap()).unwrap();
        }
    }

    fn accepted_epoch_for(certificate: &RecoveryCertificateV1) -> EpochRecordV1 {
        let ballot = &certificate.ballot;
        EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: ballot.world_id,
            epoch_number: ballot.target_epoch,
            previous_epoch_hash: None,
            base_state_hash: ballot.base_state_hash,
            authority_peer_id: ballot.candidate_peer_id,
            authority_public_key: ballot.candidate_public_key,
            mode: EpochMode::Recovery,
            fencing_token: ballot.target_fencing_token,
            reason: "storage certification test".into(),
            signature: vec![0; 64],
        }
    }

    #[test]
    fn accepted_epoch_cannot_have_its_durable_certificate_ballot_replaced() {
        let (_temp, storage, world) = fixture();
        let original = certificate(world);
        storage.save_recovery_certificate(&original).unwrap();
        storage.save_epoch_record(&accepted_epoch_for(&original)).unwrap();
        let path = storage.world_dir(world).join("metadata/recovery-certificate.postcard");
        let bytes = fs::read(&path).unwrap();

        let mut conflict = original.clone();
        conflict.ballot.round += 1;
        assert!(storage.save_recovery_certificate(&conflict).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(storage.load_recovery_certificate(world).unwrap(), original);
    }

    #[test]
    fn a_higher_ballot_round_can_replace_an_unaccepted_certificate() {
        let (_temp, storage, world) = fixture();
        let original = certificate(world);
        storage.save_recovery_certificate(&original).unwrap();
        let mut newer = original;
        newer.ballot.round += 1;

        storage.save_recovery_certificate(&newer).unwrap();
        assert_eq!(storage.load_recovery_certificate(world).unwrap(), newer);
    }

    #[test]
    fn an_older_certificate_cannot_replace_a_newer_accepted_generation() {
        let (_temp, storage, world) = fixture();
        let old = certificate(world);
        storage.save_recovery_certificate(&old).unwrap();
        let mut newer = old.clone();
        newer.ballot.base_epoch += 1;
        newer.ballot.target_epoch += 1;
        newer.ballot.base_fencing_token += 1;
        newer.ballot.target_fencing_token += 1;
        storage.save_recovery_certificate(&newer).unwrap();
        storage.save_epoch_record(&accepted_epoch_for(&newer)).unwrap();

        let path = storage.world_dir(world).join("metadata/recovery-certificate.postcard");
        let before = fs::read(&path).unwrap();
        assert!(storage.save_recovery_certificate(&old).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(storage.load_recovery_certificate(world).unwrap(), newer);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_certificate_symlink_must_not_be_replaced() {
        use std::os::unix::fs::symlink;

        let (temp, storage, world) = fixture();
        let certificate = certificate(world);
        let path = storage.world_dir(world).join("metadata/recovery-certificate.postcard");
        symlink(temp.path().join("missing-certificate"), &path).unwrap();

        assert!(storage.load_recovery_certificate(world).is_err());
        assert!(storage.save_recovery_certificate(&certificate).is_err());
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    }
}
