//! A read-only projection of locally accepted V1 member roles. This is not
//! a live quorum, player login, runtime, or administration authorization.
use anyhow::{bail, Result};
use swarm_core::{verify_signature, PeerIdentity};
use swarm_protocol::{DiscoveryMembershipProofV1, WorldId, PROTOCOL_VERSION};
use swarm_storage::{Storage, StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalMembershipRole {
    Active,
    NotMember,
    Banned,
    Synchronizing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V1RoleProjection {
    pub membership: LocalMembershipRole,
    /// V1 does not have independent player-only or observer membership.
    pub player_member: bool,
    pub replica_member: bool,
    pub voter_member: bool,
    /// Signed eligibility, NOT current hosting consent or live authority.
    pub host_candidate: bool,
    /// Identifies the locally accepted authority, NOT a live admin grant.
    pub locally_accepted_authority: bool,
    pub membership_transition_pending: bool,
}

fn optional<T>(result: Result<T, StorageError>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn inspect_v1_roles(storage: &Storage, local: &PeerIdentity, world: WorldId) -> Result<V1RoleProjection> {
    let metadata = storage.load_world(world)?;
    let pending_join = optional(storage.load_pending_join(world))?;
    let pending_leave = optional(storage.load_pending_leave(world))?;
    let promise = optional(storage.load_membership_promise(world))?;
    let synchronizing = pending_join.is_some()
        || storage.first_contact_publication_started(world)?
        || storage.stale_publication_pending(world)?
        || storage.crash_retirement_pending(world)?;
    if synchronizing {
        return Ok(V1RoleProjection {
            membership: LocalMembershipRole::Synchronizing,
            player_member: false,
            replica_member: false,
            voter_member: false,
            host_candidate: false,
            locally_accepted_authority: false,
            membership_transition_pending: true,
        });
    }
    let current = storage.load_membership_record(world)?;
    let epoch = storage.load_epoch_record(world)?;
    let descriptor = storage.load_world_descriptor(world)?;
    let proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis: metadata.genesis.clone(),
        initial_membership: storage.load_genesis_membership(world)?,
        membership_certificates: storage.load_membership_certificate_chain(world)?,
        current_membership: current.clone(),
        pending_membership: promise.as_ref().map(|value| value.proposal.clone()),
    };
    crate::live_authority::verify_genesis_anchored_membership(&proof)?;
    verify_signature(epoch.authority_peer_id, epoch.authority_public_key, &epoch.signing_bytes()?, &epoch.signature)?;
    if current.epoch != epoch.epoch_number
        || current.authority_peer_id != epoch.authority_peer_id
        || current.authority_public_key != epoch.authority_public_key
        || descriptor.members != current.members
        || descriptor.compatibility_fingerprint != metadata.genesis.compatibility_fingerprint
        || !current.members.iter().any(|member| {
            !member.banned
                && member.authority_eligible
                && member.peer_id == epoch.authority_peer_id
                && member.public_key == epoch.authority_public_key
        })
    {
        bail!("local membership and authority generation are not synchronized");
    }
    // Recovery epochs must retain their original certificate; for a solo
    // epoch the accepted roster remains informational only.
    if epoch.mode != swarm_protocol::EpochMode::Solo {
        crate::live_authority::verify_playable_epoch(storage, &epoch)?;
    }
    let local_member = current.members.iter().find(|member| member.peer_id == local.peer_id());
    if local_member.is_some_and(|member| member.public_key != local.public_key()) {
        bail!("local membership key differs from the device identity");
    }
    let membership = match local_member {
        Some(member) if member.banned => LocalMembershipRole::Banned,
        Some(_) => LocalMembershipRole::Active,
        None => LocalMembershipRole::NotMember,
    };
    let active = membership == LocalMembershipRole::Active;
    let projection = V1RoleProjection {
        membership,
        player_member: active,
        replica_member: active,
        voter_member: active,
        host_candidate: active && local_member.is_some_and(|m| m.authority_eligible),
        locally_accepted_authority: active && epoch.authority_peer_id == local.peer_id(),
        membership_transition_pending: promise.is_some() || pending_leave.is_some(),
    };
    if storage.load_membership_record(world)? != current
        || storage.load_epoch_record(world)? != epoch
        || storage.load_world_descriptor(world)? != descriptor
        || optional(storage.load_membership_promise(world))? != promise
        || optional(storage.load_pending_join(world))? != pending_join
        || optional(storage.load_pending_leave(world))? != pending_leave
        || storage.first_contact_publication_started(world)?
        || storage.stale_publication_pending(world)?
        || storage.crash_retirement_pending(world)?
    {
        bail!("local roles changed during inspection");
    }
    Ok(projection)
}
