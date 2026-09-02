from pathlib import Path


def replace(path, old, new, count=1):
    p = Path(path)
    s = p.read_text()
    n = s.count(old)
    if n != count:
        raise SystemExit(f"{path}: expected {count}, found {n}: {old[:100]!r}")
    p.write_text(s.replace(old, new, count))


replace(
    "crates/swarm-network/src/wire.rs",
    "    Hash32, JoinRequestV1, LeaveRequestV1, MembershipRecordV1, PeerHelloV1, PeerId, RecoveryBallotV1,\n    RecoveryCertificateV1, RecoveryVoteV1, SleepRecordV1, SnapshotManifestV1, SoloBranchV1, WorldAnnouncementV1,\n",
    "    Hash32, JoinRequestV1, LeaveRequestV1, MembershipCertificateV1, MembershipProposalV1, MembershipRecordV1,\n    MembershipVoteV1, PeerHelloV1, PeerId, RecoveryBallotV1, RecoveryCertificateV1, RecoveryVoteV1, SleepRecordV1,\n    SnapshotManifestV1, SoloBranchV1, WorldAnnouncementV1,\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "pub const MAX_RECOVERY_VOTES: usize = 1_024;\n",
    "pub const MAX_RECOVERY_VOTES: usize = 1_024;\npub const MAX_MEMBERSHIP_VOTES: usize = 1_024;\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "    FriendPresence { expected_peer_id: PeerId, requester_peer_id: PeerId, nonce: [u8; 32] },\n}\n",
    "    FriendPresence { expected_peer_id: PeerId, requester_peer_id: PeerId, nonce: [u8; 32] },\n    MembershipProposal(Box<MembershipProposalV1>),\n    MembershipCommit(Box<MembershipCertificateV1>),\n}\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "            Self::RecoveryEpoch { certificate, .. } if certificate.votes.len() > MAX_RECOVERY_VOTES => {\n                Err(WireLimitError::TooManyRecoveryVotes(certificate.votes.len()))\n            }\n",
    "            Self::RecoveryEpoch { certificate, .. } if certificate.votes.len() > MAX_RECOVERY_VOTES => {\n                Err(WireLimitError::TooManyRecoveryVotes(certificate.votes.len()))\n            }\n            Self::MembershipProposal(proposal)\n                if proposal.previous.members.len() > MAX_WORLD_MEMBERS\n                    || proposal.proposed.members.len() > MAX_WORLD_MEMBERS =>\n            {\n                Err(WireLimitError::TooManyMembers(\n                    proposal.previous.members.len().max(proposal.proposed.members.len()),\n                ))\n            }\n            Self::MembershipCommit(certificate)\n                if certificate.proposal.previous.members.len() > MAX_WORLD_MEMBERS\n                    || certificate.proposal.proposed.members.len() > MAX_WORLD_MEMBERS =>\n            {\n                Err(WireLimitError::TooManyMembers(\n                    certificate.proposal.previous.members.len().max(certificate.proposal.proposed.members.len()),\n                ))\n            }\n            Self::MembershipCommit(certificate) if certificate.votes.len() > MAX_MEMBERSHIP_VOTES => {\n                Err(WireLimitError::TooManyMembershipVotes(certificate.votes.len()))\n            }\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "    FriendPresence(Option<FriendPresenceV1>),\n}\n",
    "    FriendPresence(Option<FriendPresenceV1>),\n    MembershipVote(Box<MembershipVoteV1>),\n    MembershipCommitAccepted { sequence: u64 },\n}\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "    #[error(\"recovery certificate contains {0} votes; maximum is {MAX_RECOVERY_VOTES}\")]\n    TooManyRecoveryVotes(usize),\n",
    "    #[error(\"recovery certificate contains {0} votes; maximum is {MAX_RECOVERY_VOTES}\")]\n    TooManyRecoveryVotes(usize),\n    #[error(\"membership certificate contains {0} votes; maximum is {MAX_MEMBERSHIP_VOTES}\")]\n    TooManyMembershipVotes(usize),\n",
)
replace(
    "crates/swarm-network/src/lib.rs",
    "    MAX_DISCOVERY_RESULTS, MAX_DISCOVERY_TAGS, MAX_MISSING_BLOBS, MAX_RECOVERY_VOTES, MAX_WORLD_ARTIFACTS,\n    MAX_WORLD_MEMBERS,\n",
    "    MAX_DISCOVERY_RESULTS, MAX_DISCOVERY_TAGS, MAX_MEMBERSHIP_VOTES, MAX_MISSING_BLOBS, MAX_RECOVERY_VOTES,\n    MAX_WORLD_ARTIFACTS, MAX_WORLD_MEMBERS,\n",
)
