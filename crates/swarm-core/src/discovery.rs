use std::collections::{BTreeSet, HashMap, HashSet};

use swarm_protocol::{
    peer_id_from_public_key, DiscoveryFreshnessChallengeV1, DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1,
    FriendPresenceV1, PeerId, WorldAnnouncementV1, WorldId, WorldVisibilityV1, PROTOCOL_VERSION,
};

use crate::{verify_membership_signature, verify_signature, CoreError, PeerIdentity};

pub const WORLD_ANNOUNCEMENT_MAX_LIFETIME_MS: u64 = 10 * 60 * 1_000;
pub const FRIEND_PRESENCE_MAX_LIFETIME_MS: u64 = 60 * 1_000;
pub const DISCOVERY_CLOCK_SKEW_MS: u64 = 60 * 1_000;
pub const DISCOVERY_FRESHNESS_MAX_LIFETIME_MS: u64 = 15 * 1_000;
pub const MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES: usize = 256;
pub const MAX_DISCOVERY_MEMBERS: usize = 1_024;
pub const MAX_DISCOVERY_FRESHNESS_VOTES: usize = 1_024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiscoveryRecordError {
    #[error("discovery protocol version is unsupported")]
    ProtocolMismatch,
    #[error("private worlds cannot be advertised")]
    PrivateWorld,
    #[error("discovery record has an invalid lifetime")]
    InvalidLifetime,
    #[error("discovery record has expired")]
    Expired,
    #[error("discovery record is issued too far in the future")]
    FutureIssued,
    #[error("discovery record was replayed or is older than the accepted record")]
    Replay,
    #[error("presence challenge does not match the requester")]
    PresenceRequesterMismatch,
    #[error("presence challenge nonce does not match")]
    PresenceNonceMismatch,
    #[error("signature or cryptographic peer identity is invalid")]
    InvalidSignature,
    #[error("discovery membership proof is malformed or not genesis anchored")]
    InvalidMembershipProof,
    #[error("discovery freshness challenge does not bind the advertised canonical state")]
    FreshnessStateMismatch,
    #[error("discovery freshness challenge has already been accepted")]
    FreshnessReplay,
}

pub fn sign_world_announcement(
    identity: &PeerIdentity,
    announcement: &mut WorldAnnouncementV1,
) -> Result<(), CoreError> {
    announcement.announcer_peer_id = identity.peer_id();
    announcement.announcer_public_key = identity.public_key();
    announcement.signature.clear();
    announcement.signature = identity.sign(&announcement.signing_bytes()?);
    Ok(())
}

pub fn verify_world_announcement(
    announcement: &WorldAnnouncementV1,
    now_unix_ms: u64,
) -> Result<(), DiscoveryRecordError> {
    if announcement.protocol_version != PROTOCOL_VERSION {
        return Err(DiscoveryRecordError::ProtocolMismatch);
    }
    if matches!(announcement.visibility, WorldVisibilityV1::Private) {
        return Err(DiscoveryRecordError::PrivateWorld);
    }
    validate_lifetime(
        announcement.issued_unix_ms,
        announcement.expires_unix_ms,
        WORLD_ANNOUNCEMENT_MAX_LIFETIME_MS,
        now_unix_ms,
    )?;
    verify_signature(
        announcement.announcer_peer_id,
        announcement.announcer_public_key,
        &announcement.signing_bytes().map_err(|_| DiscoveryRecordError::InvalidSignature)?,
        &announcement.signature,
    )
    .map_err(|_| DiscoveryRecordError::InvalidSignature)
}

pub fn sign_friend_presence(identity: &PeerIdentity, presence: &mut FriendPresenceV1) -> Result<(), CoreError> {
    presence.peer_id = identity.peer_id();
    presence.public_key = identity.public_key();
    presence.signature.clear();
    presence.signature = identity.sign(&presence.signing_bytes()?);
    Ok(())
}

pub fn verify_friend_presence(
    presence: &FriendPresenceV1,
    expected_peer: PeerId,
    requester_peer: PeerId,
    nonce: [u8; 32],
    now_unix_ms: u64,
) -> Result<(), DiscoveryRecordError> {
    if presence.protocol_version != PROTOCOL_VERSION || presence.peer_id != expected_peer {
        return Err(DiscoveryRecordError::ProtocolMismatch);
    }
    if presence.requester_peer_id != requester_peer {
        return Err(DiscoveryRecordError::PresenceRequesterMismatch);
    }
    if presence.nonce != nonce {
        return Err(DiscoveryRecordError::PresenceNonceMismatch);
    }
    validate_lifetime(presence.issued_unix_ms, presence.expires_unix_ms, FRIEND_PRESENCE_MAX_LIFETIME_MS, now_unix_ms)?;
    verify_signature(
        presence.peer_id,
        presence.public_key,
        &presence.signing_bytes().map_err(|_| DiscoveryRecordError::InvalidSignature)?,
        &presence.signature,
    )
    .map_err(|_| DiscoveryRecordError::InvalidSignature)
}

pub fn verify_discovery_membership_proof(
    announcement: &WorldAnnouncementV1,
    proof: &DiscoveryMembershipProofV1,
) -> Result<(), DiscoveryRecordError> {
    if proof.protocol_version != PROTOCOL_VERSION || proof.world_id != announcement.world_id {
        return Err(DiscoveryRecordError::ProtocolMismatch);
    }
    if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES
        || proof.genesis.world_id().ok() != Some(proof.world_id)
        || proof.genesis.validate_semantics().is_err()
        || proof.initial_membership.members.len() > MAX_DISCOVERY_MEMBERS
        || proof.current_membership.members.len() > MAX_DISCOVERY_MEMBERS
        || proof.pending_membership.as_ref().is_some_and(|proposal| {
            proposal.previous.members.len() > MAX_DISCOVERY_MEMBERS
                || proposal.proposed.members.len() > MAX_DISCOVERY_MEMBERS
        })
        || proof.membership_certificates.iter().any(|certificate| {
            certificate.proposal.previous.members.len() > MAX_DISCOVERY_MEMBERS
                || certificate.proposal.proposed.members.len() > MAX_DISCOVERY_MEMBERS
                || certificate.votes.len() > MAX_DISCOVERY_FRESHNESS_VOTES
        })
    {
        return Err(DiscoveryRecordError::InvalidMembershipProof);
    }
    let initial = &proof.initial_membership;
    if initial.validate_semantics().is_err()
        || proof.current_membership.validate_semantics().is_err()
        || initial.protocol_version != PROTOCOL_VERSION
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
        || initial.members.iter().any(|member| peer_id_from_public_key(&member.public_key) != member.peer_id)
    {
        return Err(DiscoveryRecordError::InvalidMembershipProof);
    }

    let mut last_certificate_sequence = None;
    for certificate in &proof.membership_certificates {
        if !certificate.proposal.validate_shape().unwrap_or(false)
            || certificate.proposal.previous.world_id != proof.world_id
            || certificate.proposal.proposed.world_id != proof.world_id
            || certificate.proposal.previous.validate_semantics().is_err()
            || certificate.proposal.proposed.validate_semantics().is_err()
            || last_certificate_sequence.is_some_and(|sequence| certificate.proposal.proposed.sequence <= sequence)
        {
            return Err(DiscoveryRecordError::InvalidMembershipProof);
        }
        last_certificate_sequence = Some(certificate.proposal.proposed.sequence);
        verify_membership_signature(&certificate.proposal.previous)
            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
        verify_membership_signature(&certificate.proposal.proposed)
            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
        let mut seen = HashSet::new();
        let mut last_voter = None;
        for vote in &certificate.votes {
            if !vote.matches_proposal(&certificate.proposal).unwrap_or(false)
                || !seen.insert(vote.voter_peer_id)
                || last_voter.is_some_and(|peer| vote.voter_peer_id <= peer)
            {
                return Err(DiscoveryRecordError::InvalidMembershipProof);
            }
            last_voter = Some(vote.voter_peer_id);
            verify_signature(
                vote.voter_peer_id,
                vote.voter_public_key,
                &vote.signing_bytes().map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?,
                &vote.signature,
            )
            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
        }
    }
    verify_membership_signature(&proof.current_membership).map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;
    if proof.current_membership.world_id != proof.world_id
        || last_certificate_sequence.is_some_and(|sequence| proof.current_membership.sequence < sequence)
        || proof.current_membership.sequence != announcement.membership_sequence
        || proof.current_membership.record_hash().ok() != Some(announcement.membership_hash)
        || proof.current_membership.authority_peer_id != announcement.announcer_peer_id
        || proof.current_membership.authority_public_key != announcement.announcer_public_key
        || proof.current_membership.epoch != announcement.authority_epoch
        || !proof.current_membership.members.iter().any(|member| {
            member.peer_id == proof.current_membership.authority_peer_id
                && member.public_key == proof.current_membership.authority_public_key
                && !member.banned
        })
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
        || challenge.announcement_hash
            != announcement.announcement_hash().map_err(|_| DiscoveryRecordError::InvalidSignature)?
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

fn validate_lifetime(
    issued_unix_ms: u64,
    expires_unix_ms: u64,
    max_lifetime_ms: u64,
    now_unix_ms: u64,
) -> Result<(), DiscoveryRecordError> {
    if expires_unix_ms <= issued_unix_ms || expires_unix_ms.saturating_sub(issued_unix_ms) > max_lifetime_ms {
        return Err(DiscoveryRecordError::InvalidLifetime);
    }
    if issued_unix_ms > now_unix_ms.saturating_add(DISCOVERY_CLOCK_SKEW_MS) {
        return Err(DiscoveryRecordError::FutureIssued);
    }
    if expires_unix_ms < now_unix_ms {
        return Err(DiscoveryRecordError::Expired);
    }
    Ok(())
}

/// In-memory anti-replay guard used for short-lived remote discovery results.
/// Restarting the process is safe because signed records are also time-bounded;
/// the guard prevents regression while a consumer is active.
#[derive(Debug, Default)]
pub struct AnnouncementReplayGuard {
    highest: HashMap<WorldId, (u64, u64)>,
}

impl AnnouncementReplayGuard {
    pub fn accept(&mut self, announcement: &WorldAnnouncementV1) -> Result<(), DiscoveryRecordError> {
        let generation = (announcement.authority_epoch, announcement.announcement_sequence);
        if self.highest.get(&announcement.world_id).is_some_and(|current| generation <= *current) {
            return Err(DiscoveryRecordError::Replay);
        }
        self.highest.insert(announcement.world_id, generation);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{DiscoveryCompatibilityV1, Hash32, MembershipPolicyV1, WorldPresentationV1};

    fn signed_announcement(identity: &PeerIdentity, issued: u64, expires: u64) -> WorldAnnouncementV1 {
        let mut value = WorldAnnouncementV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            presentation: WorldPresentationV1 {
                name: "Public".into(),
                description: String::new(),
                tags: vec!["survival".into()],
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
            membership_hash: Hash32([7; 32]),
            authority_epoch: 2,
            fencing_token: 3,
            canonical_head: None,
            announcement_sequence: 10,
            issued_unix_ms: issued,
            expires_unix_ms: expires,
            announcer_peer_id: PeerId::default(),
            announcer_public_key: [0; 32],
            signature: Vec::new(),
        };
        sign_world_announcement(identity, &mut value).unwrap();
        value
    }

    #[test]
    fn signed_announcement_verifies_and_forgery_fails() {
        let identity = PeerIdentity::from_secret_bytes([7; 32]);
        let mut value = signed_announcement(&identity, 1_000, 2_000);
        verify_world_announcement(&value, 1_500).unwrap();
        value.presentation.name = "forged".into();
        assert_eq!(verify_world_announcement(&value, 1_500), Err(DiscoveryRecordError::InvalidSignature));
    }

    #[test]
    fn stale_and_overlong_announcements_are_rejected() {
        let identity = PeerIdentity::from_secret_bytes([8; 32]);
        let stale = signed_announcement(&identity, 1_000, 2_000);
        assert_eq!(verify_world_announcement(&stale, 2_001), Err(DiscoveryRecordError::Expired));

        let overlong = signed_announcement(&identity, 1_000, 1_000 + WORLD_ANNOUNCEMENT_MAX_LIFETIME_MS + 1);
        assert_eq!(verify_world_announcement(&overlong, 1_500), Err(DiscoveryRecordError::InvalidLifetime));
    }

    #[test]
    fn private_announcement_is_rejected_even_when_signed() {
        let identity = PeerIdentity::from_secret_bytes([9; 32]);
        let mut value = signed_announcement(&identity, 1_000, 2_000);
        value.visibility = WorldVisibilityV1::Private;
        sign_world_announcement(&identity, &mut value).unwrap();
        assert_eq!(verify_world_announcement(&value, 1_500), Err(DiscoveryRecordError::PrivateWorld));
    }

    #[test]
    fn replay_guard_rejects_same_or_older_sequence() {
        let identity = PeerIdentity::from_secret_bytes([10; 32]);
        let first = signed_announcement(&identity, 1_000, 2_000);
        let mut guard = AnnouncementReplayGuard::default();
        guard.accept(&first).unwrap();
        assert_eq!(guard.accept(&first), Err(DiscoveryRecordError::Replay));
        let mut older = first.clone();
        older.announcement_sequence -= 1;
        assert_eq!(guard.accept(&older), Err(DiscoveryRecordError::Replay));
    }

    #[test]
    fn friend_presence_is_bound_to_requester_and_nonce() {
        let friend = PeerIdentity::from_secret_bytes([11; 32]);
        let requester = PeerIdentity::from_secret_bytes([12; 32]);
        let nonce = [13; 32];
        let mut presence = FriendPresenceV1 {
            protocol_version: PROTOCOL_VERSION,
            peer_id: PeerId::default(),
            public_key: [0; 32],
            requester_peer_id: requester.peer_id(),
            nonce,
            issued_unix_ms: 1_000,
            expires_unix_ms: 2_000,
            signature: Vec::new(),
        };
        sign_friend_presence(&friend, &mut presence).unwrap();
        verify_friend_presence(&presence, friend.peer_id(), requester.peer_id(), nonce, 1_500).unwrap();
        assert_eq!(
            verify_friend_presence(&presence, friend.peer_id(), requester.peer_id(), [14; 32], 1_500),
            Err(DiscoveryRecordError::PresenceNonceMismatch)
        );
    }
}
