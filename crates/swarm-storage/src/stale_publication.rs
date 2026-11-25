//! Crash-resumable publication of a verified stale replica's isolated suffix.
//! The intent fences every ordinary storage transaction and live control read.
//! Only a reverified, byte-exact pinned successor can finish it.
use crate::{
    control::{crash_retirement_history_matches, transfer_history_name, StaleReplayAnchorV1},
    stale_replay::{staged_manifest_path, validate_staged_manifest},
    transaction::{
        canonical_file_exists, create_unique_temp, durable_atomic_write, durable_remove, durable_rename_new, io_error,
        read_canonical_file,
    },
    CanonicalSnapshotHeadV1, CanonicalSnapshotRefV1, DurableMembershipPromiseV1, DurableRecoveryPromiseV1, Storage,
    StorageError,
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use swarm_protocol::{
    AuthorityLeaseGrantV1, AuthorityTransferV1, BlobDescriptor, BlobEncoding, EpochMode, Hash32,
    HistoricalReplayProofV1, HistoricalTransitionV1, MembershipCertificateV1, RecoveryCertificateV1, SleepRecordV1,
    SnapshotManifestV1, WorldId,
};

const INTENT: &str = "stale-replay-publish.intent.postcard";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StalePublicationCandidate {
    pub anchor: StaleReplayAnchorV1,
    pub proof: HistoricalReplayProofV1,
    pub manifests: Vec<SnapshotManifestV1>,
    /// Local evidence not itself contained in the signed replay envelope.
    /// The application must independently verify these signatures.
    pub sleep: Option<SleepRecordV1>,
    pub reservation: Option<AuthorityLeaseGrantV1>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PublicationIntentV1 {
    world: WorldId,
    pinned_hash: Hash32,
}

impl Storage {
    fn stale_publication_path(&self, world: WorldId) -> PathBuf {
        self.world_dir(world).join("metadata").join(INTENT)
    }

    pub fn stale_publication_pending(&self, world: WorldId) -> Result<bool, StorageError> {
        canonical_file_exists(&self.stale_publication_path(world))
    }

    pub(crate) fn ensure_stale_publication_clear(&self, world: WorldId) -> Result<(), StorageError> {
        if self.stale_publication_pending(world)? {
            return Err(StorageError::StaleReplayPublicationIncomplete(world));
        }
        if self.crash_retirement_pending(world)? {
            return Err(StorageError::CrashRetirementReplayIncomplete(world));
        }
        Ok(())
    }

    /// Read the exact candidate even if a prior publication crashed. The
    /// application MUST verify the full envelope and every manifest signature
    /// before calling either publication phase. No live state is changed.
    pub fn stale_publication_candidate(
        &self,
        world: WorldId,
    ) -> Result<Option<StalePublicationCandidate>, StorageError> {
        let _gc = self.lock_blob_gc_for_snapshot_commit(world)?;
        let _guard = self.lock_world_transaction_for_replay(world)?;
        self.stale_publication_candidate_locked(world)
    }

    fn stale_publication_candidate_locked(
        &self,
        world: WorldId,
    ) -> Result<Option<StalePublicationCandidate>, StorageError> {
        let interrupted = self.stale_publication_pending(world)?;
        let pinned = if interrupted {
            self.read_pinned_stale_replay(world)?
        } else {
            self.load_staged_stale_replay_proof_locked(world)?
        };
        let Some((anchor, proof)) = pinned else {
            return if interrupted { Err(StorageError::WorldMetadataMismatch) } else { Ok(None) };
        };
        if !anchor.is_extended_by(&proof)
            || proof.genesis != self.load_world(world)?.genesis
            || proof.transitions.len().checked_add(1) != Some(proof.epochs.len())
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        if interrupted {
            let intent: PublicationIntentV1 =
                postcard::from_bytes(&read_canonical_file(&self.stale_publication_path(world))?)?;
            if intent != make_intent(&anchor, &proof)? {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        if self.stale_replay_progress_locked(&anchor, &proof)? != proof.checkpoints.len() {
            return if interrupted { Err(StorageError::WorldMetadataMismatch) } else { Ok(None) };
        }
        let root = self.stale_replay_data_dir(&anchor)?;
        let isolated = Storage::open(root.clone())?;
        let start = usize::try_from(anchor.head.snapshot_number).map_err(|_| StorageError::WorldMetadataMismatch)?;
        let live = self.raw_snapshot_manifests(world)?;
        if live.len() < start || live.len() > proof.checkpoints.len() || (!interrupted && live.len() != start) {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let mut manifests = Vec::new();
        for (index, expected) in proof.checkpoints.iter().enumerate() {
            let manifest = if index < start {
                live.get(index).ok_or(StorageError::WorldMetadataMismatch)?.clone()
            } else {
                postcard::from_bytes(&read_canonical_file(&staged_manifest_path(&root, expected.snapshot_number))?)?
            };
            validate_staged_manifest(&proof, index, &manifest)?;
            if let Some(existing) = live.get(index) {
                if *existing != manifest {
                    return Err(StorageError::WorldMetadataMismatch);
                }
            }
            for entry in &manifest.entries {
                if index < start {
                    self.verify_blob_streaming(world, &entry.blob)?;
                } else if !self.has_complete_blob(world, &entry.blob) {
                    isolated.verify_blob_streaming(world, &entry.blob)?;
                }
            }
            manifests.push(manifest);
        }
        let metadata = self.world_dir(world).join("metadata");
        let (sleep, reservation) = local_boundary_records(&metadata, &proof)?;
        let candidate = StalePublicationCandidate { anchor, proof, manifests, sleep, reservation };
        self.validate_publication_control_locked(&candidate, interrupted)?;
        Ok(Some(candidate))
    }

    fn validate_publication_control_locked(
        &self,
        candidate: &StalePublicationCandidate,
        interrupted: bool,
    ) -> Result<(), StorageError> {
        let StalePublicationCandidate { anchor, proof, manifests, .. } = candidate;
        let world = proof.world_id;
        let metadata = self.world_dir(world).join("metadata");
        // Only exact decisions already incorporated in the verified history
        // can be consumed. Unrelated, incomplete or corrupt decisions block.
        consumed_decisions(&metadata, proof)?;
        for name in [
            "solo-branch.postcard",
            "pending-join.postcard",
            "pending-leave.postcard",
            "snapshot-commit.intent.postcard",
        ] {
            if canonical_file_exists(&metadata.join(name))? {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        if !self.list_solo_conflicts(world)?.is_empty() {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let final_epoch = proof.epochs.last().ok_or(StorageError::WorldMetadataMismatch)?;
        let final_membership = proof.membership.last().ok_or(StorageError::WorldMetadataMismatch)?;
        let final_config = proof.configs.last().ok_or(StorageError::WorldMetadataMismatch)?;
        let mut descriptor = anchor.descriptor.clone();
        descriptor.members = final_membership.members.clone();
        descriptor.normalize();
        descriptor.validate_semantics()?;
        // The old descriptor's replication policy is local canonical
        // configuration; the signed successor must preserve compatibility.
        if final_config.compatibility_fingerprint()? != descriptor.compatibility_fingerprint
            || final_epoch.mode == EpochMode::Solo
            || final_epoch.epoch_number != final_membership.epoch
            || final_epoch.authority_peer_id != final_membership.authority_peer_id
            || final_epoch.authority_public_key != final_membership.authority_public_key
            || final_epoch.authority_peer_id != final_config.authority_peer_id
            || final_epoch.authority_public_key != final_config.authority_public_key
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        exact_or_target(
            &metadata.join("epoch.postcard"),
            &postcard::to_allocvec(&anchor.epoch)?,
            &postcard::to_allocvec(final_epoch)?,
            interrupted,
        )?;
        exact_or_target(
            &metadata.join("membership.postcard"),
            &postcard::to_allocvec(&anchor.membership)?,
            &postcard::to_allocvec(final_membership)?,
            interrupted,
        )?;
        exact_or_target(
            &metadata.join("world-config.postcard"),
            &postcard::to_allocvec(&anchor.config)?,
            &postcard::to_allocvec(final_config)?,
            interrupted,
        )?;
        // The JSON encoding need not match legacy formatting; compare the
        // decoded full descriptor, including non-membership fields.
        let live_descriptor = serde_json::from_slice::<swarm_protocol::WorldDescriptorV1>(&read_canonical_file(
            &metadata.join("descriptor.json"),
        )?)?;
        if live_descriptor != anchor.descriptor && (!interrupted || live_descriptor != descriptor) {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let head = self.load_or_initialize_canonical_head_locked(world)?;
        let target = manifest_ref(manifests.last().ok_or(StorageError::WorldMetadataMismatch)?)?;
        if head.head != Some(anchor.head) && (!interrupted || head.head != Some(target)) {
            return Err(StorageError::WorldMetadataMismatch);
        }
        // Any existing latest certificate must be exactly one from the
        // accepted prefix (or the intended final certificate on resume).
        let path = metadata.join("membership-certificate.postcard");
        if canonical_file_exists(&path)? {
            let live: MembershipCertificateV1 = postcard::from_bytes(&read_canonical_file(&path)?)?;
            if !proof.membership_certificates.iter().any(|cert| {
                *cert == live
                    && (cert.proposal.proposed.sequence <= anchor.membership.sequence
                        || interrupted && proof.membership_certificates.last() == Some(cert))
            }) {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        // A certified recovery may still occupy either durable slot after a
        // failed promotion. Preserve every byte-exact certificate in the
        // archived chain; no merely matching epoch or ballot is sufficient.
        let mut anchor_recovery_present = false;
        for name in ["recovery-certificate.postcard", "recovery-certificate-pending.postcard"] {
            if let Some(live) = exact_local_record::<RecoveryCertificateV1>(&metadata.join(name))? {
                if !proof
                    .transitions
                    .iter()
                    .any(|transition| matches!(transition, HistoricalTransitionV1::Recovery(cert) if **cert == live))
                {
                    return Err(StorageError::WorldMetadataMismatch);
                }
                if live.ballot.target_epoch == anchor.epoch.epoch_number
                    && live.ballot.target_fencing_token == anchor.epoch.fencing_token
                {
                    anchor_recovery_present = true;
                }
            }
        }
        if !interrupted && anchor.epoch.mode == EpochMode::Recovery && !anchor_recovery_present {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(())
    }

    /// Begin a publication after the application verifies signatures. On
    /// return the exact durable intent has fenced all ordinary live reads
    /// and writes. It is safe to exit the process at this boundary.
    pub fn begin_verified_stale_publication(&self, candidate: &StalePublicationCandidate) -> Result<(), StorageError> {
        self.begin_verified_stale_publication_inner(candidate, None, false)
    }

    /// A formerly authoritative replica must still hold its exact local
    /// committed transfer at the instant the publication intent is installed.
    /// The caller independently verifies the transcript and holds runtime
    /// exclusion across both publication phases. On resume the intent already
    /// fences live access and the consumed record may have been removed.
    pub fn begin_verified_retired_stale_publication(
        &self,
        candidate: &StalePublicationCandidate,
        retirement: &AuthorityTransferV1,
    ) -> Result<(), StorageError> {
        let anchor_epoch = &candidate.anchor.epoch;
        let index =
            usize::try_from(anchor_epoch.epoch_number.checked_sub(1).ok_or(StorageError::WorldMetadataMismatch)?)
                .map_err(|_| StorageError::WorldMetadataMismatch)?;
        if !matches!(candidate.proof.transitions.get(index),
            Some(HistoricalTransitionV1::Transfer { committed, .. }) if committed.as_ref() == retirement)
            || retirement.from_peer_id != anchor_epoch.authority_peer_id
            || candidate
                .proof
                .epochs
                .last()
                .is_none_or(|target| target.authority_peer_id == anchor_epoch.authority_peer_id)
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        self.begin_verified_stale_publication_inner(candidate, Some(retirement), false)
    }

    /// A crash-retired authority must have its independently verified,
    /// durable retirement envelope before canonical publication can start.
    pub fn begin_verified_crash_retirement_publication(
        &self,
        candidate: &StalePublicationCandidate,
    ) -> Result<(), StorageError> {
        if !crash_retirement_history_matches(&candidate.anchor, &candidate.proof) {
            return Err(StorageError::WorldMetadataMismatch);
        }
        self.begin_verified_stale_publication_inner(candidate, None, true)
    }

    fn begin_verified_stale_publication_inner(
        &self,
        candidate: &StalePublicationCandidate,
        retirement: Option<&AuthorityTransferV1>,
        crash_retirement: bool,
    ) -> Result<(), StorageError> {
        let world = candidate.proof.world_id;
        let _gc = self.lock_blob_gc_for_snapshot_commit(world)?;
        let _guard = self.lock_world_transaction_for_replay(world)?;
        if self.stale_publication_candidate_locked(world)?.as_ref() != Some(candidate) {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let intent = make_intent(&candidate.anchor, &candidate.proof)?;
        if self.crash_retirement_pending(world)? && !crash_retirement {
            return Err(StorageError::WorldMetadataMismatch);
        }
        if !self.stale_publication_pending(world)? {
            self.ensure_no_device_retirement_locked(world)?;
            if self.crash_retirement_pending(world)? != crash_retirement {
                return Err(StorageError::WorldMetadataMismatch);
            }
            if let Some(expected) = retirement {
                let path = self.world_dir(world).join("metadata/transfer.postcard");
                if read_canonical_file(&path)? != postcard::to_allocvec(expected)? {
                    return Err(StorageError::WorldMetadataMismatch);
                }
            }
            durable_atomic_write(&self.stale_publication_path(world), &postcard::to_allocvec(&intent)?)?;
        }
        Ok(())
    }

    /// Resume only the verified intent. All writes are exact or idempotent,
    /// the epoch is last among the canonical records, and the intent is
    /// removed only after the complete target state has been verified.
    pub fn finish_verified_stale_publication(&self, candidate: &StalePublicationCandidate) -> Result<(), StorageError> {
        let world = candidate.proof.world_id;
        let _gc = self.lock_blob_gc_for_snapshot_commit(world)?;
        let _guard = self.lock_world_transaction_for_replay(world)?;
        if !self.stale_publication_pending(world)?
            || self.stale_publication_candidate_locked(world)?.as_ref() != Some(candidate)
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let StalePublicationCandidate { anchor, proof, manifests, sleep, reservation } = candidate;
        let root = self.stale_replay_data_dir(anchor)?;
        let isolated = Storage::open(root)?;
        let start = usize::try_from(anchor.head.snapshot_number).map_err(|_| StorageError::WorldMetadataMismatch)?;
        // Copy verified bytes without replacing any existing canonical blob.
        // Use streaming I/O; a large checkpoint must not require a large RAM
        // buffer or consume its only durable isolated copy on interruption.
        for manifest in manifests.iter().skip(start) {
            for entry in &manifest.entries {
                copy_verified_blob(self, &isolated, world, &entry.blob)?;
            }
            self.verify_snapshot_streaming(manifest)?;
            let path =
                self.world_dir(world).join("snapshots").join(format!("{:020}.postcard", manifest.snapshot_number));
            write_once_exact(&path, &postcard::to_allocvec(manifest)?)?;
        }
        let epoch = proof.epochs.last().ok_or(StorageError::WorldMetadataMismatch)?;
        let membership = proof.membership.last().ok_or(StorageError::WorldMetadataMismatch)?;
        let config = proof.configs.last().ok_or(StorageError::WorldMetadataMismatch)?;
        for historical in &proof.epochs {
            self.archive_history_locked(
                world,
                &format!("epoch-{}.postcard", historical.record_hash()?),
                &postcard::to_allocvec(historical)?,
            )?;
        }
        for historical in &proof.configs {
            self.archive_history_locked(
                world,
                &format!("config-{}.postcard", historical.config_hash()?),
                &postcard::to_allocvec(historical)?,
            )?;
        }
        for historical in &proof.membership {
            self.archive_membership_locked(historical)?;
        }
        for certificate in &proof.membership_certificates {
            self.archive_membership_certificate_locked(certificate, &postcard::to_allocvec(certificate)?)?;
        }
        let mut final_recovery = None;
        for (index, transition) in proof.transitions.iter().enumerate() {
            match transition {
                HistoricalTransitionV1::Transfer { prepared, accepted, committed } => {
                    for phase in [prepared, accepted, committed] {
                        self.archive_history_locked(
                            world,
                            &transfer_history_name(phase, phase.phase),
                            &postcard::to_allocvec(phase.as_ref())?,
                        )?;
                    }
                }
                HistoricalTransitionV1::Recovery(certificate) => {
                    let successor = &proof.epochs[index + 1];
                    let bytes = postcard::to_allocvec(certificate.as_ref())?;
                    self.archive_history_locked(
                        world,
                        &format!("recovery-{}.postcard", successor.record_hash()?),
                        &bytes,
                    )?;
                    if successor == epoch {
                        final_recovery = Some(bytes);
                    }
                }
                HistoricalTransitionV1::SoleMemberWake(record) => self.archive_sleep_locked(record)?,
            }
        }
        // Independently verified local boundaries are immutable historical
        // evidence before any live decision is removed. Replaying after a
        // crash repeats this exact archival operation, never grants a lease.
        if let Some(record) = sleep {
            self.archive_sleep_locked(record)?;
        }
        if let Some(record) = reservation {
            let bytes = postcard::to_allocvec(record)?;
            let hash = Hash32(*blake3::hash(&bytes).as_bytes());
            self.archive_history_locked(world, &format!("recovery-reservation-{hash}.postcard"), &bytes)?;
        }
        let metadata = self.world_dir(world).join("metadata");
        if epoch.mode == EpochMode::Recovery {
            durable_atomic_write(
                &metadata.join("recovery-certificate.postcard"),
                &final_recovery.ok_or(StorageError::WorldMetadataMismatch)?,
            )?;
        } else {
            durable_remove(&metadata.join("recovery-certificate.postcard"))?;
        }
        if let Some(certificate) = proof.membership_certificates.last() {
            durable_atomic_write(
                &metadata.join("membership-certificate.postcard"),
                &postcard::to_allocvec(certificate)?,
            )?;
        }
        let mut descriptor = anchor.descriptor.clone();
        descriptor.members = membership.members.clone();
        descriptor.normalize();
        durable_atomic_write(&metadata.join("descriptor.json"), &serde_json::to_vec_pretty(&descriptor)?)?;
        durable_atomic_write(&metadata.join("membership.postcard"), &postcard::to_allocvec(membership)?)?;
        durable_atomic_write(&metadata.join("world-config.postcard"), &postcard::to_allocvec(config)?)?;
        let target = manifest_ref(manifests.last().ok_or(StorageError::WorldMetadataMismatch)?)?;
        let head = CanonicalSnapshotHeadV1 { world_id: world, head: Some(target) };
        durable_atomic_write(&metadata.join("canonical-snapshot-head.postcard"), &postcard::to_allocvec(&head)?)?;
        durable_atomic_write(&metadata.join("epoch.postcard"), &postcard::to_allocvec(epoch)?)?;
        // Check the entire live chain while still fenced. Do not remove the
        // intent merely because the final epoch file has appeared.
        if self.raw_snapshot_manifests(world)? != *manifests
            || self.load_or_initialize_canonical_head_locked(world)? != head
            || read_canonical_file(&metadata.join("epoch.postcard"))? != postcard::to_allocvec(epoch)?
            || read_canonical_file(&metadata.join("membership.postcard"))? != postcard::to_allocvec(membership)?
            || read_canonical_file(&metadata.join("world-config.postcard"))? != postcard::to_allocvec(config)?
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        // The accepted history durably preserves the exact signed evidence.
        // Consume only those records while the intent still fences all live
        // readers and writers. A crash between removals is resumable; no
        // unrelated or newer promise can be discarded on restart.
        for name in consumed_decisions(&metadata, proof)? {
            durable_remove(&metadata.join(name))?;
        }
        // All recovery certificates (including the pending slot) are now
        // archived and the primary slot is the exact final accepted proof.
        durable_remove(&metadata.join("recovery-certificate-pending.postcard"))?;
        if sleep.is_some() {
            durable_remove(&metadata.join("sleep.postcard"))?;
        }
        if reservation.is_some() {
            durable_remove(&metadata.join("recovery-reservation.postcard"))?;
        }
        self.finish_crash_retirement_replay_locked(anchor, proof)?;
        durable_remove(&self.stale_publication_path(world))?;
        Ok(())
    }
}

/// Local records outside the envelope require both exact semantic binding
/// here and independent cryptographic verification by the application.
fn exact_local_record<T>(path: &Path) -> Result<Option<T>, StorageError>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    if !canonical_file_exists(path)? {
        return Ok(None);
    }
    let bytes = read_canonical_file(path)?;
    let record = postcard::from_bytes(&bytes)?;
    if bytes != postcard::to_allocvec(&record)? {
        return Err(StorageError::WorldMetadataMismatch);
    }
    Ok(Some(record))
}

fn local_boundary_records(
    metadata: &Path,
    proof: &HistoricalReplayProofV1,
) -> Result<(Option<SleepRecordV1>, Option<AuthorityLeaseGrantV1>), StorageError> {
    let sleep = exact_local_record::<SleepRecordV1>(&metadata.join("sleep.postcard"))?;
    if let Some(record) = &sleep {
        record.validate_semantics()?;
        if record.world_id != proof.world_id
            || !proof.transitions.iter().enumerate().any(|(index, transition)| {
                let predecessor = &proof.epochs[index];
                record.epoch == predecessor.epoch_number
                    && record.fencing_token == predecessor.fencing_token
                    && record.authority_peer_id == predecessor.authority_peer_id
                    && record.authority_public_key == predecessor.authority_public_key
                    && match transition {
                        HistoricalTransitionV1::SoleMemberWake(certified) => certified.as_ref() == record,
                        HistoricalTransitionV1::Recovery(certified) => {
                            certified.ballot.base_snapshot_hash == record.latest_snapshot_hash
                                && certified.ballot.base_epoch == record.epoch
                                && certified.ballot.base_fencing_token == record.fencing_token
                        }
                        HistoricalTransitionV1::Transfer { .. } => false,
                    }
            })
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
    }
    let reservation = exact_local_record::<AuthorityLeaseGrantV1>(&metadata.join("recovery-reservation.postcard"))?;
    if let Some(record) = &reservation {
        record.validate_semantics()?;
        // Only a strictly superseded generation in the certified chain can
        // consume a reservation. A final/newer or conflicting one is retained.
        if record.world_id != proof.world_id
            || record.lease_duration_ms == 0
            || !proof.epochs.iter().take(proof.epochs.len().saturating_sub(1)).any(|epoch| {
                record.epoch == epoch.epoch_number
                    && record.fencing_token == epoch.fencing_token
                    && record.authority_peer_id == epoch.authority_peer_id
                    && record.authority_public_key == epoch.authority_public_key
            })
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
    }
    Ok((sleep, reservation))
}

/// Compare the complete canonical encoding, not just an epoch or ballot
/// number. The application has to verify the signed replay envelope before
/// beginning or finishing publication; equality to that evidence also binds
/// the signer, vote, checkpoint, generation and membership. A promise not
/// represented by an exact signed vote remains fenced (even if older).
fn consumed_decisions(metadata: &Path, proof: &HistoricalReplayProofV1) -> Result<Vec<&'static str>, StorageError> {
    let mut consumed = Vec::new();
    let name = "transfer.postcard";
    let path = metadata.join(name);
    if canonical_file_exists(&path)? {
        let bytes = read_canonical_file(&path)?;
        let record: AuthorityTransferV1 = postcard::from_bytes(&bytes)?;
        if bytes != postcard::to_allocvec(&record)?
            || !proof.transitions.iter().any(|transition| {
                matches!(transition, HistoricalTransitionV1::Transfer { prepared, accepted, committed }
                    if [prepared.as_ref(), accepted.as_ref(), committed.as_ref()].contains(&&record))
            })
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        consumed.push(name);
    }
    let name = "recovery-promise.postcard";
    let path = metadata.join(name);
    if canonical_file_exists(&path)? {
        let bytes = read_canonical_file(&path)?;
        let promise: DurableRecoveryPromiseV1 = postcard::from_bytes(&bytes)?;
        if bytes != postcard::to_allocvec(&promise)?
            || !proof.transitions.iter().any(|transition| {
                matches!(transition, HistoricalTransitionV1::Recovery(certificate)
                    if certificate.ballot == promise.ballot && certificate.votes.contains(&promise.vote))
            })
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        consumed.push(name);
    }
    let name = "membership-promise.postcard";
    let path = metadata.join(name);
    if canonical_file_exists(&path)? {
        let bytes = read_canonical_file(&path)?;
        let promise: DurableMembershipPromiseV1 = postcard::from_bytes(&bytes)?;
        if bytes != postcard::to_allocvec(&promise)?
            || !proof.membership_certificates.iter().any(|certificate| {
                certificate.proposal == promise.proposal && certificate.votes.contains(&promise.vote)
            })
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        consumed.push(name);
    }
    Ok(consumed)
}

fn make_intent(
    anchor: &StaleReplayAnchorV1,
    proof: &HistoricalReplayProofV1,
) -> Result<PublicationIntentV1, StorageError> {
    Ok(PublicationIntentV1 {
        world: proof.world_id,
        pinned_hash: Hash32(*blake3::hash(&postcard::to_allocvec(&(anchor, proof))?).as_bytes()),
    })
}

fn manifest_ref(manifest: &SnapshotManifestV1) -> Result<CanonicalSnapshotRefV1, StorageError> {
    Ok(CanonicalSnapshotRefV1 {
        snapshot_number: manifest.snapshot_number,
        manifest_hash: manifest.manifest_hash()?,
        epoch: manifest.epoch,
        sequence: manifest.sequence,
    })
}

fn exact_or_target(path: &Path, old: &[u8], target: &[u8], interrupted: bool) -> Result<(), StorageError> {
    let actual = read_canonical_file(path)?;
    if actual != old && (!interrupted || actual != target) {
        return Err(StorageError::WorldMetadataMismatch);
    }
    Ok(())
}

fn write_once_exact(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    if canonical_file_exists(path)? {
        if read_canonical_file(path)? != bytes {
            return Err(StorageError::WorldMetadataMismatch);
        }
        return Ok(());
    }
    durable_atomic_write(path, bytes)
}

fn blob_path(storage: &Storage, world: WorldId, blob: &BlobDescriptor) -> PathBuf {
    let suffix = match blob.encoding {
        BlobEncoding::Raw => "raw",
        BlobEncoding::Zstd => "zst",
    };
    storage.world_dir(world).join("blobs").join(format!("{}.{}", blob.hash.to_hex(), suffix))
}

fn copy_verified_blob(
    canonical: &Storage,
    isolated: &Storage,
    world: WorldId,
    blob: &BlobDescriptor,
) -> Result<(), StorageError> {
    let destination = blob_path(canonical, world, blob);
    if canonical_file_exists(&destination)? {
        return canonical.verify_blob_streaming(world, blob);
    }
    let source = blob_path(isolated, world, blob);
    if !canonical_file_exists(&source)? {
        return Err(StorageError::BlobCorrupt(blob.hash));
    }
    isolated.verify_blob_streaming(world, blob)?;
    let parent = destination.parent().ok_or(StorageError::WorldMetadataMismatch)?;
    let (temporary, mut output) = create_unique_temp(parent, "stale-replay", "tmp")?;
    let result = (|| {
        let mut input = fs::File::open(&source).map_err(|error| io_error(&source, error))?;
        let copied = io::copy(&mut input, &mut output).map_err(|error| io_error(&temporary, error))?;
        if copied != blob.encoded_size {
            return Err(StorageError::BlobCorrupt(blob.hash));
        }
        output.sync_all().map_err(|error| io_error(&temporary, error))?;
        drop(output);
        durable_rename_new(&temporary, &destination)?;
        canonical.verify_blob_streaming(world, blob)
    })();
    if result.is_err() {
        let _ = durable_remove(&temporary);
    }
    result
}
