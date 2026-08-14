use crate::AuthorityCandidate;
use std::time::{Duration, Instant};
use swarm_protocol::{Hash32, PeerId, TransferPhase};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct LeaseTracker {
    epoch: u64,
    fencing_token: u64,
    duration: Duration,
    expires_at: Instant,
}

impl LeaseTracker {
    pub fn new(epoch: u64, fencing_token: u64, duration: Duration, now: Instant) -> Self {
        Self { epoch, fencing_token, duration, expires_at: now + duration }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn fencing_token(&self) -> u64 {
        self.fencing_token
    }
    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }
    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }

    pub fn renew(&mut self, epoch: u64, fencing_token: u64, now: Instant) -> Result<(), LeaseError> {
        if epoch != self.epoch || fencing_token != self.fencing_token {
            return Err(LeaseError::WrongAuthorityGeneration);
        }
        self.expires_at = now + self.duration;
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("lease generation does not match current epoch/fencing token")]
    WrongAuthorityGeneration,
    #[error("old authority lease has not expired")]
    NotExpired,
    #[error("candidate does not hold the required complete snapshot")]
    SnapshotNotReady,
    #[error("candidate is incompatible or not authority eligible")]
    Ineligible,
    #[error("automatic crash takeover requires the configured quorum")]
    NoQuorum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeoverMode {
    Quorum,
    Solo,
}

#[derive(Debug, Clone, Copy)]
pub struct TakeoverPolicy {
    pub quorum_size: usize,
}

impl Default for TakeoverPolicy {
    fn default() -> Self {
        Self { quorum_size: 2 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeoverCandidate {
    pub candidate: AuthorityCandidate,
    pub snapshot_hash: Hash32,
    pub peer_votes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityGeneration {
    pub authority_peer_id: PeerId,
    pub epoch: u64,
    pub fencing_token: u64,
    pub base_snapshot_hash: Hash32,
    pub mode: TakeoverMode,
}

pub fn evaluate_crash_takeover(
    current_lease: &LeaseTracker,
    candidate: &TakeoverCandidate,
    required_snapshot_hash: Hash32,
    now: Instant,
    policy: TakeoverPolicy,
) -> Result<AuthorityGeneration, LeaseError> {
    if !current_lease.is_expired(now) {
        return Err(LeaseError::NotExpired);
    }
    if !candidate.candidate.snapshot_complete || candidate.snapshot_hash != required_snapshot_hash {
        return Err(LeaseError::SnapshotNotReady);
    }
    if !candidate.candidate.compatible || !candidate.candidate.authority_eligible || candidate.candidate.banned {
        return Err(LeaseError::Ineligible);
    }
    if candidate.peer_votes < policy.quorum_size {
        return Err(LeaseError::NoQuorum);
    }
    Ok(AuthorityGeneration {
        authority_peer_id: candidate.candidate.peer_id,
        epoch: current_lease.epoch() + 1,
        fencing_token: current_lease.fencing_token() + 1,
        base_snapshot_hash: required_snapshot_hash,
        mode: TakeoverMode::Quorum,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualTransferState {
    pub from_peer: PeerId,
    pub to_peer: PeerId,
    pub snapshot_hash: Hash32,
    pub next_epoch: u64,
    pub next_fencing_token: u64,
    pub phase: TransferPhase,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransferError {
    #[error("manual transfer phase is invalid for this operation")]
    WrongPhase,
    #[error("manual transfer target does not have the prepared snapshot")]
    SnapshotNotReady,
    #[error("manual transfer signer is not the expected participant")]
    WrongPeer,
}

impl ManualTransferState {
    pub fn prepare(
        from_peer: PeerId,
        to_peer: PeerId,
        snapshot_hash: Hash32,
        current_epoch: u64,
        current_fencing_token: u64,
    ) -> Self {
        Self {
            from_peer,
            to_peer,
            snapshot_hash,
            next_epoch: current_epoch + 1,
            next_fencing_token: current_fencing_token + 1,
            phase: TransferPhase::Prepared,
        }
    }

    pub fn accept(&mut self, accepting_peer: PeerId, target_snapshot_hash: Hash32) -> Result<(), TransferError> {
        if self.phase != TransferPhase::Prepared {
            return Err(TransferError::WrongPhase);
        }
        if accepting_peer != self.to_peer {
            return Err(TransferError::WrongPeer);
        }
        if target_snapshot_hash != self.snapshot_hash {
            return Err(TransferError::SnapshotNotReady);
        }
        self.phase = TransferPhase::Accepted;
        Ok(())
    }

    pub fn commit(&mut self, committing_peer: PeerId) -> Result<AuthorityGeneration, TransferError> {
        if self.phase != TransferPhase::Accepted {
            return Err(TransferError::WrongPhase);
        }
        if committing_peer != self.from_peer {
            return Err(TransferError::WrongPeer);
        }
        self.phase = TransferPhase::Committed;
        Ok(AuthorityGeneration {
            authority_peer_id: self.to_peer,
            epoch: self.next_epoch,
            fencing_token: self.next_fencing_token,
            base_snapshot_hash: self.snapshot_hash,
            mode: TakeoverMode::Quorum,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldRuntimeState {
    Active(AuthorityGeneration),
    Sleeping { latest_snapshot_hash: Hash32, epoch: u64, fencing_token: u64 },
}

impl WorldRuntimeState {
    pub fn sleep(&mut self, latest_snapshot_hash: Hash32) {
        let (epoch, fencing_token) = match self {
            Self::Active(generation) => (generation.epoch, generation.fencing_token),
            Self::Sleeping { epoch, fencing_token, .. } => (*epoch, *fencing_token),
        };
        *self = Self::Sleeping { latest_snapshot_hash, epoch, fencing_token };
    }

    pub fn wake(
        &mut self,
        candidate: PeerId,
        candidate_snapshot_hash: Hash32,
    ) -> Result<AuthorityGeneration, LeaseError> {
        let Self::Sleeping { latest_snapshot_hash, epoch, fencing_token } = self else {
            return Err(LeaseError::WrongAuthorityGeneration);
        };
        if candidate_snapshot_hash != *latest_snapshot_hash {
            return Err(LeaseError::SnapshotNotReady);
        }
        let generation = AuthorityGeneration {
            authority_peer_id: candidate,
            epoch: *epoch + 1,
            fencing_token: *fencing_token + 1,
            base_snapshot_hash: *latest_snapshot_hash,
            mode: TakeoverMode::Solo,
        };
        *self = Self::Active(generation.clone());
        Ok(generation)
    }
}
