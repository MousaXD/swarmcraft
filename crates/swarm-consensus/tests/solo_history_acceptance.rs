use swarm_consensus::{reconcile_solo_history, SoloReconciliation};
use swarm_protocol::{Hash32, PeerId, SoloBranchV1, WorldId, PROTOCOL_VERSION};

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
fn returning_replica_accepts_compatible_solo_history() {
    let replica = branch(1, 1, 2, 10);
    let solo_authority = branch(1, 9, 1, 20);
    assert_eq!(reconcile_solo_history(&replica, &solo_authority).unwrap(), SoloReconciliation::AdoptRemote);
}

#[test]
fn competing_solo_histories_are_detected_instead_of_merged() {
    let alice = branch(1, 8, 1, 20);
    let bob = branch(1, 9, 2, 19);
    assert_eq!(reconcile_solo_history(&alice, &bob).unwrap(), SoloReconciliation::Conflict);
}
