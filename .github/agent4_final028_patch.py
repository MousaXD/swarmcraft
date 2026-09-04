from pathlib import Path


def replace(path, old, new):
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing patch anchor in {path}: {old[:120]!r}")
    p.write_text(text.replace(old, new, 1))


def append_before(path, marker, block):
    p = Path(path)
    text = p.read_text()
    if block.strip() in text:
        return
    if marker not in text:
        raise SystemExit(f"missing insertion marker in {path}: {marker!r}")
    p.write_text(text.replace(marker, block + "\n" + marker, 1))

# ---------------------------------------------------------------------------
# Agent 3 composition seam in daemon.rs. The merge workflow resolves daemon.rs
# to Agent 4's already-validated side and this restores Agent 3's fenced final
# snapshot commit without disturbing Agent 4 network/auth hardening.
# ---------------------------------------------------------------------------
replace(
    "crates/swarm-cli/src/daemon.rs",
    "        identity.sign_snapshot(&mut promoted)?;\n        storage.commit_snapshot(&promoted)?;\n",
    "        identity.sign_snapshot(&mut promoted)?;\n        let promoted_expected_head = storage.canonical_snapshot_head(promoted.world_id)?.head;\n        storage.commit_snapshot_fenced(\n            &promoted,\n            swarm_storage::SnapshotCommitFence {\n                expected_epoch: epoch.epoch_number,\n                expected_fencing_token: epoch.fencing_token,\n                expected_head: promoted_expected_head,\n            },\n        )?;\n",
)

# ---------------------------------------------------------------------------
# Protocol: challenge-bound discovery authority freshness records.
# ---------------------------------------------------------------------------
replace(
    "crates/swarm-protocol/src/discovery.rs",
    "    Hash32, MembershipPolicyV1, PeerId, ProtocolError, WorldId, WorldPresentationV1, WorldVisibilityV1,\n    PROTOCOL_VERSION,\n",
    "    Hash32, MembershipCertificateV1, MembershipPolicyV1, MembershipProposalV1, MembershipRecordV1, PeerId,\n    ProtocolError, WorldGenesisV1, WorldId, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,\n",
)
replace(
    "crates/swarm-protocol/src/discovery.rs",
    "const WORLD_ANNOUNCEMENT_SIGN_DOMAIN: &[u8] = b\"swarmcraft/world-announcement/v1\\0\";\nconst FRIEND_PRESENCE_SIGN_DOMAIN: &[u8] = b\"swarmcraft/friend-presence/v1\\0\";\n",
    "const WORLD_ANNOUNCEMENT_SIGN_DOMAIN: &[u8] = b\"swarmcraft/world-announcement/v1\\0\";\nconst WORLD_ANNOUNCEMENT_HASH_DOMAIN: &[u8] = b\"swarmcraft/world-announcement-hash/v1\\0\";\nconst DISCOVERY_FRESHNESS_VOTE_SIGN_DOMAIN: &[u8] = b\"swarmcraft/discovery-freshness-vote/v1\\0\";\nconst FRIEND_PRESENCE_SIGN_DOMAIN: &[u8] = b\"swarmcraft/friend-presence/v1\\0\";\n",
)
replace(
    "crates/swarm-protocol/src/discovery.rs",
    "    /// Current canonical authority generation that authorized publication.\n    pub authority_epoch: u64,\n    pub fencing_token: u64,\n    /// Monotonic-within-authority publication sequence used for replay rejection.\n",
    "    /// Exact committed membership identity used by the live freshness quorum.\n    pub membership_sequence: u64,\n    pub membership_hash: Hash32,\n    /// Current canonical authority generation that authorized publication.\n    pub authority_epoch: u64,\n    pub fencing_token: u64,\n    /// Exact durable canonical snapshot head. `None` is an explicit empty-head state.\n    pub canonical_head: Option<DiscoveryCanonicalHeadV1>,\n    /// Monotonic-within-authority publication sequence used for replay rejection.\n",
)
replace(
    "crates/swarm-protocol/src/discovery.rs",
    "            self.config_sequence,\n            self.config_hash,\n            self.authority_epoch,\n            self.fencing_token,\n            self.announcement_sequence,\n",
    "            self.config_sequence,\n            self.config_hash,\n            self.membership_sequence,\n            self.membership_hash,\n            self.authority_epoch,\n            self.fencing_token,\n            self.canonical_head,\n            self.announcement_sequence,\n",
)
replace(
    "crates/swarm-protocol/src/discovery.rs",
    "    pub fn is_discoverable_visibility(&self) -> bool {\n        matches!(self.visibility, WorldVisibilityV1::Public | WorldVisibilityV1::Unlisted)\n    }\n}\n\n/// Challenge-bound liveness proof",
    "    pub fn is_discoverable_visibility(&self) -> bool {\n        matches!(self.visibility, WorldVisibilityV1::Public | WorldVisibilityV1::Unlisted)\n    }\n\n    pub fn announcement_hash(&self) -> Result<Hash32, ProtocolError> {\n        let encoded = postcard::to_allocvec(self)?;\n        Ok(Hash32::from_domain_bytes(WORLD_ANNOUNCEMENT_HASH_DOMAIN, &encoded))\n    }\n}\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]\npub struct DiscoveryCanonicalHeadV1 {\n    pub snapshot_number: u64,\n    pub manifest_hash: Hash32,\n    pub epoch: u64,\n    pub sequence: u64,\n}\n\n/// First-contact membership trust material. Membership-changing transitions are\n/// anchored to genesis by Agent 1 joint certificates. Same-voter authority/epoch\n/// refreshes are authenticated by the live quorum challenge rather than by\n/// treating an old authority signature as proof of currentness.\n#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct DiscoveryMembershipProofV1 {\n    pub protocol_version: u16,\n    pub world_id: WorldId,\n    pub genesis: WorldGenesisV1,\n    pub initial_membership: MembershipRecordV1,\n    pub membership_certificates: Vec<MembershipCertificateV1>,\n    pub current_membership: MembershipRecordV1,\n    pub pending_membership: Option<MembershipProposalV1>,\n}\n\n#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct DiscoveryFreshnessChallengeV1 {\n    pub protocol_version: u16,\n    pub verifier_peer_id: PeerId,\n    pub nonce: [u8; 32],\n    pub world_id: WorldId,\n    pub announcement_hash: Hash32,\n    pub membership_sequence: u64,\n    pub membership_hash: Hash32,\n    pub pending_membership_proposal_hash: Option<Hash32>,\n    pub authority_peer_id: PeerId,\n    pub authority_epoch: u64,\n    pub fencing_token: u64,\n    pub config_sequence: u64,\n    pub config_hash: Hash32,\n    pub canonical_head: Option<DiscoveryCanonicalHeadV1>,\n    pub issued_unix_ms: u64,\n    pub expires_unix_ms: u64,\n}\n\n#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct DiscoveryFreshnessVoteV1 {\n    pub challenge: DiscoveryFreshnessChallengeV1,\n    pub voter_peer_id: PeerId,\n    pub voter_public_key: [u8; 32],\n    pub signature: Vec<u8>,\n}\n\nimpl DiscoveryFreshnessVoteV1 {\n    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {\n        let unsigned = (&self.challenge, self.voter_peer_id, self.voter_public_key);\n        let encoded = postcard::to_allocvec(&unsigned)?;\n        let mut bytes = Vec::with_capacity(DISCOVERY_FRESHNESS_VOTE_SIGN_DOMAIN.len() + encoded.len());\n        bytes.extend_from_slice(DISCOVERY_FRESHNESS_VOTE_SIGN_DOMAIN);\n        bytes.extend_from_slice(&encoded);\n        Ok(bytes)\n    }\n}\n\n/// Challenge-bound liveness proof",
)
# Protocol unit fixture fields.
replace(
    "crates/swarm-protocol/src/discovery.rs",
    "            config_sequence: 3,\n            config_hash: Hash32([3; 32]),\n            authority_epoch: 4,\n            fencing_token: 5,\n            announcement_sequence: 6,\n",
    "            config_sequence: 3,\n            config_hash: Hash32([3; 32]),\n            membership_sequence: 3,\n            membership_hash: Hash32([7; 32]),\n            authority_epoch: 4,\n            fencing_token: 5,\n            canonical_head: Some(DiscoveryCanonicalHeadV1 {\n                snapshot_number: 9,\n                manifest_hash: Hash32([8; 32]),\n                epoch: 4,\n                sequence: 12,\n            }),\n            announcement_sequence: 6,\n",
)

# ---------------------------------------------------------------------------
# Core: cryptographic/static proof checks and one-shot verifier replay guard.
# ---------------------------------------------------------------------------
replace(
    "crates/swarm-core/src/discovery.rs",
    "use std::collections::HashMap;\n\nuse swarm_protocol::{FriendPresenceV1, PeerId, WorldAnnouncementV1, WorldId, WorldVisibilityV1, PROTOCOL_VERSION};\n\nuse crate::{verify_signature, CoreError, PeerIdentity};\n",
    "use std::collections::{BTreeSet, HashMap, HashSet};\n\nuse swarm_protocol::{\n    peer_id_from_public_key, DiscoveryFreshnessChallengeV1, DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1,\n    FriendPresenceV1, PeerId, WorldAnnouncementV1, WorldId, WorldVisibilityV1, PROTOCOL_VERSION,\n};\n\nuse crate::{verify_membership_signature, verify_signature, CoreError, PeerIdentity};\n",
)
replace(
    "crates/swarm-core/src/discovery.rs",
    "pub const DISCOVERY_CLOCK_SKEW_MS: u64 = 60 * 1_000;\n",
    "pub const DISCOVERY_CLOCK_SKEW_MS: u64 = 60 * 1_000;\npub const DISCOVERY_FRESHNESS_MAX_LIFETIME_MS: u64 = 15 * 1_000;\npub const MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES: usize = 256;\n",
)
replace(
    "crates/swarm-core/src/discovery.rs",
    "    #[error(\"signature or cryptographic peer identity is invalid\")]\n    InvalidSignature,\n",
    "    #[error(\"signature or cryptographic peer identity is invalid\")]\n    InvalidSignature,\n    #[error(\"discovery membership proof is malformed or not genesis anchored\")]\n    InvalidMembershipProof,\n    #[error(\"discovery freshness challenge does not bind the advertised canonical state\")]\n    FreshnessStateMismatch,\n    #[error(\"discovery freshness challenge has already been accepted\")]\n    FreshnessReplay,\n",
)
insert_core = r'''

pub fn verify_discovery_membership_proof(
    announcement: &WorldAnnouncementV1,
    proof: &DiscoveryMembershipProofV1,
) -> Result<(), DiscoveryRecordError> {
    if proof.protocol_version != PROTOCOL_VERSION || proof.world_id != announcement.world_id {
        return Err(DiscoveryRecordError::ProtocolMismatch);
    }
    if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES
        || proof.genesis.world_id().ok() != Some(proof.world_id)
    {
        return Err(DiscoveryRecordError::InvalidMembershipProof);
    }
    let initial = &proof.initial_membership;
    if initial.protocol_version != PROTOCOL_VERSION
        || initial.world_id != proof.world_id
        || initial.sequence != 0
        || initial.previous_membership_hash.is_some()
        || initial.authority_public_key != proof.genesis.creator_public_key
        || initial.authority_peer_id != peer_id_from_public_key(&proof.genesis.creator_public_key)
    {
        return Err(DiscoveryRecordError::InvalidMembershipProof);
    }
    verify_membership_signature(initial).map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
    let genesis_members = proof.genesis.initial_membership.iter().copied().collect::<BTreeSet<_>>();
    let initial_members = initial.members.iter().map(|member| member.peer_id).collect::<BTreeSet<_>>();
    if genesis_members.len() != proof.genesis.initial_membership.len()
        || initial_members.len() != initial.members.len()
        || genesis_members != initial_members
        || initial
            .members
            .iter()
            .any(|member| peer_id_from_public_key(&member.public_key) != member.peer_id)
    {
        return Err(DiscoveryRecordError::InvalidMembershipProof);
    }

    for certificate in &proof.membership_certificates {
        verify_membership_signature(&certificate.proposal.previous)
            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
        verify_membership_signature(&certificate.proposal.proposed)
            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
        let mut seen = HashSet::new();
        for vote in &certificate.votes {
            if !seen.insert(vote.voter_peer_id) {
                return Err(DiscoveryRecordError::InvalidMembershipProof);
            }
            verify_signature(
                vote.voter_peer_id,
                vote.voter_public_key,
                &vote.signing_bytes().map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?,
                &vote.signature,
            )
            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
        }
    }
    verify_membership_signature(&proof.current_membership)
        .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
    if proof.current_membership.world_id != proof.world_id
        || proof.current_membership.sequence != announcement.membership_sequence
        || proof.current_membership.record_hash().ok() != Some(announcement.membership_hash)
        || proof.current_membership.authority_peer_id != announcement.announcer_peer_id
        || proof.current_membership.authority_public_key != announcement.announcer_public_key
        || proof.current_membership.epoch != announcement.authority_epoch
    {
        return Err(DiscoveryRecordError::InvalidMembershipProof);
    }
    Ok(())
}

pub fn sign_discovery_freshness_vote(
    identity: &PeerIdentity,
    challenge: &DiscoveryFreshnessChallengeV1,
) -> Result<DiscoveryFreshnessVoteV1, CoreError> {
    let mut vote = DiscoveryFreshnessVoteV1 {
        challenge: challenge.clone(),
        voter_peer_id: identity.peer_id(),
        voter_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    vote.signature = identity.sign(&vote.signing_bytes()?);
    Ok(vote)
}

pub fn verify_discovery_freshness_vote(
    vote: &DiscoveryFreshnessVoteV1,
    expected: &DiscoveryFreshnessChallengeV1,
) -> Result<(), DiscoveryRecordError> {
    if vote.challenge != *expected || peer_id_from_public_key(&vote.voter_public_key) != vote.voter_peer_id {
        return Err(DiscoveryRecordError::FreshnessStateMismatch);
    }
    verify_signature(
        vote.voter_peer_id,
        vote.voter_public_key,
        &vote.signing_bytes().map_err(|_| DiscoveryRecordError::InvalidSignature)?,
        &vote.signature,
    )
    .map_err(|_| DiscoveryRecordError::InvalidSignature)
}

pub fn verify_discovery_freshness_challenge(
    announcement: &WorldAnnouncementV1,
    proof: &DiscoveryMembershipProofV1,
    challenge: &DiscoveryFreshnessChallengeV1,
    verifier_peer_id: PeerId,
    nonce: [u8; 32],
    now_unix_ms: u64,
) -> Result<(), DiscoveryRecordError> {
    verify_discovery_membership_proof(announcement, proof)?;
    let pending_hash = proof
        .pending_membership
        .as_ref()
        .map(|proposal| proposal.proposal_hash())
        .transpose()
        .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
    if challenge.protocol_version != PROTOCOL_VERSION
        || challenge.verifier_peer_id != verifier_peer_id
        || challenge.nonce != nonce
        || challenge.world_id != announcement.world_id
        || challenge.announcement_hash != announcement.announcement_hash().map_err(|_| DiscoveryRecordError::InvalidSignature)?
        || challenge.membership_sequence != announcement.membership_sequence
        || challenge.membership_hash != announcement.membership_hash
        || challenge.pending_membership_proposal_hash != pending_hash
        || challenge.authority_peer_id != announcement.announcer_peer_id
        || challenge.authority_epoch != announcement.authority_epoch
        || challenge.fencing_token != announcement.fencing_token
        || challenge.config_sequence != announcement.config_sequence
        || challenge.config_hash != announcement.config_hash
        || challenge.canonical_head != announcement.canonical_head
    {
        return Err(DiscoveryRecordError::FreshnessStateMismatch);
    }
    validate_lifetime(
        challenge.issued_unix_ms,
        challenge.expires_unix_ms,
        DISCOVERY_FRESHNESS_MAX_LIFETIME_MS,
        now_unix_ms,
    )
}

#[derive(Debug, Default)]
pub struct DiscoveryFreshnessReplayGuard {
    accepted: HashSet<(PeerId, [u8; 32])>,
}

impl DiscoveryFreshnessReplayGuard {
    pub fn accept(&mut self, challenge: &DiscoveryFreshnessChallengeV1) -> Result<(), DiscoveryRecordError> {
        if !self.accepted.insert((challenge.verifier_peer_id, challenge.nonce)) {
            return Err(DiscoveryRecordError::FreshnessReplay);
        }
        Ok(())
    }
}
'''
append_before("crates/swarm-core/src/discovery.rs", "fn validate_lifetime(", insert_core)
# Core fixture fields.
replace(
    "crates/swarm-core/src/discovery.rs",
    "            config_sequence: 1,\n            config_hash: Hash32([3; 32]),\n            authority_epoch: 2,\n            fencing_token: 3,\n            announcement_sequence: 10,\n",
    "            config_sequence: 1,\n            config_hash: Hash32([3; 32]),\n            membership_sequence: 0,\n            membership_hash: Hash32([7; 32]),\n            authority_epoch: 2,\n            fencing_token: 3,\n            canonical_head: None,\n            announcement_sequence: 10,\n",
)

# ---------------------------------------------------------------------------
# Consensus: exact Agent 1 steady/joint quorum rules for freshness signer sets.
# ---------------------------------------------------------------------------
replace(
    "crates/swarm-consensus/src/membership.rs",
    "use swarm_protocol::{MembershipCertificateV1, MembershipProposalV1, MembershipVoteV1, PeerId, WorldMemberV1};\n",
    "use swarm_protocol::{\n    DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1, MembershipCertificateV1, MembershipProposalV1,\n    MembershipVoteV1, PeerId, WorldMemberV1,\n};\n",
)
replace(
    "crates/swarm-consensus/src/membership.rs",
    "    #[error(\"new membership quorum unavailable: votes={votes}, required={required}\")]\n    NewQuorumUnavailable { votes: usize, required: usize },\n",
    "    #[error(\"new membership quorum unavailable: votes={votes}, required={required}\")]\n    NewQuorumUnavailable { votes: usize, required: usize },\n    #[error(\"current membership quorum unavailable: votes={votes}, required={required}\")]\n    CurrentQuorumUnavailable { votes: usize, required: usize },\n    #[error(\"discovery membership history is malformed\")]\n    MalformedHistory,\n    #[error(\"discovery freshness signer collection is duplicate or non-canonical\")]\n    NonCanonicalSignerSet,\n",
)
insert_consensus = r'''

/// Validate the genesis-to-current voter-set proof used by first-contact
/// discovery. Every voter-set mutation must be represented by an Agent 1 joint
/// membership certificate. Gaps that keep the exact same voter set are allowed
/// because authority/epoch refreshes are certified by the live freshness quorum.
pub fn validate_discovery_membership_proof_shape(
    proof: &DiscoveryMembershipProofV1,
) -> Result<(), MembershipConsensusError> {
    if proof.initial_membership.world_id != proof.world_id || proof.current_membership.world_id != proof.world_id {
        return Err(MembershipConsensusError::MalformedHistory);
    }
    let mut voters = active_voters(&proof.initial_membership.members)?;
    for certificate in &proof.membership_certificates {
        validate_membership_certificate_shape(certificate)?;
        let previous = active_voters(&certificate.proposal.previous.members)?;
        if previous != voters {
            return Err(MembershipConsensusError::MalformedHistory);
        }
        voters = active_voters(&certificate.proposal.proposed.members)?;
    }
    if active_voters(&proof.current_membership.members)? != voters {
        return Err(MembershipConsensusError::MalformedHistory);
    }
    if let Some(proposal) = &proof.pending_membership {
        validate_membership_proposal_shape(proposal)?;
        if proposal.previous != proof.current_membership {
            return Err(MembershipConsensusError::MalformedHistory);
        }
    }
    Ok(())
}

/// Apply the exact Agent 1 majority rule to a cryptographically verified,
/// canonical signer collection. Pending membership uses joint old+new quorum.
pub fn validate_discovery_freshness_quorum(
    proof: &DiscoveryMembershipProofV1,
    votes: &[DiscoveryFreshnessVoteV1],
) -> Result<(), MembershipConsensusError> {
    validate_discovery_membership_proof_shape(proof)?;
    let mut last = None;
    let mut signers = BTreeMap::new();
    for vote in votes {
        if last.is_some_and(|peer| vote.voter_peer_id <= peer) {
            return Err(MembershipConsensusError::NonCanonicalSignerSet);
        }
        last = Some(vote.voter_peer_id);
        signers.insert(vote.voter_peer_id, vote.voter_public_key);
    }

    if let Some(proposal) = &proof.pending_membership {
        let old = active_voters(&proposal.previous.members)?;
        let new = active_voters(&proposal.proposed.members)?;
        let mut old_votes = 0usize;
        let mut new_votes = 0usize;
        for (peer, key) in &signers {
            let old_key = old.get(peer);
            let new_key = new.get(peer);
            if old_key.is_none() && new_key.is_none() {
                return Err(MembershipConsensusError::UnknownVoter);
            }
            if old_key.is_some_and(|expected| expected != key) || new_key.is_some_and(|expected| expected != key) {
                return Err(MembershipConsensusError::VoterKeyMismatch);
            }
            old_votes += usize::from(old_key.is_some());
            new_votes += usize::from(new_key.is_some());
        }
        let old_required = quorum_size(old.len());
        if old_votes < old_required {
            return Err(MembershipConsensusError::OldQuorumUnavailable { votes: old_votes, required: old_required });
        }
        let new_required = quorum_size(new.len());
        if new_votes < new_required {
            return Err(MembershipConsensusError::NewQuorumUnavailable { votes: new_votes, required: new_required });
        }
        return Ok(());
    }

    let current = active_voters(&proof.current_membership.members)?;
    let mut count = 0usize;
    for (peer, key) in &signers {
        let expected = current.get(peer).ok_or(MembershipConsensusError::UnknownVoter)?;
        if expected != key {
            return Err(MembershipConsensusError::VoterKeyMismatch);
        }
        count += 1;
    }
    let required = quorum_size(current.len());
    if count < required {
        return Err(MembershipConsensusError::CurrentQuorumUnavailable { votes: count, required });
    }
    Ok(())
}
'''
append_before("crates/swarm-consensus/src/membership.rs", "pub fn membership_vote_for(", insert_consensus)

# ---------------------------------------------------------------------------
# Storage: retain immutable membership-changing certificate history for genesis
# anchoring, while preserving the existing latest-certificate compatibility path.
# ---------------------------------------------------------------------------
replace(
    "crates/swarm-storage/src/membership.rs",
    "    transaction::{durable_atomic_write, durable_remove},\n",
    "    transaction::{durable_atomic_write, durable_create_once, durable_remove},\n",
)
replace(
    "crates/swarm-storage/src/membership.rs",
    "    pub fn save_membership_certificate(&self, certificate: &MembershipCertificateV1) -> Result<(), StorageError> {\n        let world = certificate.proposal.proposed.world_id;\n        let _guard = self.lock_world_transaction(world)?;\n        durable_atomic_write(\n            &self.world_dir(world).join(\"metadata/membership-certificate.postcard\"),\n            &postcard::to_allocvec(certificate)?,\n        )\n    }\n\n    pub fn load_membership_certificate(&self, world: WorldId) -> Result<MembershipCertificateV1, StorageError> {",
    "    pub fn save_membership_certificate(&self, certificate: &MembershipCertificateV1) -> Result<(), StorageError> {\n        let world = certificate.proposal.proposed.world_id;\n        let _guard = self.lock_world_transaction(world)?;\n        let encoded = postcard::to_allocvec(certificate)?;\n        let history_path = self\n            .world_dir(world)\n            .join(\"metadata/membership-certificates\")\n            .join(format!(\"{:020}.postcard\", certificate.proposal.proposed.sequence));\n        if !durable_create_once(&history_path, &encoded)? {\n            let existing = fs::read(&history_path).map_err(|error| io_error(&history_path, error))?;\n            if existing != encoded {\n                return Err(StorageError::WorldMetadataMismatch);\n            }\n        }\n        durable_atomic_write(&self.world_dir(world).join(\"metadata/membership-certificate.postcard\"), &encoded)\n    }\n\n    pub fn load_membership_certificate_chain(\n        &self,\n        world: WorldId,\n    ) -> Result<Vec<MembershipCertificateV1>, StorageError> {\n        let directory = self.world_dir(world).join(\"metadata/membership-certificates\");\n        let mut paths = match fs::read_dir(&directory) {\n            Ok(entries) => entries\n                .map(|entry| entry.map(|value| value.path()).map_err(|error| io_error(&directory, error)))\n                .collect::<Result<Vec<_>, _>>()?,\n            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),\n            Err(error) => return Err(io_error(&directory, error)),\n        };\n        paths.retain(|path| path.extension().is_some_and(|value| value == \"postcard\"));\n        paths.sort();\n        let mut certificates = Vec::with_capacity(paths.len());\n        for path in paths {\n            let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;\n            let certificate: MembershipCertificateV1 = postcard::from_bytes(&bytes)?;\n            if certificate.proposal.proposed.world_id != world {\n                return Err(StorageError::WorldMetadataMismatch);\n            }\n            certificates.push(certificate);\n        }\n        if certificates.is_empty() {\n            if let Ok(latest) = self.load_membership_certificate(world) {\n                certificates.push(latest);\n            }\n        }\n        Ok(certificates)\n    }\n\n    pub fn load_membership_certificate(&self, world: WorldId) -> Result<MembershipCertificateV1, StorageError> {",
)

# ---------------------------------------------------------------------------
# Wire protocol: append-only freshness request/response variants and bounds.
# ---------------------------------------------------------------------------
replace(
    "crates/swarm-network/src/wire.rs",
    "    AuthorityLeaseGrantV1, AuthorityTransferV1, BlobEncoding, DiscoveryFilterV1, EpochRecordV1, FriendPresenceV1,\n",
    "    AuthorityLeaseGrantV1, AuthorityTransferV1, BlobEncoding, DiscoveryFilterV1, DiscoveryFreshnessChallengeV1,\n    DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1, EpochRecordV1, FriendPresenceV1,\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "pub const MAX_DISCOVERY_ANNOUNCEMENT_BYTES: usize = 16 * 1024;\n",
    "pub const MAX_DISCOVERY_ANNOUNCEMENT_BYTES: usize = 16 * 1024;\npub const MAX_DISCOVERY_MEMBERSHIP_PROOF_BYTES: usize = 512 * 1024;\npub const MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES: usize = 256;\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "    HelloProof(Box<PeerHelloProofV1>),\n}\n",
    "    HelloProof(Box<PeerHelloProofV1>),\n    // FINAL-028 challenge-bound authority freshness extensions are append-only.\n    DiscoveryFreshnessContext {\n        world_id: WorldId,\n        announcement_hash: Hash32,\n        verifier_peer_id: PeerId,\n        nonce: [u8; 32],\n        issued_unix_ms: u64,\n        expires_unix_ms: u64,\n    },\n    DiscoveryFreshnessVote(Box<DiscoveryFreshnessChallengeV1>),\n}\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "            | Self::HelloChallenge { .. }\n            | Self::HelloProof(_) => None,\n",
    "            | Self::HelloChallenge { .. }\n            | Self::HelloProof(_)\n            | Self::DiscoveryFreshnessContext { .. }\n            | Self::DiscoveryFreshnessVote(_) => None,\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "    HelloChallengeAccepted,\n}\n",
    "    HelloChallengeAccepted,\n    DiscoveryFreshnessContext(Option<Box<DiscoveryMembershipProofV1>>),\n    DiscoveryFreshnessVote(Option<Box<DiscoveryFreshnessVoteV1>>),\n}\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "            Self::DiscoveryResolved(Some(value)) => validate_announcement_size(value),\n            _ => Ok(()),\n",
    "            Self::DiscoveryResolved(Some(value)) => validate_announcement_size(value),\n            Self::DiscoveryFreshnessContext(Some(proof)) => {\n                if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES {\n                    return Err(WireLimitError::TooManyDiscoveryMembershipCertificates(\n                        proof.membership_certificates.len(),\n                    ));\n                }\n                let bytes = serde_json::to_vec(proof.as_ref())\n                    .map_err(|_| WireLimitError::DiscoveryMembershipProofTooLarge(usize::MAX))?;\n                if bytes.len() > MAX_DISCOVERY_MEMBERSHIP_PROOF_BYTES {\n                    return Err(WireLimitError::DiscoveryMembershipProofTooLarge(bytes.len()));\n                }\n                Ok(())\n            }\n            _ => Ok(()),\n",
)
replace(
    "crates/swarm-network/src/wire.rs",
    "    #[error(\"world discovery announcement is {0} encoded bytes; maximum is {MAX_DISCOVERY_ANNOUNCEMENT_BYTES}\")]\n    DiscoveryAnnouncementTooLarge(usize),\n",
    "    #[error(\"world discovery announcement is {0} encoded bytes; maximum is {MAX_DISCOVERY_ANNOUNCEMENT_BYTES}\")]\n    DiscoveryAnnouncementTooLarge(usize),\n    #[error(\"discovery membership proof contains {0} certificates; maximum is {MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES}\")]\n    TooManyDiscoveryMembershipCertificates(usize),\n    #[error(\"discovery membership proof is {0} encoded bytes; maximum is {MAX_DISCOVERY_MEMBERSHIP_PROOF_BYTES}\")]\n    DiscoveryMembershipProofTooLarge(usize),\n",
)

# ---------------------------------------------------------------------------
# CLI discovery service and verifier bridge.
# ---------------------------------------------------------------------------
replace(
    "crates/swarm-cli/src/discovery.rs",
    "use swarm_core::{\n    random_nonce, sign_friend_presence, sign_world_announcement, verify_friend_presence, verify_membership_signature,\n    verify_world_announcement, verify_world_config_signature, AnnouncementReplayGuard, DataPaths, DiscoveryRecordError,\n    PeerIdentity,\n};\n",
    "use swarm_core::{\n    random_nonce, sign_discovery_freshness_vote, sign_friend_presence, sign_world_announcement,\n    verify_discovery_freshness_challenge, verify_discovery_freshness_vote, verify_discovery_membership_proof,\n    verify_friend_presence, verify_membership_signature, verify_world_announcement, verify_world_config_signature,\n    AnnouncementReplayGuard, DataPaths, DiscoveryFreshnessReplayGuard, DiscoveryRecordError, PeerIdentity,\n    DISCOVERY_FRESHNESS_MAX_LIFETIME_MS,\n};\nuse swarm_consensus::{validate_discovery_freshness_quorum, validate_discovery_membership_proof_shape};\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "    peer_id_from_public_key, DiscoveryCompatibilityV1, DiscoveryFilterV1, FriendPresenceV1, MembershipPolicyV1, PeerId,\n    WorldAnnouncementV1, WorldId, WorldVisibilityV1, PROTOCOL_VERSION,\n",
    "    peer_id_from_public_key, DiscoveryCanonicalHeadV1, DiscoveryCompatibilityV1, DiscoveryFilterV1,\n    DiscoveryFreshnessChallengeV1, DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1, FriendPresenceV1,\n    MembershipPolicyV1, PeerId, WorldAnnouncementV1, WorldId, WorldVisibilityV1, PROTOCOL_VERSION,\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "const DISCOVERY_QUERY_TIMEOUT: Duration = Duration::from_secs(8);\n",
    "const DISCOVERY_QUERY_TIMEOUT: Duration = Duration::from_secs(8);\nconst DISCOVERY_FRESHNESS_TIMEOUT: Duration = Duration::from_secs(5);\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "    presence_requesters: HashSet<PeerId>,\n}\n",
    "    presence_requesters: HashSet<PeerId>,\n    signed_freshness_challenges: HashMap<(PeerId, [u8; 32]), u64>,\n}\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "                            &identity,\n                            &mut node,\n                            &published,\n",
    "                            &storage,\n                            &identity,\n                            &mut node,\n                            &mut published,\n",
)
# Make every active public/unlisted member a world-key provider, but only current authority publishes announcement/public directory.
replace(
    "crates/swarm-cli/src/discovery.rs",
    "        let Ok(epoch) = storage.load_epoch_record(world) else { continue };\n        if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {\n            continue;\n        }\n\n        match config.visibility {\n            WorldVisibilityV1::Private => continue,\n            WorldVisibilityV1::Unlisted | WorldVisibilityV1::Public => {}\n        }\n\n        let previous = state.sequences.get(&world).copied().unwrap_or(0);\n        let sequence = now.max(previous.saturating_add(1));\n",
    "        match config.visibility {\n            WorldVisibilityV1::Private => continue,\n            WorldVisibilityV1::Unlisted | WorldVisibilityV1::Public => {}\n        }\n        // The DHT provider identity is only a locator. Publishing all current\n        // active members under the exact-world key makes a live quorum\n        // discoverable without granting any provider authority.\n        next_worlds.insert(world);\n\n        let Ok(epoch) = storage.load_epoch_record(world) else { continue };\n        if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {\n            continue;\n        }\n\n        let previous = state.sequences.get(&world).copied().unwrap_or(0);\n        let sequence = if now > previous {\n            now\n        } else {\n            previous.checked_add(1).context(\"discovery announcement sequence exhausted\")?\n        };\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "            config_sequence: config.sequence,\n            config_hash: config.config_hash()?,\n            authority_epoch: epoch.epoch_number,\n            fencing_token: epoch.fencing_token,\n            announcement_sequence: sequence,\n            issued_unix_ms: now,\n            expires_unix_ms: now.saturating_add(WORLD_ANNOUNCEMENT_TTL_MS),\n",
    "            config_sequence: config.sequence,\n            config_hash: config.config_hash()?,\n            membership_sequence: membership.sequence,\n            membership_hash: membership.record_hash()?,\n            authority_epoch: epoch.epoch_number,\n            fencing_token: epoch.fencing_token,\n            canonical_head: storage.canonical_snapshot_head(world)?.head.map(|head| DiscoveryCanonicalHeadV1 {\n                snapshot_number: head.snapshot_number,\n                manifest_hash: head.manifest_hash,\n                epoch: head.epoch,\n                sequence: head.sequence,\n            }),\n            announcement_sequence: sequence,\n            issued_unix_ms: now,\n            expires_unix_ms: now.checked_add(WORLD_ANNOUNCEMENT_TTL_MS).context(\"discovery expiry overflow\")?,\n",
)
# next_worlds already inserted for all members; duplicate insertion after signing is harmless but remove it.
replace(
    "crates/swarm-cli/src/discovery.rs",
    "        next_worlds.insert(world);\n        next_announcements.insert(world, announcement);\n",
    "        next_announcements.insert(world, announcement);\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "fn handle_discovery_request(\n    paths: &DataPaths,\n    identity: &PeerIdentity,\n    node: &mut DiscoveryNode,\n    state: &PublishedDiscoveryState,\n",
    "fn handle_discovery_request(\n    paths: &DataPaths,\n    storage: &Storage,\n    identity: &PeerIdentity,\n    node: &mut DiscoveryNode,\n    state: &mut PublishedDiscoveryState,\n",
)
# Add freshness request handlers before friend presence.
replace(
    "crates/swarm-cli/src/discovery.rs",
    "        WireRequest::FriendPresence { expected_peer_id, requester_peer_id, nonce } => {\n",
    r'''        WireRequest::DiscoveryFreshnessContext {
            world_id,
            announcement_hash,
            verifier_peer_id,
            nonce: _,
            issued_unix_ms,
            expires_unix_ms,
        } => {
            let now = unix_millis()?;
            if application_peer != verifier_peer_id
                || expires_unix_ms <= issued_unix_ms
                || expires_unix_ms.saturating_sub(issued_unix_ms) > DISCOVERY_FRESHNESS_MAX_LIFETIME_MS
                || expires_unix_ms < now
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessContext(None))?;
                return Ok(());
            }
            let Some(announcement) = state.announcements.get(&world_id) else {
                node.respond(channel, WireResponse::DiscoveryFreshnessContext(None))?;
                return Ok(());
            };
            if announcement.announcement_hash()? != announcement_hash
                || announcement.announcer_peer_id != identity.peer_id()
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessContext(None))?;
                return Ok(());
            }
            let proof = build_discovery_membership_proof(storage, world_id)?;
            node.respond(channel, WireResponse::DiscoveryFreshnessContext(Some(Box::new(proof))))?;
        }
        WireRequest::DiscoveryFreshnessVote(challenge) => {
            let challenge = *challenge;
            let now = unix_millis()?;
            if application_peer != challenge.verifier_peer_id
                || challenge.protocol_version != PROTOCOL_VERSION
                || challenge.expires_unix_ms <= challenge.issued_unix_ms
                || challenge.expires_unix_ms.saturating_sub(challenge.issued_unix_ms)
                    > DISCOVERY_FRESHNESS_MAX_LIFETIME_MS
                || challenge.expires_unix_ms < now
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessVote(None))?;
                return Ok(());
            }
            state.signed_freshness_challenges.retain(|_, expires| *expires >= now);
            let replay_key = (challenge.verifier_peer_id, challenge.nonce);
            if state.signed_freshness_challenges.contains_key(&replay_key)
                || !local_state_matches_freshness_challenge(storage, identity, &challenge)?
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessVote(None))?;
                return Ok(());
            }
            state.signed_freshness_challenges.insert(replay_key, challenge.expires_unix_ms);
            let vote = sign_discovery_freshness_vote(identity, &challenge)?;
            node.respond(channel, WireResponse::DiscoveryFreshnessVote(Some(Box::new(vote))))?;
        }
        WireRequest::FriendPresence { expected_peer_id, requester_peer_id, nonce } => {
''',
)

helpers = r'''
fn build_discovery_membership_proof(storage: &Storage, world: WorldId) -> Result<DiscoveryMembershipProofV1> {
    let metadata = storage.load_world(world)?;
    let current = storage.load_membership_record(world)?;
    verify_membership_signature(&current)?;
    let certificates = storage.load_membership_certificate_chain(world)?;
    let initial = certificates
        .first()
        .map(|certificate| certificate.proposal.previous.clone())
        .unwrap_or_else(|| current.clone());
    let pending = storage
        .load_membership_promise(world)
        .ok()
        .map(|promise| promise.proposal);
    let proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis: metadata.genesis,
        initial_membership: initial,
        membership_certificates: certificates,
        current_membership: current,
        pending_membership: pending,
    };
    validate_discovery_membership_proof_shape(&proof).map_err(|error| anyhow!(error))?;
    Ok(proof)
}

fn local_state_matches_freshness_challenge(
    storage: &Storage,
    identity: &PeerIdentity,
    challenge: &DiscoveryFreshnessChallengeV1,
) -> Result<bool> {
    let world = challenge.world_id;
    let membership = match storage.load_membership_record(world) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    verify_membership_signature(&membership)?;
    if membership.sequence != challenge.membership_sequence
        || membership.record_hash()? != challenge.membership_hash
    {
        return Ok(false);
    }
    let epoch = match storage.load_epoch_record(world) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    if epoch.authority_peer_id != challenge.authority_peer_id
        || epoch.epoch_number != challenge.authority_epoch
        || epoch.fencing_token != challenge.fencing_token
    {
        return Ok(false);
    }
    let config = match storage.load_world_config(world) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    verify_world_config_signature(&config)?;
    if config.authority_peer_id != challenge.authority_peer_id
        || config.sequence != challenge.config_sequence
        || config.config_hash()? != challenge.config_hash
    {
        return Ok(false);
    }
    let head = storage.canonical_snapshot_head(world)?.head.map(|head| DiscoveryCanonicalHeadV1 {
        snapshot_number: head.snapshot_number,
        manifest_hash: head.manifest_hash,
        epoch: head.epoch,
        sequence: head.sequence,
    });
    if head != challenge.canonical_head {
        return Ok(false);
    }
    let pending = storage.load_membership_promise(world).ok().map(|promise| promise.proposal);
    let pending_hash = pending
        .as_ref()
        .map(|proposal| proposal.proposal_hash())
        .transpose()?;
    if pending_hash != challenge.pending_membership_proposal_hash {
        return Ok(false);
    }
    let eligible = membership.members.iter().any(|member| {
        member.peer_id == identity.peer_id()
            && member.public_key == identity.public_key()
            && !member.banned
    }) || pending.as_ref().is_some_and(|proposal| {
        proposal.proposed.members.iter().any(|member| {
            member.peer_id == identity.peer_id()
                && member.public_key == identity.public_key()
                && !member.banned
        })
    });
    Ok(eligible)
}

pub fn validate_fresh_discovery_candidate(
    announcement: &WorldAnnouncementV1,
    proof: &DiscoveryMembershipProofV1,
    challenge: &DiscoveryFreshnessChallengeV1,
    votes: &[DiscoveryFreshnessVoteV1],
    verifier_peer_id: PeerId,
    nonce: [u8; 32],
    now_unix_ms: u64,
    replay: &mut DiscoveryFreshnessReplayGuard,
) -> Result<()> {
    verify_world_announcement(announcement, now_unix_ms).map_err(|error| anyhow!(error))?;
    verify_discovery_membership_proof(announcement, proof).map_err(|error| anyhow!(error))?;
    validate_discovery_membership_proof_shape(proof).map_err(|error| anyhow!(error))?;
    verify_discovery_freshness_challenge(
        announcement,
        proof,
        challenge,
        verifier_peer_id,
        nonce,
        now_unix_ms,
    )
    .map_err(|error| anyhow!(error))?;
    for vote in votes {
        verify_discovery_freshness_vote(vote, challenge).map_err(|error| anyhow!(error))?;
    }
    validate_discovery_freshness_quorum(proof, votes).map_err(|error| anyhow!(error))?;
    replay.accept(challenge).map_err(|error| anyhow!(error))?;
    Ok(())
}

async fn prove_candidate_freshness(
    node: &mut DiscoveryNode,
    verifier: &PeerIdentity,
    announcement: &WorldAnnouncementV1,
) -> Result<bool> {
    verify_world_announcement(announcement, unix_millis()?).map_err(|error| anyhow!(error))?;
    let query = node.find_world_providers(announcement.world_id);
    let nonce = random_nonce();
    let issued_unix_ms = unix_millis()?;
    let expires_unix_ms = issued_unix_ms
        .checked_add(DISCOVERY_FRESHNESS_MAX_LIFETIME_MS)
        .context("freshness challenge expiry overflow")?;
    let announcement_hash = announcement.announcement_hash()?;
    let mut providers = HashSet::new();
    let mut applications = HashMap::new();
    let mut context_requested = HashSet::new();
    let mut vote_requested = HashSet::new();
    let mut proof: Option<DiscoveryMembershipProofV1> = None;
    let mut challenge: Option<DiscoveryFreshnessChallengeV1> = None;
    let mut votes = Vec::<DiscoveryFreshnessVoteV1>::new();
    let mut replay = DiscoveryFreshnessReplayGuard::default();

    let run = timeout(DISCOVERY_FRESHNESS_TIMEOUT, async {
        loop {
            match node.next_event().await? {
                DiscoveryNetworkEvent::ProvidersFound { query_id, providers: found } if query_id == query => {
                    for peer in found {
                        if providers.insert(peer) {
                            let _ = node.dial_peer(peer);
                        }
                    }
                }
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query && providers.is_empty() => {
                    break;
                }
                DiscoveryNetworkEvent::ProvidersFailed { query_id, .. } if query_id == query && providers.is_empty() => {
                    break;
                }
                DiscoveryNetworkEvent::Authenticated { transport_peer, application_peer }
                    if providers.contains(&transport_peer) =>
                {
                    applications.insert(transport_peer, application_peer);
                    if application_peer == announcement.announcer_peer_id && context_requested.insert(transport_peer) {
                        node.send_request(
                            &transport_peer,
                            WireRequest::DiscoveryFreshnessContext {
                                world_id: announcement.world_id,
                                announcement_hash,
                                verifier_peer_id: verifier.peer_id(),
                                nonce,
                                issued_unix_ms,
                                expires_unix_ms,
                            },
                        )?;
                    }
                    if let Some(active) = &challenge {
                        if vote_requested.insert(transport_peer) {
                            node.send_request(&transport_peer, WireRequest::DiscoveryFreshnessVote(Box::new(active.clone())))?;
                        }
                    }
                }
                DiscoveryNetworkEvent::Response { transport_peer, response: WireResponse::DiscoveryFreshnessContext(Some(value)), .. } => {
                    if applications.get(&transport_peer) != Some(&announcement.announcer_peer_id) {
                        continue;
                    }
                    let candidate = *value;
                    verify_discovery_membership_proof(announcement, &candidate).map_err(|error| anyhow!(error))?;
                    validate_discovery_membership_proof_shape(&candidate).map_err(|error| anyhow!(error))?;
                    let pending_membership_proposal_hash = candidate
                        .pending_membership
                        .as_ref()
                        .map(|proposal| proposal.proposal_hash())
                        .transpose()?;
                    let active = DiscoveryFreshnessChallengeV1 {
                        protocol_version: PROTOCOL_VERSION,
                        verifier_peer_id: verifier.peer_id(),
                        nonce,
                        world_id: announcement.world_id,
                        announcement_hash,
                        membership_sequence: announcement.membership_sequence,
                        membership_hash: announcement.membership_hash,
                        pending_membership_proposal_hash,
                        authority_peer_id: announcement.announcer_peer_id,
                        authority_epoch: announcement.authority_epoch,
                        fencing_token: announcement.fencing_token,
                        config_sequence: announcement.config_sequence,
                        config_hash: announcement.config_hash,
                        canonical_head: announcement.canonical_head,
                        issued_unix_ms,
                        expires_unix_ms,
                    };
                    verify_discovery_freshness_challenge(
                        announcement,
                        &candidate,
                        &active,
                        verifier.peer_id(),
                        nonce,
                        unix_millis()?,
                    )
                    .map_err(|error| anyhow!(error))?;
                    proof = Some(candidate);
                    challenge = Some(active.clone());
                    for transport_peer in providers.iter().copied().collect::<Vec<_>>() {
                        if applications.contains_key(&transport_peer) && vote_requested.insert(transport_peer) {
                            let _ = node.send_request(
                                &transport_peer,
                                WireRequest::DiscoveryFreshnessVote(Box::new(active.clone())),
                            );
                        }
                    }
                }
                DiscoveryNetworkEvent::Response { response: WireResponse::DiscoveryFreshnessVote(Some(value)), .. } => {
                    let Some(active) = &challenge else { continue };
                    let vote = *value;
                    if verify_discovery_freshness_vote(&vote, active).is_err() {
                        continue;
                    }
                    if votes.iter().all(|existing| existing.voter_peer_id != vote.voter_peer_id) {
                        votes.push(vote);
                        votes.sort_by_key(|value| value.voter_peer_id);
                    }
                    if let Some(candidate) = &proof {
                        if validate_discovery_freshness_quorum(candidate, &votes).is_ok() {
                            validate_fresh_discovery_candidate(
                                announcement,
                                candidate,
                                active,
                                &votes,
                                verifier.peer_id(),
                                nonce,
                                unix_millis()?,
                                &mut replay,
                            )?;
                            return Ok::<bool, anyhow::Error>(true);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok::<bool, anyhow::Error>(false)
    })
    .await;
    match run {
        Ok(value) => value,
        Err(_) => Ok(false),
    }
}
'''
append_before("crates/swarm-cli/src/discovery.rs", "pub fn add_friend(", helpers)

# Browse: collect base-authentic candidates, then require live freshness before insertion.
replace(
    "crates/swarm-cli/src/discovery.rs",
    "    let mut results = HashMap::<WorldId, WorldAnnouncementV1>::new();\n    let mut replay = AnnouncementReplayGuard::default();\n",
    "    let mut candidates = Vec::<WorldAnnouncementV1>::new();\n    let mut results = HashMap::<WorldId, WorldAnnouncementV1>::new();\n    let mut replay = AnnouncementReplayGuard::default();\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "                            match verify_world_announcement(&value, unix_millis()?) {\n                                Ok(()) => match replay.accept(&value) {\n                                    Ok(()) => {\n                                        results.insert(value.world_id, value);\n                                    }\n                                    Err(DiscoveryRecordError::Replay) => {\n                                        // Multiple providers may legitimately return the same signed record.\n                                        if !results.contains_key(&value.world_id) {\n                                            rejected_stale += 1;\n                                        }\n                                    }\n                                    Err(_) => rejected_invalid += 1,\n                                },\n                                Err(DiscoveryRecordError::Expired) => rejected_stale += 1,\n                                Err(_) => rejected_invalid += 1,\n                            }\n",
    "                            match verify_world_announcement(&value, unix_millis()?) {\n                                Ok(()) => {\n                                    let hash = value.announcement_hash()?;\n                                    if candidates.iter().all(|existing| existing.announcement_hash().ok() != Some(hash)) {\n                                        candidates.push(value);\n                                    }\n                                }\n                                Err(DiscoveryRecordError::Expired) => rejected_stale += 1,\n                                Err(_) => rejected_invalid += 1,\n                            }\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "    let mut values: Vec<_> = results.into_values().collect();\n",
    "    for candidate in candidates {\n        match prove_candidate_freshness(&mut node, &identity, &candidate).await {\n            Ok(true) => match replay.accept(&candidate) {\n                Ok(()) => { results.insert(candidate.world_id, candidate); }\n                Err(DiscoveryRecordError::Replay) => rejected_stale += 1,\n                Err(_) => rejected_invalid += 1,\n            },\n            Ok(false) => rejected_stale += 1,\n            Err(_) => rejected_invalid += 1,\n        }\n    }\n\n    let mut values: Vec<_> = results.into_values().collect();\n",
)
# Exact resolve: collect all base-valid candidates and freshness-check each; no first-valid-wins.
replace(
    "crates/swarm-cli/src/discovery.rs",
    "    let mut result = None;\n    let mut invalid = false;\n",
    "    let mut candidates = Vec::<WorldAnnouncementV1>::new();\n    let mut result = None;\n    let mut invalid = false;\n",
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    "                    match verify_world_announcement(&value, unix_millis()?) {\n                        Ok(()) => {\n                            result = Some(*value);\n                            break;\n                        }\n                        Err(DiscoveryRecordError::Expired) => stale = true,\n                        Err(_) => invalid = true,\n                    }\n",
    "                    match verify_world_announcement(&value, unix_millis()?) {\n                        Ok(()) => {\n                            let value = *value;\n                            let hash = value.announcement_hash()?;\n                            if candidates.iter().all(|existing| existing.announcement_hash().ok() != Some(hash)) {\n                                candidates.push(value);\n                            }\n                        }\n                        Err(DiscoveryRecordError::Expired) => stale = true,\n                        Err(_) => invalid = true,\n                    }\n",
)
# Insert freshness loop after initial exact query timeout.
replace(
    "crates/swarm-cli/src/discovery.rs",
    "    if let Ok(value) = run {\n        value?;\n    }\n\n    let state = if result.is_some() {\n",
    "    if let Ok(value) = run {\n        value?;\n    }\n    for candidate in candidates {\n        match prove_candidate_freshness(&mut node, &identity, &candidate).await {\n            Ok(true) => {\n                result = Some(candidate);\n                break;\n            }\n            Ok(false) => stale = true,\n            Err(_) => invalid = true,\n        }\n    }\n\n    let state = if result.is_some() {\n",
)

# ---------------------------------------------------------------------------
# Permanent FINAL-028 regression suite. It exercises the exact shared acceptance
# gate used by both browse and resolve, including real Ed25519 signatures and
# Agent 1 old/new quorum math rather than a mocked authority callback.
# ---------------------------------------------------------------------------
test_path = Path("crates/swarm-cli/tests/discovery_freshness.rs")
if not test_path.exists():
    test_path.write_text(r'''use swarm_cli::discovery::validate_fresh_discovery_candidate;
use swarm_consensus::{membership_vote_for, validate_discovery_freshness_quorum};
use swarm_core::{
    sign_discovery_freshness_vote, sign_world_announcement, DiscoveryFreshnessReplayGuard, PeerIdentity,
};
use swarm_protocol::{
    DiscoveryCanonicalHeadV1, DiscoveryCompatibilityV1, DiscoveryFreshnessChallengeV1, DiscoveryMembershipProofV1,
    Hash32, MembershipCertificateV1, MembershipPolicyV1, MembershipProposalV1, MembershipRecordV1, PeerId,
    WorldAnnouncementV1, WorldGenesisV1, WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
};

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn signed_membership(
    authority: &PeerIdentity,
    world: swarm_protocol::WorldId,
    epoch: u64,
    sequence: u64,
    previous: Option<Hash32>,
    mut members: Vec<WorldMemberV1>,
) -> MembershipRecordV1 {
    members.sort_by_key(|value| value.peer_id);
    let mut record = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch,
        sequence,
        previous_membership_hash: previous,
        members,
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    authority.sign_membership(&mut record).unwrap();
    record
}

fn fixture(count: usize) -> (Vec<PeerIdentity>, WorldAnnouncementV1, DiscoveryMembershipProofV1, DiscoveryFreshnessChallengeV1) {
    let identities = (1..=count).map(|id| PeerIdentity::from_secret_bytes([id as u8; 32])).collect::<Vec<_>>();
    let mut initial_members = identities.iter().map(member).collect::<Vec<_>>();
    initial_members.sort_by_key(|value| value.peer_id);
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "1.21.8".into(),
        fabric_loader_version: "0.17.2".into(),
        compatibility_fingerprint: Hash32([9; 32]),
        creation_nonce: [7; 32],
        creator_public_key: identities[0].public_key(),
        initial_membership: initial_members.iter().map(|value| value.peer_id).collect(),
    };
    let world = genesis.world_id().unwrap();
    let current = signed_membership(&identities[0], world, 3, 0, None, initial_members);
    let mut announcement = WorldAnnouncementV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        presentation: WorldPresentationV1 {
            name: "Fresh world".into(),
            description: String::new(),
            tags: vec!["survival".into()],
            icon_hash: None,
            approximate_region: None,
        },
        compatibility: DiscoveryCompatibilityV1 {
            minecraft_version: "1.21.8".into(),
            loader_id: "fabric".into(),
            loader_version: "0.17.2".into(),
            fabric_adapter_version: "0.5.0".into(),
            compatibility_fingerprint: Hash32([9; 32]),
        },
        visibility: WorldVisibilityV1::Public,
        membership_policy: MembershipPolicyV1::InviteOnly,
        config_sequence: 4,
        config_hash: Hash32([10; 32]),
        membership_sequence: current.sequence,
        membership_hash: current.record_hash().unwrap(),
        authority_epoch: 3,
        fencing_token: 8,
        canonical_head: Some(DiscoveryCanonicalHeadV1 {
            snapshot_number: 12,
            manifest_hash: Hash32([11; 32]),
            epoch: 3,
            sequence: 22,
        }),
        announcement_sequence: 1,
        issued_unix_ms: 1_000,
        expires_unix_ms: 50_000,
        announcer_peer_id: identities[0].peer_id(),
        announcer_public_key: identities[0].public_key(),
        signature: Vec::new(),
    };
    sign_world_announcement(&identities[0], &mut announcement).unwrap();
    let proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis,
        initial_membership: current.clone(),
        membership_certificates: Vec::new(),
        current_membership: current,
        pending_membership: None,
    };
    let verifier = PeerIdentity::from_secret_bytes([99; 32]);
    let challenge = DiscoveryFreshnessChallengeV1 {
        protocol_version: PROTOCOL_VERSION,
        verifier_peer_id: verifier.peer_id(),
        nonce: [55; 32],
        world_id: world,
        announcement_hash: announcement.announcement_hash().unwrap(),
        membership_sequence: announcement.membership_sequence,
        membership_hash: announcement.membership_hash,
        pending_membership_proposal_hash: None,
        authority_peer_id: announcement.announcer_peer_id,
        authority_epoch: announcement.authority_epoch,
        fencing_token: announcement.fencing_token,
        config_sequence: announcement.config_sequence,
        config_hash: announcement.config_hash,
        canonical_head: announcement.canonical_head,
        issued_unix_ms: 2_000,
        expires_unix_ms: 10_000,
    };
    (identities, announcement, proof, challenge)
}

fn votes(ids: &[PeerIdentity], challenge: &DiscoveryFreshnessChallengeV1) -> Vec<swarm_protocol::DiscoveryFreshnessVoteV1> {
    let mut result = ids.iter().map(|id| sign_discovery_freshness_vote(id, challenge).unwrap()).collect::<Vec<_>>();
    result.sort_by_key(|vote| vote.voter_peer_id);
    result
}

#[test]
fn current_quorum_accepts_and_replay_wrong_head_epoch_membership_world_and_verifier_fail() {
    let (ids, announcement, proof, challenge) = fixture(3);
    let verifier = challenge.verifier_peer_id;
    let valid_votes = votes(&ids[..2], &challenge);
    let mut replay = DiscoveryFreshnessReplayGuard::default();
    validate_fresh_discovery_candidate(
        &announcement, &proof, &challenge, &valid_votes, verifier, challenge.nonce, 3_000, &mut replay,
    )
    .unwrap();
    assert!(validate_fresh_discovery_candidate(
        &announcement, &proof, &challenge, &valid_votes, verifier, challenge.nonce, 3_000, &mut replay,
    )
    .is_err());

    for mutate in 0..7 {
        let mut bad = challenge.clone();
        match mutate {
            0 => bad.world_id = swarm_protocol::WorldId([42; 32]),
            1 => bad.membership_hash = Hash32([42; 32]),
            2 => bad.membership_sequence += 1,
            3 => bad.authority_epoch += 1,
            4 => bad.fencing_token += 1,
            5 => bad.canonical_head.as_mut().unwrap().manifest_hash = Hash32([42; 32]),
            _ => bad.verifier_peer_id = PeerId([42; 32]),
        }
        let bad_votes = votes(&ids[..2], &bad);
        let mut guard = DiscoveryFreshnessReplayGuard::default();
        assert!(validate_fresh_discovery_candidate(
            &announcement, &proof, &bad, &bad_votes, verifier, challenge.nonce, 3_000, &mut guard,
        )
        .is_err());
    }
}

#[test]
fn unrelated_self_signed_attacker_and_removed_or_banned_signers_do_not_form_current_quorum() {
    let (ids, mut announcement, mut proof, challenge) = fixture(3);
    let attacker = PeerIdentity::from_secret_bytes([88; 32]);
    announcement.announcer_peer_id = attacker.peer_id();
    announcement.announcer_public_key = attacker.public_key();
    sign_world_announcement(&attacker, &mut announcement).unwrap();
    let mut guard = DiscoveryFreshnessReplayGuard::default();
    assert!(validate_fresh_discovery_candidate(
        &announcement,
        &proof,
        &challenge,
        &votes(&ids[..2], &challenge),
        challenge.verifier_peer_id,
        challenge.nonce,
        3_000,
        &mut guard,
    )
    .is_err());

    proof.current_membership.members[1].banned = true;
    let mut banned_votes = votes(&[ids[0].clone(), ids[1].clone()], &challenge);
    banned_votes.sort_by_key(|vote| vote.voter_peer_id);
    assert!(validate_discovery_freshness_quorum(&proof, &banned_votes).is_err());
}

#[test]
fn joint_transition_requires_both_old_and_new_quorums_and_stale_old_side_cannot_certify() {
    let (mut ids, mut announcement, mut proof, _) = fixture(3);
    ids.push(PeerIdentity::from_secret_bytes([4; 32]));
    ids.push(PeerIdentity::from_secret_bytes([5; 32]));
    let previous = proof.current_membership.clone();
    let mut proposed_members = ids.iter().map(member).collect::<Vec<_>>();
    proposed_members.sort_by_key(|value| value.peer_id);
    let proposed = signed_membership(
        &ids[0],
        announcement.world_id,
        previous.epoch,
        1,
        Some(previous.record_hash().unwrap()),
        proposed_members,
    );
    let proposal = MembershipProposalV1 { previous: previous.clone(), proposed };
    proof.pending_membership = Some(proposal.clone());
    let mut challenge = DiscoveryFreshnessChallengeV1 {
        protocol_version: PROTOCOL_VERSION,
        verifier_peer_id: PeerIdentity::from_secret_bytes([99; 32]).peer_id(),
        nonce: [77; 32],
        world_id: announcement.world_id,
        announcement_hash: announcement.announcement_hash().unwrap(),
        membership_sequence: announcement.membership_sequence,
        membership_hash: announcement.membership_hash,
        pending_membership_proposal_hash: Some(proposal.proposal_hash().unwrap()),
        authority_peer_id: announcement.announcer_peer_id,
        authority_epoch: announcement.authority_epoch,
        fencing_token: announcement.fencing_token,
        config_sequence: announcement.config_sequence,
        config_hash: announcement.config_hash,
        canonical_head: announcement.canonical_head,
        issued_unix_ms: 2_000,
        expires_unix_ms: 10_000,
    };
    let old_only = votes(&ids[..2], &challenge);
    assert!(validate_discovery_freshness_quorum(&proof, &old_only).is_err());
    let joint = votes(&[ids[0].clone(), ids[1].clone(), ids[3].clone()], &challenge);
    validate_discovery_freshness_quorum(&proof, &joint).unwrap();

    challenge.nonce = [78; 32];
    let stale_old_partition = votes(&ids[..1], &challenge);
    assert!(validate_discovery_freshness_quorum(&proof, &stale_old_partition).is_err());
}

#[test]
fn truncated_membership_change_chain_and_noncanonical_vote_collection_fail_closed() {
    let (mut ids, mut announcement, mut proof, mut challenge) = fixture(3);
    ids.push(PeerIdentity::from_secret_bytes([4; 32]));
    ids.push(PeerIdentity::from_secret_bytes([5; 32]));
    let previous = proof.current_membership.clone();
    let mut proposed_members = ids.iter().map(member).collect::<Vec<_>>();
    proposed_members.sort_by_key(|value| value.peer_id);
    let proposed = signed_membership(
        &ids[0],
        announcement.world_id,
        previous.epoch,
        1,
        Some(previous.record_hash().unwrap()),
        proposed_members,
    );
    let proposal = MembershipProposalV1 { previous: previous.clone(), proposed: proposed.clone() };
    let mut membership_votes = [0usize, 1, 3]
        .into_iter()
        .map(|index| {
            let mut vote = membership_vote_for(&proposal, ids[index].peer_id(), ids[index].public_key()).unwrap();
            vote.signature = ids[index].sign(&vote.signing_bytes().unwrap());
            vote
        })
        .collect::<Vec<_>>();
    membership_votes.sort_by_key(|vote| vote.voter_peer_id);
    proof.membership_certificates.push(MembershipCertificateV1 { proposal, votes: membership_votes });
    proof.current_membership = proposed.clone();
    announcement.membership_sequence = proposed.sequence;
    announcement.membership_hash = proposed.record_hash().unwrap();
    sign_world_announcement(&ids[0], &mut announcement).unwrap();
    challenge.announcement_hash = announcement.announcement_hash().unwrap();
    challenge.membership_sequence = announcement.membership_sequence;
    challenge.membership_hash = announcement.membership_hash;
    let valid = votes(&[ids[0].clone(), ids[1].clone(), ids[3].clone()], &challenge);
    let mut guard = DiscoveryFreshnessReplayGuard::default();
    validate_fresh_discovery_candidate(
        &announcement,
        &proof,
        &challenge,
        &valid,
        challenge.verifier_peer_id,
        challenge.nonce,
        3_000,
        &mut guard,
    )
    .unwrap();

    let mut truncated = proof.clone();
    truncated.membership_certificates.clear();
    assert!(swarm_consensus::validate_discovery_membership_proof_shape(&truncated).is_err());

    let mut duplicated = valid.clone();
    duplicated.push(valid[0].clone());
    assert!(validate_discovery_freshness_quorum(&proof, &duplicated).is_err());
    let mut reordered = valid.clone();
    reordered.reverse();
    assert!(validate_discovery_freshness_quorum(&proof, &reordered).is_err());
}
''')

print("Agent 4 FINAL-028 patch applied")
