//! Signed history collection and data-only checkpoint replay verification.
//! A signed chain is NOT a live authority grant. Membership, transition
//! proofs, checkpoint boundaries and independent live freshness are required.
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use swarm_consensus::{membership::validate_membership_certificate_shape, validate_recovery_certificate_shape};
use swarm_core::{
    protocol_v2::{verify_recovery_ballot_signature, verify_recovery_vote_signature, verify_world_config_signature},
    verify_membership_signature, verify_signature, verify_snapshot_signature, verify_transfer_signature, DataPaths,
    PeerIdentity,
};
use swarm_protocol::{
    peer_id_from_public_key, AuthorityTransferV1, EpochMode, EpochRecordV1, HistoricalCheckpointV1,
    HistoricalReplayProofV1, HistoricalTransitionV1, MembershipRecordV1, SnapshotManifestV1, TransferPhase,
    WorldConfigV1, WorldGenesisV1, WorldId, MAX_REPLAY_CHECKPOINTS, MAX_REPLAY_CONFIGS, MAX_REPLAY_EPOCHS,
    MAX_REPLAY_MEMBERSHIP_RECORDS, MAX_REPLAY_PROOF_BYTES, PROTOCOL_VERSION,
};
use swarm_storage::{control::StaleReplayAnchorV1, Storage};

// Bounded independently of peer-supplied record counts. Large or legacy
// histories are explicitly unsupported, never silently truncated.
pub const MAX_HISTORICAL_EPOCHS: usize = MAX_REPLAY_EPOCHS;
pub const MAX_HISTORICAL_CONFIGS: usize = MAX_REPLAY_CONFIGS;

/// Follow exact signed predecessor hashes back to a new-world genesis
/// quorum. This neither writes canonical state nor verifies authority
/// transitions; callers must not use its result as a live quorum proof.
pub fn collect_signed_epoch_history(storage: &Storage, world: WorldId) -> Result<Vec<EpochRecordV1>> {
    let metadata = storage.load_world(world)?;
    let creator = peer_id_from_public_key(&metadata.genesis.creator_public_key);
    let accepted = storage.load_epoch_record(world)?;
    let mut record = accepted.clone();
    let mut history = Vec::new();
    loop {
        if history.len() >= MAX_HISTORICAL_EPOCHS {
            bail!("signed epoch history exceeds the supported replay limit");
        }
        record.validate_semantics()?;
        if record.world_id != world {
            bail!("signed epoch belongs to a different world");
        }
        verify_signature(
            record.authority_peer_id,
            record.authority_public_key,
            &record.signing_bytes()?,
            &record.signature,
        )?;
        let previous_hash = record.previous_epoch_hash;
        history.push(record);
        let Some(hash) = previous_hash else { break };
        let previous =
            storage.load_epoch_by_hash(world, hash).context("required signed predecessor epoch is unavailable")?;
        let next = history.last().context("missing successor epoch")?;
        if previous.epoch_number.checked_add(1) != Some(next.epoch_number)
            || previous.fencing_token.checked_add(1) != Some(next.fencing_token)
        {
            bail!("historical epoch does not advance the predecessor exactly once");
        }
        record = previous;
    }
    history.reverse();
    let first = history.first().context("empty signed epoch history")?;
    if first.epoch_number != crate::world_genesis::GENESIS_EPOCH
        || first.fencing_token != 1
        || first.mode != EpochMode::Quorum
        || first.authority_peer_id != creator
        || first.authority_public_key != metadata.genesis.creator_public_key
        || metadata.genesis.initial_membership != [creator]
    {
        bail!("history does not begin at the supported creator-signed genesis quorum");
    }
    let base = storage.load_snapshot(world, 1).context("original signed checkpoint is unavailable")?;
    verify_snapshot_signature(&base)?;
    storage.verify_snapshot(&base)?;
    if base.epoch != first.epoch_number
        || base.sequence != 1
        || base.previous_snapshot_hash.is_some()
        || base.authority_peer_id != creator
        || base.authority_public_key != first.authority_public_key
        || base.state_root != first.base_state_hash
    {
        bail!("initial epoch is not bound to its original verified checkpoint");
    }
    if storage.load_epoch_record(world)? != accepted {
        bail!("accepted epoch changed while collecting signed history");
    }
    Ok(history)
}

/// Verified metadata is a capability for checking data-only manifests, not
/// authority admission. No method on this type can write live control state.
pub struct VerifiedReplayProof {
    proof: HistoricalReplayProofV1,
}

impl VerifiedReplayProof {
    pub fn proof(&self) -> &HistoricalReplayProofV1 {
        &self.proof
    }

    pub fn verify_manifest(&self, manifest: &SnapshotManifestV1) -> Result<()> {
        verify_manifest_against_history(&self.proof, manifest)
    }

    /// This is the only replay publication path. No epoch, lease, or
    /// canonical config is installed during download. Storage binds the
    /// signed proof and all verified checkpoints again under its locks.
    pub fn publish(&self, storage: &Storage) -> Result<()> {
        let manifests = storage.list_snapshots(self.proof.world_id)?;
        if manifests.len() != self.proof.checkpoints.len() {
            bail!("historical replay has not downloaded every committed checkpoint");
        }
        for manifest in &manifests {
            self.verify_manifest(manifest)?;
        }
        storage.publish_verified_historical_replay(&self.proof)?;
        Ok(())
    }
}

/// Individual manifest verification is also used for an offline *copy*.
/// Only the envelope verifier below can construct VerifiedReplayProof.
fn verify_manifest_against_history(proof: &HistoricalReplayProofV1, manifest: &SnapshotManifestV1) -> Result<()> {
    let index = usize::try_from(manifest.snapshot_number.checked_sub(1).context("zero snapshot number")?)?;
    let expected = proof.checkpoints.get(index).context("checkpoint outside certified replay proof")?;
    let epoch =
        proof.epochs.get(usize::try_from(expected.epoch - 1)?).context("checkpoint references unknown epoch")?;
    let parent = index.checked_sub(1).map(|prior| proof.checkpoints[prior].manifest_hash);
    verify_snapshot_signature(manifest)?;
    if manifest.world_id != proof.world_id
        || HistoricalCheckpointV1::from_manifest(manifest)? != *expected
        || manifest.previous_snapshot_hash != parent
        || manifest.authority_peer_id != epoch.authority_peer_id
        || manifest.authority_public_key != epoch.authority_public_key
    {
        bail!("signed manifest does not match the certified replay position");
    }
    Ok(())
}

pub fn load_staged_replay(storage: &Storage, world: WorldId) -> Result<Option<VerifiedReplayProof>> {
    let Some(staged) = storage.load_first_contact_replay(world)? else {
        return Ok(None);
    };
    Ok(Some(verify_first_contact_replay(storage, &staged.admission, staged.proof)?))
}

/// A later authority's envelope must extend the exact locally signed
/// admission, not merely prove an arbitrary branch from the same genesis.
/// Do not install the later membership or epoch during download.
fn verify_first_contact_replay(
    storage: &Storage,
    admission: &MembershipRecordV1,
    proof: HistoricalReplayProofV1,
) -> Result<VerifiedReplayProof> {
    let world = admission.world_id;
    verify_membership_signature(admission)?;
    let target = proof.membership.last().context("first-contact replay lacks final membership")?.clone();
    if proof.world_id != world || proof.membership.get(usize::try_from(admission.sequence)?) != Some(admission) {
        bail!("first-contact replay does not extend the exact signed admission");
    }
    let verified = verify_replay_proof(proof, &storage.load_world(world)?.genesis, &target)?;
    for certificate in storage.load_membership_certificate_chain(world)? {
        if !verified.proof.membership_certificates.contains(&certificate) {
            bail!("first-contact replay replaces accepted membership evidence");
        }
    }
    Ok(verified)
}

pub fn verify_and_stage_first_contact_replay(
    storage: &Storage,
    local: &PeerIdentity,
    source: swarm_protocol::PeerId,
    admission_hash: swarm_protocol::Hash32,
    proof: HistoricalReplayProofV1,
) -> Result<()> {
    let world = proof.world_id;
    let admission = storage.load_membership_record(world)?;
    let target = proof.membership.last().context("first-contact replay lacks target membership")?;
    if admission.record_hash()? != admission_hash
        || target.authority_peer_id != source
        || source == local.peer_id()
        || !admission.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
        || !target.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
    {
        bail!("first-contact replay is not addressed to the admitted local member");
    }
    let verified = verify_first_contact_replay(storage, &admission, proof)?;
    for manifest in storage.list_snapshots(world)? {
        verified.verify_manifest(&manifest)?;
        storage.verify_snapshot(&manifest)?;
    }
    let bytes = postcard::to_allocvec(verified.proof())?;
    storage.stage_historical_replay_proof(world, admission_hash, &bytes)?;
    Ok(())
}

/// A locally committed outbound transfer already denies old-generation
/// leases and hosting. Planned retirement requires this exact local fence;
/// crash retirement instead uses a certified, durable replay envelope.
pub fn local_committed_retirement(
    storage: &Storage,
    local: &PeerIdentity,
    epoch: &EpochRecordV1,
) -> Result<Option<AuthorityTransferV1>> {
    if epoch.authority_peer_id != local.peer_id() || epoch.authority_public_key != local.public_key() {
        return Ok(None);
    }
    let transfer = match storage.load_transfer_record(epoch.world_id) {
        Ok(record) => record,
        Err(swarm_storage::StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    verify_transfer_signature(&transfer)?;
    if transfer.phase != TransferPhase::Committed
        || transfer.from_peer_id != local.peer_id()
        || transfer.signer_peer_id != local.peer_id()
        || transfer.signer_public_key != local.public_key()
        || epoch.epoch_number.checked_add(1) != Some(transfer.next_epoch)
        || epoch.fencing_token.checked_add(1) != Some(transfer.next_fencing_token)
    {
        return Ok(None);
    }
    Ok(Some(transfer))
}

fn replay_retirement<'a>(
    anchor: &StaleReplayAnchorV1,
    proof: &'a HistoricalReplayProofV1,
) -> Result<&'a HistoricalTransitionV1> {
    let index = usize::try_from(anchor.epoch.epoch_number.checked_sub(1).context("invalid retired epoch")?)?;
    proof.transitions.get(index).context("retired authority requires its exact immediate certified transition")
}

/// Data-only admission; the verified proof must include the local signed
/// retirement decision before a former authority can download its suffix.
pub fn stale_replay_recipient_allowed(
    storage: &Storage,
    local: &PeerIdentity,
    anchor: &StaleReplayAnchorV1,
    proof: &HistoricalReplayProofV1,
) -> Result<bool> {
    if anchor.epoch.authority_peer_id != local.peer_id() {
        return Ok(true);
    }
    match replay_retirement(anchor, proof)? {
        HistoricalTransitionV1::Transfer { committed, .. } => {
            Ok(local_committed_retirement(storage, local, &anchor.epoch)?.as_ref() == Some(committed.as_ref()))
        }
        HistoricalTransitionV1::Recovery(_) => {
            if !storage.crash_retirement_pending(anchor.epoch.world_id)? {
                return Ok(false);
            }
            Ok(storage
                .load_staged_stale_replay_proof(anchor.epoch.world_id)?
                .is_some_and(|(pinned_anchor, pinned)| pinned_anchor == *anchor && pinned == *proof))
        }
        HistoricalTransitionV1::SoleMemberWake(_) => Ok(false),
    }
}

/// Verify an authenticated newer authority's complete history against the
/// exact already accepted local generation. A valid genesis signature by
/// itself is insufficient: a signed alternate branch is never a catch-up.
/// The result remains data-only until a separate fenced replay transaction.
pub fn verify_and_stage_stale_replay(
    storage: &Storage,
    local: &PeerIdentity,
    source: swarm_protocol::PeerId,
    anchor: &StaleReplayAnchorV1,
    proof: HistoricalReplayProofV1,
) -> Result<()> {
    let world = anchor.epoch.world_id;
    let prior_crash = if storage.crash_retirement_pending(world)? {
        Some(storage.load_staged_stale_replay_proof(world)?.context("missing frozen retirement envelope")?)
    } else {
        None
    };
    let current_anchor = match &prior_crash {
        Some((pinned, _)) => pinned.clone(),
        None => storage.stale_replay_anchor(world)?,
    };
    if current_anchor != *anchor
        || source == local.peer_id()
        || !anchor.membership.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
        || !anchor.is_extended_by(&proof)
    {
        bail!("stale replay is not an exact successor of an accepted replica");
    }
    let target = proof.membership.last().context("stale replay lacks final membership")?.clone();
    if target.authority_peer_id != source
        || !target.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
    {
        bail!("stale replay source or recipient is not bound to final signed membership");
    }
    // A newly admitted authority need not be present in the old anchor.
    // The full verifier authenticates every intervening joint membership
    // change, eligible successor and transfer/recovery certificate before
    // its envelope may be pinned. An arbitrary authenticated peer cannot
    // substitute a roster or a signed alternate history.
    let verified = verify_replay_proof(proof, &storage.load_world(world)?.genesis, &target)?;
    let crash_retirement = anchor.epoch.authority_peer_id == local.peer_id()
        && matches!(replay_retirement(anchor, verified.proof())?, HistoricalTransitionV1::Recovery(_));
    if let Some((_, previous)) = prior_crash {
        if !crash_retirement || !verified.proof().extends_replay_history(&previous) {
            bail!("new recovery envelope does not extend the frozen retirement history");
        }
        let _runtime_exclusion =
            crate::migration::acquire_retired_replay_guard(&DataPaths::from_root(storage.root()), world)?;
        storage.stage_verified_crash_retirement_replay(anchor, verified.proof())?;
        return Ok(());
    }
    if !crash_retirement && !stale_replay_recipient_allowed(storage, local, anchor, verified.proof())? {
        bail!("retired authority has no matching locally committed transfer fence");
    }
    // Preserve every locally retained canonical manifest, not just the
    // endpoint. Each signature and the underlying bytes must verify.
    let manifests = storage.list_snapshots(world)?;
    if manifests.last().map(|manifest| manifest.manifest_hash()).transpose()? != Some(anchor.head.manifest_hash) {
        bail!("local retained history does not reach the exact stale replay anchor");
    }
    for manifest in &manifests {
        verified.verify_manifest(manifest)?;
        storage.verify_snapshot(manifest)?;
    }
    for certificate in storage.load_membership_certificate_chain(world)? {
        if !verified.proof.membership_certificates.contains(&certificate) {
            bail!("stale replay replaces locally accepted joint membership evidence");
        }
    }
    if anchor.epoch.mode == EpochMode::Recovery {
        let index = usize::try_from(anchor.epoch.epoch_number.checked_sub(2).context("invalid recovery generation")?)?;
        let Some(HistoricalTransitionV1::Recovery(certificate)) = verified.proof.transitions.get(index) else {
            bail!("stale replay omits the local accepted recovery certificate");
        };
        if storage.load_recovery_certificate(world)? != **certificate {
            bail!("stale replay changes the local accepted recovery certificate");
        }
    }
    if crash_retirement {
        // Never freeze a running or ambiguous local Minecraft process. A
        // certified successor is not itself permission to discard local data.
        // Holding the same runtime slot as every host launch closes the race
        // between quiescence, permit revocation and installing the fence.
        let _runtime_exclusion =
            crate::migration::acquire_retired_replay_guard(&DataPaths::from_root(storage.root()), world)?;
        storage.stage_verified_crash_retirement_replay(anchor, verified.proof())?;
    } else {
        storage.stage_stale_replay_proof(anchor, verified.proof())?;
    }
    Ok(())
}

/// Reverify the entire candidate, including on restart after a durable
/// publication intent. The anchored local identity must remain a member,
/// and this path cannot make that identity the authority.
pub fn publish_stale_replay(
    storage: &Storage,
    local: &PeerIdentity,
    world: WorldId,
) -> Result<HistoricalReplayProofV1> {
    let candidate =
        storage.stale_publication_candidate(world)?.context("stale replay is incomplete or no longer anchored")?;
    let proof = &candidate.proof;
    let anchor = &candidate.anchor;
    let target = proof.membership.last().context("stale replay lacks target membership")?;
    if !anchor.is_extended_by(proof)
        || target.authority_peer_id == local.peer_id()
        || !anchor.membership.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
        || !target.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
    {
        bail!("stale publication does not bind an accepted replica to a certified successor");
    }
    let verified = verify_replay_proof(proof.clone(), &storage.load_world(world)?.genesis, target)?;
    for manifest in &candidate.manifests {
        verified.verify_manifest(manifest)?;
    }
    // These local signed records are not necessarily included in the
    // replay envelope. Structural binding alone cannot authorize removal.
    if let Some(record) = &candidate.sleep {
        swarm_core::lifecycle::verify_sleep_record_signature(record)?;
    }
    if let Some(record) = &candidate.reservation {
        swarm_core::verify_lease_signature(record)?;
    }
    let retired = anchor.epoch.authority_peer_id == local.peer_id();
    let _runtime_exclusion = if retired {
        Some(crate::migration::acquire_retired_replay_guard(&DataPaths::from_root(storage.root()), world)?)
    } else {
        None
    };
    if retired {
        if !storage.stale_publication_pending(world)?
            && !stale_replay_recipient_allowed(storage, local, anchor, verified.proof())?
        {
            bail!("retired authority has no matching durable retirement fence");
        }
        match replay_retirement(anchor, verified.proof())? {
            HistoricalTransitionV1::Transfer { committed, .. } => {
                storage.begin_verified_retired_stale_publication(&candidate, committed)?;
            }
            HistoricalTransitionV1::Recovery(_) => {
                storage.begin_verified_crash_retirement_publication(&candidate)?;
            }
            HistoricalTransitionV1::SoleMemberWake(_) => bail!("sole-member wake cannot retire a local authority"),
        }
    } else {
        storage.begin_verified_stale_publication(&candidate)?;
    }
    storage.finish_verified_stale_publication(&candidate)?;
    Ok(verified.proof)
}

/// Strictly data-only proof validation. The expected membership must be the
/// receiver's already committed signed join configuration, not an arbitrary
/// roster supplied by the sender. Currentness still requires a fresh quorum.
pub fn verify_replay_proof(
    proof: HistoricalReplayProofV1,
    genesis: &WorldGenesisV1,
    expected_membership: &MembershipRecordV1,
) -> Result<VerifiedReplayProof> {
    validate_replay_shape(&proof, genesis, expected_membership)?;
    let final_epoch = proof.epochs.last().context("missing final epoch")?;
    verify_signature(
        final_epoch.authority_peer_id,
        final_epoch.authority_public_key,
        &proof.signing_bytes()?,
        &proof.signature,
    )?;
    verify_replay_history(&proof, genesis, expected_membership)?;
    Ok(VerifiedReplayProof { proof })
}

fn validate_replay_shape(
    proof: &HistoricalReplayProofV1,
    genesis: &WorldGenesisV1,
    expected_membership: &MembershipRecordV1,
) -> Result<()> {
    if proof.protocol_version != PROTOCOL_VERSION
        || proof.genesis != *genesis
        || genesis.world_id()? != proof.world_id
        || proof.epochs.is_empty() || proof.epochs.len() > MAX_REPLAY_EPOCHS
        || proof.membership.is_empty() || proof.membership.len() > MAX_REPLAY_MEMBERSHIP_RECORDS
        || proof.membership_certificates.len() >= MAX_REPLAY_MEMBERSHIP_RECORDS
        || proof.configs.is_empty() || proof.configs.len() > MAX_REPLAY_CONFIGS
        || proof.checkpoints.is_empty() || proof.checkpoints.len() > MAX_REPLAY_CHECKPOINTS
        || proof.transitions.len() != proof.epochs.len() - 1
        || proof.membership.last() != Some(expected_membership)
        || proof.membership.iter().any(|record| record.members.is_empty() || record.members.len() > 1024)
        || proof.membership_certificates.iter().any(|certificate| {
            certificate.proposal.previous.members.len() > 1024
                || certificate.proposal.proposed.members.len() > 1024 || certificate.votes.len() > 1024
        })
        || proof.transitions.iter().any(|transition| {
            matches!(transition, HistoricalTransitionV1::Recovery(certificate) if certificate.votes.len() > 1024)
        })
    {
        bail!("historical replay proof is oversized, incomplete or not bound to committed membership");
    }
    if postcard::to_allocvec(&proof)?.len() > MAX_REPLAY_PROOF_BYTES {
        bail!("historical replay proof exceeds encoded byte limit");
    }
    Ok(())
}

/// Common signed record/transition checks. This function returns no replay
/// capability. Network publication additionally requires the final authority's
/// signed envelope; offline copy verifies every original manifest instead.
fn verify_replay_history(
    proof: &HistoricalReplayProofV1,
    genesis: &WorldGenesisV1,
    expected_membership: &MembershipRecordV1,
) -> Result<()> {
    let creator = peer_id_from_public_key(&genesis.creator_public_key);
    let first = &proof.epochs[0];
    if first.authority_peer_id != creator
        || first.authority_public_key != genesis.creator_public_key
        || first.mode != EpochMode::Quorum
        || genesis.initial_membership != [creator]
    {
        bail!("historical replay does not start at supported signed genesis");
    }
    let final_epoch = proof.epochs.last().context("missing final epoch")?;
    for (index, epoch) in proof.epochs.iter().enumerate() {
        let generation = index as u64 + 1;
        let previous_hash = index.checked_sub(1).map(|prior| proof.epochs[prior].record_hash()).transpose()?;
        epoch.validate_semantics()?;
        if epoch.world_id != proof.world_id
            || epoch.epoch_number != generation
            || epoch.fencing_token != generation
            || epoch.previous_epoch_hash != previous_hash
            || epoch.mode == EpochMode::Solo
        {
            bail!("unsupported or nonconsecutive historical authority generation");
        }
        verify_signature(
            epoch.authority_peer_id,
            epoch.authority_public_key,
            &epoch.signing_bytes()?,
            &epoch.signature,
        )?;
    }
    // Verify the exact full membership hash chain, including same-voter
    // authority promotions that the shorter discovery proof can omit.
    let mut certificates = BTreeMap::new();
    for certificate in &proof.membership_certificates {
        if certificates.insert(certificate.proposal.proposed.sequence, certificate).is_some() {
            bail!("duplicate historical membership certificate");
        }
    }
    let mut last_memberships = BTreeMap::new();
    for (index, record) in proof.membership.iter().enumerate() {
        let epoch = proof
            .epochs
            .get(usize::try_from(record.epoch.checked_sub(1).context("zero membership epoch")?)?)
            .context("membership references unavailable historical epoch")?;
        verify_membership_signature(record)?;
        if record.world_id != proof.world_id
            || record.sequence != index as u64
            || record.authority_peer_id != epoch.authority_peer_id
            || record.authority_public_key != epoch.authority_public_key
            || record.members.iter().any(|member| peer_id_from_public_key(&member.public_key) != member.peer_id)
            || !record.members.iter().any(|member| {
                !member.banned
                    && member.authority_eligible
                    && member.peer_id == epoch.authority_peer_id
                    && member.public_key == epoch.authority_public_key
            })
        {
            bail!("historical membership does not identify its exact epoch authority");
        }
        if index == 0 {
            if record.epoch != 1
                || record.previous_membership_hash.is_some()
                || record.members.len() != 1
                || record.members[0].peer_id != creator
            {
                bail!("historical membership is not anchored at genesis");
            }
        } else {
            let prior = &proof.membership[index - 1];
            if record.previous_membership_hash != Some(prior.record_hash()?)
                || (record.epoch != prior.epoch && prior.epoch.checked_add(1) != Some(record.epoch))
                || (record.epoch != prior.epoch && record.members != prior.members)
            {
                bail!("historical membership skipped a signed predecessor or changed voters during promotion");
            }
            if let Some(certificate) = certificates.remove(&record.sequence) {
                if certificate.proposal.previous != *prior || certificate.proposal.proposed != *record {
                    bail!("historical membership certificate is not the exact transition");
                }
                validate_membership_certificate_shape(certificate)?;
                let mut seen = BTreeSet::new();
                for vote in &certificate.votes {
                    if !seen.insert(vote.voter_peer_id) {
                        bail!("repeated historical membership voter");
                    }
                    verify_signature(
                        vote.voter_peer_id,
                        vote.voter_public_key,
                        &vote.signing_bytes()?,
                        &vote.signature,
                    )?;
                }
            } else if record.members != prior.members {
                bail!("historical voter change lacks joint quorum evidence");
            }
        }
        last_memberships.insert(record.epoch, record);
    }
    if !certificates.is_empty()
        || last_memberships.len() != proof.epochs.len()
        || expected_membership.epoch != final_epoch.epoch_number
    {
        bail!("historical membership has unused certificates or missing authority generations");
    }
    // Config V1 has no epoch field. Each distinct authority change must be
    // reflected in its signed config chain in order; no retired authority
    // may append another config after its successor has signed.
    let mut authority_runs = Vec::new();
    for epoch in &proof.epochs {
        let authority = (epoch.authority_peer_id, epoch.authority_public_key);
        if authority_runs.last() != Some(&authority) {
            authority_runs.push(authority);
        }
    }
    let mut config_run = 0usize;
    for (index, config) in proof.configs.iter().enumerate() {
        verify_world_config_signature(config)?;
        let parent = index.checked_sub(1).map(|prior| proof.configs[prior].config_hash()).transpose()?;
        if config.world_id != proof.world_id
            || config.sequence != index as u64 + 1
            || config.previous_config_hash != parent
            || config.compatibility_fingerprint()? != genesis.compatibility_fingerprint
        {
            bail!("historical config does not extend the original signed configuration");
        }
        let signer = (config.authority_peer_id, config.authority_public_key);
        if index > 0 && authority_runs.get(config_run) != Some(&signer) {
            config_run += 1;
        }
        if authority_runs.get(config_run) != Some(&signer) {
            bail!("historical config signer is outside the ordered authority history");
        }
    }
    if config_run + 1 != authority_runs.len() {
        bail!("historical config is missing an authority refresh");
    }
    // The signed proof commits every manifest hash. Validate the metadata
    // shape now; the manifest's own signature and blobs are checked on replay.
    let mut current_epoch = 1u64;
    for (index, checkpoint) in proof.checkpoints.iter().enumerate() {
        if checkpoint.snapshot_number != index as u64 + 1
            || checkpoint.sequence != index as u64 + 1
            || checkpoint.manifest_hash == swarm_protocol::Hash32::default()
        {
            bail!("historical checkpoint sequence is not complete");
        }
        if index == 0 {
            if checkpoint.epoch != 1 || checkpoint.state_root != first.base_state_hash {
                bail!("genesis checkpoint does not match its epoch base");
            }
        } else if checkpoint.epoch != current_epoch {
            if current_epoch.checked_add(1) != Some(checkpoint.epoch) {
                bail!("checkpoint crosses a missing or regressed authority generation");
            }
            let previous = &proof.epochs[usize::try_from(current_epoch - 1)?];
            let next = proof.epochs.get(usize::try_from(current_epoch)?).context("checkpoint crosses unknown epoch")?;
            let boundary = &proof.checkpoints[index - 1];
            if checkpoint.state_root != boundary.state_root || next.base_state_hash != boundary.state_root {
                bail!("authority promotion changed the canonical checkpoint state");
            }
            verify_transition(
                previous,
                next,
                boundary,
                last_memberships[&current_epoch],
                &proof.transitions[usize::try_from(current_epoch - 1)?],
            )?;
            current_epoch = checkpoint.epoch;
        }
    }
    if current_epoch != final_epoch.epoch_number {
        bail!("historical checkpoint head has not reached the final authority generation");
    }
    Ok(())
}

fn verify_transition(
    previous: &EpochRecordV1,
    next: &EpochRecordV1,
    boundary: &HistoricalCheckpointV1,
    membership: &MembershipRecordV1,
    evidence: &HistoricalTransitionV1,
) -> Result<()> {
    let target = membership
        .members
        .iter()
        .find(|member| member.peer_id == next.authority_peer_id)
        .context("historical successor was not an active eligible member before promotion")?;
    if target.banned || !target.authority_eligible || target.public_key != next.authority_public_key {
        bail!("historical successor was not eligible in the predecessor membership");
    }
    match evidence {
        HistoricalTransitionV1::Transfer { prepared, accepted, committed } => {
            if next.mode != EpochMode::Quorum || previous.authority_peer_id == next.authority_peer_id {
                bail!("historical transfer does not change authority into quorum mode");
            }
            for (record, phase, signer, key) in [
                (prepared.as_ref(), TransferPhase::Prepared, previous.authority_peer_id, previous.authority_public_key),
                (accepted.as_ref(), TransferPhase::Accepted, next.authority_peer_id, next.authority_public_key),
                (
                    committed.as_ref(),
                    TransferPhase::Committed,
                    previous.authority_peer_id,
                    previous.authority_public_key,
                ),
            ] {
                if record.world_id != next.world_id
                    || record.phase != phase
                    || record.from_peer_id != previous.authority_peer_id
                    || record.to_peer_id != next.authority_peer_id
                    || record.base_snapshot_hash != boundary.manifest_hash
                    || record.next_epoch != next.epoch_number
                    || record.next_fencing_token != next.fencing_token
                    || record.signer_peer_id != signer
                    || record.signer_public_key != key
                {
                    bail!("historical transfer phase does not identify the exact signed boundary");
                }
                verify_transfer_signature(record)?;
            }
        }
        HistoricalTransitionV1::Recovery(certificate) => {
            let ballot = &certificate.ballot;
            if next.mode != EpochMode::Recovery
                || ballot.world_id != next.world_id
                || ballot.base_epoch != previous.epoch_number
                || ballot.base_fencing_token != previous.fencing_token
                || ballot.target_epoch != next.epoch_number
                || ballot.target_fencing_token != next.fencing_token
                || ballot.candidate_peer_id != next.authority_peer_id
                || ballot.candidate_public_key != next.authority_public_key
                || ballot.base_snapshot_hash != boundary.manifest_hash
                || ballot.base_state_hash != boundary.state_root
                || ballot.membership_hash != membership.record_hash()?
            {
                bail!("historical recovery ballot does not match the exact canonical predecessor");
            }
            verify_recovery_ballot_signature(ballot)?;
            let voters = membership
                .members
                .iter()
                .filter(|member| !member.banned)
                .map(|member| member.peer_id)
                .collect::<Vec<_>>();
            validate_recovery_certificate_shape(certificate, &voters)?;
            let mut seen = BTreeSet::new();
            for vote in &certificate.votes {
                if !seen.insert(vote.voter_peer_id)
                    || !membership.members.iter().any(|member| {
                        !member.banned
                            && member.peer_id == vote.voter_peer_id
                            && member.public_key == vote.voter_public_key
                    })
                {
                    bail!("historical recovery contains a duplicate or noncanonical voter");
                }
                verify_recovery_vote_signature(vote)?;
            }
        }
        HistoricalTransitionV1::SoleMemberWake(record) => {
            if previous.mode != EpochMode::Quorum
                || next.mode != EpochMode::Quorum
                || previous.authority_peer_id != next.authority_peer_id
                || previous.authority_public_key != next.authority_public_key
                || membership.members.iter().filter(|member| !member.banned).count() != 1
                || record.world_id != next.world_id
                || record.epoch != previous.epoch_number
                || record.fencing_token != previous.fencing_token
                || record.latest_snapshot_hash != boundary.manifest_hash
                || record.authority_peer_id != previous.authority_peer_id
                || record.authority_public_key != previous.authority_public_key
            {
                bail!("historical direct wake is not a signed sole-member sleep boundary");
            }
            record.validate_semantics()?;
            verify_signature(
                record.authority_peer_id,
                record.authority_public_key,
                &record.signing_bytes()?,
                &record.signature,
            )?;
        }
    }
    Ok(())
}

/// Only a local accepted authority can construct this bounded data
/// commitment. Refuse incomplete/pruned histories, concurrent generation
/// changes, and any missing historical transition evidence.
pub fn collect_replay_proof(
    storage: &Storage,
    identity: &PeerIdentity,
    world: WorldId,
) -> Result<HistoricalReplayProofV1> {
    let accepted = storage.load_epoch_record(world)?;
    if accepted.authority_peer_id != identity.peer_id() || accepted.authority_public_key != identity.public_key() {
        bail!("only the local accepted authority can sign a historical replay proof");
    }
    let (mut proof, manifests) = collect_local_history(storage, world)?;
    let final_epoch = proof.epochs.last().context("missing current authority")?;
    if final_epoch.authority_peer_id != identity.peer_id() || final_epoch.authority_public_key != identity.public_key()
    {
        bail!("accepted authority changed during historical replay collection");
    }
    proof.signature = identity.sign(&proof.signing_bytes()?);
    let membership = proof.membership.last().context("missing current membership")?.clone();
    let genesis = proof.genesis.clone();
    let verified = verify_replay_proof(proof, &genesis, &membership)?;
    for manifest in &manifests {
        verified.verify_manifest(manifest)?;
    }
    let proof = verified.proof;
    ensure_collected_history_current(storage, &proof, &manifests)?;
    Ok(proof)
}

/// Local, read-only governance audit. It verifies the complete retained
/// canonical history, including joint voter/role changes and authority
/// transitions. It deliberately returns no authority capability or tokens.
pub fn review_world_governance(storage: &Storage, local: &PeerIdentity, world: WorldId) -> Result<serde_json::Value> {
    use serde_json::json;
    use swarm_protocol::WorldMemberV1;
    use swarm_storage::StorageError;

    let (proof, manifests) = collect_local_history(storage, world)?;
    let current = proof.membership.last().context("missing current membership")?;
    let epoch = proof.epochs.last().context("missing current epoch")?;
    let config = proof.configs.last().context("missing current config")?;
    let descriptor = storage.load_world_descriptor(world)?;
    let local_member = current
        .members
        .iter()
        .find(|member| member.peer_id == local.peer_id() && member.public_key == local.public_key() && !member.banned)
        .context("governance history is available only to a locally accepted active member")?;
    if descriptor.members != current.members
        || descriptor.compatibility_fingerprint != proof.genesis.compatibility_fingerprint
        || storage.first_contact_replay_pinned(world)?
        || storage.crash_retirement_pending(world)?
        || !storage.list_solo_conflicts(world)?.is_empty()
    {
        bail!("governance history is not synchronized or has unresolved conflicts");
    }
    validate_replay_shape(&proof, &proof.genesis, current)?;
    verify_replay_history(&proof, &proof.genesis, current)?;
    for manifest in &manifests {
        verify_manifest_against_history(&proof, manifest)?;
    }

    let pending = match storage.load_membership_promise(world) {
        Ok(promise) => {
            let proposal = &promise.proposal;
            swarm_consensus::validate_membership_proposal_shape(proposal)?;
            verify_membership_signature(&proposal.previous)?;
            verify_membership_signature(&proposal.proposed)?;
            if !promise.vote.matches_proposal(proposal)? {
                bail!("pending membership vote does not match its proposal");
            }
            verify_signature(
                promise.vote.voter_peer_id,
                promise.vote.voter_public_key,
                &promise.vote.signing_bytes()?,
                &promise.vote.signature,
            )?;
            let state = if &proposal.previous == current {
                "staged_not_committed"
            } else if &proposal.proposed == current {
                "committed_cleanup_pending"
            } else {
                bail!("pending membership does not match canonical history");
            };
            Some(json!({
                "state": state,
                "proposal_hash": proposal.proposal_hash()?.to_string(),
                "proposed_sequence": proposal.proposed.sequence,
            }))
        }
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    fn role(member: &WorldMemberV1) -> serde_json::Value {
        json!({
            "active_member": !member.banned,
            "voter": !member.banned,
            "replica_member": !member.banned,
            "host_candidate": !member.banned && member.authority_eligible,
            "banned": member.banned,
        })
    }
    let mut membership_history = Vec::new();
    let mut previous_members = BTreeMap::<_, &WorldMemberV1>::new();
    for record in &proof.membership {
        let next = record.members.iter().map(|member| (member.peer_id, member)).collect::<BTreeMap<_, _>>();
        let peer_ids = previous_members.keys().chain(next.keys()).copied().collect::<BTreeSet<_>>();
        let changes = peer_ids
            .into_iter()
            .filter_map(|peer| {
                let old = previous_members.get(&peer);
                let new = next.get(&peer);
                (old != new).then(|| {
                    json!({
                        "peer_id": peer.to_string(),
                        "before": old.map(|member| role(member)),
                        "after": new.map(|member| role(member)),
                    })
                })
            })
            .collect::<Vec<_>>();
        let certificate = proof.membership_certificates.iter().find(|cert| cert.proposal.proposed == *record);
        membership_history.push(json!({
            "sequence": record.sequence,
            "epoch": record.epoch,
            "record_hash": record.record_hash()?.to_string(),
            "previous_hash": record.previous_membership_hash.map(|hash| hash.to_string()),
            "signed_by": record.authority_peer_id.to_string(),
            "changes": changes,
            "joint_quorum_voters": certificate.map(|cert| cert.votes.iter()
                .map(|vote| vote.voter_peer_id.to_string()).collect::<Vec<_>>()),
        }));
        previous_members = next;
    }
    let mut configs = Vec::new();
    let mut prior: Option<&WorldConfigV1> = None;
    for record in &proof.configs {
        configs.push(json!({
            "sequence": record.sequence,
            "record_hash": record.config_hash()?.to_string(),
            "previous_hash": record.previous_config_hash.map(|hash| hash.to_string()),
            "signed_by": record.authority_peer_id.to_string(),
            "visibility": format!("{:?}", record.visibility).to_lowercase(),
            "membership_policy": format!("{:?}", record.membership_policy),
            "presentation_changed": prior.is_some_and(|old| old.presentation != record.presentation),
            "authority_policy_changed": prior.is_some_and(|old| old.authority_policy != record.authority_policy),
        }));
        prior = Some(record);
    }
    let mut epochs = Vec::new();
    for record in &proof.epochs {
        epochs.push(json!({
            "epoch": record.epoch_number,
            "fence": record.fencing_token,
            "record_hash": record.record_hash()?.to_string(),
            "authority": record.authority_peer_id.to_string(),
            "mode": format!("{:?}", record.mode),
        }));
    }
    ensure_collected_history_current(storage, &proof, &manifests)?;
    if storage.load_world_descriptor(world)? != descriptor
        || storage.first_contact_replay_pinned(world)?
        || storage.crash_retirement_pending(world)?
    {
        bail!("governance state changed during inspection");
    }
    Ok(json!({
        "schema_version": 1,
        "world_id": world.to_string(),
        "evidence": "verified_local_canonical_history_not_live_authority",
        "signed_visibility": format!("{:?}", config.visibility).to_lowercase(),
        "locally_accepted_authority": epoch.authority_peer_id == local.peer_id(),
        "local_role": role(local_member),
        "locally_paused": storage.discovery_listing_paused(world)?,
        "pending_membership": pending,
        "config_history": configs,
        "membership_history": membership_history,
        "authority_history": epochs,
    }))
}

/// Assemble exact retained signed records without claiming a final-authority
/// envelope. This private helper is shared by the authority-only signer and
/// the offline, new-identity recovery-copy verifier.
fn collect_local_history(
    storage: &Storage,
    world: WorldId,
) -> Result<(HistoricalReplayProofV1, Vec<SnapshotManifestV1>)> {
    let genesis = storage.load_world(world)?.genesis;
    let epochs = collect_signed_epoch_history(storage, world)?;
    let current_membership = storage.load_membership_record(world)?;
    let mut membership = Vec::new();
    let mut record = current_membership.clone();
    loop {
        if membership.len() >= MAX_REPLAY_MEMBERSHIP_RECORDS {
            bail!("historical membership exceeds replay limit");
        }
        let previous = record.previous_membership_hash;
        membership.push(record);
        let Some(hash) = previous else { break };
        record = storage.load_membership_by_hash(world, hash).context("historical membership predecessor missing")?;
    }
    membership.reverse();
    let configs = collect_signed_config_history(storage, world)?;
    let manifests = storage.list_snapshots(world)?;
    if manifests.is_empty() || manifests.len() > MAX_REPLAY_CHECKPOINTS {
        bail!("historical checkpoint count exceeds supported bounds");
    }
    let checkpoints = manifests.iter().map(HistoricalCheckpointV1::from_manifest).collect::<Result<Vec<_>, _>>()?;
    let mut transitions = Vec::new();
    for pair in epochs.windows(2) {
        let (previous, next) = (&pair[0], &pair[1]);
        let boundary = checkpoints
            .iter()
            .rev()
            .find(|checkpoint| checkpoint.epoch == previous.epoch_number)
            .context("previous generation's final checkpoint is missing")?;
        let evidence = if next.mode == EpochMode::Recovery {
            HistoricalTransitionV1::Recovery(Box::new(
                storage.load_recovery_certificate_for_epoch(world, next.record_hash()?)?,
            ))
        } else if previous.authority_peer_id == next.authority_peer_id {
            HistoricalTransitionV1::SoleMemberWake(Box::new(
                storage.load_sleep_boundary(previous, boundary.manifest_hash)?,
            ))
        } else {
            let transcript = AuthorityTransferV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                from_peer_id: previous.authority_peer_id,
                to_peer_id: next.authority_peer_id,
                base_snapshot_hash: boundary.manifest_hash,
                next_epoch: next.epoch_number,
                next_fencing_token: next.fencing_token,
                phase: TransferPhase::Committed,
                signer_peer_id: previous.authority_peer_id,
                signer_public_key: previous.authority_public_key,
                signature: Vec::new(),
            };
            HistoricalTransitionV1::Transfer {
                prepared: Box::new(storage.load_transfer_phase(&transcript, TransferPhase::Prepared)?),
                accepted: Box::new(storage.load_transfer_phase(&transcript, TransferPhase::Accepted)?),
                committed: Box::new(storage.load_transfer_phase(&transcript, TransferPhase::Committed)?),
            }
        };
        transitions.push(evidence);
    }
    let proof = HistoricalReplayProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis: genesis.clone(),
        epochs,
        membership,
        membership_certificates: storage
            .load_membership_certificate_chain(world)?
            .into_iter()
            .filter(|certificate| certificate.proposal.proposed.sequence <= current_membership.sequence)
            .collect(),
        configs,
        checkpoints,
        transitions,
        signature: Vec::new(),
    };
    ensure_collected_history_current(storage, &proof, &manifests)?;
    Ok((proof, manifests))
}

fn ensure_collected_history_current(
    storage: &Storage,
    proof: &HistoricalReplayProofV1,
    manifests: &[SnapshotManifestV1],
) -> Result<()> {
    let world = proof.world_id;
    if storage.load_epoch_record(world)? != *proof.epochs.last().context("missing epoch")?
        || storage.load_membership_record(world)? != *proof.membership.last().context("missing membership")?
        || storage.load_world_config(world)? != *proof.configs.last().context("missing config")?
        || storage.latest_snapshot(world)?.as_ref() != manifests.last()
    {
        bail!("canonical history advanced during collection; retry");
    }
    Ok(())
}

/// Verified local data for creating a DIFFERENT world. No replay envelope or
/// publication method is exposed, and the old world is never advanced.
pub(crate) struct LocalRecoverySource {
    pub manifest: SnapshotManifestV1,
    pub config: WorldConfigV1,
    pub(crate) epoch: EpochRecordV1,
    pub(crate) membership: MembershipRecordV1,
    head: SnapshotManifestV1,
    conflicts: Option<swarm_protocol::Hash32>,
}

impl LocalRecoverySource {
    pub fn ensure_unchanged(&self, storage: &Storage) -> Result<()> {
        let world = self.manifest.world_id;
        let descriptor = storage.load_world_descriptor(world)?;
        if storage.load_epoch_record(world)? != self.epoch
            || storage.load_membership_record(world)? != self.membership
            || storage.load_world_config(world)? != self.config
            || storage.latest_snapshot(world)?.as_ref() != Some(&self.head)
            || storage.load_snapshot(world, self.manifest.snapshot_number)? != self.manifest
            || descriptor.members != self.membership.members
            || descriptor.compatibility_fingerprint != self.config.compatibility_fingerprint()?
            || storage.first_contact_replay_pinned(world)?
            || crate::recovery_review::conflict_digest(storage, world)? != self.conflicts
        {
            bail!("source world changed during recovery; inspect the exact checkpoint and conflict evidence and retry");
        }
        Ok(())
    }
}

/// Strict current-head verifier for authority-affecting review/retirement.
/// Do not relax this when extending the data-only recovery-copy interface.
pub(crate) fn verify_local_recovery_source(
    storage: &Storage,
    local: &PeerIdentity,
    world: WorldId,
    expected_checkpoint: swarm_protocol::Hash32,
) -> Result<LocalRecoverySource> {
    let source = verify_selected_recovery_source(storage, local, world, expected_checkpoint, None)?;
    if source.manifest != source.head {
        bail!("the explicitly selected checkpoint is not the current accepted head");
    }
    Ok(source)
}

/// Read-only verification of the whole retained certified history. Conflict
/// evidence is reviewed separately; this never certifies live authority.
pub(crate) fn verified_local_checkpoint_history(
    storage: &Storage,
    local: &PeerIdentity,
    world: WorldId,
) -> Result<(HistoricalReplayProofV1, Vec<SnapshotManifestV1>)> {
    let (proof, manifests) = collect_local_history(storage, world)?;
    let membership = proof.membership.last().context("missing accepted membership")?;
    if !membership
        .members
        .iter()
        .any(|member| !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key())
    {
        bail!("local identity is not an authorized retained-world member");
    }
    let descriptor = storage.load_world_descriptor(world)?;
    if descriptor.members != membership.members
        || descriptor.compatibility_fingerprint != proof.genesis.compatibility_fingerprint
        || storage.first_contact_replay_pinned(world)?
    {
        bail!("canonical history is not synchronized");
    }
    validate_replay_shape(&proof, &proof.genesis, membership)?;
    verify_replay_history(&proof, &proof.genesis, membership)?;
    for manifest in &manifests {
        verify_manifest_against_history(&proof, manifest)?;
    }
    ensure_collected_history_current(storage, &proof, &manifests)?;
    Ok((proof, manifests))
}

/// A data-only copy may use any exact retained checkpoint, never a snapshot
/// number supplied without its hash. A conflict requires explicit
/// acknowledgement of the exact reviewed evidence set.
pub(crate) fn verify_selected_recovery_source(
    storage: &Storage,
    local: &PeerIdentity,
    world: WorldId,
    expected_checkpoint: swarm_protocol::Hash32,
    acknowledged_conflicts: Option<swarm_protocol::Hash32>,
) -> Result<LocalRecoverySource> {
    let conflicts = crate::recovery_review::conflict_digest(storage, world)?;
    if conflicts != acknowledged_conflicts {
        bail!("conflict evidence changed or was not explicitly acknowledged; review recovery before copying");
    }
    let (proof, manifests) = verified_local_checkpoint_history(storage, local, world)?;
    let manifest = manifests
        .iter()
        .find(|manifest| manifest.manifest_hash().ok() == Some(expected_checkpoint))
        .context("exact selected checkpoint is not in the retained verified history")?
        .clone();
    storage.verify_snapshot(&manifest)?;
    let source = LocalRecoverySource {
        manifest,
        config: proof.configs.last().context("missing accepted config")?.clone(),
        epoch: proof.epochs.last().context("missing accepted epoch")?.clone(),
        membership: proof.membership.last().context("missing accepted membership")?.clone(),
        head: manifests.last().context("missing retained canonical checkpoint")?.clone(),
        conflicts,
    };
    source.ensure_unchanged(storage)?;
    Ok(source)
}

/// Collect the signed config hash chain independently. Full replay must
/// additionally bind each config signer to its exact authorized epoch.
pub fn collect_signed_config_history(storage: &Storage, world: WorldId) -> Result<Vec<WorldConfigV1>> {
    let metadata = storage.load_world(world)?;
    let creator = peer_id_from_public_key(&metadata.genesis.creator_public_key);
    let accepted = storage.load_world_config(world)?;
    let mut record = accepted.clone();
    let mut history = Vec::new();
    loop {
        if history.len() >= MAX_HISTORICAL_CONFIGS {
            bail!("signed config history exceeds the supported replay limit");
        }
        verify_world_config_signature(&record)?;
        if record.world_id != world || record.compatibility_fingerprint()? != metadata.genesis.compatibility_fingerprint
        {
            bail!("historical config does not match the original world compatibility");
        }
        let previous_hash = record.previous_config_hash;
        history.push(record);
        let Some(hash) = previous_hash else { break };
        let previous = storage
            .load_world_config_by_hash(world, hash)
            .context("required signed predecessor config is unavailable")?;
        if previous.sequence.checked_add(1) != Some(history.last().context("missing successor config")?.sequence) {
            bail!("historical config skips a predecessor sequence");
        }
        record = previous;
    }
    history.reverse();
    let first = history.first().context("empty signed config history")?;
    if first.sequence != 1
        || first.authority_peer_id != creator
        || first.authority_public_key != metadata.genesis.creator_public_key
    {
        bail!("config history does not begin at the creator-signed configuration");
    }
    if storage.load_world_config(world)? != accepted {
        bail!("accepted config changed while collecting signed history");
    }
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_core::{protocol_v2::sign_world_config, PeerIdentity};
    use swarm_protocol::{
        AuthorityPolicyV1, Hash32, MembershipCertificateV1, MembershipPolicyV1, MembershipProposalV1, MembershipVoteV1,
        RecoveryBallotV1, RecoveryCertificateV1, RecoveryVoteV1, RuntimeCompatibilityManifestV1, WorldGenesisV1,
        WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
    };
    use swarm_storage::{SnapshotContext, WorldMetadataV1};

    fn compatibility() -> RuntimeCompatibilityManifestV1 {
        RuntimeCompatibilityManifestV1 {
            minecraft_version: "26.1.2".into(),
            loader_id: "fabric".into(),
            loader_version: "0.19.3".into(),
            swarmcraft_protocol_version: PROTOCOL_VERSION,
            fabric_adapter_version: "test".into(),
            required_server_mods: Vec::new(),
            required_client_mods: Vec::new(),
            datapacks: Vec::new(),
        }
    }

    fn fixture() -> (tempfile::TempDir, Storage, PeerIdentity, EpochRecordV1) {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("data")).unwrap();
        let creator = PeerIdentity::from_secret_bytes([21; 32]);
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "26.1.2".into(),
            fabric_loader_version: "0.19.3".into(),
            compatibility_fingerprint: compatibility().fingerprint().unwrap(),
            creation_nonce: [8; 32],
            creator_public_key: creator.public_key(),
            initial_membership: vec![creator.peer_id()],
        };
        let world = genesis.world_id().unwrap();
        storage
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "historical replay".into(),
                world_id: world,
                genesis,
            })
            .unwrap();
        let source = temp.path().join("snapshot");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("level.dat"), b"first checkpoint").unwrap();
        let mut checkpoint = storage
            .snapshot_directory(
                &source,
                SnapshotContext {
                    world,
                    snapshot_number: 1,
                    epoch: 1,
                    sequence: 1,
                    previous_snapshot_hash: None,
                    authority_peer_id: creator.peer_id(),
                    authority_public_key: creator.public_key(),
                },
            )
            .unwrap();
        creator.sign_snapshot(&mut checkpoint).unwrap();
        storage.commit_snapshot(&checkpoint).unwrap();
        let mut epoch = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: 1,
            fencing_token: 1,
            previous_epoch_hash: None,
            base_state_hash: checkpoint.state_root,
            authority_peer_id: creator.peer_id(),
            authority_public_key: creator.public_key(),
            mode: EpochMode::Quorum,
            reason: "genesis".into(),
            signature: Vec::new(),
        };
        epoch.signature = creator.sign(&epoch.signing_bytes().unwrap());
        storage.save_epoch_record(&epoch).unwrap();
        (temp, storage, creator, epoch)
    }

    fn successor(first: &EpochRecordV1, identity: &PeerIdentity) -> EpochRecordV1 {
        let mut record = EpochRecordV1 {
            epoch_number: first.epoch_number + 1,
            fencing_token: first.fencing_token + 1,
            previous_epoch_hash: Some(first.record_hash().unwrap()),
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            reason: "historical successor".into(),
            signature: Vec::new(),
            ..first.clone()
        };
        record.signature = identity.sign(&record.signing_bytes().unwrap());
        record
    }

    #[test]
    fn signed_history_walks_exact_records_without_granting_authority() {
        let (temp, storage, creator, first) = fixture();
        let second = successor(&first, &creator);
        let third = successor(&second, &PeerIdentity::from_secret_bytes([22; 32]));
        storage.save_epoch_record(&second).unwrap();
        storage.save_epoch_record(&third).unwrap();
        drop(storage);
        let storage = Storage::open(temp.path().join("data")).unwrap();
        assert_eq!(collect_signed_epoch_history(&storage, first.world_id).unwrap(), vec![first, second, third.clone()]);
        assert_eq!(storage.load_epoch_record(third.world_id).unwrap(), third);
        // The collector deliberately does not grant authority to this
        // successor: no transfer/quorum evidence was provided.
    }

    #[test]
    fn missing_predecessor_and_skipped_fence_fail_closed() {
        let (_temp, storage, creator, first) = fixture();
        let mut next = successor(&first, &creator);
        next.fencing_token += 1;
        next.signature = creator.sign(&next.signing_bytes().unwrap());
        storage.save_epoch_record(&next).unwrap();
        assert!(collect_signed_epoch_history(&storage, first.world_id).is_err());

        let mut third = successor(&next, &creator);
        third.previous_epoch_hash = Some(Hash32([99; 32]));
        third.signature = creator.sign(&third.signing_bytes().unwrap());
        storage.save_epoch_record(&third).unwrap();
        assert!(collect_signed_epoch_history(&storage, first.world_id).is_err());
    }

    #[test]
    fn forged_successor_signature_fails_closed() {
        let (_temp, storage, creator, first) = fixture();
        let mut next = successor(&first, &creator);
        next.signature[0] ^= 1;
        storage.save_epoch_record(&next).unwrap();
        assert!(collect_signed_epoch_history(&storage, first.world_id).is_err());
    }

    fn signed_membership(
        epoch: &EpochRecordV1,
        authority: &PeerIdentity,
        members: Vec<WorldMemberV1>,
        previous: Option<&MembershipRecordV1>,
    ) -> MembershipRecordV1 {
        let mut members = members;
        members.sort_by_key(|member| member.peer_id);
        let mut record = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: epoch.world_id,
            epoch: epoch.epoch_number,
            sequence: previous.map_or(0, |prior| prior.sequence + 1),
            previous_membership_hash: previous.map(|prior| prior.record_hash().unwrap()),
            members,
            authority_peer_id: authority.peer_id(),
            authority_public_key: authority.public_key(),
            signature: Vec::new(),
        };
        authority.sign_membership(&mut record).unwrap();
        record
    }

    fn member(identity: &PeerIdentity) -> WorldMemberV1 {
        WorldMemberV1 {
            peer_id: identity.peer_id(),
            public_key: identity.public_key(),
            authority_eligible: true,
            banned: false,
        }
    }

    fn initial_config(storage: &Storage, creator: &PeerIdentity, world: WorldId) -> WorldConfigV1 {
        let mut config = WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            sequence: 1,
            previous_config_hash: None,
            compatibility: compatibility(),
            visibility: WorldVisibilityV1::Private,
            authority_policy: AuthorityPolicyV1 { allow_solo_advancement: false, preferred_replication_factor: 3 },
            membership_policy: MembershipPolicyV1::InviteOnly,
            presentation: WorldPresentationV1 {
                name: "historical proof".into(),
                description: String::new(),
                tags: Vec::new(),
                icon_hash: None,
                approximate_region: None,
            },
            authority_peer_id: creator.peer_id(),
            authority_public_key: creator.public_key(),
            signature: Vec::new(),
        };
        sign_world_config(creator, &mut config).unwrap();
        storage.save_world_config(&config).unwrap();
        config
    }

    #[test]
    fn v1_role_projection_keeps_nonhosting_voters_and_durable_joint_promises() {
        use crate::member_roles::{inspect_v1_roles, LocalMembershipRole};
        use swarm_consensus::{membership_vote_for, validate_membership_certificate_shape};
        use swarm_storage::MembershipPromiseResult;

        let (temp, storage, alice, epoch) = fixture();
        let world = epoch.world_id;
        let bob = PeerIdentity::from_secret_bytes([22; 32]);
        let outsider = PeerIdentity::from_secret_bytes([23; 32]);
        let genesis = storage.load_world(world).unwrap().genesis;
        let initial = signed_membership(&epoch, &alice, vec![member(&alice)], None);
        storage.save_membership_record(&initial).unwrap();
        initial_config(&storage, &alice, world);
        let mut replica = member(&bob);
        replica.authority_eligible = false;
        let accepted = signed_membership(&epoch, &alice, vec![member(&alice), replica], Some(&initial));
        let proposal = MembershipProposalV1 { previous: initial, proposed: accepted.clone() };
        let sign_vote = |proposal: &MembershipProposalV1, voter: &PeerIdentity| {
            let mut vote = membership_vote_for(proposal, voter.peer_id(), voter.public_key()).unwrap();
            vote.signature = voter.sign(&vote.signing_bytes().unwrap());
            vote
        };
        let cert =
            MembershipCertificateV1 { votes: vec![sign_vote(&proposal, &alice), sign_vote(&proposal, &bob)], proposal };
        validate_membership_certificate_shape(&cert).unwrap();
        storage.save_membership_certificate(&cert).unwrap();
        storage.save_membership_record(&accepted).unwrap();
        let descriptor = swarm_protocol::WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: genesis.compatibility_fingerprint,
            members: accepted.members.clone(),
            preferred_replication_factor: 3,
        };
        storage.save_world_descriptor(&descriptor).unwrap();

        let role = inspect_v1_roles(&storage, &bob, world).unwrap();
        assert_eq!(role.membership, LocalMembershipRole::Active);
        assert!(role.player_member && role.replica_member && role.voter_member);
        assert!(!role.host_candidate && !role.locally_accepted_authority);
        let creator = inspect_v1_roles(&storage, &alice, world).unwrap();
        assert!(creator.host_candidate && creator.locally_accepted_authority);
        let viewer = inspect_v1_roles(&storage, &outsider, world).unwrap();
        assert_eq!(viewer.membership, LocalMembershipRole::NotMember);
        assert!(!viewer.player_member && !viewer.replica_member && !viewer.voter_member);
        assert!(!viewer.host_candidate && !viewer.locally_accepted_authority);

        // Local consent is not signed membership, even with a nonzero heap.
        let paths = swarm_core::DataPaths::from_root(temp.path().join("data"));
        paths.ensure().unwrap();
        std::fs::write(paths.identity_key(), [22; 32]).unwrap();
        let consent_dir = paths.root.join("control").join(world.to_hex());
        std::fs::create_dir_all(&consent_dir).unwrap();
        let consent = crate::hosting_budget::HostingConsentV1 {
            schema_version: 1,
            world_id: world,
            peer_id: bob.peer_id(),
            enabled: true,
            heap_mib: 1024,
            revision: 1,
        };
        std::fs::write(consent_dir.join("hosting-consent.json"), serde_json::to_vec(&consent).unwrap()).unwrap();
        assert!(crate::hosting_budget::load_consent(&paths, world).unwrap().unwrap().enabled);
        assert_eq!(inspect_v1_roles(&storage, &bob, world).unwrap(), role);

        // An unsigned descriptor cannot promote a non-hosting member.
        let mut forged = descriptor.clone();
        forged.members.iter_mut().find(|m| m.peer_id == bob.peer_id()).unwrap().authority_eligible = true;
        storage.save_world_descriptor(&forged).unwrap();
        assert!(inspect_v1_roles(&storage, &bob, world).is_err());
        storage.save_world_descriptor(&descriptor).unwrap();

        let mut members = accepted.members.clone();
        members.push(member(&outsider));
        let pending = MembershipProposalV1 {
            previous: accepted.clone(),
            proposed: signed_membership(&epoch, &alice, members, Some(&accepted)),
        };
        let vote = sign_vote(&pending, &bob);
        assert_eq!(storage.promise_membership_proposal(&pending, &vote).unwrap(), MembershipPromiseResult::Accepted);
        let reopened = Storage::open(temp.path().join("data")).unwrap();
        let durable = reopened.load_membership_promise(world).unwrap();
        assert_eq!(durable.vote, vote);
        let during = inspect_v1_roles(&reopened, &bob, world).unwrap();
        assert!(during.voter_member && during.membership_transition_pending);
        assert!(!during.host_candidate);
        assert!(!inspect_v1_roles(&reopened, &outsider, world).unwrap().voter_member);

        let alternative = MembershipProposalV1 {
            previous: accepted.clone(),
            proposed: signed_membership(&epoch, &alice, vec![member(&alice)], Some(&accepted)),
        };
        assert_eq!(
            reopened.promise_membership_proposal(&alternative, &sign_vote(&alternative, &bob)).unwrap(),
            MembershipPromiseResult::Rejected
        );
        assert_eq!(reopened.load_membership_promise(world).unwrap(), durable);
    }

    #[test]
    fn v2_migration_review_uses_real_canonical_storage_and_never_activates() {
        use crate::v2_migration_review::review_stable_v2_migration;
        use swarm_protocol::{
            ProposedHostingRealmGenesisV2, SignedHostingMigrationProposalV2, SignedHostingMigrationQuorumV2,
            SignedHostingMigrationVoteV2, SignedRealmHostingPolicyV2, SleepRecordV1, SourceHistoryAnchorV2,
            WorldHostingPolicyV1, HOSTING_POLICY_FORMAT_VERSION, HOSTING_REALM_FORMAT_VERSION_V2,
            HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2,
        };

        let (temp, storage, creator, epoch) = fixture();
        let world = epoch.world_id;
        let membership = signed_membership(&epoch, &creator, vec![member(&creator)], None);
        storage.save_membership_record(&membership).unwrap();
        let config = initial_config(&storage, &creator, world);
        let metadata = storage.load_world(world).unwrap();
        storage
            .save_world_descriptor(&swarm_protocol::WorldDescriptorV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                compatibility_fingerprint: metadata.genesis.compatibility_fingerprint,
                members: membership.members.clone(),
                preferred_replication_factor: 3,
            })
            .unwrap();
        let manifest = storage.latest_snapshot(world).unwrap().unwrap();
        let checkpoint = manifest.manifest_hash().unwrap();
        let anchor = SourceHistoryAnchorV2 {
            source_world_id: world,
            source_config_sequence: config.sequence,
            source_config_hash: config.config_hash().unwrap(),
            source_membership_sequence: membership.sequence,
            source_membership_hash: membership.record_hash().unwrap(),
            source_snapshot_hash: checkpoint,
            source_epoch: epoch.epoch_number,
            source_fencing_token: epoch.fencing_token,
        };
        let envelope = |anchor: SourceHistoryAnchorV2| {
            let realm = ProposedHostingRealmGenesisV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                source: anchor,
                migration_nonce: [42; 32],
                required_authority_feature_version: HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2,
            };
            let target = realm.realm_id().unwrap();
            let mut policy = SignedRealmHostingPolicyV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                realm_id: target,
                sequence: 1,
                previous_policy_hash: None,
                policy: WorldHostingPolicyV1 {
                    format_version: HOSTING_POLICY_FORMAT_VERSION,
                    allow_member_contributions: true,
                    minimum_server_heap_mib: 1024,
                },
                signer_peer_id: creator.peer_id(),
                signer_public_key: creator.public_key(),
                signature: vec![],
            };
            policy.signature = creator.sign(&policy.signing_bytes().unwrap());
            let mut proposal = SignedHostingMigrationProposalV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                realm_genesis: realm,
                target_realm_id: target,
                initial_policy_hash: policy.record_hash().unwrap(),
                activation_epoch: anchor.source_epoch + 1,
                activation_fencing_token: anchor.source_fencing_token + 1,
                source_authority_peer_id: creator.peer_id(),
                source_authority_public_key: creator.public_key(),
                signature: vec![],
            };
            proposal.signature = creator.sign(&proposal.signing_bytes().unwrap());
            let mut vote = SignedHostingMigrationVoteV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                target_realm_id: target,
                migration_proposal_hash: proposal.proposal_hash().unwrap(),
                source_membership_hash: anchor.source_membership_hash,
                voter_peer_id: creator.peer_id(),
                voter_public_key: creator.public_key(),
                signature: vec![],
            };
            vote.signature = creator.sign(&vote.signing_bytes().unwrap());
            let quorum = SignedHostingMigrationQuorumV2 {
                format_version: HOSTING_REALM_FORMAT_VERSION_V2,
                migration_proposal_hash: proposal.proposal_hash().unwrap(),
                source_membership_hash: anchor.source_membership_hash,
                votes: vec![vote],
            };
            (policy, proposal, quorum)
        };
        let (policy, proposal, quorum) = envelope(anchor);
        let review = || review_stable_v2_migration(&storage, &creator, world, checkpoint, &policy, &proposal, &quorum);
        assert!(review().is_err(), "an awake source is not a frozen migration boundary");
        let mut sleep = SleepRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            latest_snapshot_hash: checkpoint,
            epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
            authority_peer_id: creator.peer_id(),
            authority_public_key: creator.public_key(),
            signature: vec![],
        };
        creator.sign_sleep_record(&mut sleep).unwrap();
        storage.save_sleep_record(&sleep).unwrap();
        let verified = review().unwrap();
        assert_eq!(verified.source().anchor, anchor);
        assert_eq!(verified.target_realm_id(), proposal.target_realm_id);

        let mut forged_anchor = anchor;
        forged_anchor.source_snapshot_hash = Hash32([99; 32]);
        let (forged_policy, forged_proposal, forged_quorum) = envelope(forged_anchor);
        assert!(review_stable_v2_migration(
            &storage,
            &creator,
            world,
            checkpoint,
            &forged_policy,
            &forged_proposal,
            &forged_quorum
        )
        .is_err());
        assert!(review_stable_v2_migration(&storage, &creator, world, Hash32([99; 32]), &policy, &proposal, &quorum)
            .is_err());

        let joiner = PeerIdentity::from_secret_bytes([52; 32]);
        let next = signed_membership(&epoch, &creator, vec![member(&creator), member(&joiner)], Some(&membership));
        let transition = MembershipProposalV1 { previous: membership.clone(), proposed: next };
        let mut pending_vote =
            swarm_consensus::membership_vote_for(&transition, creator.peer_id(), creator.public_key()).unwrap();
        pending_vote.signature = creator.sign(&pending_vote.signing_bytes().unwrap());
        storage.promise_membership_proposal(&transition, &pending_vote).unwrap();
        assert!(review().is_err(), "pending joint vote cannot be reviewed as stable");
        assert!(storage.load_membership_promise(world).is_ok());
        // Legacy metadata cannot be published at the distinct V2 realm ID.
        let target = proposal.target_realm_id;
        let mut legacy_metadata = metadata.clone();
        legacy_metadata.world_id = target;
        assert!(storage.create_world(&legacy_metadata).is_err());
        assert!(storage.load_world(target).is_err());

        // Neither successful nor failed review changes signed V1 state.
        drop(storage);
        let storage = Storage::open(temp.path().join("data")).unwrap();
        assert_eq!(storage.load_epoch_record(world).unwrap(), epoch);
        assert_eq!(storage.load_world_config(world).unwrap(), config);
        assert_eq!(storage.load_membership_record(world).unwrap(), membership);
        assert_eq!(storage.latest_snapshot(world).unwrap().unwrap(), manifest);
        assert_eq!(storage.load_sleep_record(world).unwrap(), sleep);
        assert_eq!(storage.list_worlds().unwrap().len(), 1);
        assert!(storage.load_world(target).is_err());
    }

    fn promote(
        storage: &Storage,
        epoch: &EpochRecordV1,
        authority: &PeerIdentity,
        prior_membership: &MembershipRecordV1,
        prior_config: &WorldConfigV1,
    ) -> (MembershipRecordV1, WorldConfigV1) {
        let prior = storage.latest_snapshot(epoch.world_id).unwrap().unwrap();
        storage.save_epoch_record(epoch).unwrap();
        let mut promoted = SnapshotManifestV1 {
            snapshot_number: prior.snapshot_number + 1,
            sequence: prior.sequence + 1,
            epoch: epoch.epoch_number,
            previous_snapshot_hash: Some(prior.manifest_hash().unwrap()),
            ..prior
        };
        authority.sign_snapshot(&mut promoted).unwrap();
        storage.commit_snapshot(&promoted).unwrap();
        let membership = signed_membership(epoch, authority, prior_membership.members.clone(), Some(prior_membership));
        storage.save_membership_record(&membership).unwrap();
        let mut config = prior_config.clone();
        if config.authority_peer_id != authority.peer_id() {
            config.previous_config_hash = Some(config.config_hash().unwrap());
            config.sequence += 1;
            sign_world_config(authority, &mut config).unwrap();
            storage.save_world_config(&config).unwrap();
        }
        (membership, config)
    }

    #[test]
    fn crash_retirement_fences_old_authority_and_resumes_certified_suffix() {
        let (_temp, old, alice, first) = fixture();
        let bob = PeerIdentity::from_secret_bytes([22; 32]);
        let carol = PeerIdentity::from_secret_bytes([23; 32]);
        let world = first.world_id;
        let genesis = old.load_world(world).unwrap().genesis;
        let initial = signed_membership(&first, &alice, vec![member(&alice)], None);
        old.save_membership_record(&initial).unwrap();
        let config = initial_config(&old, &alice, world);
        let membership =
            signed_membership(&first, &alice, vec![member(&alice), member(&bob), member(&carol)], Some(&initial));
        let proposal = MembershipProposalV1 { previous: initial, proposed: membership.clone() };
        let votes = [&alice, &bob, &carol]
            .iter()
            .map(|voter| {
                let mut vote = MembershipVoteV1 {
                    protocol_version: PROTOCOL_VERSION,
                    world_id: world,
                    previous_membership_hash: proposal.previous.record_hash().unwrap(),
                    proposed_membership_hash: proposal.proposed.record_hash().unwrap(),
                    proposed_sequence: proposal.proposed.sequence,
                    voter_peer_id: voter.peer_id(),
                    voter_public_key: voter.public_key(),
                    signature: Vec::new(),
                };
                vote.signature = voter.sign(&vote.signing_bytes().unwrap());
                vote
            })
            .collect();
        old.save_membership_certificate(&MembershipCertificateV1 { proposal, votes }).unwrap();
        old.save_membership_record(&membership).unwrap();
        old.save_world_descriptor(&swarm_protocol::WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: genesis.compatibility_fingerprint,
            members: membership.members.clone(),
            preferred_replication_factor: 3,
        })
        .unwrap();
        let anchor = old.stale_replay_anchor(world).unwrap();
        let first_checkpoint = old.latest_snapshot(world).unwrap().unwrap();
        // Clone the fully accepted on-disk prefix into a separate fixture.
        // The successor has a replicated checkpoint not retained by the
        // crashed source. No live authority or consent is manufactured.
        fn copy_fixture_tree(source: &std::path::Path, destination: &std::path::Path) {
            std::fs::create_dir_all(destination).unwrap();
            for entry in std::fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let target = destination.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_fixture_tree(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        let winner_dir = tempfile::tempdir().unwrap();
        let winner = Storage::open(winner_dir.path().join("data")).unwrap();
        copy_fixture_tree(&old.world_dir(world), &winner.world_dir(world));
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("level.dat"), b"replicated save before host crash").unwrap();
        let mut boundary = winner
            .snapshot_directory(
                source.path(),
                SnapshotContext {
                    world,
                    snapshot_number: 2,
                    sequence: 2,
                    epoch: 1,
                    previous_snapshot_hash: Some(first_checkpoint.manifest_hash().unwrap()),
                    authority_peer_id: alice.peer_id(),
                    authority_public_key: alice.public_key(),
                },
            )
            .unwrap();
        alice.sign_snapshot(&mut boundary).unwrap();
        winner.commit_snapshot(&boundary).unwrap();
        let mut next = successor(&first, &bob);
        next.mode = EpochMode::Recovery;
        next.base_state_hash = boundary.state_root;
        next.signature = bob.sign(&next.signing_bytes().unwrap());
        let mut ballot = RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            base_epoch: first.epoch_number,
            base_fencing_token: first.fencing_token,
            target_epoch: next.epoch_number,
            target_fencing_token: next.fencing_token,
            round: 1,
            candidate_peer_id: bob.peer_id(),
            candidate_public_key: bob.public_key(),
            base_snapshot_hash: boundary.manifest_hash().unwrap(),
            base_state_hash: boundary.state_root,
            membership_hash: membership.record_hash().unwrap(),
            signature: Vec::new(),
        };
        ballot.signature = bob.sign(&ballot.signing_bytes().unwrap());
        let votes = [&bob, &carol]
            .iter()
            .map(|voter| {
                let mut vote = RecoveryVoteV1 {
                    protocol_version: PROTOCOL_VERSION,
                    world_id: world,
                    ballot_hash: ballot.ballot_hash().unwrap(),
                    base_epoch: ballot.base_epoch,
                    target_epoch: ballot.target_epoch,
                    round: ballot.round,
                    candidate_peer_id: ballot.candidate_peer_id,
                    voter_peer_id: voter.peer_id(),
                    voter_public_key: voter.public_key(),
                    signature: Vec::new(),
                };
                vote.signature = voter.sign(&vote.signing_bytes().unwrap());
                vote
            })
            .collect();
        let certificate = RecoveryCertificateV1 { ballot, votes };
        winner.save_recovery_certificate(&certificate).unwrap();
        let (membership2, config2) = promote(&winner, &next, &bob, &membership, &config);
        let proof = collect_replay_proof(&winner, &bob, world).unwrap();
        let mut invalid = proof.clone();
        let HistoricalTransitionV1::Recovery(invalid_certificate) = &mut invalid.transitions[0] else {
            panic!("expected certified recovery");
        };
        invalid_certificate.votes[0].signature[0] ^= 1;
        invalid.signature = bob.sign(&invalid.signing_bytes().unwrap());
        assert!(verify_and_stage_stale_replay(&old, &alice, bob.peer_id(), &anchor, invalid).is_err());
        assert!(!old.crash_retirement_pending(world).unwrap());

        let paths = DataPaths::from_root(old.root());
        let control = paths.root.join("control").join(world.to_hex());
        swarm_core::create_private_dir_all(&control).unwrap();
        let process_record = control.join("runtime-process.json");
        std::fs::write(
            &process_record,
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "controller_pid": 1, "java_pid": std::process::id()
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(verify_and_stage_stale_replay(&old, &alice, bob.peer_id(), &anchor, proof.clone()).is_err());
        assert!(!old.crash_retirement_pending(world).unwrap());
        assert_eq!(old.load_epoch_record(world).unwrap(), first);
        std::fs::remove_file(&process_record).unwrap();
        verify_and_stage_stale_replay(&old, &alice, bob.peer_id(), &anchor, proof.clone()).unwrap();
        assert!(old.crash_retirement_pending(world).unwrap());
        assert!(!old.stale_publication_pending(world).unwrap());
        assert!(old.load_epoch_record(world).is_err());
        assert!(old.load_membership_record(world).is_err());
        assert!(old.load_world_config(world).is_err());
        assert!(old.load_world_descriptor(world).is_err());
        assert!(old.latest_snapshot(world).is_err());
        assert!(old.save_epoch_record(&first).is_err());
        assert!(old.save_membership_record(&membership).is_err());
        assert!(old.garbage_collect_blobs(world).is_err());
        assert!(crate::live_authority::verify_playable_epoch(&old, &first).is_err());
        assert!(publish_stale_replay(&old, &alice, world).is_err());

        let marker = old.world_dir(world).join("metadata/crash-retirement-replay.postcard");
        let original = std::fs::read(&marker).unwrap();
        std::fs::write(&marker, b"unreadable retirement marker").unwrap();
        assert!(old.load_epoch_record(world).is_err());
        assert!(old.stale_replay_progress(world).is_err());
        std::fs::write(&marker, original).unwrap();
        let reopened = Storage::open(old.root()).unwrap();
        assert_eq!(reopened.stale_replay_progress(world).unwrap().unwrap().completed, 1);
        assert!(reopened.load_epoch_record(world).is_err());

        // A busy authority must not move an already pinned target merely
        // by saving another checkpoint. A certified roster extension can
        // advance that target, including to a previously unknown peer.
        let previous = winner.latest_snapshot(world).unwrap().unwrap();
        let mut extra = SnapshotManifestV1 {
            snapshot_number: previous.snapshot_number + 1,
            sequence: previous.sequence + 1,
            previous_snapshot_hash: Some(previous.manifest_hash().unwrap()),
            ..previous
        };
        bob.sign_snapshot(&mut extra).unwrap();
        winner.commit_snapshot(&extra).unwrap();
        let moving = collect_replay_proof(&winner, &bob, world).unwrap();
        assert!(moving.extends_replay_history(&proof));
        assert!(verify_and_stage_stale_replay(&reopened, &alice, bob.peer_id(), &anchor, moving).is_err());
        assert_eq!(reopened.load_staged_stale_replay_proof(world).unwrap(), Some((anchor.clone(), proof.clone())));

        let dora = PeerIdentity::from_secret_bytes([24; 32]);
        assert!(!anchor.membership.members.iter().any(|member| member.peer_id == dora.peer_id()));
        let mut members = membership2.members.clone();
        members.push(member(&dora));
        let admitted = signed_membership(&next, &bob, members, Some(&membership2));
        let proposal = MembershipProposalV1 { previous: membership2.clone(), proposed: admitted.clone() };
        let votes = [&bob, &carol, &dora]
            .iter()
            .map(|voter| {
                let mut vote = MembershipVoteV1 {
                    protocol_version: PROTOCOL_VERSION,
                    world_id: world,
                    previous_membership_hash: proposal.previous.record_hash().unwrap(),
                    proposed_membership_hash: proposal.proposed.record_hash().unwrap(),
                    proposed_sequence: proposal.proposed.sequence,
                    voter_peer_id: voter.peer_id(),
                    voter_public_key: voter.public_key(),
                    signature: Vec::new(),
                };
                vote.signature = voter.sign(&vote.signing_bytes().unwrap());
                vote
            })
            .collect();
        winner.save_membership_certificate(&MembershipCertificateV1 { proposal, votes }).unwrap();
        winner.save_membership_record(&admitted).unwrap();
        winner
            .save_world_descriptor(&swarm_protocol::WorldDescriptorV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                compatibility_fingerprint: genesis.compatibility_fingerprint,
                members: admitted.members.clone(),
                preferred_replication_factor: 3,
            })
            .unwrap();

        // Bob transfers to Dora, admitted while the crashed source is
        // fenced. A full certified extension must authorize the new peer.
        let second_base = winner.latest_snapshot(world).unwrap().unwrap();
        let third = successor(&next, &dora);
        let mut third = EpochRecordV1 { mode: EpochMode::Quorum, base_state_hash: second_base.state_root, ..third };
        third.signature = dora.sign(&third.signing_bytes().unwrap());
        let mut transfer = AuthorityTransferV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            from_peer_id: bob.peer_id(),
            to_peer_id: dora.peer_id(),
            base_snapshot_hash: second_base.manifest_hash().unwrap(),
            next_epoch: third.epoch_number,
            next_fencing_token: third.fencing_token,
            phase: TransferPhase::Prepared,
            signer_peer_id: bob.peer_id(),
            signer_public_key: bob.public_key(),
            signature: Vec::new(),
        };
        for (kind, signer) in
            [(TransferPhase::Prepared, &bob), (TransferPhase::Accepted, &dora), (TransferPhase::Committed, &bob)]
        {
            transfer.phase = kind;
            transfer.signer_peer_id = signer.peer_id();
            transfer.signer_public_key = signer.public_key();
            transfer.signature = signer.sign(&transfer.signing_bytes().unwrap());
            winner.save_transfer_record(&transfer).unwrap();
        }
        let (membership3, _) = promote(&winner, &third, &dora, &admitted, &config2);
        let extended = collect_replay_proof(&winner, &dora, world).unwrap();
        assert!(extended.extends_replay_history(&proof));
        let mut uncertified = extended.clone();
        uncertified.membership_certificates.retain(|certificate| certificate.proposal.proposed != admitted);
        uncertified.signature = dora.sign(&uncertified.signing_bytes().unwrap());
        assert!(verify_and_stage_stale_replay(&reopened, &alice, dora.peer_id(), &anchor, uncertified).is_err());
        assert!(verify_and_stage_stale_replay(&reopened, &alice, carol.peer_id(), &anchor, extended.clone()).is_err());
        assert_eq!(reopened.load_staged_stale_replay_proof(world).unwrap(), Some((anchor.clone(), proof.clone())));
        verify_and_stage_stale_replay(&reopened, &alice, dora.peer_id(), &anchor, extended.clone()).unwrap();
        assert!(verify_and_stage_stale_replay(&reopened, &alice, bob.peer_id(), &anchor, proof).is_err());
        assert_eq!(reopened.load_staged_stale_replay_proof(world).unwrap(), Some((anchor.clone(), extended.clone())));

        let verified = verify_replay_proof(extended.clone(), &genesis, &membership3).unwrap();
        let canonical = winner.list_snapshots(world).unwrap();
        for manifest in canonical.iter().skip(1) {
            verified.verify_manifest(manifest).unwrap();
            assert!(reopened
                .open_stale_replay_download(bob.peer_id(), membership3.record_hash().unwrap(), manifest)
                .is_err());
            let download = reopened
                .open_stale_replay_download(dora.peer_id(), membership3.record_hash().unwrap(), manifest)
                .unwrap();
            for (hash, offset) in download.missing_blobs().unwrap() {
                let blob = &manifest.entries.iter().find(|entry| entry.blob.hash == hash).unwrap().blob;
                let (data, finished) = winner
                    .read_encoded_blob_chunk(world, blob, offset, usize::try_from(blob.encoded_size - offset).unwrap())
                    .unwrap();
                assert!(finished);
                download.receive_blob_chunk(blob, offset, &data, true).unwrap();
            }
            download.finalize().unwrap();
            assert!(reopened.load_epoch_record(world).is_err());
        }
        // A crash can also leave signed sleep/recovery records on disk.
        // Inject exact durable bytes because ordinary writes are already
        // fenced. Corruption, unrelated boundaries and future reservations
        // must remain blocking, without consuming any record.
        let metadata = reopened.world_dir(world).join("metadata");
        let accepted_path = metadata.join("recovery-certificate.postcard");
        let pending_path = metadata.join("recovery-certificate-pending.postcard");
        let sleep_path = metadata.join("sleep.postcard");
        let reservation_path = metadata.join("recovery-reservation.postcard");
        let certificate_bytes = postcard::to_allocvec(&certificate).unwrap();
        let mut corrupted = certificate_bytes.clone();
        corrupted.push(0);
        std::fs::write(&pending_path, &corrupted).unwrap();
        assert!(reopened.stale_publication_candidate(world).is_err());
        assert_eq!(std::fs::read(&pending_path).unwrap(), corrupted);
        let mut conflicting = certificate.clone();
        conflicting.votes[0].signature[0] ^= 1;
        std::fs::write(&accepted_path, postcard::to_allocvec(&conflicting).unwrap()).unwrap();
        std::fs::write(&pending_path, &certificate_bytes).unwrap();
        assert!(reopened.stale_publication_candidate(world).is_err());
        std::fs::write(&accepted_path, &certificate_bytes).unwrap();
        let mut sleep = swarm_protocol::SleepRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            latest_snapshot_hash: boundary.manifest_hash().unwrap(),
            epoch: first.epoch_number,
            fencing_token: first.fencing_token,
            authority_peer_id: alice.peer_id(),
            authority_public_key: alice.public_key(),
            signature: Vec::new(),
        };
        alice.sign_sleep_record(&mut sleep).unwrap();
        let mut different_sleep = sleep.clone();
        different_sleep.latest_snapshot_hash = first_checkpoint.manifest_hash().unwrap();
        alice.sign_sleep_record(&mut different_sleep).unwrap();
        std::fs::write(&sleep_path, postcard::to_allocvec(&different_sleep).unwrap()).unwrap();
        assert!(reopened.stale_publication_candidate(world).is_err());
        let mut forged_sleep = sleep.clone();
        forged_sleep.signature[0] ^= 1;
        std::fs::write(&sleep_path, postcard::to_allocvec(&forged_sleep).unwrap()).unwrap();
        assert!(reopened.stale_publication_candidate(world).unwrap().is_some());
        assert!(publish_stale_replay(&reopened, &alice, world).is_err());
        assert!(!reopened.stale_publication_pending(world).unwrap());
        std::fs::write(&sleep_path, postcard::to_allocvec(&sleep).unwrap()).unwrap();
        let mut reservation = swarm_protocol::AuthorityLeaseGrantV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: next.epoch_number,
            fencing_token: next.fencing_token,
            lease_duration_ms: 5_000,
            authority_peer_id: bob.peer_id(),
            authority_public_key: bob.public_key(),
            nonce: [71; 32],
            signature: Vec::new(),
        };
        bob.sign_lease(&mut reservation).unwrap();
        let mut final_reservation = reservation.clone();
        final_reservation.epoch = third.epoch_number;
        final_reservation.fencing_token = third.fencing_token;
        dora.sign_lease(&mut final_reservation).unwrap();
        std::fs::write(&reservation_path, postcard::to_allocvec(&final_reservation).unwrap()).unwrap();
        assert!(reopened.stale_publication_candidate(world).is_err());
        let mut forged_reservation = reservation.clone();
        forged_reservation.signature[0] ^= 1;
        std::fs::write(&reservation_path, postcard::to_allocvec(&forged_reservation).unwrap()).unwrap();
        assert!(reopened.stale_publication_candidate(world).unwrap().is_some());
        assert!(publish_stale_replay(&reopened, &alice, world).is_err());
        assert!(!reopened.stale_publication_pending(world).unwrap());
        let reservation_bytes = postcard::to_allocvec(&reservation).unwrap();
        std::fs::write(&reservation_path, &reservation_bytes).unwrap();
        let candidate = reopened.stale_publication_candidate(world).unwrap().unwrap();
        assert_eq!(candidate.manifests, canonical);
        assert_eq!(candidate.sleep.as_ref(), Some(&sleep));
        assert_eq!(candidate.reservation.as_ref(), Some(&reservation));
        assert!(reopened.begin_verified_stale_publication(&candidate).is_err());
        reopened.begin_verified_crash_retirement_publication(&candidate).unwrap();
        assert!(reopened.crash_retirement_pending(world).unwrap());
        assert!(reopened.stale_publication_pending(world).unwrap());
        assert!(reopened.stale_replay_progress(world).is_err());
        drop(reopened);
        let reopened = Storage::open(old.root()).unwrap();
        assert_eq!(publish_stale_replay(&reopened, &alice, world).unwrap(), extended);
        assert!(!reopened.crash_retirement_pending(world).unwrap());
        assert!(!reopened.stale_publication_pending(world).unwrap());
        assert_eq!(reopened.load_epoch_record(world).unwrap(), third);
        assert!(!accepted_path.exists());
        assert!(!pending_path.exists());
        assert!(!sleep_path.exists());
        assert!(!reservation_path.exists());
        assert_eq!(
            reopened.load_recovery_certificate_for_epoch(world, next.record_hash().unwrap()).unwrap(),
            certificate
        );
        assert_eq!(reopened.load_sleep_boundary(&first, boundary.manifest_hash().unwrap()).unwrap(), sleep);
        let reservation_hash = Hash32(*blake3::hash(&reservation_bytes).as_bytes());
        assert_eq!(
            std::fs::read(
                metadata.join("authority-history").join(format!("recovery-reservation-{reservation_hash}.postcard"))
            )
            .unwrap(),
            reservation_bytes
        );
        assert_eq!(reopened.load_membership_record(world).unwrap(), membership3);
        assert_eq!(reopened.list_snapshots(world).unwrap(), canonical);
        assert!(reopened.load_staged_stale_replay_proof(world).unwrap().is_none());
        assert!(crate::hosting_budget::load_consent(&paths, world).unwrap().is_none());
        crate::live_authority::verify_playable_epoch(&reopened, &third).unwrap();
    }

    #[test]
    fn replay_proof_requires_exact_signed_transfer_and_recovery_quorum() {
        let (_temp, storage, alice, first) = fixture();
        let bob = PeerIdentity::from_secret_bytes([22; 32]);
        let carol = PeerIdentity::from_secret_bytes([23; 32]);
        let dave = PeerIdentity::from_secret_bytes([27; 32]);
        let world = first.world_id;
        let genesis = storage.load_world(world).unwrap().genesis;
        let initial = signed_membership(&first, &alice, vec![member(&alice)], None);
        storage.save_membership_record(&initial).unwrap();
        let config = initial_config(&storage, &alice, world);
        let mut dave_member = member(&dave);
        dave_member.authority_eligible = false;
        let admitted = signed_membership(
            &first,
            &alice,
            vec![member(&alice), member(&bob), member(&carol), dave_member],
            Some(&initial),
        );
        let proposal = MembershipProposalV1 { previous: initial, proposed: admitted.clone() };
        let votes = [&alice, &bob, &carol]
            .iter()
            .map(|voter| {
                let mut vote = MembershipVoteV1 {
                    protocol_version: PROTOCOL_VERSION,
                    world_id: world,
                    previous_membership_hash: proposal.previous.record_hash().unwrap(),
                    proposed_membership_hash: proposal.proposed.record_hash().unwrap(),
                    proposed_sequence: proposal.proposed.sequence,
                    voter_peer_id: voter.peer_id(),
                    voter_public_key: voter.public_key(),
                    signature: Vec::new(),
                };
                vote.signature = voter.sign(&vote.signing_bytes().unwrap());
                vote
            })
            .collect();
        storage.save_membership_certificate(&MembershipCertificateV1 { proposal, votes }).unwrap();
        storage.save_membership_record(&admitted).unwrap();

        let base = storage.latest_snapshot(world).unwrap().unwrap();
        // Preserve a fully accepted replica at epoch one before either
        // signed transition. It will never install an intermediate epoch
        // merely because it learns about a valid newer history.
        let stale_dir = tempfile::tempdir().unwrap();
        let stale = Storage::open(stale_dir.path()).unwrap();
        stale.create_world(&storage.load_world(world).unwrap()).unwrap();
        for entry in &base.entries {
            let blob = &entry.blob;
            if stale.has_complete_blob(world, blob) {
                continue;
            }
            let (data, finished) =
                storage.read_encoded_blob_chunk(world, blob, 0, usize::try_from(blob.encoded_size).unwrap()).unwrap();
            assert!(finished);
            stale.receive_blob_chunk(world, blob, 0, &data, true).unwrap();
        }
        stale.finalize_replica(&base).unwrap();
        stale.save_epoch_record(&first).unwrap();
        stale.save_membership_record(&storage.load_genesis_membership(world).unwrap()).unwrap();
        stale.save_membership_certificate(&storage.load_membership_certificate(world).unwrap()).unwrap();
        stale.save_membership_record(&admitted).unwrap();
        stale.save_world_config(&config).unwrap();
        stale
            .save_world_descriptor(&swarm_protocol::WorldDescriptorV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                compatibility_fingerprint: genesis.compatibility_fingerprint,
                members: admitted.members.clone(),
                preferred_replication_factor: 3,
            })
            .unwrap();
        let anchor = stale.stale_replay_anchor(world).unwrap();
        let second = successor(&first, &bob);
        let mut phase = AuthorityTransferV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            from_peer_id: alice.peer_id(),
            to_peer_id: bob.peer_id(),
            base_snapshot_hash: base.manifest_hash().unwrap(),
            next_epoch: second.epoch_number,
            next_fencing_token: second.fencing_token,
            phase: TransferPhase::Prepared,
            signer_peer_id: alice.peer_id(),
            signer_public_key: alice.public_key(),
            signature: Vec::new(),
        };
        for (kind, signer) in
            [(TransferPhase::Prepared, &alice), (TransferPhase::Accepted, &bob), (TransferPhase::Committed, &alice)]
        {
            phase.phase = kind;
            phase.signer_peer_id = signer.peer_id();
            phase.signer_public_key = signer.public_key();
            phase.signature = signer.sign(&phase.signing_bytes().unwrap());
            storage.save_transfer_record(&phase).unwrap();
        }
        let (membership2, config2) = promote(&storage, &second, &bob, &admitted, &config);
        let proof = collect_replay_proof(&storage, &bob, world).unwrap();
        assert_eq!(proof.transitions.len(), 1);
        assert!(matches!(proof.transitions[0], HistoricalTransitionV1::Transfer { .. }));
        assert!(verify_and_stage_stale_replay(&stale, &dave, carol.peer_id(), &anchor, proof.clone()).is_err());
        assert!(verify_and_stage_stale_replay(&stale, &alice, bob.peer_id(), &anchor, proof.clone()).is_err());
        let mut wrong_anchor = anchor.clone();
        wrong_anchor.head.manifest_hash = Hash32([42; 32]);
        assert!(verify_and_stage_stale_replay(&stale, &dave, bob.peer_id(), &wrong_anchor, proof.clone()).is_err());
        let mut fork = proof.clone();
        fork.epochs[0].reason = "a different signed genesis epoch".into();
        fork.epochs[0].signature = alice.sign(&fork.epochs[0].signing_bytes().unwrap());
        fork.epochs[1].previous_epoch_hash = Some(fork.epochs[0].record_hash().unwrap());
        fork.epochs[1].signature = bob.sign(&fork.epochs[1].signing_bytes().unwrap());
        fork.signature = bob.sign(&fork.signing_bytes().unwrap());
        assert!(verify_replay_proof(fork.clone(), &genesis, &membership2).is_ok());
        assert!(verify_and_stage_stale_replay(&stale, &dave, bob.peer_id(), &anchor, fork).is_err());
        assert!(stale.load_staged_stale_replay_proof(world).unwrap().is_none());
        verify_and_stage_stale_replay(&stale, &dave, bob.peer_id(), &anchor, proof.clone()).unwrap();
        let first_pinned = proof.clone();
        let verified = verify_replay_proof(proof, &genesis, &membership2).unwrap();
        for manifest in storage.list_snapshots(world).unwrap() {
            verified.verify_manifest(&manifest).unwrap();
        }

        // A separate first-contact replica has the original signed joint
        // admission, but has not accepted any epoch. Its pinned proof may
        // advance from the first successor to a later certified authority.
        let joining_dir = tempfile::tempdir().unwrap();
        let joining = Storage::open(joining_dir.path()).unwrap();
        joining.create_world(&storage.load_world(world).unwrap()).unwrap();
        let mut invite = swarm_protocol::InviteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            display_name: "retargeted first contact".into(),
            genesis: genesis.clone(),
            inviter_peer_id: alice.peer_id(),
            inviter_public_key: alice.public_key(),
            bootstrap_addrs: Vec::new(),
            expires_unix_ms: u64::MAX,
            nonce: [71; 32],
            signature: Vec::new(),
        };
        alice.sign_invite(&mut invite).unwrap();
        let mut join = swarm_protocol::JoinRequestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            invite,
            joining_member: admitted.members.iter().find(|member| member.peer_id == dave.peer_id()).unwrap().clone(),
            nonce: [72; 32],
            signature: Vec::new(),
        };
        dave.sign_join_request(&mut join).unwrap();
        joining.save_pending_join(&join).unwrap();
        let admission_proof = swarm_protocol::DiscoveryMembershipProofV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            genesis: genesis.clone(),
            initial_membership: first_pinned.membership[0].clone(),
            membership_certificates: first_pinned.membership_certificates.clone(),
            current_membership: admitted.clone(),
            pending_membership: None,
        };
        crate::live_authority::verify_genesis_anchored_membership(&admission_proof).unwrap();
        joining.archive_join_membership_history(&admission_proof).unwrap();
        joining.save_membership_certificate(first_pinned.membership_certificates.last().unwrap()).unwrap();
        joining.save_membership_record(&admitted).unwrap();
        joining.save_world_descriptor(&anchor.descriptor).unwrap();
        joining.clear_pending_join(world).unwrap();
        let first_contact_admission = admitted.clone();
        let admission_hash = first_contact_admission.record_hash().unwrap();
        verify_and_stage_first_contact_replay(&joining, &dave, bob.peer_id(), admission_hash, first_pinned.clone())
            .unwrap();
        for entry in &base.entries {
            let blob = &entry.blob;
            if joining.has_complete_blob(world, blob) {
                continue;
            }
            let (bytes, finished) =
                storage.read_encoded_blob_chunk(world, blob, 0, usize::try_from(blob.encoded_size).unwrap()).unwrap();
            assert!(finished);
            joining.receive_blob_chunk(world, blob, 0, &bytes, true).unwrap();
        }
        joining.finalize_replica(&base).unwrap();
        assert!(joining.load_epoch_record(world).is_err());
        assert!(joining.save_epoch_record(&second).is_err());
        assert!(joining.save_membership_record(&membership2).is_err());

        let base2 = storage.latest_snapshot(world).unwrap().unwrap();
        let previous_download =
            stale.open_stale_replay_download(bob.peer_id(), membership2.record_hash().unwrap(), &base2).unwrap();
        assert!(previous_download.missing_blobs().unwrap().is_empty());
        assert!(previous_download.is_current());
        let mut third = successor(&second, &carol);
        third.mode = EpochMode::Recovery;
        third.signature = carol.sign(&third.signing_bytes().unwrap());
        let mut ballot = RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            base_epoch: second.epoch_number,
            base_fencing_token: second.fencing_token,
            target_epoch: third.epoch_number,
            target_fencing_token: third.fencing_token,
            round: 1,
            candidate_peer_id: carol.peer_id(),
            candidate_public_key: carol.public_key(),
            base_snapshot_hash: base2.manifest_hash().unwrap(),
            base_state_hash: base2.state_root,
            membership_hash: membership2.record_hash().unwrap(),
            signature: Vec::new(),
        };
        ballot.signature = carol.sign(&ballot.signing_bytes().unwrap());
        let votes = [&alice, &bob, &carol]
            .iter()
            .map(|voter| {
                let mut vote = RecoveryVoteV1 {
                    protocol_version: PROTOCOL_VERSION,
                    world_id: world,
                    ballot_hash: ballot.ballot_hash().unwrap(),
                    base_epoch: ballot.base_epoch,
                    target_epoch: ballot.target_epoch,
                    round: ballot.round,
                    candidate_peer_id: ballot.candidate_peer_id,
                    voter_peer_id: voter.peer_id(),
                    voter_public_key: voter.public_key(),
                    signature: Vec::new(),
                };
                vote.signature = voter.sign(&vote.signing_bytes().unwrap());
                vote
            })
            .collect();
        storage.save_recovery_certificate(&RecoveryCertificateV1 { ballot, votes }).unwrap();
        let (membership3, _) = promote(&storage, &third, &carol, &membership2, &config2);
        let proof = collect_replay_proof(&storage, &carol, world).unwrap();
        assert_eq!(proof.epochs.len(), 3);
        assert_eq!(proof.checkpoints.len(), 3);
        assert_eq!(proof.membership.last(), Some(&membership3));
        assert!(matches!(proof.transitions[1], HistoricalTransitionV1::Recovery(_)));
        assert!(proof.extends_replay_history(&first_pinned));
        verify_and_stage_stale_replay(&stale, &dave, carol.peer_id(), &anchor, proof.clone()).unwrap();
        assert!(!previous_download.is_current());
        assert!(previous_download.finalize().is_err(), "a superseded source cannot finish a negotiation");
        assert!(
            stale
                .open_stale_replay_download(
                    carol.peer_id(),
                    membership3.record_hash().unwrap(),
                    &storage.load_snapshot(world, 3).unwrap()
                )
                .is_err(),
            "staged checkpoint sequence cannot skip a parent"
        );
        stale
            .open_stale_replay_download(carol.peer_id(), membership3.record_hash().unwrap(), &base2)
            .unwrap()
            .finalize()
            .unwrap();
        assert_eq!(stale.stale_replay_progress(world).unwrap().unwrap().completed, 2);
        assert!(verify_and_stage_stale_replay(&stale, &dave, bob.peer_id(), &anchor, first_pinned.clone()).is_err());
        drop(stale);
        let stale = Storage::open(stale_dir.path()).unwrap();
        assert_eq!(stale.load_staged_stale_replay_proof(world).unwrap(), Some((anchor.clone(), proof.clone())));
        assert_eq!(stale.stale_replay_anchor(world).unwrap(), anchor);
        assert_eq!(stale.load_epoch_record(world).unwrap(), first);
        assert_eq!(stale.latest_snapshot(world).unwrap().unwrap(), base);
        assert!(stale.load_transfer_record(world).is_err());
        assert!(stale.load_recovery_certificate(world).is_err());
        let verified = verify_replay_proof(proof.clone(), &genesis, &membership3).unwrap();
        for manifest in storage.list_snapshots(world).unwrap() {
            verified.verify_manifest(&manifest).unwrap();
        }
        stale
            .open_stale_replay_download(
                carol.peer_id(),
                membership3.record_hash().unwrap(),
                &storage.load_snapshot(world, 3).unwrap(),
            )
            .unwrap()
            .finalize()
            .unwrap();
        assert_eq!(stale.verified_stale_replay_suffix(world).unwrap(), storage.list_snapshots(world).unwrap()[1..]);
        let mut forged = proof.clone();
        if let HistoricalTransitionV1::Recovery(cert) = &mut forged.transitions[1] {
            cert.votes.truncate(1);
        }
        forged.signature = carol.sign(&forged.signing_bytes().unwrap());
        assert!(verify_replay_proof(forged, &genesis, &membership3).is_err());
        let mut wrong_base = proof.clone();
        if let HistoricalTransitionV1::Transfer { accepted, .. } = &mut wrong_base.transitions[0] {
            accepted.base_snapshot_hash = Hash32([42; 32]);
            accepted.signature = bob.sign(&accepted.signing_bytes().unwrap());
        }
        wrong_base.signature = carol.sign(&wrong_base.signing_bytes().unwrap());
        assert!(verify_replay_proof(wrong_base, &genesis, &membership3).is_err());
        let mut missing_joint = proof.clone();
        missing_joint.membership_certificates.clear();
        missing_joint.signature = carol.sign(&missing_joint.signing_bytes().unwrap());
        assert!(verify_replay_proof(missing_joint, &genesis, &membership3).is_err());
        assert!(verify_replay_proof(proof, &genesis, &membership2).is_err());
        assert_eq!(storage.load_epoch_record(world).unwrap(), third);

        // A new player joins *after* both a host transfer and a certified
        // crash recovery. Its new membership must preserve the original
        // ballot roster and extend the promoted membership by a joint quorum.
        let dan = PeerIdentity::from_secret_bytes([24; 32]);
        let mut player = member(&dan);
        player.authority_eligible = false;
        let mut members = membership3.members.clone();
        members.push(player.clone());
        let admitted = signed_membership(&third, &carol, members, Some(&membership3));
        let proposal = MembershipProposalV1 { previous: membership3.clone(), proposed: admitted.clone() };
        let votes = [&alice, &bob, &carol, &dan]
            .iter()
            .map(|voter| {
                let mut vote = MembershipVoteV1 {
                    protocol_version: PROTOCOL_VERSION,
                    world_id: world,
                    previous_membership_hash: proposal.previous.record_hash().unwrap(),
                    proposed_membership_hash: proposal.proposed.record_hash().unwrap(),
                    proposed_sequence: proposal.proposed.sequence,
                    voter_peer_id: voter.peer_id(),
                    voter_public_key: voter.public_key(),
                    signature: Vec::new(),
                };
                vote.signature = voter.sign(&vote.signing_bytes().unwrap());
                vote
            })
            .collect();
        let admission = MembershipCertificateV1 { proposal, votes };
        storage.save_membership_certificate(&admission).unwrap();
        storage.save_membership_record(&admitted).unwrap();
        let descriptor = swarm_protocol::WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: genesis.compatibility_fingerprint,
            members: admitted.members.clone(),
            preferred_replication_factor: 3,
        };
        storage.save_world_descriptor(&descriptor).unwrap();
        crate::live_authority::verify_playable_epoch(&storage, &third).unwrap();
        let parent = storage.latest_snapshot(world).unwrap().unwrap();
        let fresh = tempfile::tempdir().unwrap();
        std::fs::write(fresh.path().join("level.dat"), b"new canonical checkpoint after recovery").unwrap();
        let mut seed = 0xCAFE1234_u64;
        let bytes: Vec<u8> = (0..1024 * 1024)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        std::fs::write(fresh.path().join("region.dat"), bytes).unwrap();
        let mut saved = storage
            .snapshot_directory(
                fresh.path(),
                SnapshotContext {
                    world,
                    snapshot_number: parent.snapshot_number + 1,
                    sequence: parent.sequence + 1,
                    epoch: third.epoch_number,
                    previous_snapshot_hash: Some(parent.manifest_hash().unwrap()),
                    authority_peer_id: carol.peer_id(),
                    authority_public_key: carol.public_key(),
                },
            )
            .unwrap();
        carol.sign_snapshot(&mut saved).unwrap();
        storage.commit_snapshot(&saved).unwrap();
        let fresh_manifest = saved.manifest().clone();
        let envelope = collect_replay_proof(&storage, &carol, world).unwrap();
        assert!(verify_and_stage_first_contact_replay(
            &joining,
            &dave,
            bob.peer_id(),
            admission_hash,
            envelope.clone()
        )
        .is_err());
        verify_and_stage_first_contact_replay(&joining, &dave, carol.peer_id(), admission_hash, envelope.clone())
            .unwrap();
        assert!(verify_and_stage_first_contact_replay(
            &joining,
            &dave,
            bob.peer_id(),
            admission_hash,
            first_pinned.clone()
        )
        .is_err());
        assert_eq!(joining.load_membership_record(world).unwrap(), first_contact_admission);
        assert_eq!(joining.load_first_contact_replay(world).unwrap().unwrap().proof, envelope);
        assert!(joining.load_epoch_record(world).is_err());
        drop(joining);
        let joining = Storage::open(joining_dir.path()).unwrap();
        let first_contact = load_staged_replay(&joining, world).unwrap().unwrap();
        assert_eq!(first_contact.proof(), &envelope);
        assert!(first_contact.publish(&joining).is_err());
        let history = storage.list_snapshots(world).unwrap();
        for manifest in history.iter().skip(1) {
            first_contact.verify_manifest(manifest).unwrap();
            for entry in &manifest.entries {
                let blob = &entry.blob;
                if joining.has_complete_blob(world, blob) {
                    continue;
                }
                let (bytes, finished) = storage
                    .read_encoded_blob_chunk(world, blob, 0, usize::try_from(blob.encoded_size).unwrap())
                    .unwrap();
                assert!(finished);
                joining.receive_blob_chunk(world, blob, 0, &bytes, true).unwrap();
            }
            joining.finalize_replica(manifest).unwrap();
            assert!(joining.load_epoch_record(world).is_err());
        }
        // Simulate an interruption after the retargeted publication has
        // committed its intent and written some target control records.
        let metadata = joining.world_dir(world).join("metadata");
        let config_path = metadata.join("world-config.postcard");
        std::fs::write(&config_path, b"conflicting config").unwrap();
        assert!(first_contact.publish(&joining).is_err());
        assert!(joining.first_contact_publication_started(world).unwrap());
        std::fs::remove_file(&config_path).unwrap();
        std::fs::write(metadata.join("membership.postcard"), postcard::to_allocvec(&admitted).unwrap()).unwrap();
        std::fs::write(metadata.join("descriptor.json"), serde_json::to_vec_pretty(&descriptor).unwrap()).unwrap();
        assert!(joining.save_epoch_record(&third).is_err());
        drop(joining);
        let joining = Storage::open(joining_dir.path()).unwrap();
        load_staged_replay(&joining, world).unwrap().unwrap().publish(&joining).unwrap();
        assert_eq!(joining.load_epoch_record(world).unwrap(), third);
        assert_eq!(joining.load_membership_record(world).unwrap(), admitted);
        assert_eq!(joining.load_world_descriptor(world).unwrap(), descriptor);
        assert_eq!(joining.list_snapshots(world).unwrap(), history);
        assert!(!joining.first_contact_publication_started(world).unwrap());
        crate::live_authority::verify_playable_epoch(&joining, &third).unwrap();

        verify_and_stage_stale_replay(&stale, &dave, carol.peer_id(), &anchor, envelope.clone()).unwrap();
        assert_eq!(stale.load_staged_stale_replay_proof(world).unwrap(), Some((anchor.clone(), envelope.clone())));
        assert_eq!(stale.stale_replay_anchor(world).unwrap(), anchor);
        let verified = verify_replay_proof(envelope.clone(), &genesis, &admitted).unwrap();
        assert_eq!(stale.stale_replay_progress(world).unwrap().unwrap().completed, 3);
        assert!(stale.verified_stale_replay_suffix(world).is_err());
        verified.verify_manifest(&fresh_manifest).unwrap();
        let download = stale
            .open_stale_replay_download(carol.peer_id(), admitted.record_hash().unwrap(), &fresh_manifest)
            .unwrap();
        let large = &fresh_manifest.entries.iter().max_by_key(|entry| entry.blob.encoded_size).unwrap().blob;
        let small = &fresh_manifest.entries.iter().min_by_key(|entry| entry.blob.encoded_size).unwrap().blob;
        assert!(large.encoded_size > 256 * 1024);
        let (chunk, finished) = storage.read_encoded_blob_chunk(world, large, 0, 128 * 1024).unwrap();
        assert!(!finished);
        let quota_paused = swarm_storage::WorldStoragePolicy { max_world_bytes: Some(0), ..Default::default() };
        stale.set_world_storage_policy(world, quota_paused).unwrap();
        assert!(matches!(download.receive_blob_chunk(large, 0, &chunk, false),
            Err(swarm_storage::replica::ReplicationError::Storage(error)) if error.is_storage_pressure()));
        assert!(download.missing_blobs().unwrap().contains(&(large.hash, 0)));
        assert!(!stale.stale_publication_pending(world).unwrap());
        assert_eq!(stale.stale_replay_anchor(world).unwrap(), anchor);
        stale.set_world_storage_policy(world, Default::default()).unwrap();
        assert_eq!(download.receive_blob_chunk(large, 0, &chunk, false).unwrap(), 128 * 1024);
        let (chunk, finished) =
            storage.read_encoded_blob_chunk(world, small, 0, usize::try_from(small.encoded_size).unwrap()).unwrap();
        assert!(finished);
        // Model interruption just before promoting an EOF partial.
        download.receive_blob_chunk(small, 0, &chunk, false).unwrap();
        assert!(download.finalize().is_err());
        drop(download);
        drop(stale);
        let stale = Storage::open(stale_dir.path()).unwrap();
        let download = stale
            .open_stale_replay_download(carol.peer_id(), admitted.record_hash().unwrap(), &fresh_manifest)
            .unwrap();
        assert_eq!(download.missing_blobs().unwrap(), vec![(large.hash, 128 * 1024)]);
        let (chunk, finished) = storage
            .read_encoded_blob_chunk(world, large, 128 * 1024, usize::try_from(large.encoded_size).unwrap())
            .unwrap();
        assert!(finished);
        download.receive_blob_chunk(large, 128 * 1024, &chunk, true).unwrap();
        download.finalize().unwrap();
        let suffix = stale.verified_stale_replay_suffix(world).unwrap();
        assert_eq!(suffix, storage.list_snapshots(world).unwrap()[1..]);
        for manifest in &suffix {
            verified.verify_manifest(manifest).unwrap();
        }
        assert_eq!(stale.stale_replay_anchor(world).unwrap(), anchor);
        assert!(!stale.has_complete_blob(world, large));
        assert!(!stale.has_complete_blob(world, small));

        // A complete suffix is not an authority grant. An outstanding
        // durable local decision must prevent the publication intent.
        let candidate = stale.stale_publication_candidate(world).unwrap().unwrap();
        assert_eq!(candidate.manifests, storage.list_snapshots(world).unwrap());
        for manifest in &candidate.manifests {
            verified.verify_manifest(manifest).unwrap();
        }
        let metadata = stale.world_dir(world).join("metadata");
        let fence = metadata.join("recovery-promise.postcard");
        std::fs::write(&fence, b"unreadable durable recovery promise").unwrap();
        assert!(stale.begin_verified_stale_publication(&candidate).is_err());
        assert!(!stale.stale_publication_pending(world).unwrap());
        assert_eq!(std::fs::read(&fence).unwrap(), b"unreadable durable recovery promise");
        std::fs::remove_file(&fence).unwrap();

        // Only decisions whose exact signed bytes occur in this certified
        // history can be consumed. A valid but different decision must remain.
        let mut unrelated = phase.clone();
        unrelated.base_snapshot_hash = Hash32([99; 32]);
        unrelated.signature = alice.sign(&unrelated.signing_bytes().unwrap());
        stale.save_transfer_record(&unrelated).unwrap();
        let transfer_path = metadata.join("transfer.postcard");
        assert!(stale.begin_verified_stale_publication(&candidate).is_err());
        assert_eq!(stale.load_transfer_record(world).unwrap(), unrelated);
        std::fs::remove_file(&transfer_path).unwrap();
        stale.save_transfer_record(&phase).unwrap();
        let HistoricalTransitionV1::Recovery(recovery) = &envelope.transitions[1] else {
            panic!("expected recovery certificate");
        };
        let recovery_promise = swarm_storage::DurableRecoveryPromiseV1 {
            ballot: recovery.ballot.clone(),
            vote: recovery.votes[0].clone(),
        };
        let mut conflicting = recovery_promise.clone();
        conflicting.vote.signature[0] ^= 1;
        std::fs::write(&fence, postcard::to_allocvec(&conflicting).unwrap()).unwrap();
        assert!(stale.begin_verified_stale_publication(&candidate).is_err());
        assert!(!stale.stale_publication_pending(world).unwrap());
        std::fs::write(&fence, postcard::to_allocvec(&recovery_promise).unwrap()).unwrap();
        let membership_promise = swarm_storage::DurableMembershipPromiseV1 {
            proposal: admission.proposal.clone(),
            vote: admission.votes[0].clone(),
        };
        let membership_fence = metadata.join("membership-promise.postcard");
        let mut corrupt_bytes = postcard::to_allocvec(&membership_promise).unwrap();
        corrupt_bytes.push(0);
        std::fs::write(&membership_fence, &corrupt_bytes).unwrap();
        assert!(stale.begin_verified_stale_publication(&candidate).is_err());
        assert_eq!(std::fs::read(&membership_fence).unwrap(), corrupt_bytes);
        std::fs::write(&membership_fence, postcard::to_allocvec(&membership_promise).unwrap()).unwrap();
        // A pending certificate can also be the exact final recovery proof.
        // Preserve its canonical bytes in the primary slot before removal.
        let pending_recovery = metadata.join("recovery-certificate-pending.postcard");
        std::fs::write(&pending_recovery, postcard::to_allocvec(recovery.as_ref()).unwrap()).unwrap();

        // The quota also accounts for isolated replay's additional canonical
        // copies. Denial must not install the publication fence prematurely.
        stale.set_world_storage_policy(world, quota_paused).unwrap();
        assert!(stale.begin_verified_stale_publication(&candidate).unwrap_err().is_storage_pressure());
        assert!(!stale.stale_publication_pending(world).unwrap());
        assert!(stale.load_epoch_record(world).is_ok());
        assert_eq!(stale.stale_publication_candidate(world).unwrap(), Some(candidate.clone()));
        stale.set_world_storage_policy(world, Default::default()).unwrap();

        // Simulate process death after durable intent and a subset of the
        // exact canonical writes. No ordinary reader or writer may use
        // this mixed state. Conflicting data cannot be silently replaced.
        stale.begin_verified_stale_publication(&candidate).unwrap();
        assert!(stale.stale_publication_pending(world).unwrap());
        assert!(transfer_path.exists());
        assert!(fence.exists());
        assert!(membership_fence.exists());
        assert!(stale.load_epoch_record(world).is_err());
        assert!(stale.load_membership_record(world).is_err());
        assert!(stale.load_world_descriptor(world).is_err());
        assert!(stale.load_world_config(world).is_err());
        assert!(stale.latest_snapshot(world).is_err());
        assert!(stale.save_epoch_record(&third).is_err());
        assert!(stale.save_membership_record(&admitted).is_err());
        assert!(stale.garbage_collect_blobs(world).is_err());
        let copied_manifest = &candidate.manifests[1];
        std::fs::write(
            stale.world_dir(world).join("snapshots").join(format!("{:020}.postcard", copied_manifest.snapshot_number)),
            postcard::to_allocvec(copied_manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(metadata.join("membership.postcard"), postcard::to_allocvec(&admitted).unwrap()).unwrap();
        let config_path = metadata.join("world-config.postcard");
        let old_config = std::fs::read(&config_path).unwrap();
        std::fs::write(&config_path, b"conflicting control record").unwrap();
        drop(stale);
        let stale = Storage::open(stale_dir.path()).unwrap();
        assert!(publish_stale_replay(&stale, &dave, world).is_err());
        assert!(stale.stale_publication_pending(world).unwrap());
        assert!(stale.load_epoch_record(world).is_err());
        assert_eq!(std::fs::read(&config_path).unwrap(), b"conflicting control record");
        std::fs::write(&config_path, old_config).unwrap();
        // Even if the new epoch has reached disk, the intent is the
        // visibility fence until every other canonical write has finished.
        std::fs::write(metadata.join("epoch.postcard"), postcard::to_allocvec(&third).unwrap()).unwrap();
        assert!(stale.load_epoch_record(world).is_err());
        assert!(stale.latest_snapshot(world).is_err());
        assert_eq!(stale.stale_publication_candidate(world).unwrap(), Some(candidate));
        assert_eq!(publish_stale_replay(&stale, &dave, world).unwrap(), envelope);
        assert!(!stale.stale_publication_pending(world).unwrap());
        assert!(!transfer_path.exists());
        assert!(!fence.exists());
        assert!(!membership_fence.exists());
        assert!(!pending_recovery.exists());
        assert_eq!(stale.load_epoch_record(world).unwrap(), third);
        assert_eq!(stale.load_membership_record(world).unwrap(), admitted);
        assert_eq!(stale.load_world_descriptor(world).unwrap(), descriptor);
        assert_eq!(stale.load_world_config(world).unwrap(), storage.load_world_config(world).unwrap());
        assert_eq!(stale.list_snapshots(world).unwrap(), storage.list_snapshots(world).unwrap());
        assert_eq!(stale.load_recovery_certificate(world).unwrap(), storage.load_recovery_certificate(world).unwrap());
        assert_eq!(stale.load_transfer_phase(&phase, TransferPhase::Committed).unwrap(), phase);
        assert!(stale.load_transfer_record(world).is_err());
        assert!(stale.has_complete_blob(world, large));
        assert!(stale.has_complete_blob(world, small));
        crate::live_authority::verify_playable_epoch(&stale, &third).unwrap();
        assert!(stale.load_staged_stale_replay_proof(world).unwrap().is_none());

        // Build the real storage first-contact path with a signed invite,
        // committed certificate, and descriptor, but no authority epoch.
        let replica_dir = tempfile::tempdir().unwrap();
        let replica = Storage::open(replica_dir.path()).unwrap();
        replica.create_world(&storage.load_world(world).unwrap()).unwrap();
        let mut invite = swarm_protocol::InviteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            display_name: "recovery late join".into(),
            genesis: genesis.clone(),
            inviter_peer_id: carol.peer_id(),
            inviter_public_key: carol.public_key(),
            bootstrap_addrs: Vec::new(),
            expires_unix_ms: u64::MAX,
            nonce: [25; 32],
            signature: Vec::new(),
        };
        carol.sign_invite(&mut invite).unwrap();
        let mut join = swarm_protocol::JoinRequestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            invite,
            joining_member: player,
            nonce: [26; 32],
            signature: Vec::new(),
        };
        dan.sign_join_request(&mut join).unwrap();
        replica.save_pending_join(&join).unwrap();
        let membership_proof = swarm_protocol::DiscoveryMembershipProofV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            genesis: genesis.clone(),
            initial_membership: envelope.membership[0].clone(),
            membership_certificates: envelope.membership_certificates.clone(),
            current_membership: admitted.clone(),
            pending_membership: None,
        };
        crate::live_authority::verify_genesis_anchored_membership(&membership_proof).unwrap();
        replica.archive_join_membership_history(&membership_proof).unwrap();
        replica.save_membership_certificate(&admission).unwrap();
        replica.save_membership_record(&admitted).unwrap();
        replica.save_world_descriptor(&descriptor).unwrap();
        replica.clear_pending_join(world).unwrap();
        let encoded = postcard::to_allocvec(&envelope).unwrap();
        replica.stage_historical_replay_proof(world, admitted.record_hash().unwrap(), &encoded).unwrap();
        let manifests = storage.list_snapshots(world).unwrap();
        for (index, manifest) in manifests.iter().enumerate() {
            verified.verify_manifest(manifest).unwrap();
            for entry in &manifest.entries {
                let blob = &entry.blob;
                if replica.has_complete_blob(world, blob) {
                    continue;
                }
                let (data, finished) = storage
                    .read_encoded_blob_chunk(world, blob, 0, usize::try_from(blob.encoded_size).unwrap())
                    .unwrap();
                assert!(finished);
                replica.receive_blob_chunk(world, blob, 0, &data, true).unwrap();
            }
            replica.finalize_replica(manifest).unwrap();
            assert!(replica.load_epoch_record(world).is_err());
            if index + 1 < manifests.len() {
                assert!(verified.publish(&replica).is_err());
            }
        }
        // Inject an interruption after some historical evidence could
        // already have been written. A conflicting existing config must
        // prevent the final epoch commit, never be replaced silently.
        let path = replica.world_dir(world).join("metadata/world-config.postcard");
        std::fs::write(&path, b"conflicting config").unwrap();
        assert!(verified.publish(&replica).is_err());
        assert!(replica.load_epoch_record(world).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"conflicting config");
        std::fs::remove_file(&path).unwrap();
        drop(replica);
        let replica = Storage::open(replica_dir.path()).unwrap();
        load_staged_replay(&replica, world).unwrap().unwrap().publish(&replica).unwrap();
        assert_eq!(replica.load_epoch_record(world).unwrap(), third);
        assert_eq!(replica.list_snapshots(world).unwrap(), manifests);
        assert_eq!(replica.load_world_config(world).unwrap(), storage.load_world_config(world).unwrap());
        assert_eq!(replica.load_epoch_by_hash(world, first.record_hash().unwrap()).unwrap(), first);
        assert_eq!(replica.load_epoch_by_hash(world, second.record_hash().unwrap()).unwrap(), second);
        assert_eq!(replica.load_membership_by_hash(world, membership2.record_hash().unwrap()).unwrap(), membership2);
        assert_eq!(
            replica.load_recovery_certificate(world).unwrap(),
            storage.load_recovery_certificate(world).unwrap()
        );
        assert!(replica.load_transfer_record(world).is_err(), "retired transfer must not become a live fence");
        assert_eq!(replica.load_transfer_phase(&phase, TransferPhase::Committed).unwrap(), phase);
        crate::live_authority::verify_playable_epoch(&replica, &third).unwrap();
        // The current membership alone cannot replace the old ballot or the
        // intervening signed joint admission certificate.
        let latest = replica.load_membership_certificate_chain(world).unwrap().last().unwrap().clone();
        let cert_path = replica
            .world_dir(world)
            .join("metadata/membership-certificates")
            .join(format!("{:020}.postcard", latest.proposal.proposed.sequence));
        let saved = std::fs::read(&cert_path).unwrap();
        std::fs::remove_file(&cert_path).unwrap();
        assert!(crate::live_authority::verify_playable_epoch(&replica, &third).is_err());
        std::fs::write(cert_path, saved).unwrap();
        crate::live_authority::verify_playable_epoch(&replica, &third).unwrap();
    }

    #[test]
    fn replay_proof_requires_exact_solo_member_sleep_boundary_for_wake() {
        let (_temp, storage, creator, first) = fixture();
        let world = first.world_id;
        let genesis = storage.load_world(world).unwrap().genesis;
        let member = signed_membership(&first, &creator, vec![member(&creator)], None);
        storage.save_membership_record(&member).unwrap();
        let config = initial_config(&storage, &creator, world);
        let checkpoint = storage.latest_snapshot(world).unwrap().unwrap();
        let mut sleep = swarm_protocol::SleepRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            latest_snapshot_hash: checkpoint.manifest_hash().unwrap(),
            epoch: 1,
            fencing_token: 1,
            authority_peer_id: creator.peer_id(),
            authority_public_key: creator.public_key(),
            signature: Vec::new(),
        };
        sleep.signature = creator.sign(&sleep.signing_bytes().unwrap());
        storage.save_sleep_record(&sleep).unwrap();
        let second = successor(&first, &creator);
        let (membership, _) = promote(&storage, &second, &creator, &member, &config);
        storage.clear_sleep_record(world).unwrap();
        let proof = collect_replay_proof(&storage, &creator, world).unwrap();
        assert!(matches!(proof.transitions[0], HistoricalTransitionV1::SoleMemberWake(_)));
        let mut forged = proof.clone();
        if let HistoricalTransitionV1::SoleMemberWake(record) = &mut forged.transitions[0] {
            record.latest_snapshot_hash = Hash32([42; 32]);
            record.signature = creator.sign(&record.signing_bytes().unwrap());
        }
        forged.signature = creator.sign(&forged.signing_bytes().unwrap());
        assert!(verify_replay_proof(forged, &genesis, &membership).is_err());
        assert!(verify_replay_proof(proof, &genesis, &membership).is_ok());
    }

    #[test]
    fn signed_config_chain_survives_authority_change_and_rejects_corrupt_history() {
        let (_temp, storage, creator, epoch) = fixture();
        let world = epoch.world_id;
        let mut first = WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            sequence: 1,
            previous_config_hash: None,
            compatibility: compatibility(),
            visibility: WorldVisibilityV1::Private,
            authority_policy: AuthorityPolicyV1 { allow_solo_advancement: false, preferred_replication_factor: 3 },
            membership_policy: MembershipPolicyV1::InviteOnly,
            presentation: WorldPresentationV1 {
                name: "original".into(),
                description: String::new(),
                tags: Vec::new(),
                icon_hash: None,
                approximate_region: None,
            },
            authority_peer_id: creator.peer_id(),
            authority_public_key: creator.public_key(),
            signature: Vec::new(),
        };
        sign_world_config(&creator, &mut first).unwrap();
        storage.save_world_config(&first).unwrap();
        let next_authority = PeerIdentity::from_secret_bytes([23; 32]);
        storage.save_epoch_record(&successor(&epoch, &next_authority)).unwrap();
        let first_hash = first.config_hash().unwrap();
        let mut second = WorldConfigV1 { sequence: 2, previous_config_hash: Some(first_hash), ..first.clone() };
        second.presentation.name = "successor".into();
        sign_world_config(&next_authority, &mut second).unwrap();
        storage.save_world_config(&second).unwrap();
        assert_eq!(collect_signed_config_history(&storage, world).unwrap(), vec![first.clone(), second.clone()]);
        // The config hash intentionally covers the unsigned canonical data.
        // Replacing only the archived signature must still be rejected.
        let mut corrupt = first;
        corrupt.signature[0] ^= 1;
        std::fs::write(
            storage.world_dir(world).join("metadata/authority-history").join(format!("config-{first_hash}.postcard")),
            postcard::to_allocvec(&corrupt).unwrap(),
        )
        .unwrap();
        assert!(collect_signed_config_history(&storage, world).is_err());
        assert_eq!(storage.load_world_config(world).unwrap(), second);
    }
}
