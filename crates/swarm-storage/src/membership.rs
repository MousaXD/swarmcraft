use crate::{
    transaction::{durable_atomic_write, durable_create_once, durable_remove},
    Storage, StorageError,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use swarm_protocol::{MembershipCertificateV1, MembershipProposalV1, MembershipVoteV1, WorldId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableMembershipPromiseV1 {
    pub proposal: MembershipProposalV1,
    pub vote: MembershipVoteV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipPromiseResult {
    Accepted,
    Idempotent,
    Rejected,
}

impl Storage {
    pub fn promise_membership_proposal(
        &self,
        proposal: &MembershipProposalV1,
        vote: &MembershipVoteV1,
    ) -> Result<MembershipPromiseResult, StorageError> {
        let _guard = self.lock_world_transaction(proposal.proposed.world_id)?;
        if !proposal.validate_shape()? || !vote.matches_proposal(proposal)? {
            return Ok(MembershipPromiseResult::Rejected);
        }
        if let Ok(existing) = self.load_membership_promise(proposal.proposed.world_id) {
            if existing.proposal.proposal_hash()? == proposal.proposal_hash()? {
                return Ok(MembershipPromiseResult::Idempotent);
            }
            return Ok(MembershipPromiseResult::Rejected);
        }
        let promise = DurableMembershipPromiseV1 { proposal: proposal.clone(), vote: vote.clone() };
        durable_atomic_write(
            &self.world_dir(proposal.proposed.world_id).join("metadata/membership-promise.postcard"),
            &postcard::to_allocvec(&promise)?,
        )?;
        Ok(MembershipPromiseResult::Accepted)
    }

    pub fn load_membership_promise(&self, world: WorldId) -> Result<DurableMembershipPromiseV1, StorageError> {
        let path = self.world_dir(world).join("metadata/membership-promise.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let promise: DurableMembershipPromiseV1 = postcard::from_bytes(&bytes)?;
        if promise.proposal.proposed.world_id != world || !promise.vote.matches_proposal(&promise.proposal)? {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(promise)
    }

    pub fn clear_membership_promise_after_commit(
        &self,
        world: WorldId,
        committed_hash: swarm_protocol::Hash32,
    ) -> Result<bool, StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let Ok(promise) = self.load_membership_promise(world) else {
            return Ok(false);
        };
        if promise.proposal.proposed.record_hash()? != committed_hash {
            return Ok(false);
        }
        durable_remove(&self.world_dir(world).join("metadata/membership-promise.postcard"))?;
        Ok(true)
    }

    pub fn save_membership_certificate(&self, certificate: &MembershipCertificateV1) -> Result<(), StorageError> {
        let world = certificate.proposal.proposed.world_id;
        let _guard = self.lock_world_transaction(world)?;
        let encoded = postcard::to_allocvec(certificate)?;
        let history_path = self
            .world_dir(world)
            .join("metadata/membership-certificates")
            .join(format!("{:020}.postcard", certificate.proposal.proposed.sequence));
        if !durable_create_once(&history_path, &encoded)? {
            let existing = fs::read(&history_path).map_err(|error| io_error(&history_path, error))?;
            if existing != encoded {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        durable_atomic_write(&self.world_dir(world).join("metadata/membership-certificate.postcard"), &encoded)
    }

    pub fn load_membership_certificate_chain(
        &self,
        world: WorldId,
    ) -> Result<Vec<MembershipCertificateV1>, StorageError> {
        let directory = self.world_dir(world).join("metadata/membership-certificates");
        let mut paths = match fs::read_dir(&directory) {
            Ok(entries) => entries
                .map(|entry| entry.map(|value| value.path()).map_err(|error| io_error(&directory, error)))
                .collect::<Result<Vec<_>, _>>()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(io_error(&directory, error)),
        };
        paths.retain(|path| path.extension().is_some_and(|value| value == "postcard"));
        paths.sort();
        let mut certificates = Vec::with_capacity(paths.len());
        for path in paths {
            let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
            let certificate: MembershipCertificateV1 = postcard::from_bytes(&bytes)?;
            if certificate.proposal.proposed.world_id != world {
                return Err(StorageError::WorldMetadataMismatch);
            }
            certificates.push(certificate);
        }
        if certificates.is_empty() {
            if let Ok(latest) = self.load_membership_certificate(world) {
                certificates.push(latest);
            }
        }
        Ok(certificates)
    }

    pub fn load_membership_certificate(&self, world: WorldId) -> Result<MembershipCertificateV1, StorageError> {
        let path = self.world_dir(world).join("metadata/membership-certificate.postcard");
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let certificate: MembershipCertificateV1 = postcard::from_bytes(&bytes)?;
        if certificate.proposal.proposed.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(certificate)
    }
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{Hash32, MembershipRecordV1, PeerId, WorldMemberV1, PROTOCOL_VERSION};

    fn member(id: u8) -> WorldMemberV1 {
        WorldMemberV1 { peer_id: PeerId([id; 32]), public_key: [id; 32], authority_eligible: true, banned: false }
    }

    fn proposal(world: WorldId, new_peer: u8) -> MembershipProposalV1 {
        let previous = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: 1,
            sequence: 1,
            previous_membership_hash: None,
            members: vec![member(1)],
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            signature: vec![1],
        };
        let proposed = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: 1,
            sequence: 2,
            previous_membership_hash: Some(previous.record_hash().unwrap()),
            members: vec![member(1), member(new_peer)],
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            signature: vec![2],
        };
        MembershipProposalV1 { previous, proposed }
    }

    fn vote(proposal: &MembershipProposalV1) -> MembershipVoteV1 {
        MembershipVoteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: proposal.proposed.world_id,
            previous_membership_hash: proposal.previous.record_hash().unwrap(),
            proposed_membership_hash: proposal.proposed.record_hash().unwrap(),
            proposed_sequence: proposal.proposed.sequence,
            voter_peer_id: PeerId([1; 32]),
            voter_public_key: [1; 32],
            signature: vec![3],
        }
    }

    #[test]
    fn promise_is_durable_and_cannot_switch_proposals() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([7; 32]);
        let first = proposal(world, 2);
        assert_eq!(
            store.promise_membership_proposal(&first, &vote(&first)).unwrap(),
            MembershipPromiseResult::Accepted
        );
        drop(store);

        let store = Storage::open(temp.path()).unwrap();
        assert_eq!(
            store.promise_membership_proposal(&first, &vote(&first)).unwrap(),
            MembershipPromiseResult::Idempotent
        );
        let conflicting = proposal(world, 3);
        assert_eq!(
            store.promise_membership_proposal(&conflicting, &vote(&conflicting)).unwrap(),
            MembershipPromiseResult::Rejected
        );
        assert!(!store.clear_membership_promise_after_commit(world, Hash32([0; 32])).unwrap());
        assert!(store.clear_membership_promise_after_commit(world, first.proposed.record_hash().unwrap()).unwrap());
    }
}
