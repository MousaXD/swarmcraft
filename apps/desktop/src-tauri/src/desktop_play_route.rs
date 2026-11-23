//! Desktop Play routing for a single-player world versus multiplayer.
//!
//! The normal Play button historically spawned a second `swarmcraft-runtime`
//! process. That direct host is deliberately not registered in the daemon's
//! live Fabric IPC registry and therefore cannot authorize a signed P0 game
//! route. Multi-member Play MUST use the daemon-owned supervisor and its
//! authenticated Fabric session instead; only an unambiguously local solo
//! world can retain the original direct-host path.

use std::io::ErrorKind;
use swarm_core::{verify_membership_signature, verify_signature, DataPaths, PeerIdentity};
use swarm_protocol::{EpochMode, MembershipRecordV1, PeerId, WorldDescriptorV1, WorldId};
use swarm_storage::{Storage, StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlayRouteMode {
    SoloDirect,
    DaemonSignedMultiplayer,
}

fn classify(
    descriptor: &WorldDescriptorV1,
    signed: &MembershipRecordV1,
    local_peer: PeerId,
    local_public_key: &[u8; 32],
    epoch_mode: Option<EpochMode>,
) -> Result<PlayRouteMode, String> {
    if descriptor.world_id != signed.world_id {
        return Err("World membership and runtime identity disagree".into());
    }
    verify_membership_signature(signed).map_err(|_| "Cannot verify current signed world membership")?;
    let mut unsigned_members = descriptor.members.clone();
    let mut signed_members = signed.members.clone();
    unsigned_members.sort_by_key(|member| member.peer_id);
    signed_members.sort_by_key(|member| member.peer_id);
    if unsigned_members != signed_members {
        return Err("World descriptor does not match signed membership; refusing direct runtime launch".into());
    }
    let member = signed_members
        .iter()
        .find(|member| member.peer_id == local_peer)
        .ok_or("This device is not a signed world member")?;
    if member.banned || member.public_key != *local_public_key {
        return Err("This device's signed world membership is not eligible for Play".into());
    }
    // Even a single remaining active voter in a previously multi-member
    // world must not bypass quorum/recovery via the direct sidecar. Banned
    // historical participants still count for this conservative boundary.
    if signed_members.len() == 1
        && member.authority_eligible
        && signed.authority_peer_id == local_peer
        && signed.authority_public_key == *local_public_key
        && matches!(epoch_mode, None | Some(EpochMode::Solo))
    {
        Ok(PlayRouteMode::SoloDirect)
    } else {
        Ok(PlayRouteMode::DaemonSignedMultiplayer)
    }
}

pub(super) fn current_play_route_mode(world: WorldId) -> Result<PlayRouteMode, String> {
    let paths = DataPaths::discover().map_err(|_| "SwarmCraft data directory unavailable")?;
    let storage = Storage::open(paths.root.clone()).map_err(|_| "Cannot read canonical local worlds")?;
    storage.load_world(world).map_err(|_| "Selected world does not exist locally")?;
    let identity = PeerIdentity::load_or_create(&paths).map_err(|_| "Local cryptographic identity unavailable")?;
    let descriptor = storage.load_world_descriptor(world).map_err(|_| "Cannot read canonical world descriptor")?;
    let membership =
        storage.load_membership_record(world).map_err(|_| "Cannot read current signed world membership")?;
    let epoch_mode = match storage.load_epoch_record(world) {
        Ok(epoch) => {
            if epoch.world_id != world {
                return Err("Accepted epoch does not belong to the selected world".into());
            }
            verify_signature(
                epoch.authority_peer_id,
                epoch.authority_public_key,
                &epoch.signing_bytes().map_err(|_| "Accepted epoch is invalid")?,
                &epoch.signature,
            )
            .map_err(|_| "Accepted epoch is not signed by its claimed authority")?;
            Some(epoch.mode)
        }
        Err(StorageError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => None,
        Err(_) => return Err("Accepted epoch is corrupt or unreadable; refusing direct runtime launch".into()),
    };
    classify(&descriptor, &membership, identity.peer_id(), &identity.public_key(), epoch_mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{Hash32, WorldMemberV1, PROTOCOL_VERSION};

    fn signed_world() -> (PeerIdentity, WorldDescriptorV1, MembershipRecordV1) {
        let identity = PeerIdentity::generate();
        let world = WorldId([11; 32]);
        let member = WorldMemberV1 {
            peer_id: identity.peer_id(),
            public_key: identity.public_key(),
            authority_eligible: true,
            banned: false,
        };
        let descriptor = WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: Hash32([3; 32]),
            members: vec![member.clone()],
            preferred_replication_factor: 2,
        };
        let mut membership = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: 0,
            sequence: 0,
            previous_membership_hash: None,
            members: vec![member],
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        identity.sign_membership(&mut membership).unwrap();
        (identity, descriptor, membership)
    }

    #[test]
    fn only_signed_unmodified_local_solo_world_keeps_the_direct_runtime_path() {
        let (identity, descriptor, membership) = signed_world();
        let mode = |epoch| classify(&descriptor, &membership, identity.peer_id(), &identity.public_key(), epoch);
        assert_eq!(mode(None).unwrap(), PlayRouteMode::SoloDirect);
        assert_eq!(mode(Some(EpochMode::Solo)).unwrap(), PlayRouteMode::SoloDirect);
        assert_eq!(mode(Some(EpochMode::Quorum)).unwrap(), PlayRouteMode::DaemonSignedMultiplayer);
        assert_eq!(mode(Some(EpochMode::Recovery)).unwrap(), PlayRouteMode::DaemonSignedMultiplayer);
    }

    #[test]
    fn wrong_signatures_modified_membership_and_disallowed_local_identity_fail_closed() {
        let (identity, mut descriptor, mut signed) = signed_world();
        let stranger = PeerIdentity::generate();
        assert!(classify(&descriptor, &signed, stranger.peer_id(), &stranger.public_key(), None).is_err());
        signed.members[0].banned = true;
        assert!(classify(&descriptor, &signed, identity.peer_id(), &identity.public_key(), None).is_err());
        signed.members[0].banned = false;
        descriptor.members[0].authority_eligible = false;
        assert!(classify(&descriptor, &signed, identity.peer_id(), &identity.public_key(), None).is_err());
    }

    #[test]
    fn adding_a_signed_peer_forces_daemon_owned_runtime_even_with_one_eligible_host() {
        let (identity, mut descriptor, mut membership) = signed_world();
        let second = PeerIdentity::generate();
        membership.members.push(WorldMemberV1 {
            peer_id: second.peer_id(),
            public_key: second.public_key(),
            authority_eligible: false,
            banned: true,
        });
        membership.members.sort_by_key(|member| member.peer_id);
        descriptor.members = membership.members.clone();
        identity.sign_membership(&mut membership).unwrap();
        assert_eq!(
            classify(&descriptor, &membership, identity.peer_id(), &identity.public_key(), None).unwrap(),
            PlayRouteMode::DaemonSignedMultiplayer,
        );
    }
}
