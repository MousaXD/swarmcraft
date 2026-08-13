use swarm_consensus::simulator::{FailureSimulator, SimError, SimPeer};
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
fn one_thousand_forced_crashes_preserve_single_authority_and_monotonic_fencing() {
    let peers = [PeerId([1; 32]), PeerId([2; 32]), PeerId([3; 32])];
    let initial = Hash32([1; 32]);
    let mut sim = FailureSimulator::new(peer(1, initial), [peer(2, initial), peer(3, initial)], 5);
    let mut now_ms = 0_u64;

    for round in 0_u64..1_000 {
        let old_authority = sim.authority().expect("each completed round must have exactly one authority");
        let old_token = sim.fencing_token();
        let old_epoch = sim.epoch();

        let snapshot_byte = ((round % 251) + 2) as u8;
        let snapshot = Hash32([snapshot_byte; 32]);
        sim.publish_snapshot(old_authority, snapshot).unwrap();
        for replica in peers.iter().copied().filter(|peer| *peer != old_authority) {
            sim.install_snapshot(replica, snapshot).unwrap();
        }

        sim.set_online(old_authority, false).unwrap();
        now_ms = now_ms.saturating_add(5);
        let successor = peers
            .iter()
            .copied()
            .filter(|peer| *peer != old_authority)
            .min()
            .expect("three-peer test always has a successor");

        sim.attempt_takeover(successor, now_ms, 2).unwrap();

        assert_eq!(sim.authority(), Some(successor));
        assert_eq!(sim.epoch(), old_epoch + 1);
        assert_eq!(sim.fencing_token(), old_token + 1);
        assert_eq!(
            sim.renew_authority_lease(old_authority, old_token, now_ms),
            Err(SimError::StaleGeneration)
        );

        sim.set_online(old_authority, true).unwrap();
    }

    assert_eq!(sim.epoch(), 1_001);
    assert_eq!(sim.fencing_token(), 1_001);
}
