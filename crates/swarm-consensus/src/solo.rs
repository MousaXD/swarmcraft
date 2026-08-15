use swarm_protocol::SoloBranchV1;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoloReconciliation {
    Equivalent,
    KeepLocal,
    AdoptRemote,
    Conflict,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SoloHistoryError {
    #[error("solo histories belong to different worlds")]
    DifferentWorlds,
    #[error("solo history is malformed: head precedes base")]
    MalformedHistory,
}

/// Reconcile explicit solo-history ancestry without attempting semantic world merges.
///
/// A branch may be adopted automatically only when the other side is exactly its
/// recorded base, or both sides name the same head. Independently advanced heads
/// are surfaced as a conflict and must be preserved for manual recovery.
pub fn reconcile_solo_history(
    local: &SoloBranchV1,
    remote: &SoloBranchV1,
) -> Result<SoloReconciliation, SoloHistoryError> {
    if local.world_id != remote.world_id {
        return Err(SoloHistoryError::DifferentWorlds);
    }
    if local.head_epoch < local.base_epoch || remote.head_epoch < remote.base_epoch {
        return Err(SoloHistoryError::MalformedHistory);
    }
    if local.head_snapshot_hash == remote.head_snapshot_hash && local.state_hash == remote.state_hash {
        return Ok(SoloReconciliation::Equivalent);
    }
    if local.head_snapshot_hash == remote.base_snapshot_hash {
        return Ok(SoloReconciliation::AdoptRemote);
    }
    if remote.head_snapshot_hash == local.base_snapshot_hash {
        return Ok(SoloReconciliation::KeepLocal);
    }
    Ok(SoloReconciliation::Conflict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{Hash32, PeerId, WorldId, PROTOCOL_VERSION};

    fn branch(base: u8, head: u8, writer: u8, sequence: u64) -> SoloBranchV1 {
        SoloBranchV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            base_snapshot_hash: Hash32([base; 32]),
            base_epoch: 4,
            head_snapshot_hash: Hash32([head; 32]),
            head_epoch: 5,
            head_sequence: sequence,
            state_hash: Hash32([head; 32]),
            authority_peer_id: PeerId([writer; 32]),
            authority_public_key: [writer; 32],
            signature: Vec::new(),
        }
    }

    #[test]
    fn returning_replica_at_solo_base_adopts_solo_head() {
        let bob = branch(1, 1, 2, 10);
        let alice = branch(1, 9, 1, 20);
        assert_eq!(reconcile_solo_history(&bob, &alice).unwrap(), SoloReconciliation::AdoptRemote);
    }

    #[test]
    fn independently_advanced_solo_branches_are_explicit_conflict() {
        let alice = branch(1, 8, 1, 20);
        let bob = branch(1, 9, 2, 19);
        assert_eq!(reconcile_solo_history(&alice, &bob).unwrap(), SoloReconciliation::Conflict);
    }

    #[test]
    fn no_silent_merge_when_branches_have_unrelated_bases() {
        let alice = branch(3, 8, 1, 20);
        let bob = branch(4, 9, 2, 19);
        assert_eq!(reconcile_solo_history(&alice, &bob).unwrap(), SoloReconciliation::Conflict);
    }
}
