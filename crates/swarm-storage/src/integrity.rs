use crate::{
    transaction::{durable_atomic_write, durable_create_once, durable_remove, WorldTransactionGuard},
    Storage, StorageError,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use swarm_protocol::{Hash32, SnapshotManifestV1, WorldId};

const HEAD_FILE: &str = "canonical-snapshot-head.postcard";
const HEAD_REQUIRED_FILE: &str = "canonical-snapshot-head.required";
const COMMIT_INTENT_FILE: &str = "snapshot-commit.intent.postcard";
const HEAD_REQUIRED_BYTES: &[u8] = b"swarmcraft-canonical-head-v1\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalSnapshotRefV1 {
    pub snapshot_number: u64,
    pub manifest_hash: Hash32,
    pub epoch: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalSnapshotHeadV1 {
    pub world_id: WorldId,
    pub head: Option<CanonicalSnapshotRefV1>,
}

/// Optional caller-provided generation fence for a canonical snapshot commit.
///
/// Ordinary commits are still serialized with epoch/head mutation and verify the
/// currently durable epoch/authority. Callers that own an authority lease can
/// additionally bind the exact fencing token and exact previously observed head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotCommitFence {
    pub expected_epoch: u64,
    pub expected_fencing_token: u64,
    pub expected_head: Option<CanonicalSnapshotRefV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotCommitIntentV1 {
    world_id: WorldId,
    previous_head: Option<CanonicalSnapshotRefV1>,
    next_head: CanonicalSnapshotRefV1,
}

#[derive(Debug)]
pub(crate) struct SnapshotCommitTransaction {
    _guard: WorldTransactionGuard,
    world: WorldId,
    next_head: CanonicalSnapshotRefV1,
    intent_path: PathBuf,
}

impl Storage {
    pub fn canonical_snapshot_head(&self, world: WorldId) -> Result<CanonicalSnapshotHeadV1, StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        self.load_or_initialize_canonical_head_locked(world)
    }

    pub(crate) fn begin_snapshot_commit_transaction(
        &self,
        manifest: &SnapshotManifestV1,
        fence: Option<SnapshotCommitFence>,
    ) -> Result<SnapshotCommitTransaction, StorageError> {
        let guard = self.lock_world_transaction(manifest.world_id)?;
        let current = self.load_or_initialize_canonical_head_locked(manifest.world_id)?;
        let previous = current.head;
        validate_direct_extension(previous, manifest)?;

        let epoch_path = self.world_dir(manifest.world_id).join("metadata").join("epoch.postcard");
        let current_epoch = if epoch_path.exists() { Some(self.load_epoch_record(manifest.world_id)?) } else { None };
        if let Some(epoch) = current_epoch.as_ref() {
            if epoch.epoch_number != manifest.epoch
                || epoch.authority_peer_id != manifest.authority_peer_id
                || epoch.authority_public_key != manifest.authority_public_key
            {
                return Err(StorageError::SnapshotFenceMismatch {
                    world: manifest.world_id,
                    expected_epoch: epoch.epoch_number,
                    expected_fencing_token: epoch.fencing_token,
                });
            }
        }

        if let Some(fence) = fence {
            let epoch = current_epoch.ok_or(StorageError::SnapshotFenceMismatch {
                world: manifest.world_id,
                expected_epoch: fence.expected_epoch,
                expected_fencing_token: fence.expected_fencing_token,
            })?;
            if epoch.epoch_number != fence.expected_epoch
                || epoch.fencing_token != fence.expected_fencing_token
                || previous != fence.expected_head
            {
                return Err(StorageError::SnapshotFenceMismatch {
                    world: manifest.world_id,
                    expected_epoch: fence.expected_epoch,
                    expected_fencing_token: fence.expected_fencing_token,
                });
            }
        }

        let next_head = canonical_ref(manifest)?;
        let intent = SnapshotCommitIntentV1 { world_id: manifest.world_id, previous_head: previous, next_head };
        let intent_path = self.commit_intent_path(manifest.world_id);
        durable_atomic_write(&intent_path, &postcard::to_allocvec(&intent)?)?;

        Ok(SnapshotCommitTransaction { _guard: guard, world: manifest.world_id, next_head, intent_path })
    }

    pub(crate) fn finish_snapshot_commit_transaction(
        &self,
        transaction: SnapshotCommitTransaction,
    ) -> Result<(), StorageError> {
        let head = CanonicalSnapshotHeadV1 { world_id: transaction.world, head: Some(transaction.next_head) };
        durable_atomic_write(&self.canonical_head_path(transaction.world), &postcard::to_allocvec(&head)?)?;
        durable_create_once(&self.head_required_path(transaction.world), HEAD_REQUIRED_BYTES)?;
        durable_remove(&transaction.intent_path)?;
        Ok(())
    }

    pub(crate) fn cancel_snapshot_commit_before_manifest(
        &self,
        transaction: SnapshotCommitTransaction,
    ) -> Result<(), StorageError> {
        durable_remove(&transaction.intent_path)?;
        Ok(())
    }

    pub(crate) fn validate_canonical_snapshot_namespace(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let head = self.load_or_initialize_canonical_head_locked(world)?;
        self.validate_no_orphan_manifests_locked(world, head.head)
    }

    fn load_or_initialize_canonical_head_locked(
        &self,
        world: WorldId,
    ) -> Result<CanonicalSnapshotHeadV1, StorageError> {
        let head_path = self.canonical_head_path(world);
        let required_path = self.head_required_path(world);
        let intent_path = self.commit_intent_path(world);

        if intent_path.exists() {
            let bytes =
                fs::read(&intent_path).map_err(|source| StorageError::Io { path: intent_path.clone(), source })?;
            let intent: SnapshotCommitIntentV1 = postcard::from_bytes(&bytes)?;
            if intent.world_id != world {
                return Err(StorageError::WorldMetadataMismatch);
            }
            if head_path.exists() {
                let head = self.read_head_file(world, &head_path)?;
                if head.head == Some(intent.next_head) {
                    self.validate_head_target_locked(world, head.head)?;
                    durable_remove(&intent_path)?;
                    durable_create_once(&required_path, HEAD_REQUIRED_BYTES)?;
                    return Ok(head);
                }
            }
            return Err(StorageError::SnapshotCommitIncomplete {
                world,
                snapshot_number: intent.next_head.snapshot_number,
            });
        }

        if head_path.exists() {
            let head = self.read_head_file(world, &head_path)?;
            self.validate_head_target_locked(world, head.head)?;
            durable_create_once(&required_path, HEAD_REQUIRED_BYTES)?;
            return Ok(head);
        }

        if required_path.exists() {
            return Err(StorageError::MissingCanonicalHead(world));
        }

        // One-time legacy migration. Before this campaign no durable head file
        // existed, so the highest internally consistent surviving manifest is
        // adopted exactly once. The required marker is then made durable; any
        // later head loss is distinguishable and fails closed.
        let legacy = self.highest_legacy_manifest_locked(world)?;
        let head = CanonicalSnapshotHeadV1 { world_id: world, head: legacy.as_ref().map(canonical_ref).transpose()? };
        durable_atomic_write(&head_path, &postcard::to_allocvec(&head)?)?;
        durable_create_once(&required_path, HEAD_REQUIRED_BYTES)?;
        self.validate_head_target_locked(world, head.head)?;
        Ok(head)
    }

    fn read_head_file(&self, world: WorldId, path: &PathBuf) -> Result<CanonicalSnapshotHeadV1, StorageError> {
        let bytes = fs::read(path).map_err(|source| StorageError::Io { path: path.clone(), source })?;
        let head: CanonicalSnapshotHeadV1 = postcard::from_bytes(&bytes)?;
        if head.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(head)
    }

    fn validate_head_target_locked(
        &self,
        world: WorldId,
        head: Option<CanonicalSnapshotRefV1>,
    ) -> Result<(), StorageError> {
        let Some(head) = head else {
            return Ok(());
        };
        let manifest = self.load_snapshot_file_unchecked(world, head.snapshot_number).map_err(|error| match error {
            StorageError::SnapshotNotFound(_) => StorageError::MissingCanonicalHeadTarget {
                world,
                snapshot_number: head.snapshot_number,
                manifest_hash: head.manifest_hash,
            },
            other => other,
        })?;
        let actual = canonical_ref(&manifest)?;
        if actual != head {
            return Err(StorageError::CanonicalHeadMismatch { world, snapshot_number: head.snapshot_number });
        }
        Ok(())
    }

    fn validate_no_orphan_manifests_locked(
        &self,
        world: WorldId,
        head: Option<CanonicalSnapshotRefV1>,
    ) -> Result<(), StorageError> {
        let manifests = self.raw_snapshot_manifests(world)?;
        let highest_allowed = head.map(|value| value.snapshot_number).unwrap_or(0);
        if let Some(orphan) = manifests.iter().find(|manifest| manifest.snapshot_number > highest_allowed) {
            return Err(StorageError::UncommittedSnapshotOrphan { world, snapshot_number: orphan.snapshot_number });
        }
        Ok(())
    }

    fn highest_legacy_manifest_locked(&self, world: WorldId) -> Result<Option<SnapshotManifestV1>, StorageError> {
        let mut manifests = self.raw_snapshot_manifests(world)?;
        manifests.sort_by_key(|manifest| manifest.snapshot_number);
        Ok(manifests.pop())
    }

    pub(crate) fn raw_snapshot_manifests(&self, world: WorldId) -> Result<Vec<SnapshotManifestV1>, StorageError> {
        let dir = self.world_dir(world).join("snapshots");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut manifests = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|source| StorageError::Io { path: dir.clone(), source })? {
            let entry = entry.map_err(|source| StorageError::Io { path: dir.clone(), source })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("postcard") {
                continue;
            }
            let stem = path.file_stem().and_then(|value| value.to_str()).ok_or(StorageError::WorldMetadataMismatch)?;
            let number = stem.parse::<u64>().map_err(|_| StorageError::WorldMetadataMismatch)?;
            let manifest = self.load_snapshot_file_unchecked(world, number)?;
            if manifest.world_id != world || manifest.snapshot_number != number {
                return Err(StorageError::WorldMetadataMismatch);
            }
            manifests.push(manifest);
        }
        manifests.sort_by_key(|manifest| manifest.snapshot_number);
        Ok(manifests)
    }

    pub(crate) fn load_snapshot_file_unchecked(
        &self,
        world: WorldId,
        number: u64,
    ) -> Result<SnapshotManifestV1, StorageError> {
        let path = self.world_dir(world).join("snapshots").join(format!("{number:020}.postcard"));
        if !path.exists() {
            return Err(StorageError::SnapshotNotFound(number));
        }
        let bytes = fs::read(&path).map_err(|source| StorageError::Io { path: path.clone(), source })?;
        let manifest: SnapshotManifestV1 = postcard::from_bytes(&bytes)?;
        if manifest.world_id != world || manifest.snapshot_number != number {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(manifest)
    }

    fn canonical_head_path(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("metadata").join(HEAD_FILE)
    }

    fn head_required_path(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("metadata").join(HEAD_REQUIRED_FILE)
    }

    fn commit_intent_path(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("metadata").join(COMMIT_INTENT_FILE)
    }
}

fn canonical_ref(manifest: &SnapshotManifestV1) -> Result<CanonicalSnapshotRefV1, StorageError> {
    Ok(CanonicalSnapshotRefV1 {
        snapshot_number: manifest.snapshot_number,
        manifest_hash: manifest.manifest_hash()?,
        epoch: manifest.epoch,
        sequence: manifest.sequence,
    })
}

fn validate_direct_extension(
    previous: Option<CanonicalSnapshotRefV1>,
    manifest: &SnapshotManifestV1,
) -> Result<(), StorageError> {
    match previous {
        None => {
            if manifest.snapshot_number != 1 || manifest.previous_snapshot_hash.is_some() {
                return Err(StorageError::SnapshotHistoryConflict { snapshot_number: manifest.snapshot_number });
            }
        }
        Some(previous) => {
            let expected_number =
                previous.snapshot_number.checked_add(1).ok_or(StorageError::SnapshotNumberExhausted)?;
            let expected_sequence = previous.sequence.checked_add(1).ok_or(StorageError::SnapshotNumberExhausted)?;
            if manifest.snapshot_number != expected_number
                || manifest.sequence != expected_sequence
                || manifest.previous_snapshot_hash != Some(previous.manifest_hash)
            {
                return Err(StorageError::SnapshotHistoryConflict { snapshot_number: manifest.snapshot_number });
            }
        }
    }
    Ok(())
}
