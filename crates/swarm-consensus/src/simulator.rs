//! Deterministic failure simulation for authority safety invariants.

use crate::FencingState;
use std::collections::{BTreeMap, BTreeSet};
use swarm_protocol::{Hash32, PeerId};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimPeer {
    pub peer_id: PeerId,
    pub online: bool,
    pub complete_snapshot: bool,
    pub compatible: bool,
    pub authority_eligible: bool,
    pub banned: bool,
    pub snapshot_hash: Hash32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimWorldState {
    Active { authority: PeerId },
    Sleeping,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SimError {
    #[error("unknown peer")]
    UnknownPeer,
    #[error("authority lease has not expired")]
    LeaseActive,
    #[error("candidate is not visible to enough peers")]
    NoQuorum,
    #[error("candidate does not hold the latest complete compatible snapshot")]
    InvalidReplica,
    #[error("stale authority generation")]
    StaleGeneration,
}

#[derive(Debug, Clone)]
pub struct FailureSimulator {
    peers: BTreeMap<PeerId, SimPeer>,
    isolated: BTreeSet<PeerId>,
    state: SimWorldState,
    latest_snapshot_hash: Hash32,
    epoch: u64,
    fencing: FencingState,
    lease_expires_ms: u64,
    lease_duration_ms: u64,
    solo_extra_delay_ms: u64,
}

impl FailureSimulator {
    pub fn new(authority: SimPeer, replicas: impl IntoIterator<Item = SimPeer>, lease_duration_ms: u64) -> Self {
        let latest_snapshot_hash = authority.snapshot_hash;
        let authority_id = authority.peer_id;
        let mut peers = BTreeMap::new();
        peers.insert(authority_id, authority);
        for peer in replicas {
            peers.insert(peer.peer_id, peer);
        }
        Self {
            peers,
            isolated: BTreeSet::new(),
            state: SimWorldState::Active { authority: authority_id },
            latest_snapshot_hash,
            epoch: 1,
            fencing: FencingState::new(1),
            lease_expires_ms: lease_duration_ms,
            lease_duration_ms,
            solo_extra_delay_ms: 15_000,
        }
    }

    pub fn state(&self) -> &SimWorldState { &self.state }
    pub fn epoch(&self) -> u64 { self.epoch }
    pub fn fencing_token(&self) -> u64 { self.fencing.accepted_token() }
    pub fn authority(&self) -> Option<PeerId> {
        match self.state { SimWorldState::Active { authority } => Some(authority), SimWorldState::Sleeping => None }
    }

    pub fn set_online(&mut self, peer: PeerId, online: bool) -> Result<(), SimError> {
        self.peers.get_mut(&peer).ok_or(SimError::UnknownPeer)?.online = online;
        if !online && self.peers.values().all(|candidate| !candidate.online) { self.state = SimWorldState::Sleeping; }
        Ok(())
    }

    pub fn set_partitioned(&mut self, peer: PeerId, partitioned: bool) -> Result<(), SimError> {
        if !self.peers.contains_key(&peer) { return Err(SimError::UnknownPeer); }
        if partitioned { self.isolated.insert(peer); } else { self.isolated.remove(&peer); }
        Ok(())
    }

    pub fn corrupt_snapshot(&mut self, peer: PeerId) -> Result<(), SimError> {
        let peer = self.peers.get_mut(&peer).ok_or(SimError::UnknownPeer)?;
        peer.complete_snapshot = false;
        peer.snapshot_hash = Hash32([0xee; 32]);
        Ok(())
    }

    pub fn install_snapshot(&mut self, peer: PeerId, hash: Hash32) -> Result<(), SimError> {
        let peer = self.peers.get_mut(&peer).ok_or(SimError::UnknownPeer)?;
        peer.snapshot_hash = hash;
        peer.complete_snapshot = true;
        Ok(())
    }

    pub fn publish_snapshot(&mut self, authority: PeerId, hash: Hash32) -> Result<(), SimError> {
        if self.authority() != Some(authority) { return Err(SimError::StaleGeneration); }
        let peer = self.peers.get_mut(&authority).ok_or(SimError::UnknownPeer)?;
        peer.snapshot_hash = hash;
        peer.complete_snapshot = true;
        self.latest_snapshot_hash = hash;
        Ok(())
    }

    pub fn renew_authority_lease(&mut self, authority: PeerId, token: u64, now_ms: u64) -> Result<(), SimError> {
        if self.authority() != Some(authority) || self.fencing.validate_write(token).is_err() {
            return Err(SimError::StaleGeneration);
        }
        self.lease_expires_ms = now_ms.saturating_add(self.lease_duration_ms);
        Ok(())
    }

    pub fn attempt_takeover(&mut self, candidate: PeerId, now_ms: u64, quorum_size: usize) -> Result<(), SimError> {
        if now_ms < self.lease_expires_ms { return Err(SimError::LeaseActive); }
        let candidate_state = self.peers.get(&candidate).ok_or(SimError::UnknownPeer)?;
        if !candidate_state.online
            || self.isolated.contains(&candidate)
            || !candidate_state.complete_snapshot
            || !candidate_state.compatible
            || !candidate_state.authority_eligible
            || candidate_state.banned
            || candidate_state.snapshot_hash != self.latest_snapshot_hash
        {
            return Err(SimError::InvalidReplica);
        }
        let visible_votes = self.peers.values().filter(|peer| {
            peer.online && !self.isolated.contains(&peer.peer_id) && peer.snapshot_hash == self.latest_snapshot_hash && peer.complete_snapshot
        }).count();
        if visible_votes < quorum_size && now_ms < self.lease_expires_ms.saturating_add(self.solo_extra_delay_ms) {
            return Err(SimError::NoQuorum);
        }
        self.epoch += 1;
        self.fencing.advance(self.fencing.accepted_token() + 1).expect("simulator only increments tokens");
        self.state = SimWorldState::Active { authority: candidate };
        self.lease_expires_ms = now_ms.saturating_add(self.lease_duration_ms);
        Ok(())
    }

    pub fn wake(&mut self, candidate: PeerId, now_ms: u64) -> Result<(), SimError> {
        if self.state != SimWorldState::Sleeping { return Err(SimError::StaleGeneration); }
        let candidate_state = self.peers.get(&candidate).ok_or(SimError::UnknownPeer)?;
        if !candidate_state.online || !candidate_state.complete_snapshot || candidate_state.snapshot_hash != self.latest_snapshot_hash {
            return Err(SimError::InvalidReplica);
        }
        self.epoch += 1;
        self.fencing.advance(self.fencing.accepted_token() + 1).expect("simulator only increments tokens");
        self.state = SimWorldState::Active { authority: candidate };
        self.lease_expires_ms = now_ms.saturating_add(self.lease_duration_ms);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: u8, hash: Hash32) -> SimPeer {
        SimPeer { peer_id: PeerId([id; 32]), online: true, complete_snapshot: true, compatible: true, authority_eligible: true, banned: false, snapshot_hash: hash }
    }

    #[test]
    fn partition_and_crash_never_create_two_authorities() {
        let hash = Hash32([7; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 5_000);
        sim.set_partitioned(PeerId([1; 32]), true).unwrap();
        assert_eq!(sim.attempt_takeover(PeerId([2; 32]), 4_999, 2), Err(SimError::LeaseActive));
        sim.attempt_takeover(PeerId([2; 32]), 5_000, 2).unwrap();
        assert_eq!(sim.authority(), Some(PeerId([2; 32])));
        assert_eq!(sim.epoch(), 2);
        assert_eq!(sim.fencing_token(), 2);
        assert_eq!(sim.renew_authority_lease(PeerId([1; 32]), 1, 5_001), Err(SimError::StaleGeneration));
        assert_eq!(sim.authority(), Some(PeerId([2; 32])));
    }

    #[test]
    fn divergent_or_corrupt_replica_cannot_take_over() {
        let hash = Hash32([9; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 1_000);
        sim.corrupt_snapshot(PeerId([2; 32])).unwrap();
        assert_eq!(sim.attempt_takeover(PeerId([2; 32]), 1_000, 2), Err(SimError::InvalidReplica));
        sim.install_snapshot(PeerId([2; 32]), hash).unwrap();
        sim.attempt_takeover(PeerId([2; 32]), 1_000, 2).unwrap();
        assert_eq!(sim.authority(), Some(PeerId([2; 32])));
    }

    #[test]
    fn all_offline_sleeps_and_latest_replica_wakes() {
        let hash = Hash32([4; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash)], 1_000);
        sim.set_online(PeerId([1; 32]), false).unwrap();
        sim.set_online(PeerId([2; 32]), false).unwrap();
        assert_eq!(sim.state(), &SimWorldState::Sleeping);
        sim.set_online(PeerId([2; 32]), true).unwrap();
        sim.wake(PeerId([2; 32]), 50_000).unwrap();
        assert_eq!(sim.authority(), Some(PeerId([2; 32])));
        assert_eq!(sim.epoch(), 2);
    }
}
