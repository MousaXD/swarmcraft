use crate::{
    transaction::{canonical_file_exists, durable_atomic_write, durable_remove, read_canonical_file},
    Storage, StorageError,
};
use swarm_protocol::{RecoveryCertificateV1, WorldId};

impl Storage {
    pub fn save_recovery_certificate(&self, certificate: &RecoveryCertificateV1) -> Result<(), StorageError> {
        let world = certificate.ballot.world_id;
        let _guard = self.lock_world_transaction(world)?;
        self.load_world(world)?;
        validate_stored_certificate(certificate, world)?;
        let path = self.world_dir(world).join("metadata").join("recovery-certificate.postcard");

        // A present but corrupt/unreadable canonical record cannot be silently
        // overwritten. Only genuine NotFound is an absent certificate.
        let existing = if canonical_file_exists(&path)? { Some(self.load_recovery_certificate(world)?) } else { None };
        let proposed_generation = (certificate.ballot.target_epoch, certificate.ballot.target_fencing_token);
        let accepted = match self.load_epoch_record(world) {
            Ok(epoch) => Some(epoch),
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if let Some(epoch) = &accepted {
            let accepted_generation = (epoch.epoch_number, epoch.fencing_token);
            if proposed_generation < accepted_generation {
                // An old certificate must never replace the current recovery
                // proof after this world has advanced its accepted generation.
                return Err(StorageError::WorldMetadataMismatch);
            }
            if proposed_generation == accepted_generation {
                // Once the epoch is durable, its recovery ballot is immutable.
                // A new vote subset for the same ballot is still idempotent.
                if epoch.mode != swarm_protocol::EpochMode::Recovery
                    || epoch.authority_peer_id != certificate.ballot.candidate_peer_id
                    || epoch.authority_public_key != certificate.ballot.candidate_public_key
                    || epoch.base_state_hash != certificate.ballot.base_state_hash
                {
                    return Err(StorageError::WorldMetadataMismatch);
                }
                if let Some(previous) = &existing {
                    if previous.ballot.ballot_hash()? != certificate.ballot.ballot_hash()? {
                        return Err(StorageError::WorldMetadataMismatch);
                    }
                }
            }
        }
        if let Some(previous) = &existing {
            let prior_generation = (previous.ballot.target_epoch, previous.ballot.target_fencing_token);
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
        durable_atomic_write(&path, &postcard::to_allocvec(certificate)?)
    }

    pub fn load_recovery_certificate(&self, world: WorldId) -> Result<RecoveryCertificateV1, StorageError> {
        let path = self.world_dir(world).join("metadata").join("recovery-certificate.postcard");
        let bytes = read_canonical_file(&path)?;
        let certificate: RecoveryCertificateV1 = postcard::from_bytes(&bytes)?;
        validate_stored_certificate(&certificate, world)?;
        Ok(certificate)
    }

    pub fn clear_recovery_certificate(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let path = self.world_dir(world).join("metadata").join("recovery-certificate.postcard");
        durable_remove(&path)?;
        Ok(())
    }
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
        peer_id_from_public_key, EpochMode, EpochRecordV1, Hash32, RecoveryBallotV1, WorldGenesisV1, PROTOCOL_VERSION,
        STORAGE_SCHEMA_VERSION,
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
