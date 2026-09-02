from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    file = Path(path)
    text = file.read_text()
    actual = text.count(old)
    if actual != count:
        raise SystemExit(f"{path}: expected {count} copies, found {actual}: {old[:100]!r}")
    file.write_text(text.replace(old, new, count))


def write(path: str, content: str) -> None:
    file = Path(path)
    if file.exists():
        raise SystemExit(f"refusing to overwrite existing {path}")
    file.write_text(content)


# Signed, hash-bound membership transition primitives. These are append-only v2
# protocol types so the existing v1 membership record remains stable on disk/wire.
replace(
    "crates/swarm-protocol/src/v2.rs",
    "use super::{Hash32, PeerId, ProtocolError, WorldId, PROTOCOL_VERSION};\n",
    "use super::{Hash32, MembershipRecordV1, PeerId, ProtocolError, WorldId, PROTOCOL_VERSION};\n",
)
replace(
    "crates/swarm-protocol/src/v2.rs",
    "const WORLD_CONFIG_HASH_DOMAIN: &[u8] = b\"swarmcraft/world-config/v1\\0\";\n",
    "const WORLD_CONFIG_HASH_DOMAIN: &[u8] = b\"swarmcraft/world-config/v1\\0\";\nconst MEMBERSHIP_PROPOSAL_HASH_DOMAIN: &[u8] = b\"swarmcraft/membership-proposal/v1\\0\";\nconst MEMBERSHIP_VOTE_SIGN_DOMAIN: &[u8] = b\"swarmcraft/membership-vote-sign/v1\\0\";\n",
)
replace(
    "crates/swarm-protocol/src/v2.rs",
    """#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct RecoveryBallotV1 {\n""",
    """#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct MembershipProposalV1 {\n    pub previous: MembershipRecordV1,\n    pub proposed: MembershipRecordV1,\n}\n\nimpl MembershipProposalV1 {\n    pub fn validate_shape(&self) -> Result<bool, ProtocolError> {\n        Ok(self.previous.protocol_version == PROTOCOL_VERSION\n            && self.proposed.protocol_version == PROTOCOL_VERSION\n            && self.previous.world_id == self.proposed.world_id\n            && self.previous.epoch == self.proposed.epoch\n            && self.previous.sequence.checked_add(1) == Some(self.proposed.sequence)\n            && self.proposed.previous_membership_hash == Some(self.previous.record_hash()?)\n            && self.previous.authority_peer_id == self.proposed.authority_peer_id\n            && self.previous.authority_public_key == self.proposed.authority_public_key\n            && !self.proposed.members.is_empty())\n    }\n\n    pub fn proposal_hash(&self) -> Result<Hash32, ProtocolError> {\n        let bytes = postcard::to_allocvec(&(self.previous.record_hash()?, self.proposed.record_hash()?))?;\n        Ok(Hash32::from_domain_bytes(MEMBERSHIP_PROPOSAL_HASH_DOMAIN, &bytes))\n    }\n}\n\n#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct MembershipVoteV1 {\n    pub protocol_version: u16,\n    pub world_id: WorldId,\n    pub previous_membership_hash: Hash32,\n    pub proposed_membership_hash: Hash32,\n    pub proposed_sequence: u64,\n    pub voter_peer_id: PeerId,\n    pub voter_public_key: [u8; 32],\n    pub signature: Vec<u8>,\n}\n\nimpl MembershipVoteV1 {\n    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {\n        let unsigned = (\n            self.protocol_version,\n            self.world_id,\n            self.previous_membership_hash,\n            self.proposed_membership_hash,\n            self.proposed_sequence,\n            self.voter_peer_id,\n            self.voter_public_key,\n        );\n        let encoded = postcard::to_allocvec(&unsigned)?;\n        let mut bytes = Vec::with_capacity(MEMBERSHIP_VOTE_SIGN_DOMAIN.len() + encoded.len());\n        bytes.extend_from_slice(MEMBERSHIP_VOTE_SIGN_DOMAIN);\n        bytes.extend_from_slice(&encoded);\n        Ok(bytes)\n    }\n\n    pub fn matches_proposal(&self, proposal: &MembershipProposalV1) -> Result<bool, ProtocolError> {\n        Ok(self.protocol_version == PROTOCOL_VERSION\n            && self.world_id == proposal.proposed.world_id\n            && self.previous_membership_hash == proposal.previous.record_hash()?\n            && self.proposed_membership_hash == proposal.proposed.record_hash()?\n            && self.proposed_sequence == proposal.proposed.sequence)\n    }\n}\n\n#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct MembershipCertificateV1 {\n    pub proposal: MembershipProposalV1,\n    pub votes: Vec<MembershipVoteV1>,\n}\n\n#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct RecoveryBallotV1 {\n""",
)

# Shared consensus validation for joint old/new configuration quorum. A valid commit
# requires a majority of BOTH voter universes, so either side intersects every later
# quorum and stale removed members cannot independently keep the old configuration alive.
write(
    "crates/swarm-consensus/src/membership.rs",
    '''use std::collections::{BTreeMap, BTreeSet};

use swarm_protocol::{MembershipCertificateV1, MembershipProposalV1, MembershipVoteV1, PeerId, WorldMemberV1};
use thiserror::Error;

use crate::quorum_size;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MembershipConsensusError {
    #[error("membership proposal is malformed")]
    MalformedProposal,
    #[error("membership contains duplicate peer {0}")]
    DuplicateMember(PeerId),
    #[error("membership authority is not an active voter in both configurations")]
    AuthorityNotActive,
    #[error("membership vote does not match the proposed transition")]
    VoteMismatch,
    #[error("membership vote came from a peer outside the old/new voter union")]
    UnknownVoter,
    #[error("membership vote public key does not match the canonical member key")]
    VoterKeyMismatch,
    #[error("old membership quorum unavailable: votes={votes}, required={required}")]
    OldQuorumUnavailable { votes: usize, required: usize },
    #[error("new membership quorum unavailable: votes={votes}, required={required}")]
    NewQuorumUnavailable { votes: usize, required: usize },
}

fn active_voters(members: &[WorldMemberV1]) -> Result<BTreeMap<PeerId, [u8; 32]>, MembershipConsensusError> {
    let mut all = BTreeSet::new();
    let mut active = BTreeMap::new();
    for member in members {
        if !all.insert(member.peer_id) {
            return Err(MembershipConsensusError::DuplicateMember(member.peer_id));
        }
        if !member.banned {
            active.insert(member.peer_id, member.public_key);
        }
    }
    Ok(active)
}

pub fn validate_membership_proposal_shape(
    proposal: &MembershipProposalV1,
) -> Result<(), MembershipConsensusError> {
    if !proposal.validate_shape().unwrap_or(false) {
        return Err(MembershipConsensusError::MalformedProposal);
    }
    let old = active_voters(&proposal.previous.members)?;
    let new = active_voters(&proposal.proposed.members)?;
    let authority = proposal.previous.authority_peer_id;
    let key = proposal.previous.authority_public_key;
    if old.get(&authority) != Some(&key) || new.get(&authority) != Some(&key) {
        return Err(MembershipConsensusError::AuthorityNotActive);
    }
    Ok(())
}

/// Validate a joint-consensus membership certificate. Cryptographic signature
/// verification remains in swarm-core/daemon; this function owns voter-universe,
/// uniqueness and old+new quorum intersection rules.
pub fn validate_membership_certificate_shape(
    certificate: &MembershipCertificateV1,
) -> Result<(), MembershipConsensusError> {
    let proposal = &certificate.proposal;
    validate_membership_proposal_shape(proposal)?;
    let old = active_voters(&proposal.previous.members)?;
    let new = active_voters(&proposal.proposed.members)?;
    let mut seen = BTreeSet::new();
    let mut old_votes = 0usize;
    let mut new_votes = 0usize;

    for vote in &certificate.votes {
        if !vote.matches_proposal(proposal).unwrap_or(false) {
            return Err(MembershipConsensusError::VoteMismatch);
        }
        if !seen.insert(vote.voter_peer_id) {
            continue;
        }
        let old_key = old.get(&vote.voter_peer_id);
        let new_key = new.get(&vote.voter_peer_id);
        let expected = old_key.or(new_key).ok_or(MembershipConsensusError::UnknownVoter)?;
        if old_key.is_some_and(|key| key != &vote.voter_public_key)
            || new_key.is_some_and(|key| key != &vote.voter_public_key)
            || expected != &vote.voter_public_key
        {
            return Err(MembershipConsensusError::VoterKeyMismatch);
        }
        if old_key.is_some() {
            old_votes += 1;
        }
        if new_key.is_some() {
            new_votes += 1;
        }
    }

    let old_required = quorum_size(old.len());
    if old_votes < old_required {
        return Err(MembershipConsensusError::OldQuorumUnavailable { votes: old_votes, required: old_required });
    }
    let new_required = quorum_size(new.len());
    if new_votes < new_required {
        return Err(MembershipConsensusError::NewQuorumUnavailable { votes: new_votes, required: new_required });
    }
    Ok(())
}

pub fn membership_vote_for(
    proposal: &MembershipProposalV1,
    voter_peer_id: PeerId,
    voter_public_key: [u8; 32],
) -> Result<MembershipVoteV1, swarm_protocol::ProtocolError> {
    Ok(MembershipVoteV1 {
        protocol_version: swarm_protocol::PROTOCOL_VERSION,
        world_id: proposal.proposed.world_id,
        previous_membership_hash: proposal.previous.record_hash()?,
        proposed_membership_hash: proposal.proposed.record_hash()?,
        proposed_sequence: proposal.proposed.sequence,
        voter_peer_id,
        voter_public_key,
        signature: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{Hash32, MembershipRecordV1, WorldId, PROTOCOL_VERSION};

    fn member(id: u8) -> WorldMemberV1 {
        WorldMemberV1 { peer_id: PeerId([id; 32]), public_key: [id; 32], authority_eligible: true, banned: false }
    }

    fn proposal(old_ids: &[u8], new_ids: &[u8]) -> MembershipProposalV1 {
        let previous = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([9; 32]),
            epoch: 7,
            sequence: 4,
            previous_membership_hash: Some(Hash32([1; 32])),
            members: old_ids.iter().copied().map(member).collect(),
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            signature: vec![1],
        };
        let mut proposed = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: previous.world_id,
            epoch: previous.epoch,
            sequence: 5,
            previous_membership_hash: Some(previous.record_hash().unwrap()),
            members: new_ids.iter().copied().map(member).collect(),
            authority_peer_id: previous.authority_peer_id,
            authority_public_key: previous.authority_public_key,
            signature: vec![2],
        };
        proposed.members.sort_by_key(|value| value.peer_id);
        MembershipProposalV1 { previous, proposed }
    }

    fn votes(proposal: &MembershipProposalV1, ids: &[u8]) -> Vec<MembershipVoteV1> {
        ids.iter()
            .map(|id| membership_vote_for(proposal, PeerId([*id; 32]), [*id; 32]).unwrap())
            .collect()
    }

    #[test]
    fn three_to_five_requires_old_and_new_majorities() {
        let p = proposal(&[1, 2, 3], &[1, 2, 3, 4, 5]);
        let invalid = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 4, 5]) };
        assert_eq!(
            validate_membership_certificate_shape(&invalid),
            Err(MembershipConsensusError::OldQuorumUnavailable { votes: 1, required: 2 })
        );

        let valid = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 2, 4]) };
        validate_membership_certificate_shape(&valid).unwrap();
        // The only old voter outside this committed joint quorum is peer 3, which
        // cannot form the stale old 2-of-3 majority after voters 1/2 lock the proposal.
        assert_eq!(quorum_size(3), 2);
    }

    #[test]
    fn five_to_three_removal_requires_intersecting_majorities() {
        let p = proposal(&[1, 2, 3, 4, 5], &[1, 2, 3]);
        let valid = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 2, 3]) };
        validate_membership_certificate_shape(&valid).unwrap();
        assert_eq!(quorum_size(5), 3);
        assert_eq!(quorum_size(3), 2);

        let invalid = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 4, 5]) };
        assert_eq!(
            validate_membership_certificate_shape(&invalid),
            Err(MembershipConsensusError::NewQuorumUnavailable { votes: 1, required: 2 })
        );
    }

    #[test]
    fn one_to_two_join_can_commit_with_old_authority_and_new_peer() {
        let p = proposal(&[1], &[1, 2]);
        let cert = MembershipCertificateV1 { proposal: p.clone(), votes: votes(&p, &[1, 2]) };
        validate_membership_certificate_shape(&cert).unwrap();
    }
}
''',
)
replace(
    "crates/swarm-consensus/src/root.rs",
    "pub mod migration;\npub mod recovery;\n",
    "pub mod membership;\npub mod migration;\npub mod recovery;\n",
)
replace(
    "crates/swarm-consensus/src/root.rs",
    "pub use recovery::*;\n",
    "pub use membership::*;\npub use recovery::*;\n",
)

# Align the shared recovery model with the durable production rule: higher rounds
# may advance only the same accepted candidate/value for a target generation.
replace(
    "crates/swarm-consensus/src/recovery.rs",
    """    #[error(\"new recovery round changed its canonical base\")]\n    CanonicalBaseChanged,\n""",
    """    #[error(\"new recovery round changed its canonical base\")]\n    CanonicalBaseChanged,\n    #[error(\"new recovery round changed the previously accepted candidate value\")]\n    AcceptedValueChanged,\n""",
)
replace(
    "crates/swarm-consensus/src/recovery.rs",
    """    if !same_canonical_base(existing, proposed) {\n        return Err(RecoveryBallotError::CanonicalBaseChanged);\n    }\n    Ok(RecoveryBallotDecision::Accept)\n""",
    """    if !same_canonical_base(existing, proposed) {\n        return Err(RecoveryBallotError::CanonicalBaseChanged);\n    }\n    if existing.candidate_peer_id != proposed.candidate_peer_id\n        || existing.candidate_public_key != proposed.candidate_public_key\n    {\n        return Err(RecoveryBallotError::AcceptedValueChanged);\n    }\n    Ok(RecoveryBallotDecision::Accept)\n""",
)
replace(
    "crates/swarm-consensus/src/recovery.rs",
    """    fn successor_can_supersede_abandoned_candidate_with_higher_round() {\n        let bob = ballot(2, 1);\n        let charlie = ballot(3, 2);\n        assert_eq!(evaluate_recovery_ballot(Some(&bob), &charlie), Ok(RecoveryBallotDecision::Accept));\n    }\n""",
    """    fn higher_round_preserves_the_previously_accepted_candidate_value() {\n        let bob = ballot(2, 1);\n        let charlie = ballot(3, 2);\n        assert_eq!(\n            evaluate_recovery_ballot(Some(&bob), &charlie),\n            Err(RecoveryBallotError::AcceptedValueChanged)\n        );\n        let bob_round_two = ballot(2, 2);\n        assert_eq!(evaluate_recovery_ballot(Some(&bob), &bob_round_two), Ok(RecoveryBallotDecision::Accept));\n    }\n""",
)
replace(
    "crates/swarm-consensus/src/recovery.rs",
    """        for index in [1usize, 2] {\n            assert!(evaluate_recovery_ballot(promises[index].as_ref(), &charlie).is_ok());\n            promises[index] = Some(charlie.clone());\n        }\n        assert!(evaluate_recovery_ballot(promises[1].as_ref(), &bob).is_err());\n""",
    """        assert_eq!(\n            evaluate_recovery_ballot(promises[1].as_ref(), &charlie),\n            Err(RecoveryBallotError::AcceptedValueChanged)\n        );\n        assert!(evaluate_recovery_ballot(promises[2].as_ref(), &charlie).is_ok());\n        promises[2] = Some(charlie.clone());\n        assert!(evaluate_recovery_ballot(promises[1].as_ref(), &bob).is_ok());\n""",
)

# Durable membership prepare locks. A voter persists its exact proposal before
# returning a vote; while this promise exists the daemon will fail closed until a
# matching joint certificate commits the configuration.
write(
    "crates/swarm-storage/src/membership.rs",
    '''use crate::{Storage, StorageError};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
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
        atomic_write(
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
        let Ok(promise) = self.load_membership_promise(world) else {
            return Ok(false);
        };
        if promise.proposal.proposed.record_hash()? != committed_hash {
            return Ok(false);
        }
        remove_if_present(&self.world_dir(world).join("metadata/membership-promise.postcard"))?;
        Ok(true)
    }

    pub fn save_membership_certificate(&self, certificate: &MembershipCertificateV1) -> Result<(), StorageError> {
        let world = certificate.proposal.proposed.world_id;
        atomic_write(
            &self.world_dir(world).join("metadata/membership-certificate.postcard"),
            &postcard::to_allocvec(certificate)?,
        )
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

fn remove_if_present(path: &Path) -> Result<(), StorageError> {
    if path.exists() {
        fs::remove_file(path).map_err(|error| io_error(path, error))?;
        if let Some(parent) = path.parent() {
            sync_parent(parent)?;
        }
    }
    Ok(())
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StorageError {
    StorageError::Io { path: path.into(), source }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or_else(|| StorageError::UnsafeRelativePath(path.to_string_lossy().into_owned()))?;
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    let tmp = path.with_extension("tmp");
    let mut file =
        OpenOptions::new().create(true).truncate(true).write(true).open(&tmp).map_err(|error| io_error(&tmp, error))?;
    file.write_all(bytes).map_err(|error| io_error(&tmp, error))?;
    file.sync_all().map_err(|error| io_error(&tmp, error))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|error| io_error(path, error))?;
    sync_parent(parent)
}

fn sync_parent(parent: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        fs::File::open(parent).and_then(|file| file.sync_all()).map_err(|error| io_error(parent, error))?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
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
        assert_eq!(store.promise_membership_proposal(&first, &vote(&first)).unwrap(), MembershipPromiseResult::Accepted);
        drop(store);

        let store = Storage::open(temp.path()).unwrap();
        assert_eq!(store.promise_membership_proposal(&first, &vote(&first)).unwrap(), MembershipPromiseResult::Idempotent);
        let conflicting = proposal(world, 3);
        assert_eq!(
            store.promise_membership_proposal(&conflicting, &vote(&conflicting)).unwrap(),
            MembershipPromiseResult::Rejected
        );
        assert!(!store.clear_membership_promise_after_commit(world, Hash32([0; 32])).unwrap());
        assert!(store
            .clear_membership_promise_after_commit(world, first.proposed.record_hash().unwrap())
            .unwrap());
    }
}
''',
)
replace(
    "crates/swarm-storage/src/root.rs",
    "pub mod control;\npub mod recovery_v2;\n",
    "pub mod control;\npub mod membership;\npub mod recovery_v2;\n",
)
replace(
    "crates/swarm-storage/src/root.rs",
    "pub use retention::{\n",
    "pub use membership::{DurableMembershipPromiseV1, MembershipPromiseResult};\npub use retention::{\n",
)
replace(
    "crates/swarm-storage/src/world.rs",
    """        for name in [\"descriptor.json\", \"membership.postcard\", \"pending-join.postcard\", \"pending-leave.postcard\"] {\n""",
    """        for name in [\n            \"descriptor.json\",\n            \"membership.postcard\",\n            \"membership-promise.postcard\",\n            \"membership-certificate.postcard\",\n            \"pending-join.postcard\",\n            \"pending-leave.postcard\",\n        ] {\n""",
)
