//! Establish the initial signed quorum-of-one without granting hosting consent.
use crate::{Storage, StorageError};
use anyhow::{bail, Result};
use swarm_core::{verify_snapshot_signature, PeerIdentity};
use swarm_protocol::{EpochMode, EpochRecordV1, SnapshotManifestV1, PROTOCOL_VERSION};

pub const GENESIS_EPOCH: u64 = 1;

/// Fresh worlds start with a real accepted generation, before anyone may
/// join. This is not a recovery or a writable solo branch: the creator is
/// the entire initial signed voter set. Java still requires local consent,
/// verified artifacts, and the current quorum after membership expands.
pub fn establish_initial_epoch(
    storage: &Storage,
    identity: &PeerIdentity,
    snapshot: &SnapshotManifestV1,
) -> Result<()> {
    let world = snapshot.world_id;
    let metadata = storage.load_world(world)?;
    let descriptor = storage.load_world_descriptor(world)?;
    if metadata.genesis.creator_public_key != identity.public_key()
        || metadata.genesis.initial_membership != [identity.peer_id()]
        || descriptor.members.len() != 1
        || !descriptor.members.iter().any(|member| {
            member.peer_id == identity.peer_id()
                && member.public_key == identity.public_key()
                && member.authority_eligible
                && !member.banned
        })
        || snapshot.snapshot_number != 1
        || snapshot.sequence != 1
        || snapshot.epoch != GENESIS_EPOCH
        || snapshot.previous_snapshot_hash.is_some()
        || snapshot.authority_peer_id != identity.peer_id()
        || snapshot.authority_public_key != identity.public_key()
        || storage.latest_snapshot(world)?.as_ref() != Some(snapshot)
    {
        bail!("initial authority must be bound to the exact signed genesis and first checkpoint");
    }
    match storage.load_epoch_record(world) {
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => bail!("initial authority already exists"),
        Err(error) => return Err(error.into()),
    }
    storage.verify_snapshot(snapshot)?;
    verify_snapshot_signature(snapshot)?;
    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: GENESIS_EPOCH,
        previous_epoch_hash: None,
        base_state_hash: snapshot.state_root,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: 1,
        reason: "signed genesis quorum-of-one".into(),
        signature: Vec::new(),
    };
    epoch.signature = identity.sign(&epoch.signing_bytes()?);
    storage.save_epoch_record(&epoch)?;
    Ok(())
}
