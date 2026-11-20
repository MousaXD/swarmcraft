//! Inert, signed V2 migration quorum contract for an independently accepted V1 source.
//!
//! This verifies *signatures and voter quorum*, not canonical V1 history, a
//! complete snapshot, durable single-use cutover, or authority admission.
//! The caller MUST obtain all source records from independently verified
//! canonical storage. Old voters are required again ONLY for an independently
//! certified pending joint transition, not completed historical changes.
//! No production V2 authority can be activated by this helper.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    peer_id_from_public_key, Hash32, HostingRealmV2Error, MembershipRecordV1, SignedHostingMigrationProposalV2,
    SignedHostingMigrationVoteV2, SignedRealmHostingPolicyV2, WorldConfigV1, WorldGenesisV1,
    HOSTING_REALM_FORMAT_VERSION_V2,
};

/// Bound maximum for this protocol-only certificate; larger worlds require
/// a separately reviewed wire/storage resource contract.
pub const MAX_HOSTING_MIGRATION_VOTES_V2: usize = 1024;

/// Deserialize a signed vote sequence without an attacker-controlled Vec
/// reservation. The signed envelope must be checked before consumption even
/// after bounded decoding; this bound is not a quorum proof.
fn deserialize_bounded_migration_votes<'de, D>(deserializer: D) -> Result<Vec<SignedHostingMigrationVoteV2>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct BoundedMigrationVotes;

    impl<'de> serde::de::Visitor<'de> for BoundedMigrationVotes {
        type Value = Vec<SignedHostingMigrationVoteV2>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("at most 1024 signed hosting migration votes")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut votes = Vec::new();
            while let Some(vote) = seq.next_element::<SignedHostingMigrationVoteV2>()? {
                if votes.len() >= MAX_HOSTING_MIGRATION_VOTES_V2 {
                    return Err(serde::de::Error::custom("too many signed hosting migration votes"));
                }
                votes.push(vote);
            }
            Ok(votes)
        }
    }

    deserializer.deserialize_seq(BoundedMigrationVotes)
}

/// A set of individual signed V2 votes, NOT a canonical cutover certificate.
/// Sorting and unique signers are mandatory to prevent vote amplification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedHostingMigrationQuorumV2 {
    pub format_version: u16,
    pub migration_proposal_hash: Hash32,
    pub source_membership_hash: Hash32,
    #[serde(deserialize_with = "deserialize_bounded_migration_votes")]
    pub votes: Vec<SignedHostingMigrationVoteV2>,
}

/// Which V1 voter universe the independently verified consensus layer chose.
///
/// `AcceptedStable` is legal ONLY after independently establishing that the
/// current V1 membership is canonically committed with no pending transition.
/// Historical completed transitions must not require a new vote from old voters.
/// `PendingJoint` requires a separately certified V1 old/new transition
/// before these signed V2 votes can be considered. Neither case activates V2.
#[derive(Debug, Clone, Copy)]
pub enum HostingMigrationVoterSetV2<'a> {
    AcceptedStable,
    PendingJoint { previous: &'a MembershipRecordV1 },
}

/// Caller-selected signed V1 source and proposed V2 migration transcript.
/// This struct does not prove canonical acceptance, durable fencing or a
/// complete source snapshot, and is never a runtime authority permit.
#[derive(Debug, Clone, Copy)]
pub struct HostingMigrationSourceContextV2<'a> {
    pub genesis: &'a WorldGenesisV1,
    pub config: &'a WorldConfigV1,
    pub current: &'a MembershipRecordV1,
    pub voter_set: HostingMigrationVoterSetV2<'a>,
    pub initial_policy: &'a SignedRealmHostingPolicyV2,
    pub proposal: &'a SignedHostingMigrationProposalV2,
}

fn require_membership_signatures(
    config: &WorldConfigV1,
    current: &MembershipRecordV1,
    previous: Option<&MembershipRecordV1>,
    verify_ed25519: &impl Fn(&[u8; 32], &[u8], &[u8]) -> bool,
) -> Result<(), HostingRealmV2Error> {
    if peer_id_from_public_key(&config.authority_public_key) != config.authority_peer_id
        || current.authority_peer_id != config.authority_peer_id
        || current.authority_public_key != config.authority_public_key
    {
        return Err(HostingRealmV2Error::Invalid("V1 authority identity or membership signer differs"));
    }
    let source_authority = current.members.iter().find(|member| member.peer_id == current.authority_peer_id);
    if source_authority.is_none_or(|member| {
        member.banned || !member.authority_eligible || member.public_key != current.authority_public_key
    }) {
        return Err(HostingRealmV2Error::Invalid("V1 source authority is banned, ineligible or not in membership"));
    }

    // A signed record alone does not establish canonical acceptance. The
    // caller must also independently authenticate the full V1 history.
    if config.signature.len() != 64
        || !verify_ed25519(&config.authority_public_key, &config.signing_bytes()?, &config.signature)
        || current.signature.len() != 64
        || !verify_ed25519(&current.authority_public_key, &current.signing_bytes()?, &current.signature)
    {
        return Err(HostingRealmV2Error::InvalidSignature);
    }

    for record in [Some(current), previous].into_iter().flatten() {
        record.validate_semantics()?;
        if record.members.iter().any(|member| peer_id_from_public_key(&member.public_key) != member.peer_id) {
            return Err(HostingRealmV2Error::Invalid("V1 membership includes a mismatched member identity"));
        }
    }

    match previous {
        None if current.sequence > 0 && (current.sequence == 1) == current.previous_membership_hash.is_none() => {}
        Some(old)
            if old.world_id == current.world_id
                && old.epoch == current.epoch
                && old.authority_peer_id == current.authority_peer_id
                && old.authority_public_key == current.authority_public_key
                && old.sequence.checked_add(1) == Some(current.sequence)
                && current.previous_membership_hash == Some(old.record_hash()?) =>
        {
            if old.signature.len() != 64
                || !verify_ed25519(&old.authority_public_key, &old.signing_bytes()?, &old.signature)
            {
                return Err(HostingRealmV2Error::InvalidSignature);
            }
        }
        _ => {
            return Err(HostingRealmV2Error::Invalid("missing, malformed or non-contiguous V1 predecessor membership"));
        }
    }
    Ok(())
}

fn eligible_voter_count(record: &MembershipRecordV1) -> usize {
    // Membership voting is independent of authority eligibility and hosting
    // consent; player-only peers still vote. Banned members cannot vote.
    record.members.iter().filter(|member| !member.banned).count()
}

fn matches_voter(record: &MembershipRecordV1, vote: &SignedHostingMigrationVoteV2) -> bool {
    record.members.iter().any(|member| {
        !member.banned && member.peer_id == vote.voter_peer_id && member.public_key == vote.voter_public_key
    })
}

impl SignedHostingMigrationQuorumV2 {
    /// Verify source-bound V2 signatures and either a stable voter majority
    /// or separately signed old/new majorities for a certified transition.
    ///
    /// The supplied V1 records MUST be independently proven canonical and the
    /// snapshot/fence MUST be authenticated by the storage/consensus layer.
    /// The caller MUST prove AcceptedStable has no pending transition; a
    /// historical sequence >1 alone does not imply a current joint quorum.
    /// For PendingJoint, the V1 transition certificate must be accepted and
    /// independently verified before invoking this checker.
    ///
    /// Returns no authority token, lease, accepted root or replay permission.
    /// Agent 01 must atomically fence/consume the source cutover only after
    /// durable canonical acceptance. This pure function cannot prevent replay.
    pub fn verify_for_accepted_source(
        &self,
        source: &HostingMigrationSourceContextV2<'_>,
        verify_ed25519: impl Fn(&[u8; 32], &[u8], &[u8]) -> bool,
    ) -> Result<(), HostingRealmV2Error> {
        let HostingMigrationSourceContextV2 { genesis, config, current, voter_set, initial_policy, proposal } = *source;
        let previous = match voter_set {
            HostingMigrationVoterSetV2::AcceptedStable => None,
            HostingMigrationVoterSetV2::PendingJoint { previous } => Some(previous),
        };
        if self.format_version != HOSTING_REALM_FORMAT_VERSION_V2 {
            return Err(HostingRealmV2Error::UnsupportedVersion(self.format_version));
        }
        if self.votes.is_empty() || self.votes.len() > MAX_HOSTING_MIGRATION_VOTES_V2 {
            return Err(HostingRealmV2Error::Invalid("migration quorum vote count is empty or exceeds the limit"));
        }
        if current.members.len() > MAX_HOSTING_MIGRATION_VOTES_V2
            || previous.is_some_and(|record| record.members.len() > MAX_HOSTING_MIGRATION_VOTES_V2)
        {
            return Err(HostingRealmV2Error::Invalid("migration source voter universe exceeds the limit"));
        }
        if self.votes.windows(2).any(|pair| pair[0].voter_peer_id >= pair[1].voter_peer_id) {
            return Err(HostingRealmV2Error::Invalid("migration quorum votes must be strictly ordered and unique"));
        }

        proposal.verify_against_accepted_source(genesis, config, current, initial_policy, &verify_ed25519)?;
        require_membership_signatures(config, current, previous, &verify_ed25519)?;
        if self.migration_proposal_hash != proposal.proposal_hash()?
            || self.source_membership_hash != current.record_hash()?
            || self.source_membership_hash != proposal.realm_genesis.source.source_membership_hash
        {
            return Err(HostingRealmV2Error::Invalid(
                "migration quorum certificate binds a different proposal or V1 source",
            ));
        }
        let new_size = eligible_voter_count(current);
        let old_size = previous.map(eligible_voter_count);
        if new_size == 0 || old_size == Some(0) {
            return Err(HostingRealmV2Error::Invalid("a migration voter universe has no eligible members"));
        }

        let mut new_votes = 0_usize;
        let mut old_votes = 0_usize;
        let mut unique = BTreeSet::new();
        for vote in &self.votes {
            if vote.target_realm_id != proposal.target_realm_id
                || vote.migration_proposal_hash != self.migration_proposal_hash
                || vote.source_membership_hash != self.source_membership_hash
                || vote.signature.len() != 64
                || !unique.insert(vote.voter_peer_id)
            {
                return Err(HostingRealmV2Error::Invalid("invalid, mismatched or repeated migration vote"));
            }
            let in_new = matches_voter(current, vote);
            let in_old = previous.is_some_and(|old| matches_voter(old, vote));
            if !in_new && !in_old {
                return Err(HostingRealmV2Error::Invalid("vote signer is not a current or prior unbanned V1 voter"));
            }
            let bytes = vote.signing_bytes()?;
            if !verify_ed25519(&vote.voter_public_key, &bytes, &vote.signature) {
                return Err(HostingRealmV2Error::InvalidSignature);
            }
            new_votes += usize::from(in_new);
            old_votes += usize::from(in_old);
        }
        if new_votes <= new_size / 2 || old_size.is_some_and(|size| old_votes <= size / 2) {
            return Err(HostingRealmV2Error::Invalid("signed V1 voter majority or joint majority is missing"));
        }
        Ok(())
    }
}
