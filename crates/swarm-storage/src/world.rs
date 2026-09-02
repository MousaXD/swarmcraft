use crate::{
    transaction::{durable_atomic_write, durable_remove},
    Storage, StorageError,
};
use std::{fs, path::PathBuf};
use swarm_protocol::{JoinRequestV1, LeaveRequestV1, MembershipRecordV1, WorldDescriptorV1, WorldId};

impl Storage {
    pub fn save_world_descriptor(&self, descriptor: &WorldDescriptorV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(descriptor.world_id)?;
        let mut descriptor = descriptor.clone();
        descriptor.normalize();
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
        Ok(descriptor)
    }

    pub fn save_membership_record(&self, record: &MembershipRecordV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(record.world_id)?;
        let bytes = postcard::to_allocvec(record)?;
        durable_atomic_write(&self.world_protocol_path(record.world_id, "membership.postcard"), &bytes)
    }

    pub fn load_membership_record(&self, world: WorldId) -> Result<MembershipRecordV1, StorageError> {
        let path = self.world_protocol_path(world, "membership.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let record: MembershipRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(record)
    }

    pub fn save_pending_join(&self, request: &JoinRequestV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(request.world_id)?;
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
        Ok(request)
    }

    pub fn clear_pending_join(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        durable_remove(&self.world_protocol_path(world, "pending-join.postcard"))?;
        Ok(())
    }

    pub fn save_pending_leave(&self, request: &LeaveRequestV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(request.world_id)?;
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
        Ok(request)
    }

    pub fn clear_pending_leave(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        durable_remove(&self.world_protocol_path(world, "pending-leave.postcard"))?;
        Ok(())
    }

    pub fn remove_local_membership(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        for name in ["descriptor.json", "membership.postcard", "pending-join.postcard", "pending-leave.postcard"] {
            durable_remove(&self.world_protocol_path(world, name))?;
        }
        Ok(())
    }

    fn world_protocol_path(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{Hash32, PeerId, WorldMemberV1, PROTOCOL_VERSION};

    #[test]
    fn descriptor_round_trip_normalizes_members() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([5; 32]);
        let descriptor = WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: Hash32([6; 32]),
            members: vec![
                WorldMemberV1 {
                    peer_id: PeerId([8; 32]),
                    public_key: [8; 32],
                    authority_eligible: true,
                    banned: false,
                },
                WorldMemberV1 {
                    peer_id: PeerId([2; 32]),
                    public_key: [2; 32],
                    authority_eligible: true,
                    banned: false,
                },
            ],
            preferred_replication_factor: 2,
        };
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        store.save_world_descriptor(&descriptor).unwrap();
        let loaded = store.load_world_descriptor(world).unwrap();
        assert_eq!(loaded.members[0].peer_id, PeerId([2; 32]));
    }
}
