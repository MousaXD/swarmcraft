use serde::{Deserialize, Serialize};
use swarm_protocol::{
    AuthorityLeaseGrantV1, AuthorityTransferV1, BlobEncoding, DiscoveryFilterV1, DiscoveryFreshnessChallengeV1,
    DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1, EpochRecordV1, FriendPresenceV1, Hash32, JoinRequestV1,
    LeaveRequestV1, MembershipCertificateV1, MembershipProposalV1, MembershipRecordV1, MembershipVoteV1, PeerHelloV1,
    PeerId, RecoveryBallotV1, RecoveryCertificateV1, RecoveryVoteV1, SleepRecordV1, SnapshotManifestV1, SoloBranchV1,
    WorldAnnouncementV1, WorldConfigV1, WorldDescriptorV1, WorldId, WorldStatusV1,
};
use thiserror::Error;

pub const MAX_BLOB_CHUNK: usize = 256 * 1024;
pub const MAX_MISSING_BLOBS: usize = 16_384;
pub const MAX_WORLD_MEMBERS: usize = 1_024;
pub const MAX_RECOVERY_VOTES: usize = 1_024;
pub const MAX_MEMBERSHIP_VOTES: usize = 1_024;
pub const MAX_WORLD_ARTIFACTS: usize = 4_096;
pub const MAX_PRESENTATION_TAGS: usize = 64;
pub const MAX_DISCOVERY_RESULTS: usize = 64;
pub const MAX_DISCOVERY_TAGS: usize = 16;
pub const MAX_DISCOVERY_QUERY_BYTES: usize = 512;
pub const MAX_DISCOVERY_ANNOUNCEMENT_BYTES: usize = 16 * 1024;
pub const MAX_DISCOVERY_MEMBERSHIP_PROOF_BYTES: usize = 512 * 1024;
pub const MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES: usize = 256;
pub const MAX_HANDSHAKE_TRANSPORT_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaAckV1 {
    pub world_id: WorldId,
    pub snapshot_number: u64,
    pub manifest_hash: Hash32,
    pub state_root: Hash32,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRuntimeReadinessV1 {
    Ready,
    MissingConfiguration,
    EulaRequired,
    MissingJava,
    MissingServerJar,
    MissingSwarmCraftMod,
    Unverified,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerModsReadinessV1 {
    Ready,
    Missing,
    Incompatible,
    Unverified,
}

/// Ephemeral, authenticated machine-local host capability. This is deliberately
/// not part of signed world state: it describes what this device can do now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilityV1 {
    pub world_id: WorldId,
    pub compatibility_fingerprint: Hash32,
    pub runtime: HostRuntimeReadinessV1,
    pub server_mods: ServerModsReadinessV1,
    pub conflict_free: bool,
    pub recovery_quorum_without_authority: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobResumeV1 {
    pub hash: Hash32,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHelloProofV1 {
    pub hello: PeerHelloV1,
    pub challenge: [u8; 32],
    pub claimant_transport_peer: Vec<u8>,
    pub receiver_transport_peer: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireRequest {
    Hello(PeerHelloV1),
    Ping {
        nonce: u64,
    },
    WorldStatus {
        world_id: WorldId,
    },
    WorldDescriptor {
        world_id: WorldId,
    },
    JoinRequest(Box<JoinRequestV1>),
    LeaveRequest(Box<LeaveRequestV1>),
    SnapshotManifest(SnapshotManifestV1),
    MissingBlobs {
        world_id: WorldId,
        snapshot_number: u64,
        hashes: Vec<Hash32>,
    },
    BlobChunk {
        world_id: WorldId,
        hash: Hash32,
        encoding: BlobEncoding,
        offset: u64,
        data: Vec<u8>,
        finished: bool,
    },
    ReplicaAck(ReplicaAckV1),
    Membership(MembershipRecordV1),
    Epoch(EpochRecordV1),
    AuthorityTransfer(AuthorityTransferV1),
    LeaseGrant(AuthorityLeaseGrantV1),
    Sleep(SleepRecordV1),
    // 0.2 extensions are appended so existing postcard enum discriminants remain stable.
    RecoveryBallot(Box<RecoveryBallotV1>),
    RecoveryEpoch {
        record: EpochRecordV1,
        certificate: Box<RecoveryCertificateV1>,
    },
    WorldConfig(Box<WorldConfigV1>),
    SoloBranch(Box<SoloBranchV1>),
    // Host readiness is appended so all earlier postcard discriminants stay stable.
    HostCapability {
        world_id: WorldId,
    },
    // Discovery extensions are append-only for postcard compatibility.
    DiscoveryPublic {
        filter: DiscoveryFilterV1,
    },
    DiscoveryResolve {
        world_id: WorldId,
    },
    FriendPresence {
        expected_peer_id: PeerId,
        requester_peer_id: PeerId,
        nonce: [u8; 32],
    },
    MembershipProposal(Box<MembershipProposalV1>),
    MembershipCommit(Box<MembershipCertificateV1>),
    // Connection-bound authentication extensions follow integrated membership variants.
    HelloChallenge {
        challenge: [u8; 32],
    },
    HelloProof(Box<PeerHelloProofV1>),
    // FINAL-028 challenge-bound authority freshness extensions are append-only.
    DiscoveryFreshnessContext {
        world_id: WorldId,
        announcement_hash: Hash32,
        verifier_peer_id: PeerId,
        nonce: [u8; 32],
        issued_unix_ms: u64,
        expires_unix_ms: u64,
    },
    DiscoveryFreshnessVote(Box<DiscoveryFreshnessChallengeV1>),
}

impl WireRequest {
    /// Return the canonical world whose current membership is required before
    /// this request may be dispatched by the replication daemon.
    ///
    /// This match is intentionally exhaustive. Adding a new wire request must
    /// make an explicit authorization decision instead of silently inheriting
    /// an unsafe default.
    pub fn membership_world_id(&self) -> Option<WorldId> {
        match self {
            Self::Hello(_)
            | Self::Ping { .. }
            | Self::JoinRequest(_)
            | Self::DiscoveryPublic { .. }
            | Self::DiscoveryResolve { .. }
            | Self::FriendPresence { .. }
            | Self::MembershipProposal(_)
            | Self::MembershipCommit(_)
            | Self::HelloChallenge { .. }
            | Self::HelloProof(_)
            | Self::DiscoveryFreshnessContext { .. }
            | Self::DiscoveryFreshnessVote(_) => None,
            Self::WorldStatus { world_id }
            | Self::WorldDescriptor { world_id }
            | Self::MissingBlobs { world_id, .. }
            | Self::BlobChunk { world_id, .. }
            | Self::HostCapability { world_id } => Some(*world_id),
            Self::LeaveRequest(request) => Some(request.world_id),
            Self::SnapshotManifest(manifest) => Some(manifest.world_id),
            Self::ReplicaAck(ack) => Some(ack.world_id),
            Self::Membership(record) => Some(record.world_id),
            Self::Epoch(record) => Some(record.world_id),
            Self::AuthorityTransfer(transfer) => Some(transfer.world_id),
            Self::LeaseGrant(lease) => Some(lease.world_id),
            Self::Sleep(record) => Some(record.world_id),
            Self::RecoveryBallot(ballot) => Some(ballot.world_id),
            Self::RecoveryEpoch { record, .. } => Some(record.world_id),
            Self::WorldConfig(config) => Some(config.world_id),
            Self::SoloBranch(branch) => Some(branch.world_id),
        }
    }

    pub fn validate_limits(&self) -> Result<(), WireLimitError> {
        match self {
            Self::BlobChunk { data, .. } if data.len() > MAX_BLOB_CHUNK => {
                Err(WireLimitError::BlobChunkTooLarge(data.len()))
            }
            Self::MissingBlobs { hashes, .. } if hashes.len() > MAX_MISSING_BLOBS => {
                Err(WireLimitError::TooManyBlobHashes(hashes.len()))
            }
            Self::Membership(record) if record.members.len() > MAX_WORLD_MEMBERS => {
                Err(WireLimitError::TooManyMembers(record.members.len()))
            }
            Self::RecoveryEpoch { certificate, .. } if certificate.votes.len() > MAX_RECOVERY_VOTES => {
                Err(WireLimitError::TooManyRecoveryVotes(certificate.votes.len()))
            }
            Self::MembershipProposal(proposal)
                if proposal.previous.members.len() > MAX_WORLD_MEMBERS
                    || proposal.proposed.members.len() > MAX_WORLD_MEMBERS =>
            {
                Err(WireLimitError::TooManyMembers(
                    proposal.previous.members.len().max(proposal.proposed.members.len()),
                ))
            }
            Self::MembershipCommit(certificate)
                if certificate.proposal.previous.members.len() > MAX_WORLD_MEMBERS
                    || certificate.proposal.proposed.members.len() > MAX_WORLD_MEMBERS =>
            {
                Err(WireLimitError::TooManyMembers(
                    certificate.proposal.previous.members.len().max(certificate.proposal.proposed.members.len()),
                ))
            }
            Self::MembershipCommit(certificate) if certificate.votes.len() > MAX_MEMBERSHIP_VOTES => {
                Err(WireLimitError::TooManyMembershipVotes(certificate.votes.len()))
            }
            Self::WorldConfig(config) => {
                let artifacts = config.compatibility.required_server_mods.len()
                    + config.compatibility.required_client_mods.len()
                    + config.compatibility.datapacks.len();
                if artifacts > MAX_WORLD_ARTIFACTS {
                    return Err(WireLimitError::TooManyWorldArtifacts(artifacts));
                }
                if config.presentation.tags.len() > MAX_PRESENTATION_TAGS {
                    return Err(WireLimitError::TooManyPresentationTags(config.presentation.tags.len()));
                }
                Ok(())
            }
            Self::DiscoveryPublic { filter } => validate_discovery_filter(filter),
            Self::HelloProof(proof)
                if proof.claimant_transport_peer.len() > MAX_HANDSHAKE_TRANSPORT_ID_BYTES
                    || proof.receiver_transport_peer.len() > MAX_HANDSHAKE_TRANSPORT_ID_BYTES =>
            {
                Err(WireLimitError::HandshakeTransportIdTooLarge)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireResponse {
    HelloAccepted { protocol_version: u16 },
    Pong { nonce: u64 },
    WorldStatus(Option<WorldStatusV1>),
    WorldDescriptor(Option<WorldDescriptorV1>),
    JoinAccepted { membership_sequence: u64 },
    LeaveAccepted { membership_sequence: u64 },
    ManifestAccepted { snapshot_number: u64, missing: Vec<BlobResumeV1> },
    MissingBlobs(Vec<BlobResumeV1>),
    BlobChunkAccepted { hash: Hash32, next_offset: u64 },
    ReplicaAckAccepted,
    MembershipAccepted { sequence: u64 },
    EpochAccepted { epoch: u64, fencing_token: u64 },
    TransferAccepted,
    LeaseAccepted { epoch: u64, fencing_token: u64 },
    SleepAccepted { epoch: u64, fencing_token: u64 },
    Error { code: String, message: String },
    RecoveryVote(Box<RecoveryVoteV1>),
    RecoveryRejected { highest_round: u64, reason: String },
    WorldConfigAccepted { sequence: u64 },
    SoloBranchAccepted,
    // Host readiness is appended so all earlier postcard discriminants stay stable.
    HostCapability(Option<HostCapabilityV1>),
    // Discovery extensions are append-only for postcard compatibility.
    DiscoveryWorlds(Vec<WorldAnnouncementV1>),
    DiscoveryResolved(Option<Box<WorldAnnouncementV1>>),
    FriendPresence(Option<FriendPresenceV1>),
    MembershipVote(Box<MembershipVoteV1>),
    MembershipCommitAccepted { sequence: u64 },
    HelloChallengeAccepted,
    DiscoveryFreshnessContext(Option<Box<DiscoveryMembershipProofV1>>),
    DiscoveryFreshnessVote(Option<Box<DiscoveryFreshnessVoteV1>>),
}

impl WireResponse {
    pub fn validate_limits(&self) -> Result<(), WireLimitError> {
        match self {
            Self::DiscoveryWorlds(values) => {
                if values.len() > MAX_DISCOVERY_RESULTS {
                    return Err(WireLimitError::TooManyDiscoveryResults(values.len()));
                }
                for value in values {
                    validate_announcement_size(value)?;
                }
                Ok(())
            }
            Self::DiscoveryResolved(Some(value)) => validate_announcement_size(value),
            Self::DiscoveryFreshnessContext(Some(proof)) => {
                if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES {
                    return Err(WireLimitError::TooManyDiscoveryMembershipCertificates(
                        proof.membership_certificates.len(),
                    ));
                }
                let member_count = proof
                    .membership_certificates
                    .iter()
                    .flat_map(|certificate| {
                        [certificate.proposal.previous.members.len(), certificate.proposal.proposed.members.len()]
                    })
                    .chain([proof.initial_membership.members.len(), proof.current_membership.members.len()])
                    .chain(
                        proof
                            .pending_membership
                            .iter()
                            .flat_map(|proposal| [proposal.previous.members.len(), proposal.proposed.members.len()]),
                    )
                    .max()
                    .unwrap_or(0);
                if member_count > MAX_WORLD_MEMBERS {
                    return Err(WireLimitError::TooManyMembers(member_count));
                }
                let vote_count =
                    proof.membership_certificates.iter().map(|certificate| certificate.votes.len()).max().unwrap_or(0);
                if vote_count > MAX_MEMBERSHIP_VOTES {
                    return Err(WireLimitError::TooManyMembershipVotes(vote_count));
                }
                let bytes = serde_json::to_vec(proof.as_ref())
                    .map_err(|_| WireLimitError::DiscoveryMembershipProofTooLarge(usize::MAX))?;
                if bytes.len() > MAX_DISCOVERY_MEMBERSHIP_PROOF_BYTES {
                    return Err(WireLimitError::DiscoveryMembershipProofTooLarge(bytes.len()));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn validate_discovery_filter(filter: &DiscoveryFilterV1) -> Result<(), WireLimitError> {
    if filter.tags.len() > MAX_DISCOVERY_TAGS {
        return Err(WireLimitError::TooManyDiscoveryTags(filter.tags.len()));
    }
    let bytes = serde_json::to_vec(filter).map_err(|_| WireLimitError::DiscoveryFilterTooLarge(usize::MAX))?;
    if bytes.len() > MAX_DISCOVERY_QUERY_BYTES {
        return Err(WireLimitError::DiscoveryFilterTooLarge(bytes.len()));
    }
    Ok(())
}

fn validate_announcement_size(value: &WorldAnnouncementV1) -> Result<(), WireLimitError> {
    let bytes = serde_json::to_vec(value).map_err(|_| WireLimitError::DiscoveryAnnouncementTooLarge(usize::MAX))?;
    if bytes.len() > MAX_DISCOVERY_ANNOUNCEMENT_BYTES {
        return Err(WireLimitError::DiscoveryAnnouncementTooLarge(bytes.len()));
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WireLimitError {
    #[error("blob chunk is {0} bytes; maximum is {MAX_BLOB_CHUNK}")]
    BlobChunkTooLarge(usize),
    #[error("missing-blob request contains {0} hashes; maximum is {MAX_MISSING_BLOBS}")]
    TooManyBlobHashes(usize),
    #[error("membership record contains {0} peers; maximum is {MAX_WORLD_MEMBERS}")]
    TooManyMembers(usize),
    #[error("recovery certificate contains {0} votes; maximum is {MAX_RECOVERY_VOTES}")]
    TooManyRecoveryVotes(usize),
    #[error("membership certificate contains {0} votes; maximum is {MAX_MEMBERSHIP_VOTES}")]
    TooManyMembershipVotes(usize),
    #[error("world compatibility manifest contains {0} artifacts; maximum is {MAX_WORLD_ARTIFACTS}")]
    TooManyWorldArtifacts(usize),
    #[error("world presentation contains {0} tags; maximum is {MAX_PRESENTATION_TAGS}")]
    TooManyPresentationTags(usize),
    #[error("discovery response contains {0} worlds; maximum is {MAX_DISCOVERY_RESULTS}")]
    TooManyDiscoveryResults(usize),
    #[error("discovery filter contains {0} tags; maximum is {MAX_DISCOVERY_TAGS}")]
    TooManyDiscoveryTags(usize),
    #[error("discovery filter is {0} encoded bytes; maximum is {MAX_DISCOVERY_QUERY_BYTES}")]
    DiscoveryFilterTooLarge(usize),
    #[error("world discovery announcement is {0} encoded bytes; maximum is {MAX_DISCOVERY_ANNOUNCEMENT_BYTES}")]
    DiscoveryAnnouncementTooLarge(usize),
    #[error(
        "discovery membership proof contains {0} certificates; maximum is {MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES}"
    )]
    TooManyDiscoveryMembershipCertificates(usize),
    #[error("discovery membership proof is {0} encoded bytes; maximum is {MAX_DISCOVERY_MEMBERSHIP_PROOF_BYTES}")]
    DiscoveryMembershipProofTooLarge(usize),
    #[error("handshake transport peer identifier exceeds {MAX_HANDSHAKE_TRANSPORT_ID_BYTES} bytes")]
    HandshakeTransportIdTooLarge,
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{
        AuthorityPolicyV1, DiscoveryCompatibilityV1, MembershipPolicyV1, PeerId, RuntimeCompatibilityManifestV1,
        WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
    };

    #[test]
    fn rejects_oversized_blob_chunks() {
        let request = WireRequest::BlobChunk {
            world_id: WorldId([1; 32]),
            hash: Hash32([2; 32]),
            encoding: BlobEncoding::Zstd,
            offset: 0,
            data: vec![0; MAX_BLOB_CHUNK + 1],
            finished: false,
        };
        assert_eq!(request.validate_limits(), Err(WireLimitError::BlobChunkTooLarge(MAX_BLOB_CHUNK + 1)));
    }

    #[test]
    fn rejects_unbounded_missing_blob_lists() {
        let request = WireRequest::MissingBlobs {
            world_id: WorldId([1; 32]),
            snapshot_number: 4,
            hashes: vec![Hash32([2; 32]); MAX_MISSING_BLOBS + 1],
        };
        assert_eq!(request.validate_limits(), Err(WireLimitError::TooManyBlobHashes(MAX_MISSING_BLOBS + 1)));
    }

    #[test]
    fn rejects_unbounded_world_presentation_tags() {
        let config = WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            sequence: 1,
            previous_config_hash: None,
            compatibility: RuntimeCompatibilityManifestV1 {
                minecraft_version: "1.21.8".into(),
                loader_id: "fabric".into(),
                loader_version: "0.17.2".into(),
                swarmcraft_protocol_version: PROTOCOL_VERSION,
                fabric_adapter_version: "0.2.0".into(),
                required_server_mods: Vec::new(),
                required_client_mods: Vec::new(),
                datapacks: Vec::new(),
            },
            visibility: WorldVisibilityV1::Private,
            authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 3 },
            membership_policy: MembershipPolicyV1::InviteOnly,
            presentation: WorldPresentationV1 {
                name: "test".into(),
                description: String::new(),
                tags: vec!["tag".into(); MAX_PRESENTATION_TAGS + 1],
                icon_hash: None,
                approximate_region: None,
            },
            authority_peer_id: PeerId([2; 32]),
            authority_public_key: [3; 32],
            signature: Vec::new(),
        };
        assert_eq!(
            WireRequest::WorldConfig(Box::new(config)).validate_limits(),
            Err(WireLimitError::TooManyPresentationTags(MAX_PRESENTATION_TAGS + 1))
        );
    }

    #[test]
    fn rejects_oversized_discovery_filter() {
        let filter = DiscoveryFilterV1 {
            query: Some("x".repeat(MAX_DISCOVERY_QUERY_BYTES + 1)),
            limit: 10,
            ..Default::default()
        };
        assert!(matches!(
            WireRequest::DiscoveryPublic { filter }.validate_limits(),
            Err(WireLimitError::DiscoveryFilterTooLarge(_))
        ));
    }

    #[test]
    fn rejects_oversized_world_announcement() {
        let announcement = WorldAnnouncementV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            presentation: WorldPresentationV1 {
                name: "world".into(),
                description: "x".repeat(MAX_DISCOVERY_ANNOUNCEMENT_BYTES + 1),
                tags: Vec::new(),
                icon_hash: None,
                approximate_region: None,
            },
            compatibility: DiscoveryCompatibilityV1 {
                minecraft_version: "1.21.8".into(),
                loader_id: "fabric".into(),
                loader_version: "0.17.2".into(),
                fabric_adapter_version: "0.4.0".into(),
                compatibility_fingerprint: Hash32([2; 32]),
            },
            visibility: WorldVisibilityV1::Public,
            membership_policy: MembershipPolicyV1::InviteOnly,
            config_sequence: 1,
            config_hash: Hash32([3; 32]),
            membership_sequence: 0,
            membership_hash: Hash32([6; 32]),
            authority_epoch: 1,
            fencing_token: 1,
            canonical_head: None,
            announcement_sequence: 1,
            issued_unix_ms: 1,
            expires_unix_ms: 2,
            announcer_peer_id: PeerId([4; 32]),
            announcer_public_key: [5; 32],
            signature: vec![0; 64],
        };
        assert!(matches!(
            WireResponse::DiscoveryResolved(Some(Box::new(announcement))).validate_limits(),
            Err(WireLimitError::DiscoveryAnnouncementTooLarge(_))
        ));
    }
}
