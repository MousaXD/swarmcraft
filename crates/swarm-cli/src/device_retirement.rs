//! Explicit lost-device retirement. A staged vote is not a committed ban.
use anyhow::{bail, Context, Result};
use swarm_consensus::{membership_vote_for, quorum_size, validate_membership_proposal_shape};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{EpochMode, Hash32, MembershipProposalV1, PeerId, WorldId};
use swarm_storage::{MembershipPromiseResult, Storage, StorageError};

#[derive(Debug, Clone, Copy)]
pub struct RetirementRequest {
    pub world: WorldId,
    pub peer: PeerId,
    pub membership_hash: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub checkpoint_hash: Hash32,
}

/// The local operator selects an exact signed sleeping boundary. Preserve
/// the current authority and all checkpoint bytes. Existing quorum machinery
/// collects the votes and publishes the joint membership certificate.
pub fn stage_device_retirement(
    storage: &Storage,
    identity: &PeerIdentity,
    request: RetirementRequest,
) -> Result<MembershipProposalV1> {
    let world = request.world;
    let paths = DataPaths::from_root(storage.root());
    let source =
        crate::historical_authority::verify_local_recovery_source(storage, identity, world, request.checkpoint_hash)?;
    if source.epoch.mode == EpochMode::Solo
        || source.epoch.epoch_number != request.epoch
        || source.epoch.fencing_token != request.fencing_token
        || source.epoch.authority_peer_id != identity.peer_id()
        || source.epoch.authority_public_key != identity.public_key()
        || source.membership.record_hash()? != request.membership_hash
        || source.membership.epoch != source.epoch.epoch_number
        || source.membership.authority_peer_id != identity.peer_id()
        || source.membership.authority_public_key != identity.public_key()
    {
        bail!("retirement requires the explicitly selected current quorum authority and signed membership");
    }
    if request.peer == identity.peer_id() {
        bail!("transfer or recover authority before retiring the current authority device");
    }
    let sleep = crate::launch_guard::load_sleep_record_fail_closed(storage, world)?
        .context("stop the world at a verified checkpoint before retiring a device")?;
    if sleep.latest_snapshot_hash != request.checkpoint_hash
        || sleep.epoch != request.epoch
        || sleep.fencing_token != request.fencing_token
        || sleep.authority_peer_id != identity.peer_id()
        || sleep.authority_public_key != identity.public_key()
        || source.manifest.epoch != request.epoch
        || source.manifest.authority_peer_id != identity.peer_id()
        || source.manifest.authority_public_key != identity.public_key()
    {
        bail!("retirement sleep boundary does not match the exact current checkpoint and authority");
    }
    for (description, present) in [
        ("pending join", optional_present(storage.load_pending_join(world))?),
        ("pending leave", optional_present(storage.load_pending_leave(world))?),
    ] {
        if present {
            bail!("cannot retire a device during {description}");
        }
    }
    let mut proposed = source.membership.clone();
    let old_active = proposed.members.iter().filter(|m| !m.banned).count();
    let member = proposed
        .members
        .iter_mut()
        .find(|m| m.peer_id == request.peer)
        .context("device is not in the accepted signed membership")?;
    if member.banned {
        bail!("device is already banned in the accepted signed membership");
    }
    // A lost device is not assumed to be available to vote. If the surviving
    // roster cannot possibly satisfy the old majority, refuse to strand it.
    if old_active.saturating_sub(1) < quorum_size(old_active) {
        bail!("surviving devices cannot satisfy the old membership quorum without the lost device; preserve the original history");
    }
    member.banned = true;
    member.authority_eligible = false;
    proposed.sequence = proposed.sequence.checked_add(1).context("membership sequence exhausted")?;
    proposed.previous_membership_hash = Some(request.membership_hash);
    identity.sign_membership(&mut proposed)?;
    let proposal = MembershipProposalV1 { previous: source.membership.clone(), proposed };
    validate_membership_proposal_shape(&proposal)?;
    if proposal.retired_device() != Some(request.peer) {
        bail!("invalid retirement proposal shape");
    }
    let mut vote = membership_vote_for(&proposal, identity.peer_id(), identity.public_key())?;
    vote.signature = identity.sign(&vote.signing_bytes()?);
    // Exclude every local supported runtime launch and reject orphaned
    // process records. This guard also retires stale cached route readiness.
    let _slot = crate::migration::acquire_retired_replay_guard(&paths, world)?;
    crate::migration::ensure_no_v2_source_transition(&paths, storage, &source.epoch)?;
    source.ensure_unchanged(storage)?;
    match storage.promise_device_retirement(&source.epoch, &sleep, &proposal, &vote)? {
        MembershipPromiseResult::Accepted | MembershipPromiseResult::Idempotent => Ok(proposal),
        MembershipPromiseResult::Rejected => {
            bail!("retirement conflicts with a durable membership or generation change")
        }
    }
}

fn optional_present<T>(value: Result<T, StorageError>) -> Result<bool> {
    match value {
        Ok(_) => Ok(true),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
