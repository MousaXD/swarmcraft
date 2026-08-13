use swarm_consensus::simulator::{FailureSimulator, SimError, SimPeer, SimWorldState};
use swarm_protocol::{Hash32, PeerId};

fn peer(id: u8, snapshot: Hash32) -> SimPeer {
    SimPeer {
        peer_id: PeerId([id; 32]),
        online: true,
        complete_snapshot: true,
        compatible: true,
        authority_eligible: true,
        banned: false,
        snapshot_hash: snapshot,
    }
}

#[test]
fn preview_three_peer_failover_sleep_and_wake_flow() {
    let a = PeerId([1; 32]);
    let b = PeerId([2; 32]);
    let c = PeerId([3; 32]);
    let snapshot_one = Hash32([0x11; 32]);
    let snapshot_two = Hash32([0x22; 32]);

    let mut sim = FailureSimulator::new(peer(1, snapshot_one), [peer(2, snapshot_one), peer(3, snapshot_one)], 5_000);
    assert_eq!(sim.authority(), Some(a));

    sim.set_online(a, false).unwrap();
    assert_eq!(sim.attempt_takeover(b, 4_999, 2), Err(SimError::LeaseActive));
    sim.attempt_takeover(b, 5_000, 2).unwrap();
    assert_eq!(sim.authority(), Some(b));
    assert_eq!(sim.epoch(), 2);
    assert_eq!(sim.fencing_token(), 2);
    assert_eq!(sim.renew_authority_lease(a, 1, 5_001), Err(SimError::StaleGeneration));

    sim.publish_snapshot(b, snapshot_two).unwrap();
    sim.install_snapshot(c, snapshot_two).unwrap();

    sim.set_online(a, true).unwrap();
    assert_eq!(sim.attempt_takeover(a, 10_000, 2), Err(SimError::AuthorityVisible));
    sim.install_snapshot(a, snapshot_two).unwrap();

    sim.set_online(a, false).unwrap();
    sim.set_online(b, false).unwrap();
    sim.set_online(c, false).unwrap();
    assert_eq!(sim.state(), &SimWorldState::Sleeping);

    sim.set_online(c, true).unwrap();
    sim.wake(c, 50_000).unwrap();
    assert_eq!(sim.authority(), Some(c));
    assert_eq!(sim.epoch(), 3);
    assert_eq!(sim.fencing_token(), 3);

    sim.set_online(b, true).unwrap();
    assert_eq!(sim.attempt_takeover(b, 55_000, 3), Err(SimError::AuthorityVisible));
    assert_eq!(sim.authority(), Some(c));
}
