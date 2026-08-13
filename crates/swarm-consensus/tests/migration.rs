use std::time::{Duration, Instant};
use swarm_consensus::{
    migration::{evaluate_crash_takeover, AuthorityGeneration, LeaseError, LeaseTracker, ManualTransferState, TakeoverCandidate, TakeoverMode, TakeoverPolicy, TransferError, WorldRuntimeState},
    AuthorityCandidate,
};
use swarm_protocol::{Hash32, PeerId};

fn eligible(peer: u8, epoch: u64, sequence: u64) -> AuthorityCandidate {
    AuthorityCandidate {
        peer_id: PeerId([peer; 32]),
        accepted_epoch: epoch,
        canonical_sequence: sequence,
        snapshot_complete: true,
        compatible: true,
        authority_eligible: true,
        banned: false,
    }
}

#[test]
fn crash_takeover_waits_for_monotonic_lease_and_solo_delay() {
    let start = Instant::now();
    let lease = LeaseTracker::new(5, 9, Duration::from_secs(10), start);
    let candidate = TakeoverCandidate { candidate: eligible(2, 5, 10), snapshot_hash: Hash32([7; 32]), peer_votes: 1 };
    let policy = TakeoverPolicy::default();
    assert_eq!(evaluate_crash_takeover(&lease, &candidate, Hash32([7; 32]), start + Duration::from_secs(9), policy), Err(LeaseError::NotExpired));
    assert_eq!(evaluate_crash_takeover(&lease, &candidate, Hash32([7; 32]), start + Duration::from_secs(20), policy), Err(LeaseError::SoloDelay));
    let generation = evaluate_crash_takeover(&lease, &candidate, Hash32([7; 32]), start + Duration::from_secs(26), policy).unwrap();
    assert_eq!(generation.epoch, 6);
    assert_eq!(generation.fencing_token, 10);
    assert_eq!(generation.mode, TakeoverMode::Solo);
}

#[test]
fn quorum_takeover_can_start_immediately_after_expiry() {
    let start = Instant::now();
    let lease = LeaseTracker::new(2, 4, Duration::from_secs(5), start);
    let candidate = TakeoverCandidate { candidate: eligible(8, 2, 11), snapshot_hash: Hash32([3; 32]), peer_votes: 2 };
    let generation = evaluate_crash_takeover(&lease, &candidate, Hash32([3; 32]), start + Duration::from_secs(5), TakeoverPolicy::default()).unwrap();
    assert_eq!(generation.mode, TakeoverMode::Quorum);
    assert_eq!(generation.epoch, 3);
    assert_eq!(generation.fencing_token, 5);
}

#[test]
fn candidate_must_hold_exact_complete_snapshot() {
    let start = Instant::now();
    let lease = LeaseTracker::new(1, 1, Duration::from_secs(1), start);
    let mut candidate = TakeoverCandidate { candidate: eligible(3, 1, 5), snapshot_hash: Hash32([4; 32]), peer_votes: 2 };
    assert_eq!(evaluate_crash_takeover(&lease, &candidate, Hash32([5; 32]), start + Duration::from_secs(2), TakeoverPolicy::default()), Err(LeaseError::SnapshotNotReady));
    candidate.candidate.snapshot_complete = false;
    candidate.snapshot_hash = Hash32([5; 32]);
    assert_eq!(evaluate_crash_takeover(&lease, &candidate, Hash32([5; 32]), start + Duration::from_secs(2), TakeoverPolicy::default()), Err(LeaseError::SnapshotNotReady));
}

#[test]
fn manual_transfer_requires_target_snapshot_before_commit() {
    let from = PeerId([1; 32]);
    let to = PeerId([2; 32]);
    let mut transfer = ManualTransferState::prepare(from, to, Hash32([9; 32]), 7, 11);
    assert_eq!(transfer.accept(to, Hash32([8; 32])), Err(TransferError::SnapshotNotReady));
    transfer.accept(to, Hash32([9; 32])).unwrap();
    let generation = transfer.commit(from).unwrap();
    assert_eq!(generation.authority_peer_id, to);
    assert_eq!(generation.epoch, 8);
    assert_eq!(generation.fencing_token, 12);
}

#[test]
fn sleeping_world_wakes_only_from_latest_snapshot() {
    let mut state = WorldRuntimeState::Active(AuthorityGeneration {
        authority_peer_id: PeerId([1; 32]),
        epoch: 3,
        fencing_token: 6,
        base_snapshot_hash: Hash32([4; 32]),
        mode: TakeoverMode::Quorum,
    });
    state.sleep(Hash32([7; 32]));
    assert_eq!(state.wake(PeerId([2; 32]), Hash32([6; 32])), Err(LeaseError::SnapshotNotReady));
    let generation = state.wake(PeerId([2; 32]), Hash32([7; 32])).unwrap();
    assert_eq!(generation.epoch, 4);
    assert_eq!(generation.fencing_token, 7);
}
