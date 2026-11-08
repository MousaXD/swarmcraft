//! Inert protocol contracts for voluntary per-device Minecraft hosting.
//!
//! This does NOT activate V1 hosting contributions. Signed WorldConfigV1 records
//! remain unchanged. A future canonical migration must enforce policy support on
//! all authority-capable peers before activation is allowed.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    peer_id_from_public_key, Hash32, PeerId, ProtocolError, WorldConfigV1, WorldId, WorldMemberV1, PROTOCOL_VERSION,
};

pub const HOSTING_POLICY_FORMAT_VERSION: u16 = 1;
pub const MIN_HOSTING_HEAP_MIB: u32 = 512;
pub const MAX_HOSTING_HEAP_MIB: u32 = 65_536;
pub const HOSTING_POLICY_CAPABILITY: &str = "swarmcraft/hosting-policy/v1";

const HOSTING_POLICY_SIGN_DOMAIN: &[u8] = b"swarmcraft/hosting-policy-sign/v1\0";
const HOSTING_POLICY_HASH_DOMAIN: &[u8] = b"swarmcraft/hosting-policy-record/v1\0";

#[derive(Debug, Error)]
pub enum HostingPolicyError {
    #[error("invalid hosting policy or consent: {0}")]
    Invalid(&'static str),
    #[error("unsupported hosting policy/consent format version: {0}")]
    UnsupportedVersion(u16),
    #[error("hosting policy signature was not verified")]
    InvalidSignature,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

/// World-level requested policy, NOT a local resource allocation.
/// Policy activation requires separate canonical consensus and capability migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldHostingPolicyV1 {
    pub format_version: u16,
    pub allow_member_contributions: bool,
    /// Minimum JVM heap, NOT total process RSS or physical RAM.
    pub minimum_server_heap_mib: u32,
}

impl WorldHostingPolicyV1 {
    pub fn validate(&self) -> Result<(), HostingPolicyError> {
        if self.format_version != HOSTING_POLICY_FORMAT_VERSION {
            return Err(HostingPolicyError::UnsupportedVersion(self.format_version));
        }
        if !self.allow_member_contributions {
            if self.minimum_server_heap_mib != 0 {
                return Err(HostingPolicyError::Invalid("disabled policy must have zero minimum heap"));
            }
        } else if !(MIN_HOSTING_HEAP_MIB..=MAX_HOSTING_HEAP_MIB).contains(&self.minimum_server_heap_mib) {
            return Err(HostingPolicyError::Invalid("enabled policy has an invalid minimum heap"));
        }
        Ok(())
    }
}

/// Separately signed policy proposal; signature alone DOES NOT activate it.
/// Never append this to a WorldConfigV1 or accept it without a canonical commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedWorldHostingPolicyV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub config_sequence: u64,
    pub config_hash: Hash32,
    pub sequence: u64,
    pub previous_policy_hash: Option<Hash32>,
    pub policy: WorldHostingPolicyV1,
    pub authority_peer_id: PeerId,
    pub authority_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl SignedWorldHostingPolicyV1 {
    pub fn validate_shape(&self) -> Result<(), HostingPolicyError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(HostingPolicyError::UnsupportedVersion(self.protocol_version));
        }
        self.policy.validate()?;
        if self.sequence == 0 {
            return Err(HostingPolicyError::Invalid("policy sequence must begin at one"));
        }
        if (self.sequence == 1) != self.previous_policy_hash.is_none() {
            return Err(HostingPolicyError::Invalid("previous hash/sequence mismatch"));
        }
        if peer_id_from_public_key(&self.authority_public_key) != self.authority_peer_id {
            return Err(HostingPolicyError::Invalid("policy signer peer ID does not match public key"));
        }
        if self.signature.len() != 64 {
            return Err(HostingPolicyError::Invalid("Ed25519 signature must be 64 bytes"));
        }
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, HostingPolicyError> {
        self.policy.validate()?;
        let unsigned = (
            self.protocol_version,
            self.world_id,
            self.config_sequence,
            self.config_hash,
            self.sequence,
            self.previous_policy_hash,
            self.policy,
            self.authority_peer_id,
            self.authority_public_key,
        );
        let encoded = postcard::to_allocvec(&unsigned).map_err(ProtocolError::from)?;
        let mut out = Vec::with_capacity(HOSTING_POLICY_SIGN_DOMAIN.len() + encoded.len());
        out.extend_from_slice(HOSTING_POLICY_SIGN_DOMAIN);
        out.extend_from_slice(&encoded);
        Ok(out)
    }

    /// Full-record hash includes the signature. Caller MUST check authenticity and
    /// canonical acceptance before using the hash as an activation commitment.
    pub fn record_hash(&self) -> Result<Hash32, HostingPolicyError> {
        self.validate_shape()?;
        let encoded = postcard::to_allocvec(self).map_err(ProtocolError::from)?;
        Ok(Hash32::from_domain_bytes(HOSTING_POLICY_HASH_DOMAIN, &encoded))
    }

    /// Validates shape, signed-config binding and Ed25519 callback. Does NOT prove that
    /// the policy was committed to the canonical world history, or that quorum was met.
    pub fn verify_for_config(
        &self,
        accepted_config: &WorldConfigV1,
        verify_ed25519: impl FnOnce(&[u8; 32], &[u8], &[u8]) -> bool,
    ) -> Result<(), HostingPolicyError> {
        self.validate_shape()?;
        accepted_config.validate_semantics()?;
        if self.world_id != accepted_config.world_id
            || self.config_sequence != accepted_config.sequence
            || self.config_hash != accepted_config.config_hash()?
            || self.authority_peer_id != accepted_config.authority_peer_id
            || self.authority_public_key != accepted_config.authority_public_key
        {
            return Err(HostingPolicyError::Invalid("policy does not match accepted signed world config"));
        }
        let bytes = self.signing_bytes()?;
        if !verify_ed25519(&self.authority_public_key, &bytes, &self.signature) {
            return Err(HostingPolicyError::InvalidSignature);
        }
        Ok(())
    }

    /// Only a local hash-chain check. The future migration must prevent forks and
    /// enforce current authority, accepted configuration, quorum and fencing.
    pub fn validate_successor(&self, previous: Option<&Self>) -> Result<(), HostingPolicyError> {
        self.validate_shape()?;
        match previous {
            None if self.sequence == 1 => Ok(()),
            Some(previous)
                if previous.world_id == self.world_id
                    && previous.sequence.checked_add(1) == Some(self.sequence)
                    && self.previous_policy_hash == Some(previous.record_hash()?)
                    && self.config_sequence >= previous.config_sequence =>
            {
                Ok(())
            }
            _ => Err(HostingPolicyError::Invalid("policy does not extend accepted predecessor")),
        }
    }
}

/// Machine-local per-world, per-identity consent. NEVER trust consent sent by a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalHostingConsentV1 {
    pub format_version: u16,
    pub world_id: WorldId,
    pub peer_id: PeerId,
    pub revision: u64,
    pub enabled: bool,
    /// Chosen JVM -Xmx ceiling in MiB; zero is player-only.
    pub max_server_heap_mib: u32,
}

impl LocalHostingConsentV1 {
    pub fn validate(&self) -> Result<(), HostingPolicyError> {
        if self.format_version != HOSTING_POLICY_FORMAT_VERSION {
            return Err(HostingPolicyError::UnsupportedVersion(self.format_version));
        }
        if self.revision == 0 {
            return Err(HostingPolicyError::Invalid("consent revision must be positive"));
        }
        if !self.enabled {
            if self.max_server_heap_mib != 0 {
                return Err(HostingPolicyError::Invalid("disabled consent must allocate zero heap"));
            }
        } else if !(MIN_HOSTING_HEAP_MIB..=MAX_HOSTING_HEAP_MIB).contains(&self.max_server_heap_mib) {
            return Err(HostingPolicyError::Invalid("enabled consent has invalid heap ceiling"));
        }
        Ok(())
    }

    pub fn supersedes(&self, previous: &Self) -> Result<(), HostingPolicyError> {
        self.validate()?;
        previous.validate()?;
        if self.world_id != previous.world_id || self.peer_id != previous.peer_id || self.revision <= previous.revision
        {
            return Err(HostingPolicyError::Invalid("consent revision must increase for the same device and world"));
        }
        Ok(())
    }
}

/// No current V1 runtime may legitimately claim Enforced. Capability advertisements
/// alone never establish activation or prevent legacy authority candidacy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostingPolicyActivationV1 {
    NotEnforced,
    Enforced { accepted_policy_hash: Hash32 },
}

#[derive(Debug, Clone, Copy)]
pub struct HostingContributionAdmissionV1<'a> {
    pub accepted_config: &'a WorldConfigV1,
    pub policy: Option<&'a SignedWorldHostingPolicyV1>,
    pub activation: HostingPolicyActivationV1,
    pub member: &'a WorldMemberV1,
    pub local_consent: Option<&'a LocalHostingConsentV1>,
    /// Atomic local consent revision recorded for this launch/takeover attempt.
    pub expected_consent_revision: u64,
    /// Exact intended -Xmx in MiB, not process RSS.
    pub server_heap_mib: u32,
    /// Local runtime evidence; NEVER derive these booleans from untrusted peer claims.
    pub accepted_membership_verified: bool,
    pub negotiated_authority_capability_enforced: bool,
    pub runtime_compatible: bool,
    pub snapshot_complete: bool,
    pub runtime_budget_verified: bool,
    pub quorum_and_fencing_verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostingAdmissionDenialV1 {
    PolicyNotActivated,
    PolicyMissingOrStale,
    PolicyInvalidOrUnverified,
    MemberNotEligible,
    LocalConsentMissingOrRevoked,
    ConsentStaleOrInvalid,
    HeapOutsideConsentOrPolicy,
    CapabilityNotEnforced,
    IncompleteAuthorityProof,
}

/// Conservative ADDITIONAL admission gate. Passing never creates a lease, vote,
/// runtime slot or authority token; existing consensus/fencing rules still apply.
pub fn evaluate_hosting_contribution_v1(
    input: &HostingContributionAdmissionV1<'_>,
    verify_ed25519: impl FnOnce(&[u8; 32], &[u8], &[u8]) -> bool,
) -> Result<(), HostingAdmissionDenialV1> {
    let HostingPolicyActivationV1::Enforced { accepted_policy_hash } = input.activation else {
        return Err(HostingAdmissionDenialV1::PolicyNotActivated);
    };
    let policy = input.policy.ok_or(HostingAdmissionDenialV1::PolicyMissingOrStale)?;
    if policy.record_hash().ok() != Some(accepted_policy_hash) {
        return Err(HostingAdmissionDenialV1::PolicyMissingOrStale);
    }
    policy
        .verify_for_config(input.accepted_config, verify_ed25519)
        .map_err(|_| HostingAdmissionDenialV1::PolicyInvalidOrUnverified)?;
    if !policy.policy.allow_member_contributions {
        return Err(HostingAdmissionDenialV1::PolicyMissingOrStale);
    }
    if input.member.banned
        || !input.member.authority_eligible
        || peer_id_from_public_key(&input.member.public_key) != input.member.peer_id
    {
        return Err(HostingAdmissionDenialV1::MemberNotEligible);
    }
    let consent = input.local_consent.ok_or(HostingAdmissionDenialV1::LocalConsentMissingOrRevoked)?;
    if consent.validate().is_err()
        || consent.world_id != policy.world_id
        || consent.peer_id != input.member.peer_id
        || consent.revision != input.expected_consent_revision
    {
        return Err(HostingAdmissionDenialV1::ConsentStaleOrInvalid);
    }
    if !consent.enabled {
        return Err(HostingAdmissionDenialV1::LocalConsentMissingOrRevoked);
    }
    if input.server_heap_mib < policy.policy.minimum_server_heap_mib
        || input.server_heap_mib > consent.max_server_heap_mib
    {
        return Err(HostingAdmissionDenialV1::HeapOutsideConsentOrPolicy);
    }
    if !input.negotiated_authority_capability_enforced {
        return Err(HostingAdmissionDenialV1::CapabilityNotEnforced);
    }
    if !input.accepted_membership_verified
        || !input.runtime_compatible
        || !input.snapshot_complete
        || !input.runtime_budget_verified
        || !input.quorum_and_fencing_verified
    {
        return Err(HostingAdmissionDenialV1::IncompleteAuthorityProof);
    }
    Ok(())
}

/// Short-lived, challenge-bound peer assertion. It is NOT a quorum vote, local
/// admission permit, executable budget proof, or a new V1 wire variant.
///
/// This standalone record may only be carried over a *future negotiated*
/// capability-aware transport. Existing PeerHelloV1 and HostCapabilityV1 do
/// not enforce this structure and cannot grant policy-enabled authority.
pub const HOST_CONTRIBUTION_CLAIM_MAX_TTL_MS: u64 = 30_000;
const HOST_CONTRIBUTION_CLAIM_SIGN_DOMAIN: &[u8] = b"swarmcraft/host-contribution-claim/v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedHostContributionClaimV1 {
    pub format_version: u16,
    pub world_id: WorldId,
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub accepted_policy_hash: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub snapshot_hash: Hash32,
    pub consent_revision: u64,
    pub selected_heap_mib: u32,
    pub sampled_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    /// Unpredictable fresh challenge supplied by the verifying recipient.
    pub challenge_nonce: [u8; 32],
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedHostContributionClaimV1 {
    pub world_id: WorldId,
    pub peer_id: PeerId,
    pub policy_hash: Hash32,
    pub epoch: u64,
    pub fencing_token: u64,
    pub snapshot_hash: Hash32,
    pub challenge_nonce: [u8; 32],
    /// Receiver-side wall clock; never trust claimed sender time alone.
    pub now_unix_ms: u64,
}

impl SignedHostContributionClaimV1 {
    pub fn validate_shape(&self) -> Result<(), HostingPolicyError> {
        self.validate_fields()?;
        if self.signature.len() != 64 {
            return Err(HostingPolicyError::Invalid("host claim requires a 64-byte signature"));
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<(), HostingPolicyError> {
        if self.format_version != HOSTING_POLICY_FORMAT_VERSION {
            return Err(HostingPolicyError::UnsupportedVersion(self.format_version));
        }
        if peer_id_from_public_key(&self.public_key) != self.peer_id {
            return Err(HostingPolicyError::Invalid("host claim key does not match peer"));
        }
        if self.consent_revision == 0
            || !(MIN_HOSTING_HEAP_MIB..=MAX_HOSTING_HEAP_MIB).contains(&self.selected_heap_mib)
        {
            return Err(HostingPolicyError::Invalid("host claim has invalid consent revision or heap"));
        }
        let duration = self
            .expires_at_unix_ms
            .checked_sub(self.sampled_at_unix_ms)
            .ok_or(HostingPolicyError::Invalid("host claim expires before its sample"))?;
        if duration == 0 || duration > HOST_CONTRIBUTION_CLAIM_MAX_TTL_MS {
            return Err(HostingPolicyError::Invalid("host claim has invalid expiration window"));
        }
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, HostingPolicyError> {
        self.validate_fields()?;
        let unsigned = (
            self.format_version,
            self.world_id,
            self.peer_id,
            self.public_key,
            self.accepted_policy_hash,
            self.epoch,
            self.fencing_token,
            self.snapshot_hash,
            self.consent_revision,
            self.selected_heap_mib,
            self.sampled_at_unix_ms,
            self.expires_at_unix_ms,
            self.challenge_nonce,
        );
        let encoded = postcard::to_allocvec(&unsigned).map_err(ProtocolError::from)?;
        let mut bytes = Vec::with_capacity(HOST_CONTRIBUTION_CLAIM_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(HOST_CONTRIBUTION_CLAIM_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    /// Authentication, freshness, challenge and head binding only. Caller
    /// MUST also validate accepted canonical policy/membership/capability,
    /// observe snapshot readiness and enforce independent local launch
    /// consent + budget. A remote claim is never proof of physical free RAM.
    pub fn verify_for_challenge(
        &self,
        expected: &ExpectedHostContributionClaimV1,
        verify_ed25519: impl FnOnce(&[u8; 32], &[u8], &[u8]) -> bool,
    ) -> Result<(), HostingPolicyError> {
        self.validate_shape()?;
        if self.world_id != expected.world_id
            || self.peer_id != expected.peer_id
            || self.accepted_policy_hash != expected.policy_hash
            || self.epoch != expected.epoch
            || self.fencing_token != expected.fencing_token
            || self.snapshot_hash != expected.snapshot_hash
            || self.challenge_nonce != expected.challenge_nonce
        {
            return Err(HostingPolicyError::Invalid("host claim does not match challenge or canonical head"));
        }
        // Sender timestamps are untrusted. Never accept a future sample.
        if expected.now_unix_ms < self.sampled_at_unix_ms || expected.now_unix_ms > self.expires_at_unix_ms {
            return Err(HostingPolicyError::Invalid("host claim is expired or from the future"));
        }
        let encoded = self.signing_bytes()?;
        if !verify_ed25519(&self.public_key, &encoded, &self.signature) {
            return Err(HostingPolicyError::InvalidSignature);
        }
        Ok(())
    }
}
