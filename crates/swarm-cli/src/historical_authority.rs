//! Signed history collection and data-only checkpoint replay verification.
//! A signed chain is NOT a live authority grant. Membership, transition
//! proofs, checkpoint boundaries and independent live freshness are required.
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use swarm_consensus::{membership::validate_membership_certificate_shape, validate_recovery_certificate_shape};
use swarm_core::{
    protocol_v2::{verify_recovery_ballot_signature, verify_recovery_vote_signature, verify_world_config_signature},
    verify_membership_signature, verify_signature, verify_snapshot_signature, verify_transfer_signature, PeerIdentity,
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
        let index = usize::try_from(manifest.snapshot_number.checked_sub(1).context("zero snapshot number")?)?;
        let expected = self.proof.checkpoints.get(index).context("checkpoint outside certified replay proof")?;
        let epoch = self
            .proof
            .epochs
            .get(usize::try_from(expected.epoch - 1)?)
            .context("checkpoint references unknown epoch")?;
        let parent = index.checked_sub(1).map(|prior| self.proof.checkpoints[prior].manifest_hash);
        verify_snapshot_signature(manifest)?;
        if manifest.world_id != self.proof.world_id
            || HistoricalCheckpointV1::from_manifest(manifest)? != *expected
            || manifest.previous_snapshot_hash != parent
            || manifest.authority_peer_id != epoch.authority_peer_id
            || manifest.authority_public_key != epoch.authority_public_key
        {
            bail!("signed manifest does not match the certified replay position");
        }
        Ok(())
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
    if proof.world_id != world
        || proof.membership.get(usize::try_from(admission.sequence)?) != Some(admission)
        || !admission.members.iter().any(|member| {
            !member.banned
                && member.authority_eligible
                && member.peer_id == target.authority_peer_id
                && member.public_key == target.authority_public_key
        })
    {
        bail!("first-contact replay does not extend the signed admission through a known authority");
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
    if storage.stale_replay_anchor(world)? != *anchor
        || anchor.epoch.authority_peer_id == local.peer_id()
        || source == local.peer_id()
        || !anchor.membership.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
        || !anchor
            .membership
            .members
            .iter()
            .any(|member| !member.banned && member.authority_eligible && member.peer_id == source)
        || !anchor.is_extended_by(&proof)
    {
        bail!("stale replay is not an exact successor of an accepted non-authority replica");
    }
    let target = proof.membership.last().context("stale replay lacks final membership")?.clone();
    if target.authority_peer_id != source
        || !target.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
    {
        bail!("stale replay source or recipient is not bound to final signed membership");
    }
    let verified = verify_replay_proof(proof, &storage.load_world(world)?.genesis, &target)?;
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
    storage.stage_stale_replay_proof(anchor, verified.proof())?;
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
        || anchor.epoch.authority_peer_id == local.peer_id()
        || target.authority_peer_id == local.peer_id()
        || !anchor.membership.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
        || !anchor
            .membership
            .members
            .iter()
            .any(|member| !member.banned && member.authority_eligible && member.peer_id == target.authority_peer_id)
        || !target.members.iter().any(|member| {
            !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key()
        })
    {
        bail!("stale publication does not bind an accepted non-authority replica to a known successor");
    }
    let verified = verify_replay_proof(proof.clone(), &storage.load_world(world)?.genesis, target)?;
    for manifest in &candidate.manifests {
        verified.verify_manifest(manifest)?;
    }
    storage.begin_verified_stale_publication(&candidate)?;
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
    verify_signature(
        final_epoch.authority_peer_id,
        final_epoch.authority_public_key,
        &proof.signing_bytes()?,
        &proof.signature,
    )?;
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
    Ok(VerifiedReplayProof { proof })
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
    let genesis = storage.load_world(world)?.genesis;
    let epochs = collect_signed_epoch_history(storage, world)?;
    let accepted = epochs.last().context("missing current authority")?;
    if accepted.authority_peer_id != identity.peer_id() || accepted.authority_public_key != identity.public_key() {
        bail!("only the local accepted authority can sign a historical replay proof");
    }
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
    let mut proof = HistoricalReplayProofV1 {
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
    proof.signature = identity.sign(&proof.signing_bytes()?);
    let verified = verify_replay_proof(proof, &genesis, &current_membership)?;
    for manifest in &manifests {
        verified.verify_manifest(manifest)?;
    }
    let proof = verified.proof;
    if storage.load_epoch_record(world)? != *proof.epochs.last().context("missing epoch")?
        || storage.load_membership_record(world)? != current_membership
        || storage.load_world_config(world)? != *proof.configs.last().context("missing config")?
        || storage.latest_snapshot(world)?.as_ref() != manifests.last()
    {
        bail!("canonical history advanced during proof collection; retry");
    }
    Ok(proof)
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
