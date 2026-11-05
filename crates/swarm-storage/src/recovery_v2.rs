use crate::{
    transaction::{durable_atomic_write, durable_remove},
    Storage, StorageError,
};
use std::fs;
use swarm_protocol::{RecoveryCertificateV1, WorldId};

impl Storage {
    pub fn save_recovery_certificate(&self, certificate: &RecoveryCertificateV1) -> Result<(), StorageError> {
        let world = certificate.ballot.world_id;
        let _guard = self.lock_world_transaction(world)?;
        self.load_world(world)?;
        let path = self.world_dir(world).join("metadata").join("recovery-certificate.postcard");
        durable_atomic_write(&path, &postcard::to_allocvec(certificate)?)
    }

    pub fn load_recovery_certificate(&self, world: WorldId) -> Result<RecoveryCertificateV1, StorageError> {
        let path = self.world_dir(world).join("metadata").join("recovery-certificate.postcard");
        let bytes = fs::read(&path).map_err(|source| StorageError::Io { path: path.clone(), source })?;
        let certificate: RecoveryCertificateV1 = postcard::from_bytes(&bytes)?;
        if certificate.ballot.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(certificate)
    }

    pub fn clear_recovery_certificate(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let path = self.world_dir(world).join("metadata").join("recovery-certificate.postcard");
        durable_remove(&path)?;
        Ok(())
    }
}
