use crate::{Storage, StorageError};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use swarm_protocol::{AuthorityTransferV1, EpochRecordV1, SleepRecordV1, WorldId};

impl Storage {
    pub fn save_epoch_record(&self, record: &EpochRecordV1) -> Result<(), StorageError> {
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
        Ok(record)
    }

    pub fn save_transfer_record(&self, transfer: &AuthorityTransferV1) -> Result<(), StorageError> {
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
        Ok(record)
    }

    pub fn save_sleep_record(&self, record: &SleepRecordV1) -> Result<(), StorageError> {
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
        Ok(record)
    }

    pub fn clear_sleep_record(&self, world: WorldId) -> Result<(), StorageError> {
        let path = self.control_path(world, "sleep.postcard");
        if path.exists() {
            fs::remove_file(&path).map_err(|error| io_error(&path, error))?;
        }
        Ok(())
    }

    fn control_path(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
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
}
