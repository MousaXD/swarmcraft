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
    Recovering { candidate: PeerId },
    Sleeping,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SimError {
    #[error("unknown peer")]
    UnknownPeer,
    #[error("authority lease has not expired")]
    LeaseActive,
    #[error("accepted authority is still visible")]
    AuthorityVisible,
    #[error("candidate is not visible to enough peers")]
    NoQuorum,
    #[error("candidate does not hold the latest complete compatible snapshot")]
    InvalidReplica,
    #[error("another deterministic recovery candidate already owns this generation")]
    RecoveryInProgress,
    #[error("candidate is not the deterministic recovery winner")]
    WrongCandidate,
    #[error("recovery phase is not ready")]
    RecoveryNotReady,
    #[error("stale authority generation")]
    StaleGeneration,
}

#[derive(Debug, Clone)]
struct RecoveryProgress {
    candidate: PeerId,
    quorum_size: usize,
    reservation_acks: BTreeSet<PeerId>,
    epoch_acks: BTreeSet<PeerId>,
    lease_acks: BTreeSet<PeerId>,
    epoch_committed: bool,
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
    recovery: Option<RecoveryProgress>,
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
            recovery: None,
        }
    }

    pub fn state(&self) -> &SimWorldState {
        &self.state
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn fencing_token(&self) -> u64 {
        self.fencing.accepted_token()
    }

    pub fn authority(&self) -> Option<PeerId> {
        match self.state {
            SimWorldState::Active { authority } => Some(authority),
            SimWorldState::Recovering { .. } | SimWorldState::Sleeping => None,
        }
    }

    pub fn recovery_candidate(&self) -> Option<PeerId> {
        self.recovery.as_ref().map(|recovery| recovery.candidate)
    }

    pub fn set_online(&mut self, peer: PeerId, online: bool) -> Result<(), SimError> {
        self.peers.get_mut(&peer).ok_or(SimError::UnknownPeer)?.online = online;
        if !online && self.peers.values().all(|candidate| !candidate.online) {
            self.state = SimWorldState::Sleeping;
            self.recovery = None;
        }
        Ok(())
    }

    pub fn set_partitioned(&mut self, peer: PeerId, partitioned: bool) -> Result<(), SimError> {
        if !self.peers.contains_key(&peer) {
            return Err(SimError::UnknownPeer);
        }
        if partitioned {
            self.isolated.insert(peer);
        } else {
            self.isolated.remove(&peer);
        }
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
        if self.authority() != Some(authority) {
            return Err(SimError::StaleGeneration);
        }
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

    pub fn begin_recovery(&mut self, candidate: PeerId, now_ms: u64, quorum_size: usize) -> Result<(), SimError> {
        if now_ms < self.lease_expires_ms {
            return Err(SimError::LeaseActive);
        }
        if self.recovery.is_some() {
            return Err(SimError::RecoveryInProgress);
        }
        let old_authority = match self.state {
            SimWorldState::Active { authority } => authority,
            SimWorldState::Recovering { .. } => return Err(SimError::RecoveryInProgress),
            SimWorldState::Sleeping => return Err(SimError::StaleGeneration),
        };
        if self.is_visible(old_authority) {
            return Err(SimError::AuthorityVisible);
        }
        self.validate_candidate(candidate)?;
        let visible = self.visible_canonical_peers();
        if visible.len() < quorum_size {
            return Err(SimError::NoQuorum);
        }
        let winner = visible
            .iter()
            .copied()
            .filter(|peer| self.peers.get(peer).is_some_and(|state| state.authority_eligible && !state.banned))
            .min()
            .ok_or(SimError::NoQuorum)?;
        if candidate != winner {
            return Err(SimError::WrongCandidate);
        }
        let mut reservation_acks = BTreeSet::new();
        reservation_acks.insert(candidate);
        self.recovery = Some(RecoveryProgress {
            candidate,
            quorum_size,
            reservation_acks,
            epoch_acks: BTreeSet::new(),
            lease_acks: BTreeSet::new(),
            epoch_committed: false,
        });
        Ok(())
    }

    pub fn acknowledge_reservation(&mut self, peer: PeerId) -> Result<(), SimError> {
        self.validate_recovery_ack_peer(peer)?;
        self.recovery.as_mut().ok_or(SimError::RecoveryNotReady)?.reservation_acks.insert(peer);
        Ok(())
    }

    pub fn commit_recovery_epoch(&mut self) -> Result<(), SimError> {
        let recovery = self.recovery.as_mut().ok_or(SimError::RecoveryNotReady)?;
        if recovery.reservation_acks.len() < recovery.quorum_size {
            return Err(SimError::NoQuorum);
        }
        if !recovery.epoch_committed {
            self.epoch = self.epoch.saturating_add(1);
            self.fencing
                .advance(self.fencing.accepted_token().saturating_add(1))
                .expect("simulator only increments fencing tokens");
            recovery.epoch_committed = true;
            recovery.epoch_acks.insert(recovery.candidate);
            self.state = SimWorldState::Recovering { candidate: recovery.candidate };
        }
        Ok(())
    }

    pub fn acknowledge_epoch(&mut self, peer: PeerId) -> Result<(), SimError> {
        self.validate_recovery_ack_peer(peer)?;
        let recovery = self.recovery.as_mut().ok_or(SimError::RecoveryNotReady)?;
        if !recovery.epoch_committed {
            return Err(SimError::RecoveryNotReady);
        }
        recovery.epoch_acks.insert(peer);
        Ok(())
    }

    pub fn acknowledge_live_lease(&mut self, peer: PeerId) -> Result<(), SimError> {
        self.validate_recovery_ack_peer(peer)?;
        let recovery = self.recovery.as_mut().ok_or(SimError::RecoveryNotReady)?;
        if !recovery.epoch_committed || recovery.epoch_acks.len() < recovery.quorum_size {
            return Err(SimError::RecoveryNotReady);
        }
        recovery.lease_acks.insert(peer);
        Ok(())
    }

    pub fn activate_recovery(&mut self, now_ms: u64) -> Result<(), SimError> {
        let recovery = self.recovery.as_ref().ok_or(SimError::RecoveryNotReady)?;
        if !recovery.epoch_committed
            || recovery.epoch_acks.len() < recovery.quorum_size
            || recovery.lease_acks.len().saturating_add(1) < recovery.quorum_size
        {
            return Err(SimError::NoQuorum);
        }
        let candidate = recovery.candidate;
        self.state = SimWorldState::Active { authority: candidate };
        self.lease_expires_ms = now_ms.saturating_add(self.lease_duration_ms);
        self.recovery = None;
        Ok(())
    }

    pub fn attempt_takeover(&mut self, candidate: PeerId, now_ms: u64, quorum_size: usize) -> Result<(), SimError> {
        self.begin_recovery(candidate, now_ms, quorum_size)?;
        let visible = self.visible_canonical_peers();
        for peer in visible.iter().copied().filter(|peer| *peer != candidate) {
            self.acknowledge_reservation(peer)?;
        }
        self.commit_recovery_epoch()?;
        for peer in visible.iter().copied().filter(|peer| *peer != candidate) {
            self.acknowledge_epoch(peer)?;
        }
        for peer in visible.into_iter().filter(|peer| *peer != candidate) {
            self.acknowledge_live_lease(peer)?;
        }
        self.activate_recovery(now_ms)
    }

    pub fn wake(&mut self, candidate: PeerId, now_ms: u64) -> Result<(), SimError> {
        if self.state != SimWorldState::Sleeping {
            return Err(SimError::StaleGeneration);
        }
        let candidate_state = self.peers.get(&candidate).ok_or(SimError::UnknownPeer)?;
        if !candidate_state.online
            || !candidate_state.complete_snapshot
            || candidate_state.snapshot_hash != self.latest_snapshot_hash
        {
            return Err(SimError::InvalidReplica);
        }
        self.epoch = self.epoch.saturating_add(1);
        self.fencing
            .advance(self.fencing.accepted_token().saturating_add(1))
            .expect("simulator only increments fencing tokens");
        self.state = SimWorldState::Active { authority: candidate };
        self.lease_expires_ms = now_ms.saturating_add(self.lease_duration_ms);
        Ok(())
    }

    fn validate_candidate(&self, candidate: PeerId) -> Result<(), SimError> {
        let state = self.peers.get(&candidate).ok_or(SimError::UnknownPeer)?;
        if !self.is_visible(candidate)
            || !state.complete_snapshot
            || !state.compatible
            || !state.authority_eligible
            || state.banned
            || state.snapshot_hash != self.latest_snapshot_hash
        {
            return Err(SimError::InvalidReplica);
        }
        Ok(())
    }

    fn validate_recovery_ack_peer(&self, peer: PeerId) -> Result<(), SimError> {
        if !self.peers.contains_key(&peer) {
            return Err(SimError::UnknownPeer);
        }
        if !self.visible_canonical_peers().contains(&peer) {
            return Err(SimError::InvalidReplica);
        }
        Ok(())
    }

    fn is_visible(&self, peer: PeerId) -> bool {
        self.peers.get(&peer).is_some_and(|state| state.online) && !self.isolated.contains(&peer)
    }

    fn visible_canonical_peers(&self) -> Vec<PeerId> {
        self.peers
            .values()
            .filter(|peer| {
                peer.online
                    && !self.isolated.contains(&peer.peer_id)
                    && peer.snapshot_hash == self.latest_snapshot_hash
                    && peer.complete_snapshot
                    && peer.compatible
                    && !peer.banned
            })
            .map(|peer| peer.peer_id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: u8, hash: Hash32) -> SimPeer {
        SimPeer {
            peer_id: PeerId([id; 32]),
            online: true,
            complete_snapshot: true,
            compatible: true,
            authority_eligible: true,
            banned: false,
            snapshot_hash: hash,
        }
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
        sim.set_online(PeerId([1; 32]), false).unwrap();
        sim.corrupt_snapshot(PeerId([2; 32])).unwrap();
        assert_eq!(sim.attempt_takeover(PeerId([2; 32]), 1_000, 2), Err(SimError::InvalidReplica));
        sim.install_snapshot(PeerId([2; 32]), hash).unwrap();
        sim.attempt_takeover(PeerId([2; 32]), 1_000, 2).unwrap();
        assert_eq!(sim.authority(), Some(PeerId([2; 32])));
    }

    #[test]
    fn automatic_crash_recovery_never_falls_back_to_solo() {
        let hash = Hash32([5; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 1_000);
        sim.set_online(PeerId([1; 32]), false).unwrap();
        sim.set_online(PeerId([3; 32]), false).unwrap();
        assert_eq!(sim.attempt_takeover(PeerId([2; 32]), 100_000, 2), Err(SimError::NoQuorum));
        assert_eq!(sim.authority(), Some(PeerId([1; 32])));
        assert_eq!(sim.epoch(), 1);
    }

    #[test]
    fn reservation_quorum_alone_does_not_activate_candidate() {
        let hash = Hash32([6; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 1_000);
        sim.set_online(PeerId([1; 32]), false).unwrap();
        sim.begin_recovery(PeerId([2; 32]), 1_000, 2).unwrap();
        sim.acknowledge_reservation(PeerId([3; 32])).unwrap();
        assert_eq!(sim.authority(), Some(PeerId([1; 32])));
        assert_eq!(sim.activate_recovery(1_001), Err(SimError::NoQuorum));
    }

    #[test]
    fn epoch_quorum_without_live_lease_quorum_cannot_activate() {
        let hash = Hash32([8; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 1_000);
        sim.set_online(PeerId([1; 32]), false).unwrap();
        sim.begin_recovery(PeerId([2; 32]), 1_000, 2).unwrap();
        sim.acknowledge_reservation(PeerId([3; 32])).unwrap();
        sim.commit_recovery_epoch().unwrap();
        sim.acknowledge_epoch(PeerId([3; 32])).unwrap();
        assert_eq!(sim.state(), &SimWorldState::Recovering { candidate: PeerId([2; 32]) });
        assert_eq!(sim.activate_recovery(1_001), Err(SimError::NoQuorum));
        assert_eq!(sim.renew_authority_lease(PeerId([1; 32]), 1, 1_002), Err(SimError::StaleGeneration));
    }

    #[test]
    fn conflicting_candidate_cannot_replace_reserved_generation() {
        let hash = Hash32([10; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 1_000);
        sim.set_online(PeerId([1; 32]), false).unwrap();
        sim.begin_recovery(PeerId([2; 32]), 1_000, 2).unwrap();
        assert_eq!(sim.begin_recovery(PeerId([3; 32]), 1_000, 2), Err(SimError::RecoveryInProgress));
        assert_eq!(sim.recovery_candidate(), Some(PeerId([2; 32])));
    }

    #[test]
    fn deterministic_candidate_prevents_visible_racers_from_choosing_different_winners() {
        let hash = Hash32([11; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 1_000);
        sim.set_online(PeerId([1; 32]), false).unwrap();
        assert_eq!(sim.begin_recovery(PeerId([3; 32]), 1_000, 2), Err(SimError::WrongCandidate));
        sim.begin_recovery(PeerId([2; 32]), 1_000, 2).unwrap();
    }

    #[test]
    fn candidate_crash_after_reservation_is_safe_but_stalls_until_future_round_logic() {
        let hash = Hash32([12; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 1_000);
        sim.set_online(PeerId([1; 32]), false).unwrap();
        sim.begin_recovery(PeerId([2; 32]), 1_000, 2).unwrap();
        sim.acknowledge_reservation(PeerId([3; 32])).unwrap();
        sim.set_online(PeerId([2; 32]), false).unwrap();
        assert_eq!(sim.begin_recovery(PeerId([3; 32]), 2_000, 2), Err(SimError::RecoveryInProgress));
        assert_eq!(sim.authority(), Some(PeerId([1; 32])));
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
