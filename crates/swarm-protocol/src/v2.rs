use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::{Hash32, MembershipRecordV1, PeerId, ProtocolError, WorldId, PROTOCOL_VERSION};

const COMPATIBILITY_MANIFEST_DOMAIN: &[u8] = b"swarmcraft/compatibility-manifest/v1\0";
const WORLD_CONFIG_SIGN_DOMAIN: &[u8] = b"swarmcraft/world-config-sign/v1\0";
const WORLD_CONFIG_HASH_DOMAIN: &[u8] = b"swarmcraft/world-config/v1\0";
const MEMBERSHIP_PROPOSAL_HASH_DOMAIN: &[u8] = b"swarmcraft/membership-proposal/v1\0";
const MEMBERSHIP_VOTE_SIGN_DOMAIN: &[u8] = b"swarmcraft/membership-vote-sign/v1\0";
const RECOVERY_BALLOT_SIGN_DOMAIN: &[u8] = b"swarmcraft/recovery-ballot-sign/v1\0";
const RECOVERY_BALLOT_HASH_DOMAIN: &[u8] = b"swarmcraft/recovery-ballot/v1\0";
const RECOVERY_VOTE_SIGN_DOMAIN: &[u8] = b"swarmcraft/recovery-vote-sign/v1\0";
const SOLO_BRANCH_SIGN_DOMAIN: &[u8] = b"swarmcraft/solo-branch-sign/v1\0";
const SOLO_BRANCH_HASH_DOMAIN: &[u8] = b"swarmcraft/solo-branch/v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ArtifactSideV1 {
    Server,
    Client,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRequirementV1 {
    /// Stable ecosystem identifier, for example a Fabric mod ID or datapack namespace.
    pub artifact_id: String,
    pub version: String,
    /// Hash of the exact artifact bytes expected by the world configuration.
    pub artifact_hash: Hash32,
    pub side: ArtifactSideV1,
    /// Optional non-canonical discovery hint such as a Modrinth project ID.
    pub provider_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCompatibilityManifestV1 {
    pub minecraft_version: String,
    pub loader_id: String,
    pub loader_version: String,
    pub swarmcraft_protocol_version: u16,
    pub fabric_adapter_version: String,
    pub required_server_mods: Vec<ArtifactRequirementV1>,
    pub required_client_mods: Vec<ArtifactRequirementV1>,
    pub datapacks: Vec<ArtifactRequirementV1>,
}

impl RuntimeCompatibilityManifestV1 {
    pub fn normalize(&mut self) {
        normalize_artifacts(&mut self.required_server_mods);
        normalize_artifacts(&mut self.required_client_mods);
        normalize_artifacts(&mut self.datapacks);
    }

    pub fn fingerprint(&self) -> Result<Hash32, ProtocolError> {
        let mut normalized = self.clone();
        normalized.normalize();
        let bytes = postcard::to_allocvec(&normalized)?;
        Ok(Hash32::from_domain_bytes(COMPATIBILITY_MANIFEST_DOMAIN, &bytes))
    }
}

fn normalize_artifacts(values: &mut Vec<ArtifactRequirementV1>) {
    values.sort_by(|a, b| {
        a.artifact_id
            .cmp(&b.artifact_id)
            .then(a.version.cmp(&b.version))
            .then(a.artifact_hash.0.cmp(&b.artifact_hash.0))
            .then(a.side.cmp(&b.side))
    });
    values.dedup_by(|a, b| {
        a.artifact_id == b.artifact_id
            && a.version == b.version
            && a.artifact_hash == b.artifact_hash
            && a.side == b.side
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldVisibilityV1 {
    Private,
    Unlisted,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MembershipPolicyV1 {
    InviteOnly,
    ApprovalRequired,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityPolicyV1 {
    pub allow_solo_advancement: bool,
    pub preferred_replication_factor: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldPresentationV1 {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub icon_hash: Option<Hash32>,
    pub approximate_region: Option<String>,
}

impl WorldPresentationV1 {
    pub fn normalize(&mut self) {
        self.tags.sort();
        self.tags.dedup();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldConfigV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub sequence: u64,
    pub previous_config_hash: Option<Hash32>,
    pub compatibility: RuntimeCompatibilityManifestV1,
    pub visibility: WorldVisibilityV1,
    pub authority_policy: AuthorityPolicyV1,
    pub membership_policy: MembershipPolicyV1,
    pub presentation: WorldPresentationV1,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl WorldConfigV1 {
    fn canonical_unsigned(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut compatibility = self.compatibility.clone();
        compatibility.normalize();
        let mut presentation = self.presentation.clone();
        presentation.normalize();
        Ok(postcard::to_allocvec(&(
            self.protocol_version,
            self.world_id,
            self.sequence,
            self.previous_config_hash,
            compatibility,
            self.visibility,
            &self.authority_policy,
            self.membership_policy,
            presentation,
            self.authority_peer_id,
            self.authority_public_key,
        ))?)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let encoded = self.canonical_unsigned()?;
        let mut bytes = Vec::with_capacity(WORLD_CONFIG_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(WORLD_CONFIG_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    pub fn config_hash(&self) -> Result<Hash32, ProtocolError> {
        let encoded = self.canonical_unsigned()?;
        Ok(Hash32::from_domain_bytes(WORLD_CONFIG_HASH_DOMAIN, &encoded))
    }

    pub fn compatibility_fingerprint(&self) -> Result<Hash32, ProtocolError> {
        self.compatibility.fingerprint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipProposalV1 {
    pub previous: MembershipRecordV1,
    pub proposed: MembershipRecordV1,
}

impl MembershipProposalV1 {
    pub fn validate_shape(&self) -> Result<bool, ProtocolError> {
        Ok(self.previous.protocol_version == PROTOCOL_VERSION
            && self.proposed.protocol_version == PROTOCOL_VERSION
            && self.previous.world_id == self.proposed.world_id
            && self.previous.epoch == self.proposed.epoch
            && self.previous.sequence.checked_add(1) == Some(self.proposed.sequence)
            && self.proposed.previous_membership_hash == Some(self.previous.record_hash()?)
            && self.previous.authority_peer_id == self.proposed.authority_peer_id
            && self.previous.authority_public_key == self.proposed.authority_public_key
            && !self.proposed.members.is_empty())
    }

    pub fn proposal_hash(&self) -> Result<Hash32, ProtocolError> {
        let bytes = postcard::to_allocvec(&(self.previous.record_hash()?, self.proposed.record_hash()?))?;
        Ok(Hash32::from_domain_bytes(MEMBERSHIP_PROPOSAL_HASH_DOMAIN, &bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipVoteV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub previous_membership_hash: Hash32,
    pub proposed_membership_hash: Hash32,
    pub proposed_sequence: u64,
    pub voter_peer_id: PeerId,
    pub voter_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl MembershipVoteV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.previous_membership_hash,
            self.proposed_membership_hash,
            self.proposed_sequence,
            self.voter_peer_id,
            self.voter_public_key,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(MEMBERSHIP_VOTE_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(MEMBERSHIP_VOTE_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    pub fn matches_proposal(&self, proposal: &MembershipProposalV1) -> Result<bool, ProtocolError> {
        Ok(self.protocol_version == PROTOCOL_VERSION
            && self.world_id == proposal.proposed.world_id
            && self.previous_membership_hash == proposal.previous.record_hash()?
            && self.proposed_membership_hash == proposal.proposed.record_hash()?
            && self.proposed_sequence == proposal.proposed.sequence)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipCertificateV1 {
    pub proposal: MembershipProposalV1,
    pub votes: Vec<MembershipVoteV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryBallotV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub base_epoch: u64,
    pub base_fencing_token: u64,
    pub target_epoch: u64,
    pub target_fencing_token: u64,
    pub round: u64,
    pub candidate_peer_id: PeerId,
    pub candidate_public_key: [u8; 32],
    pub base_snapshot_hash: Hash32,
    pub base_state_hash: Hash32,
    pub membership_hash: Hash32,
    pub signature: Vec<u8>,
}

impl RecoveryBallotV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.base_epoch,
            self.base_fencing_token,
            self.target_epoch,
            self.target_fencing_token,
            self.round,
            self.candidate_peer_id,
            self.candidate_public_key,
            self.base_snapshot_hash,
            self.base_state_hash,
            self.membership_hash,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(RECOVERY_BALLOT_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(RECOVERY_BALLOT_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    pub fn ballot_hash(&self) -> Result<Hash32, ProtocolError> {
        Ok(Hash32::from_domain_bytes(RECOVERY_BALLOT_HASH_DOMAIN, &self.signing_bytes()?))
    }

    pub fn generation_is_well_formed(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
            && self.base_epoch.checked_add(1) == Some(self.target_epoch)
            && self.base_fencing_token.checked_add(1) == Some(self.target_fencing_token)
            && self.round > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryVoteV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub ballot_hash: Hash32,
    pub base_epoch: u64,
    pub target_epoch: u64,
    pub round: u64,
    pub candidate_peer_id: PeerId,
    pub voter_peer_id: PeerId,
    pub voter_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl RecoveryVoteV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.ballot_hash,
            self.base_epoch,
            self.target_epoch,
            self.round,
            self.candidate_peer_id,
            self.voter_peer_id,
            self.voter_public_key,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(RECOVERY_VOTE_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(RECOVERY_VOTE_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    pub fn matches_ballot(&self, ballot: &RecoveryBallotV1) -> Result<bool, ProtocolError> {
        Ok(self.protocol_version == ballot.protocol_version
            && self.world_id == ballot.world_id
            && self.ballot_hash == ballot.ballot_hash()?
            && self.base_epoch == ballot.base_epoch
            && self.target_epoch == ballot.target_epoch
            && self.round == ballot.round
            && self.candidate_peer_id == ballot.candidate_peer_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCertificateV1 {
    pub ballot: RecoveryBallotV1,
    pub votes: Vec<RecoveryVoteV1>,
}

impl RecoveryCertificateV1 {
    pub fn unique_voters(&self) -> BTreeSet<PeerId> {
        self.votes.iter().map(|vote| vote.voter_peer_id).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldSafetyLevelV1 {
    Canonical,
    Replicated,
    QuorumConfirmed,
    SoloUnreplicated,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoloBranchV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub base_snapshot_hash: Hash32,
    pub base_epoch: u64,
    pub head_snapshot_hash: Hash32,
    pub head_epoch: u64,
    pub head_sequence: u64,
    pub state_hash: Hash32,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl SoloBranchV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.base_snapshot_hash,
            self.base_epoch,
            self.head_snapshot_hash,
            self.head_epoch,
            self.head_sequence,
            self.state_hash,
            self.authority_peer_id,
            self.authority_public_key,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(SOLO_BRANCH_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(SOLO_BRANCH_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    pub fn branch_hash(&self) -> Result<Hash32, ProtocolError> {
        Ok(Hash32::from_domain_bytes(SOLO_BRANCH_HASH_DOMAIN, &self.signing_bytes()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(id: &str, byte: u8) -> ArtifactRequirementV1 {
        ArtifactRequirementV1 {
            artifact_id: id.into(),
            version: "1.0.0".into(),
            artifact_hash: Hash32([byte; 32]),
            side: ArtifactSideV1::Server,
            provider_hint: None,
        }
    }

    fn compatibility(mods: Vec<ArtifactRequirementV1>) -> RuntimeCompatibilityManifestV1 {
        RuntimeCompatibilityManifestV1 {
            minecraft_version: "1.21.8".into(),
            loader_id: "fabric".into(),
            loader_version: "0.17.2".into(),
            swarmcraft_protocol_version: PROTOCOL_VERSION,
            fabric_adapter_version: "0.2.0".into(),
            required_server_mods: mods,
            required_client_mods: Vec::new(),
            datapacks: Vec::new(),
        }
    }

    #[test]
    fn compatibility_fingerprint_is_order_independent_after_canonical_normalization() {
        let a = compatibility(vec![artifact("zeta", 9), artifact("alpha", 1)]);
        let b = compatibility(vec![artifact("alpha", 1), artifact("zeta", 9)]);
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
    }

    #[test]
    fn compatibility_fingerprint_changes_when_exact_artifact_changes() {
        let a = compatibility(vec![artifact("example", 1)]);
        let b = compatibility(vec![artifact("example", 2)]);
        assert_ne!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
    }

    #[test]
    fn recovery_ballot_binds_candidate_round_and_canonical_base() {
        let mut ballot = RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            base_epoch: 5,
            base_fencing_token: 9,
            target_epoch: 6,
            target_fencing_token: 10,
            round: 1,
            candidate_peer_id: PeerId([2; 32]),
            candidate_public_key: [3; 32],
            base_snapshot_hash: Hash32([4; 32]),
            base_state_hash: Hash32([5; 32]),
            membership_hash: Hash32([6; 32]),
            signature: Vec::new(),
        };
        assert!(ballot.generation_is_well_formed());
        let first = ballot.ballot_hash().unwrap();
        ballot.round = 2;
        assert_ne!(first, ballot.ballot_hash().unwrap());
        ballot.round = 1;
        ballot.candidate_peer_id = PeerId([8; 32]);
        assert_ne!(first, ballot.ballot_hash().unwrap());
    }

    #[test]
    fn presentation_tag_order_does_not_change_config_hash() {
        let mut a = WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            sequence: 1,
            previous_config_hash: None,
            compatibility: compatibility(Vec::new()),
            visibility: WorldVisibilityV1::Public,
            authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 3 },
            membership_policy: MembershipPolicyV1::ApprovalRequired,
            presentation: WorldPresentationV1 {
                name: "Friends".into(),
                description: "test".into(),
                tags: vec!["survival".into(), "vanilla-plus".into()],
                icon_hash: None,
                approximate_region: Some("me-central".into()),
            },
            authority_peer_id: PeerId([2; 32]),
            authority_public_key: [3; 32],
            signature: Vec::new(),
        };
        let mut b = a.clone();
        b.presentation.tags.reverse();
        assert_eq!(a.config_hash().unwrap(), b.config_hash().unwrap());
        a.presentation.tags.push("survival".into());
        assert_eq!(a.config_hash().unwrap(), b.config_hash().unwrap());
    }
}
