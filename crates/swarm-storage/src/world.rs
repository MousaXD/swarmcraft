use crate::{atomic_write, io_error, Storage, StorageError};
use std::{fs, path::PathBuf};
use swarm_protocol::{MembershipRecordV1, WorldDescriptorV1, WorldId};

impl Storage {
    pub fn save_world_descriptor(&self, descriptor: &WorldDescriptorV1) -> Result<(), StorageError> {
        let mut descriptor = descriptor.clone();
        descriptor.normalize();
        let bytes = serde_json::to_vec_pretty(&descriptor)?;
        atomic_write(&self.world_protocol_path(descriptor.world_id, "descriptor.json"), &bytes)
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
        let bytes = postcard::to_allocvec(record)?;
        atomic_write(&self.world_protocol_path(record.world_id, "membership.postcard"), &bytes)
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

    pub fn remove_local_membership(&self, world: WorldId) -> Result<(), StorageError> {
        for name in ["descriptor.json", "membership.postcard"] {
            let path = self.world_protocol_path(world, name);
            if path.exists() {
                fs::remove_file(&path).map_err(|error| io_error(&path, error))?;
            }
        }
        Ok(())
    }

    fn world_protocol_path(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
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
