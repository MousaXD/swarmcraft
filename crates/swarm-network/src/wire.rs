use serde::{Deserialize, Serialize};
use swarm_protocol::{
    AuthorityLeaseGrantV1, AuthorityTransferV1, BlobEncoding, DiscoveryFilterV1, EpochRecordV1, FriendPresenceV1,
    Hash32, JoinRequestV1, LeaveRequestV1, MembershipRecordV1, PeerHelloV1, PeerId, RecoveryBallotV1,
    RecoveryCertificateV1, RecoveryVoteV1, SleepRecordV1, SnapshotManifestV1, SoloBranchV1, WorldAnnouncementV1,
    WorldConfigV1, WorldDescriptorV1, WorldId, WorldStatusV1,
};
use thiserror::Error;

pub const MAX_BLOB_CHUNK: usize = 256 * 1024;
pub const MAX_MISSING_BLOBS: usize = 16_384;
pub const MAX_WORLD_MEMBERS: usize = 1_024;
pub const MAX_RECOVERY_VOTES: usize = 1_024;
pub const MAX_WORLD_ARTIFACTS: usize = 4_096;
pub const MAX_PRESENTATION_TAGS: usize = 64;
pub const MAX_DISCOVERY_RESULTS: usize = 64;
pub const MAX_DISCOVERY_TAGS: usize = 16;
pub const MAX_DISCOVERY_QUERY_BYTES: usize = 512;
pub const MAX_DISCOVERY_ANNOUNCEMENT_BYTES: usize = 16 * 1024;

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
pub enum WireRequest {
    Hello(PeerHelloV1),
    Ping { nonce: u64 },
    WorldStatus { world_id: WorldId },
    WorldDescriptor { world_id: WorldId },
    JoinRequest(Box<JoinRequestV1>),
    LeaveRequest(Box<LeaveRequestV1>),
    SnapshotManifest(SnapshotManifestV1),
    MissingBlobs { world_id: WorldId, snapshot_number: u64, hashes: Vec<Hash32> },
    BlobChunk { world_id: WorldId, hash: Hash32, encoding: BlobEncoding, offset: u64, data: Vec<u8>, finished: bool },
    ReplicaAck(ReplicaAckV1),
    Membership(MembershipRecordV1),
    Epoch(EpochRecordV1),
    AuthorityTransfer(AuthorityTransferV1),
    LeaseGrant(AuthorityLeaseGrantV1),
    Sleep(SleepRecordV1),
    // 0.2 extensions are appended so existing postcard enum discriminants remain stable.
    RecoveryBallot(Box<RecoveryBallotV1>),
    RecoveryEpoch { record: EpochRecordV1, certificate: Box<RecoveryCertificateV1> },
    WorldConfig(Box<WorldConfigV1>),
    SoloBranch(Box<SoloBranchV1>),
    // Host readiness is appended so all earlier postcard discriminants stay stable.
    HostCapability { world_id: WorldId },
    // Discovery extensions are append-only for postcard compatibility.
    DiscoveryPublic { filter: DiscoveryFilterV1 },
    DiscoveryResolve { world_id: WorldId },
    FriendPresence { expected_peer_id: PeerId, requester_peer_id: PeerId, nonce: [u8; 32] },
}

impl WireRequest {
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
    DiscoveryResolved(Option<WorldAnnouncementV1>),
    FriendPresence(Option<FriendPresenceV1>),
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
            _ => Ok(()),
        }
    }
}

fn validate_discovery_filter(filter: &DiscoveryFilterV1) -> Result<(), WireLimitError> {
    if filter.tags.len() > MAX_DISCOVERY_TAGS {
        return Err(WireLimitError::TooManyDiscoveryTags(filter.tags.len()));
    }
    let bytes = postcard::to_allocvec(filter).map_err(|_| WireLimitError::DiscoveryFilterTooLarge(usize::MAX))?;
    if bytes.len() > MAX_DISCOVERY_QUERY_BYTES {
        return Err(WireLimitError::DiscoveryFilterTooLarge(bytes.len()));
    }
    Ok(())
}

fn validate_announcement_size(value: &WorldAnnouncementV1) -> Result<(), WireLimitError> {
    let bytes = postcard::to_allocvec(value).map_err(|_| WireLimitError::DiscoveryAnnouncementTooLarge(usize::MAX))?;
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
            authority_epoch: 1,
            fencing_token: 1,
            announcement_sequence: 1,
            issued_unix_ms: 1,
            expires_unix_ms: 2,
            announcer_peer_id: PeerId([4; 32]),
            announcer_public_key: [5; 32],
            signature: vec![0; 64],
        };
        assert!(matches!(
            WireResponse::DiscoveryResolved(Some(announcement)).validate_limits(),
            Err(WireLimitError::DiscoveryAnnouncementTooLarge(_))
        ));
    }
}
