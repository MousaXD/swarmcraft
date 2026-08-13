//! Core identity, durable paths, signatures, and world genesis helpers.

use directories::BaseDirs;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use swarm_protocol::{
    peer_id_from_public_key, AuthorityLeaseGrantV1, AuthorityTransferV1, Hash32, InviteV1, MembershipRecordV1,
    PeerHelloV1, PeerId, SnapshotManifestV1, WorldGenesisV1, WorldId, PROTOCOL_VERSION,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("could not determine the operating system data directory")]
    DataDirectoryUnavailable,
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid private identity key length: expected 32 bytes, got {0}")]
    InvalidIdentityKey(usize),
    #[error("peer identity mismatch")]
    PeerIdentityMismatch,
    #[error("signature verification failed")]
    SignatureInvalid,
    #[error(transparent)]
    Protocol(#[from] swarm_protocol::ProtocolError),
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> CoreError {
    StorageErrorMarker::map(path, source)
}

struct StorageErrorMarker;

impl StorageErrorMarker {
    fn map(path: impl Into<PathBuf>, source: std::io::Error) -> CoreError {
        CoreError::Io { path: path.into(), source }
    }
}

#[derive(Debug, Clone)]
pub struct DataPaths {
    pub root: PathBuf,
}

impl DataPaths {
    pub fn discover() -> Result<Self, CoreError> {
        let base = BaseDirs::new().ok_or(CoreError::DataDirectoryUnavailable)?;
        Ok(Self { root: base.data_local_dir().join("swarmcraft") })
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn identity_dir(&self) -> PathBuf {
        self.root.join("identity")
    }

    pub fn identity_key(&self) -> PathBuf {
        self.identity_dir().join("peer.key")
    }

    pub fn transport_key(&self) -> PathBuf {
        self.identity_dir().join("transport.key")
    }

    pub fn worlds_dir(&self) -> PathBuf {
        self.root.join("worlds")
    }

    pub fn ensure(&self) -> Result<(), CoreError> {
        for path in [self.identity_dir(), self.worlds_dir()] {
            fs::create_dir_all(&path).map_err(|e| io_error(path, e))?;
        }
        Ok(())
    }
}

pub struct PeerIdentity {
    signing_key: SigningKey,
}

impl std::fmt::Debug for PeerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerIdentity").field("peer_id", &self.peer_id()).finish_non_exhaustive()
    }
}

impl PeerIdentity {
    pub fn generate() -> Self {
        Self { signing_key: SigningKey::generate(&mut OsRng) }
    }

    pub fn from_secret_bytes(secret: [u8; 32]) -> Self {
        Self { signing_key: SigningKey::from_bytes(&secret) }
    }

    pub fn load_or_create(paths: &DataPaths) -> Result<Self, CoreError> {
        paths.ensure()?;
        let key_path = paths.identity_key();
        if key_path.exists() {
            let bytes = fs::read(&key_path).map_err(|e| io_error(&key_path, e))?;
            let secret: [u8; 32] =
                bytes.as_slice().try_into().map_err(|_| CoreError::InvalidIdentityKey(bytes.len()))?;
            return Ok(Self::from_secret_bytes(secret));
        }

        let identity = Self::generate();
        atomic_write_private(&key_path, &identity.signing_key.to_bytes())?;
        Ok(identity)
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn peer_id(&self) -> PeerId {
        peer_id_from_public_key(&self.public_key())
    }

    pub fn sign(&self, bytes: &[u8]) -> Vec<u8> {
        self.signing_key.sign(bytes).to_bytes().to_vec()
    }

    pub fn sign_snapshot(&self, manifest: &mut SnapshotManifestV1) -> Result<(), CoreError> {
        manifest.authority_public_key = self.public_key();
        manifest.authority_peer_id = self.peer_id();
        manifest.signature.clear();
        manifest.signature = self.sign(&manifest.signing_bytes()?);
        Ok(())
    }

    pub fn sign_membership(&self, record: &mut MembershipRecordV1) -> Result<(), CoreError> {
        record.authority_public_key = self.public_key();
        record.authority_peer_id = self.peer_id();
        record.signature.clear();
        record.signature = self.sign(&record.signing_bytes()?);
        Ok(())
    }

    pub fn sign_invite(&self, invite: &mut InviteV1) -> Result<(), CoreError> {
        invite.inviter_public_key = self.public_key();
        invite.inviter_peer_id = self.peer_id();
        invite.signature.clear();
        invite.signature = self.sign(&invite.signing_bytes()?);
        Ok(())
    }

    pub fn sign_transfer(&self, transfer: &mut AuthorityTransferV1) -> Result<(), CoreError> {
        transfer.signer_public_key = self.public_key();
        transfer.signer_peer_id = self.peer_id();
        transfer.signature.clear();
        transfer.signature = self.sign(&transfer.signing_bytes()?);
        Ok(())
    }

    pub fn sign_lease(&self, lease: &mut AuthorityLeaseGrantV1) -> Result<(), CoreError> {
        lease.authority_public_key = self.public_key();
        lease.authority_peer_id = self.peer_id();
        lease.signature.clear();
        lease.signature = self.sign(&lease.signing_bytes()?);
        Ok(())
    }

    pub fn signed_peer_hello(&self, capabilities: Vec<String>) -> Result<PeerHelloV1, CoreError> {
        let mut hello = PeerHelloV1 {
            peer_id: self.peer_id(),
            public_key: self.public_key(),
            protocol_versions: vec![PROTOCOL_VERSION],
            capabilities,
            nonce: random_nonce(),
            signature: Vec::new(),
        };
        hello.signature = self.sign(&hello.signing_bytes()?);
        Ok(hello)
    }
}

pub fn random_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

pub fn verify_signature(
    expected_peer: PeerId,
    public_key: [u8; 32],
    message: &[u8],
    signature: &[u8],
) -> Result<(), CoreError> {
    if peer_id_from_public_key(&public_key) != expected_peer {
        return Err(CoreError::PeerIdentityMismatch);
    }
    let verifying_key = VerifyingKey::from_bytes(&public_key).map_err(|_| CoreError::SignatureInvalid)?;
    let signature = Signature::from_slice(signature).map_err(|_| CoreError::SignatureInvalid)?;
    verifying_key.verify(message, &signature).map_err(|_| CoreError::SignatureInvalid)
}

pub fn verify_snapshot_signature(manifest: &SnapshotManifestV1) -> Result<(), CoreError> {
    verify_signature(
        manifest.authority_peer_id,
        manifest.authority_public_key,
        &manifest.signing_bytes()?,
        &manifest.signature,
    )
}

pub fn verify_membership_signature(record: &MembershipRecordV1) -> Result<(), CoreError> {
    verify_signature(
        record.authority_peer_id,
        record.authority_public_key,
        &record.signing_bytes()?,
        &record.signature,
    )
}

pub fn verify_invite_signature(invite: &InviteV1) -> Result<(), CoreError> {
    verify_signature(
        invite.inviter_peer_id,
        invite.inviter_public_key,
        &invite.signing_bytes()?,
        &invite.signature,
    )
}

pub fn verify_transfer_signature(transfer: &AuthorityTransferV1) -> Result<(), CoreError> {
    verify_signature(
        transfer.signer_peer_id,
        transfer.signer_public_key,
        &transfer.signing_bytes()?,
        &transfer.signature,
    )
}

pub fn verify_lease_signature(lease: &AuthorityLeaseGrantV1) -> Result<(), CoreError> {
    verify_signature(
        lease.authority_peer_id,
        lease.authority_public_key,
        &lease.signing_bytes()?,
        &lease.signature,
    )
}

pub fn create_world_genesis(
    identity: &PeerIdentity,
    minecraft_version: String,
    fabric_loader_version: String,
    compatibility_material: &[u8],
) -> Result<(WorldId, WorldGenesisV1), CoreError> {
    let compatibility_fingerprint = Hash32(*blake3::hash(compatibility_material).as_bytes());
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version,
        fabric_loader_version,
        compatibility_fingerprint,
        creation_nonce: random_nonce(),
        creator_public_key: identity.public_key(),
        initial_membership: vec![identity.peer_id()],
    };
    let world_id = genesis.world_id()?;
    Ok((world_id, genesis))
}

fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let parent = path.parent().ok_or(CoreError::DataDirectoryUnavailable)?;
    fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    let tmp = path.with_extension("tmp");

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| io_error(&tmp, e))?
    };

    #[cfg(not(unix))]
    let mut file =
        OpenOptions::new().create(true).write(true).truncate(true).open(&tmp).map_err(|e| io_error(&tmp, e))?;

    file.write_all(bytes).map_err(|e| io_error(&tmp, e))?;
    file.sync_all().map_err(|e| io_error(&tmp, e))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| io_error(path, e))?;
    sync_parent(parent)?;
    Ok(())
}

fn sync_parent(_parent: &Path) -> Result<(), CoreError> {
    #[cfg(unix)]
    {
        let file = fs::File::open(_parent).map_err(|e| io_error(_parent, e))?;
        file.sync_all().map_err(|e| io_error(_parent, e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let first = PeerIdentity::load_or_create(&paths).unwrap();
        let second = PeerIdentity::load_or_create(&paths).unwrap();
        assert_eq!(first.peer_id(), second.peer_id());
        assert_eq!(fs::metadata(paths.identity_key()).unwrap().len(), 32);
    }

    #[test]
    fn signatures_reject_wrong_peer() {
        let a = PeerIdentity::generate();
        let b = PeerIdentity::generate();
        let msg = b"hello";
        let sig = a.sign(msg);
        assert!(verify_signature(a.peer_id(), a.public_key(), msg, &sig).is_ok());
        assert!(verify_signature(b.peer_id(), a.public_key(), msg, &sig).is_err());
    }

    #[test]
    fn signed_peer_hello_verifies() {
        let identity = PeerIdentity::generate();
        let hello = identity.signed_peer_hello(vec!["snapshot-replication-v1".into()]).unwrap();
        verify_signature(hello.peer_id, hello.public_key, &hello.signing_bytes().unwrap(), &hello.signature).unwrap();
    }
}
