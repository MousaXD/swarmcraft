use crate::{Storage, StorageError};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
use swarm_protocol::{RecoveryCertificateV1, WorldId};

impl Storage {
    pub fn save_recovery_certificate(&self, certificate: &RecoveryCertificateV1) -> Result<(), StorageError> {
        let world = certificate.ballot.world_id;
        self.load_world(world)?;
        let path = self.world_dir(world).join("metadata").join("recovery-certificate.postcard");
        atomic_write(&path, &postcard::to_allocvec(certificate)?)
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
        let path = self.world_dir(world).join("metadata").join("recovery-certificate.postcard");
        if path.exists() {
            fs::remove_file(&path).map_err(|source| StorageError::Io { path: path.clone(), source })?;
            sync_parent(path.parent().expect("recovery certificate has parent"))?;
        }
        Ok(())
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    fs::create_dir_all(parent).map_err(|source| StorageError::Io { path: parent.to_path_buf(), source })?;
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .map_err(|source| StorageError::Io { path: tmp.clone(), source })?;
    file.write_all(bytes).map_err(|source| StorageError::Io { path: tmp.clone(), source })?;
    file.sync_all().map_err(|source| StorageError::Io { path: tmp.clone(), source })?;
    drop(file);
    fs::rename(&tmp, path).map_err(|source| StorageError::Io { path: path.to_path_buf(), source })?;
    sync_parent(parent)
}

fn sync_parent(parent: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|source| StorageError::Io { path: parent.to_path_buf(), source })?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
}
