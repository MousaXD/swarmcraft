use crate::{
    transaction::{durable_atomic_write, durable_remove},
    Storage, StorageError,
};
use std::{fs, path::PathBuf};
use swarm_protocol::{JoinRequestV1, LeaveRequestV1, MembershipRecordV1, WorldDescriptorV1, WorldId};

impl Storage {
    pub fn save_world_descriptor(&self, descriptor: &WorldDescriptorV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(descriptor.world_id)?;
        let original_member_count = descriptor.members.len();
        let mut descriptor = descriptor.clone();
        descriptor.normalize();
        if descriptor.members.len() != original_member_count {
            return Err(StorageError::WorldMetadataMismatch);
        }
        descriptor.validate_semantics()?;
        let bytes = serde_json::to_vec_pretty(&descriptor)?;
        durable_atomic_write(&self.world_protocol_path(descriptor.world_id, "descriptor.json"), &bytes)
    }

    pub fn load_world_descriptor(&self, world: WorldId) -> Result<WorldDescriptorV1, StorageError> {
        let path = self.world_protocol_path(world, "descriptor.json");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let descriptor: WorldDescriptorV1 = serde_json::from_slice(&bytes)?;
        if descriptor.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        descriptor.validate_semantics()?;
        Ok(descriptor)
    }

    pub fn save_membership_record(&self, record: &MembershipRecordV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(record.world_id)?;
        record.validate_semantics()?;
        let membership_path = self.world_protocol_path(record.world_id, "membership.postcard");
        if membership_path.exists() {
            let current = self.load_membership_record(record.world_id)?;
            if record == &current {
                return Ok(());
            }
            let expected_sequence =
                current.sequence.checked_add(1).ok_or(StorageError::CounterExhausted("membership sequence"))?;
            if record.sequence != expected_sequence
                || record.previous_membership_hash != Some(current.record_hash()?)
                || record.epoch < current.epoch
            {
                return Err(StorageError::WorldMetadataMismatch);
            }
            if let Ok(epoch) = self.load_epoch_record(record.world_id) {
                if record.epoch != epoch.epoch_number
                    || record.authority_peer_id != epoch.authority_peer_id
                    || record.authority_public_key != epoch.authority_public_key
                {
                    return Err(StorageError::WorldMetadataMismatch);
                }
            } else if record.epoch != current.epoch
                || record.authority_peer_id != current.authority_peer_id
                || record.authority_public_key != current.authority_public_key
            {
                return Err(StorageError::WorldMetadataMismatch);
            }
        } else {
            let certified_bootstrap = self.certified_membership_bootstrap(record)?;
            if !certified_bootstrap && (record.sequence != 0 || record.previous_membership_hash.is_some()) {
                return Err(StorageError::WorldMetadataMismatch);
            }
            if let Ok(epoch) = self.load_epoch_record(record.world_id) {
                if record.epoch != epoch.epoch_number
                    || record.authority_peer_id != epoch.authority_peer_id
                    || record.authority_public_key != epoch.authority_public_key
                {
                    return Err(StorageError::WorldMetadataMismatch);
                }
            } else if record.epoch != 0 && !certified_bootstrap {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        let bytes = postcard::to_allocvec(record)?;
        durable_atomic_write(&membership_path, &bytes)
    }

    fn certified_membership_bootstrap(&self, record: &MembershipRecordV1) -> Result<bool, StorageError> {
        let certificate_path = self.world_protocol_path(record.world_id, "membership-certificate.postcard");
        let pending_join_path = self.world_protocol_path(record.world_id, "pending-join.postcard");
        if !certificate_path.exists() || !pending_join_path.exists() {
            return Ok(false);
        }

        let certificate = self.load_membership_certificate(record.world_id)?;
        let pending_join = self.load_pending_join(record.world_id)?;
        let metadata = self.load_world(record.world_id)?;
        if !certificate.proposal.validate_shape()? {
            return Ok(false);
        }

        Ok(certificate.proposal.proposed == *record
            && pending_join.world_id == record.world_id
            && pending_join.invite.world_id == record.world_id
            && pending_join.invite.genesis == metadata.genesis
            && pending_join.invite.inviter_peer_id == certificate.proposal.previous.authority_peer_id
            && pending_join.invite.inviter_public_key == certificate.proposal.previous.authority_public_key
            && certificate.proposal.proposed.members.iter().any(|member| member == &pending_join.joining_member))
    }

    pub fn load_membership_record(&self, world: WorldId) -> Result<MembershipRecordV1, StorageError> {
        let path = self.world_protocol_path(world, "membership.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let record: MembershipRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    pub fn save_pending_join(&self, request: &JoinRequestV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(request.world_id)?;
        request.validate_semantics()?;
        let bytes = postcard::to_allocvec(request)?;
        durable_atomic_write(&self.world_protocol_path(request.world_id, "pending-join.postcard"), &bytes)
    }

    pub fn load_pending_join(&self, world: WorldId) -> Result<JoinRequestV1, StorageError> {
        let path = self.world_protocol_path(world, "pending-join.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let request: JoinRequestV1 = postcard::from_bytes(&bytes)?;
        if request.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        request.validate_semantics()?;
        Ok(request)
    }

    pub fn clear_pending_join(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        remove_protocol_file(self, world, "pending-join.postcard")
    }

    pub fn save_pending_leave(&self, request: &LeaveRequestV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(request.world_id)?;
        request.validate_semantics()?;
        let bytes = postcard::to_allocvec(request)?;
        durable_atomic_write(&self.world_protocol_path(request.world_id, "pending-leave.postcard"), &bytes)
    }

    pub fn load_pending_leave(&self, world: WorldId) -> Result<LeaveRequestV1, StorageError> {
        let path = self.world_protocol_path(world, "pending-leave.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let request: LeaveRequestV1 = postcard::from_bytes(&bytes)?;
        if request.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        request.validate_semantics()?;
        Ok(request)
    }

    pub fn clear_pending_leave(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        remove_protocol_file(self, world, "pending-leave.postcard")
    }

    pub fn remove_local_membership(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        for name in [
            "descriptor.json",
            "membership.postcard",
            "membership-promise.postcard",
            "membership-certificate.postcard",
            "pending-join.postcard",
            "pending-leave.postcard",
        ] {
            remove_protocol_file(self, world, name)?;
        }
        Ok(())
    }

    fn world_protocol_path(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
}

fn remove_protocol_file(storage: &Storage, world: WorldId, name: &str) -> Result<(), StorageError> {
    durable_remove(&storage.world_protocol_path(world, name)).map(|_| ())
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldMetadataV1;
    use swarm_protocol::{
        EpochMode, EpochRecordV1, Hash32, PeerId, WorldGenesisV1, WorldMemberV1, PROTOCOL_VERSION,
        STORAGE_SCHEMA_VERSION,
    };

    fn member(peer: u8) -> WorldMemberV1 {
        WorldMemberV1 { peer_id: PeerId([peer; 32]), public_key: [peer; 32], authority_eligible: true, banned: false }
    }

    fn create_test_world(store: &Storage) -> WorldId {
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([6; 32]),
            creation_nonce: [8; 32],
            creator_public_key: [1; 32],
            initial_membership: vec![PeerId([1; 32])],
        };
        let world = genesis.world_id().unwrap();
        store
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "test".into(),
                world_id: world,
                genesis,
            })
            .unwrap();
        store
            .save_world_descriptor(&WorldDescriptorV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                compatibility_fingerprint: Hash32([6; 32]),
                members: vec![member(1), member(2)],
                preferred_replication_factor: 2,
            })
            .unwrap();
        world
    }

    fn membership(
        world: WorldId,
        authority: u8,
        epoch: u64,
        sequence: u64,
        previous: Option<Hash32>,
    ) -> MembershipRecordV1 {
        MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch,
            sequence,
            previous_membership_hash: previous,
            members: vec![member(1), member(2)],
            authority_peer_id: PeerId([authority; 32]),
            authority_public_key: [authority; 32],
            signature: vec![authority; 64],
        }
    }

    #[test]
    fn descriptor_round_trip_normalizes_members() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([5; 32]);
        let descriptor = WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: Hash32([6; 32]),
            members: vec![member(8), member(2)],
            preferred_replication_factor: 2,
        };
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        store.save_world_descriptor(&descriptor).unwrap();
        let loaded = store.load_world_descriptor(world).unwrap();
        assert_eq!(loaded.members[0].peer_id, PeerId([2; 32]));
    }

    #[test]
    fn membership_rejects_previous_authority_after_epoch_transition() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = create_test_world(&store);
        let initial = membership(world, 1, 0, 0, None);
        store.save_membership_record(&initial).unwrap();
        store
            .save_epoch_record(&EpochRecordV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                epoch_number: 1,
                previous_epoch_hash: None,
                base_state_hash: Hash32([7; 32]),
                authority_peer_id: PeerId([2; 32]),
                authority_public_key: [2; 32],
                mode: EpochMode::Recovery,
                fencing_token: 1,
                reason: "test transition".into(),
                signature: vec![2; 64],
            })
            .unwrap();

        let stale = membership(world, 1, 0, 1, Some(initial.record_hash().unwrap()));
        assert!(store.save_membership_record(&stale).is_err());
        assert_eq!(store.load_membership_record(world).unwrap(), initial);

        let promoted = membership(world, 2, 1, 1, Some(initial.record_hash().unwrap()));
        store.save_membership_record(&promoted).unwrap();
        assert_eq!(store.load_membership_record(world).unwrap(), promoted);
        store.save_membership_record(&promoted).unwrap();

        let mut conflict = promoted.clone();
        conflict.signature.push(9);
        assert!(store.save_membership_record(&conflict).is_err());
        assert_eq!(store.load_membership_record(world).unwrap(), promoted);
    }
}
