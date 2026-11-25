//! Data-only download namespace for an accepted replica catching up across
//! generations. Nothing in this module publishes canonical snapshots, epochs,
//! membership, receipts, or hosting permission.
use crate::{
    control::StaleReplayAnchorV1,
    replica::ReplicationError,
    transaction::{canonical_file_exists, durable_atomic_write, io_error, read_canonical_file},
    Storage, StorageError,
};
use std::{collections::BTreeSet, fs, path::PathBuf};
use swarm_protocol::{
    BlobDescriptor, Hash32, HistoricalCheckpointV1, HistoricalReplayProofV1, PeerId, SnapshotManifestV1, WorldId,
};

#[derive(Debug)]
pub struct StaleReplayProgress {
    pub anchor: StaleReplayAnchorV1,
    pub proof: HistoricalReplayProofV1,
    /// Count includes the already accepted prefix. The next index is also
    /// the index into the proof's ordered checkpoint references.
    pub completed: usize,
}

#[derive(Debug)]
pub struct StaleReplayDownload {
    canonical: Storage,
    isolated: Storage,
    anchor: StaleReplayAnchorV1,
    source: PeerId,
    membership_hash: Hash32,
    manifest: SnapshotManifestV1,
}

impl Storage {
    pub(crate) fn stale_replay_data_dir(&self, anchor: &StaleReplayAnchorV1) -> Result<PathBuf, StorageError> {
        let bytes = postcard::to_allocvec(anchor)?;
        let hash = blake3::hash(&bytes).to_hex();
        let root = self.world_dir(anchor.epoch.world_id).join("metadata").join("stale-replay-data").join(hash.as_str());
        // Never treat a symlink or a non-directory as a staging namespace.
        for directory in [root.parent().ok_or(StorageError::WorldMetadataMismatch)?, root.as_path()] {
            match fs::symlink_metadata(directory) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(StorageError::SymlinkUnsupported(directory.to_path_buf()))
                }
                Ok(meta) if !meta.is_dir() => return Err(StorageError::WorldMetadataMismatch),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(directory, error)),
            }
        }
        Ok(root)
    }

    pub fn stale_replay_progress(&self, world: WorldId) -> Result<Option<StaleReplayProgress>, StorageError> {
        let _guard = self.lock_world_transaction_for_stale_download(world)?;
        let Some((anchor, proof)) = self.load_staged_stale_replay_proof_locked(world)? else {
            return Ok(None);
        };
        let completed = self.stale_replay_progress_locked(&anchor, &proof)?;
        Ok(Some(StaleReplayProgress { anchor, proof, completed }))
    }

    pub(crate) fn stale_replay_progress_locked(
        &self,
        anchor: &StaleReplayAnchorV1,
        proof: &HistoricalReplayProofV1,
    ) -> Result<usize, StorageError> {
        let root = self.stale_replay_data_dir(anchor)?;
        let start = usize::try_from(anchor.head.snapshot_number).map_err(|_| StorageError::WorldMetadataMismatch)?;
        if start > proof.checkpoints.len() {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let mut completed = start;
        let mut gap = false;
        for (index, expected) in proof.checkpoints.iter().enumerate().skip(start) {
            let path = staged_manifest_path(&root, expected.snapshot_number);
            if !canonical_file_exists(&path)? {
                gap = true;
                continue;
            }
            if gap {
                return Err(StorageError::WorldMetadataMismatch);
            }
            let manifest: SnapshotManifestV1 = postcard::from_bytes(&read_canonical_file(&path)?)?;
            validate_staged_manifest(proof, index, &manifest)?;
            completed += 1;
        }
        Ok(completed)
    }

    /// The caller verifies the complete signed envelope and manifest
    /// signature. Repeat the exact pinned anchor, membership, sequence,
    /// parent and manifest hash checks under the storage transaction lock.
    pub fn open_stale_replay_download(
        &self,
        source: PeerId,
        membership_hash: Hash32,
        manifest: &SnapshotManifestV1,
    ) -> Result<StaleReplayDownload, StorageError> {
        let world = manifest.world_id;
        let _guard = self.lock_world_transaction_for_stale_download(world)?;
        let (anchor, proof) =
            self.load_staged_stale_replay_proof_locked(world)?.ok_or(StorageError::WorldMetadataMismatch)?;
        let completed = self.stale_replay_progress_locked(&anchor, &proof)?;
        let index =
            usize::try_from(manifest.snapshot_number.checked_sub(1).ok_or(StorageError::WorldMetadataMismatch)?)
                .map_err(|_| StorageError::WorldMetadataMismatch)?;
        if index != completed || !matches_target(&proof, source, membership_hash)? {
            return Err(StorageError::WorldMetadataMismatch);
        }
        validate_staged_manifest(&proof, index, manifest)?;
        let isolated = Storage::open(self.stale_replay_data_dir(&anchor)?)?;
        Ok(StaleReplayDownload {
            canonical: self.clone(),
            isolated,
            anchor,
            source,
            membership_hash,
            manifest: manifest.clone(),
        })
    }

    /// Verify a complete staged suffix without publishing it. The caller
    /// must re-verify every manifest signature and the complete proof before
    /// any subsequent canonical publication transaction.
    pub fn verified_stale_replay_suffix(&self, world: WorldId) -> Result<Vec<SnapshotManifestV1>, StorageError> {
        let _gc = self.lock_blob_gc_for_snapshot_commit(world)?;
        let _guard = self.lock_world_transaction_for_stale_download(world)?;
        let (anchor, proof) =
            self.load_staged_stale_replay_proof_locked(world)?.ok_or(StorageError::WorldMetadataMismatch)?;
        if self.stale_replay_progress_locked(&anchor, &proof)? != proof.checkpoints.len() {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let root = self.stale_replay_data_dir(&anchor)?;
        let isolated = Storage::open(root.clone())?;
        let start = usize::try_from(anchor.head.snapshot_number).map_err(|_| StorageError::WorldMetadataMismatch)?;
        let mut suffix = Vec::new();
        for (index, checkpoint) in proof.checkpoints.iter().enumerate().skip(start) {
            let manifest: SnapshotManifestV1 =
                postcard::from_bytes(&read_canonical_file(&staged_manifest_path(&root, checkpoint.snapshot_number))?)?;
            validate_staged_manifest(&proof, index, &manifest)?;
            for entry in &manifest.entries {
                if !self.has_complete_blob(world, &entry.blob) {
                    isolated.verify_blob_streaming(world, &entry.blob)?;
                }
            }
            suffix.push(manifest);
        }
        Ok(suffix)
    }
}

impl StaleReplayDownload {
    pub fn is_current(&self) -> bool {
        let Ok(_guard) = self.canonical.lock_world_transaction_for_stale_download(self.manifest.world_id) else {
            return false;
        };
        self.ensure_current_locked().is_ok()
    }

    fn ensure_current_locked(&self) -> Result<(), StorageError> {
        let world = self.manifest.world_id;
        let (anchor, proof) =
            self.canonical.load_staged_stale_replay_proof_locked(world)?.ok_or(StorageError::WorldMetadataMismatch)?;
        let index =
            usize::try_from(self.manifest.snapshot_number.checked_sub(1).ok_or(StorageError::WorldMetadataMismatch)?)
                .map_err(|_| StorageError::WorldMetadataMismatch)?;
        if anchor != self.anchor
            || !matches_target(&proof, self.source, self.membership_hash)?
            || self.canonical.stale_replay_progress_locked(&anchor, &proof)? != index
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        validate_staged_manifest(&proof, index, &self.manifest)
    }

    fn has_blob(&self, descriptor: &BlobDescriptor) -> bool {
        let world = self.manifest.world_id;
        self.canonical.has_complete_blob(world, descriptor) || self.isolated.has_complete_blob(world, descriptor)
    }

    pub fn missing_blobs(&self) -> Result<Vec<(Hash32, u64)>, ReplicationError> {
        let world = self.manifest.world_id;
        let _guard = self.canonical.lock_world_transaction_for_stale_download(world)?;
        self.ensure_current_locked()?;
        let mut seen = BTreeSet::new();
        let mut missing = Vec::new();
        for entry in &self.manifest.entries {
            let blob = &entry.blob;
            if !seen.insert(blob.hash) || self.has_blob(blob) {
                continue;
            }
            if let Some(offset) = self.isolated.resume_historical_blob(world, blob)? {
                missing.push((blob.hash, offset));
            }
        }
        Ok(missing)
    }

    pub fn receive_blob_chunk(
        &self,
        descriptor: &BlobDescriptor,
        offset: u64,
        bytes: &[u8],
        finished: bool,
    ) -> Result<u64, ReplicationError> {
        let world = self.manifest.world_id;
        let _guard = self.canonical.lock_world_transaction_for_stale_download(world)?;
        self.ensure_current_locked()?;
        if !self.manifest.entries.iter().any(|entry| entry.blob == *descriptor) {
            return Err(StorageError::WorldMetadataMismatch.into());
        }
        // The nested staging store cannot bypass the outer world's quota.
        let budget = self.canonical.try_begin_blob_write(world)?;
        budget.require(bytes.len() as u64)?;
        self.isolated.receive_blob_chunk(world, descriptor, offset, bytes, finished)
    }

    /// Verify all bytes and write only the staged manifest slot. A crash
    /// before this write merely renegotiates the already durable blob offsets.
    pub fn finalize(&self) -> Result<(), ReplicationError> {
        let world = self.manifest.world_id;
        let _gc = self.canonical.lock_blob_gc_for_snapshot_commit(world)?;
        let _guard = self.canonical.lock_world_transaction_for_stale_download(world)?;
        self.ensure_current_locked()?;
        if self.manifest.entries.iter().any(|entry| !self.has_blob(&entry.blob)) {
            return Err(ReplicationError::Incomplete(self.manifest.snapshot_number));
        }
        let path =
            staged_manifest_path(&self.canonical.stale_replay_data_dir(&self.anchor)?, self.manifest.snapshot_number);
        if canonical_file_exists(&path)? {
            return Err(StorageError::WorldMetadataMismatch.into());
        }
        let bytes = postcard::to_allocvec(&self.manifest).map_err(StorageError::from)?;
        durable_atomic_write(&path, &bytes).map_err(Into::into)
    }
}

pub(crate) fn staged_manifest_path(root: &std::path::Path, number: u64) -> PathBuf {
    root.join("manifests").join(format!("{number:020}.postcard"))
}

fn matches_target(
    proof: &HistoricalReplayProofV1,
    source: PeerId,
    membership_hash: Hash32,
) -> Result<bool, StorageError> {
    let membership = proof.membership.last().ok_or(StorageError::WorldMetadataMismatch)?;
    Ok(membership.authority_peer_id == source && membership.record_hash()? == membership_hash)
}

pub(crate) fn validate_staged_manifest(
    proof: &HistoricalReplayProofV1,
    index: usize,
    manifest: &SnapshotManifestV1,
) -> Result<(), StorageError> {
    manifest.validate_semantics()?;
    let expected = proof.checkpoints.get(index).ok_or(StorageError::WorldMetadataMismatch)?;
    let epoch_index = usize::try_from(expected.epoch.checked_sub(1).ok_or(StorageError::WorldMetadataMismatch)?)
        .map_err(|_| StorageError::WorldMetadataMismatch)?;
    let epoch = proof.epochs.get(epoch_index).ok_or(StorageError::WorldMetadataMismatch)?;
    let parent = index.checked_sub(1).map(|prior| proof.checkpoints[prior].manifest_hash);
    if manifest.world_id != proof.world_id
        || HistoricalCheckpointV1::from_manifest(manifest)? != *expected
        || manifest.previous_snapshot_hash != parent
        || manifest.authority_peer_id != epoch.authority_peer_id
        || manifest.authority_public_key != epoch.authority_public_key
    {
        return Err(StorageError::WorldMetadataMismatch);
    }
    Ok(())
}
