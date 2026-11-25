//! Read-only review of a prospective, distinct V2 realm against independently
//! verified local V1 ancestry. No writes, cutover lock, activation, or live
//! V2 wire registration are performed. A successful review is not a promise
//! that the V1 head will remain unchanged after the review.
use anyhow::{bail, Context, Result};
use swarm_consensus::v2_migration_quorum::{
    verify_v2_migration_votes, AcceptedSourceV1Pins, AcceptedVoterUniverseV1, VerifiedV2MigrationVoteSet,
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{
    EpochMode, Hash32, HostingMigrationSourceContextV2, HostingMigrationVoterSetV2, SignedHostingMigrationProposalV2,
    SignedHostingMigrationQuorumV2, SignedRealmHostingPolicyV2, SleepRecordV1, SourceHistoryAnchorV2, WorldId,
};
use swarm_storage::{Storage, StorageError};

use crate::historical_authority::{verify_local_recovery_source, LocalRecoverySource};

/// The stable path requires no pending membership or recovery promise. Only
/// NotFound proves absence; malformed or inaccessible records deny review.
fn absent<T>(result: Result<T, StorageError>, description: &str) -> Result<()> {
    match result {
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context(format!("cannot establish absence of {description}")),
        Ok(_) => bail!("source has unresolved {description}"),
    }
}

fn verify_quiescent_source(
    paths: &DataPaths,
    storage: &Storage,
    source: &LocalRecoverySource,
) -> Result<SleepRecordV1> {
    let world = source.manifest.world_id;
    if source.epoch.mode == EpochMode::Solo
        || storage.first_contact_publication_started(world)?
        || storage.stale_publication_pending(world)?
        || storage.crash_retirement_pending(world)?
    {
        bail!("source has solo or incomplete canonical history");
    }
    absent(storage.load_membership_promise(world), "membership promise")?;
    absent(storage.load_recovery_promise(world), "recovery promise")?;
    absent(storage.load_pending_join(world), "pending join")?;
    absent(storage.load_pending_leave(world), "pending leave")?;
    crate::migration::ensure_no_v2_source_transition(paths, storage, &source.epoch)?;
    let sleep = crate::launch_guard::load_sleep_record_fail_closed(storage, world)?
        .context("source must have a verified durable sleep boundary")?;
    if sleep.world_id != world
        || sleep.latest_snapshot_hash != source.manifest.manifest_hash()?
        || sleep.epoch != source.epoch.epoch_number
        || sleep.fencing_token != source.epoch.fencing_token
        || sleep.authority_peer_id != source.epoch.authority_peer_id
        || sleep.authority_public_key != source.epoch.authority_public_key
        || source.manifest.epoch != source.epoch.epoch_number
        || source.manifest.authority_peer_id != source.epoch.authority_peer_id
        || source.manifest.authority_public_key != source.epoch.authority_public_key
    {
        bail!("source sleep boundary is not bound to its exact final checkpoint and current authority fence");
    }
    source.ensure_unchanged(storage)?;
    Ok(sleep)
}

/// Verify a stable V1 cutover candidate. The source world and checkpoint are
/// independently selected by the local operator, not taken from the proposal.
/// Joint-certificate verification is provided by the consensus adapter, but
/// this preflight deliberately refuses any unresolved V1 transition. Complete
/// the V1 transition first; its newly committed roster is then stable.
///
/// There is no publication method on the returned verified vote set.
/// A future durable coordinator MUST rerun these checks under its transaction
/// and obtain a fresh migration fence; this is neither a fence nor a lease.
pub fn review_stable_v2_migration(
    storage: &Storage,
    local: &PeerIdentity,
    world: WorldId,
    checkpoint: Hash32,
    policy: &SignedRealmHostingPolicyV2,
    proposal: &SignedHostingMigrationProposalV2,
    quorum: &SignedHostingMigrationQuorumV2,
) -> Result<VerifiedV2MigrationVoteSet> {
    let paths = DataPaths::from_root(storage.root());
    let source = verify_local_recovery_source(storage, local, world, checkpoint)?;
    let sleep = verify_quiescent_source(&paths, storage, &source)?;
    let genesis = storage.load_world(world)?.genesis;
    let anchor = SourceHistoryAnchorV2 {
        source_world_id: world,
        source_config_sequence: source.config.sequence,
        source_config_hash: source.config.config_hash()?,
        source_membership_sequence: source.membership.sequence,
        source_membership_hash: source.membership.record_hash()?,
        source_snapshot_hash: checkpoint,
        source_epoch: source.epoch.epoch_number,
        source_fencing_token: source.epoch.fencing_token,
    };
    let pins = AcceptedSourceV1Pins {
        anchor,
        authority_peer_id: source.epoch.authority_peer_id,
        authority_public_key: source.epoch.authority_public_key,
        voters: AcceptedVoterUniverseV1::Stable,
    };
    let context = HostingMigrationSourceContextV2 {
        genesis: &genesis,
        config: &source.config,
        current: &source.membership,
        voter_set: HostingMigrationVoterSetV2::AcceptedStable,
        initial_policy: policy,
        proposal,
    };
    let verified = verify_v2_migration_votes(pins, &context, quorum, None)?;
    if verify_quiescent_source(&paths, storage, &source)? != sleep {
        bail!("source sleep boundary changed during review");
    }
    if storage.load_world(world)?.genesis != genesis {
        bail!("source genesis changed during review");
    }
    Ok(verified)
}
