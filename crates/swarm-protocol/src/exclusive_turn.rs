//! Explicit two-device reference policy, separate from ordinary quorum worlds.
//! These records cannot be decoded as a V1 world, epoch, vote or runtime permit.
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

use crate::{peer_id_from_public_key, Hash32, PeerId, ProtocolError};

pub const EXCLUSIVE_TURN_FORMAT_V1: u16 = 1;
pub const MAX_EXCLUSIVE_TURN_RELEASES: usize = 128;
pub const MAX_EXCLUSIVE_TURN_HISTORY_BYTES: usize = 256 * 1024;
pub const MAX_EXCLUSIVE_TURN_REFERENCE_BYTES: usize = 1024 * 1024;
const GENESIS_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/genesis/v1\0";
const APPROVAL_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/creation-approval/v1\0";
const INITIAL_GRANT_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/initial-grant/v1\0";
const OFFER_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/handoff-offer/v1\0";
const SIGNED_OFFER_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/signed-offer/v1\0";
const RECEIPT_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/checkpoint-receipt/v1\0";
const RELEASE_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/committed-release/v1\0";
const GRANT_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/accepted-grant/v1\0";
const REFERENCE_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/reference-checkpoint/v1\0";

#[derive(Debug, Error)]
pub enum ExclusiveTurnError {
    #[error("invalid exclusive-turn record: {0}")]
    Invalid(&'static str),
    #[error("exclusive-turn signature invalid")]
    Signature,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

fn encoded<T: Serialize>(value: &T) -> Result<Vec<u8>, ExclusiveTurnError> {
    Ok(postcard::to_allocvec(value).map_err(ProtocolError::from)?)
}

fn signing<T: Serialize>(domain: &[u8], value: &T) -> Result<Vec<u8>, ExclusiveTurnError> {
    let mut bytes = domain.to_vec();
    bytes.extend(encoded(value)?);
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExclusiveTurnWorldIdV1(Hash32);

impl fmt::Display for ExclusiveTurnWorldIdV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scpass:{}", self.0)
    }
}

impl FromStr for ExclusiveTurnWorldIdV1 {
    type Err = ExclusiveTurnError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hash: Hash32 = value
            .strip_prefix("scpass:")
            .ok_or(ExclusiveTurnError::Invalid("exclusive-turn identity prefix required"))?
            .parse()?;
        if hash == Hash32::default() {
            return Err(ExclusiveTurnError::Invalid("zero exclusive-turn identity"));
        }
        Ok(Self(hash))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnDeviceV1 {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
}

impl ExclusiveTurnDeviceV1 {
    pub fn from_key(public_key: [u8; 32]) -> Self {
        Self { peer_id: peer_id_from_public_key(&public_key), public_key }
    }
}

/// The separate reference adapter binds opaque bytes as well as the two
/// claimed snapshot digests. This is NOT a verified Minecraft manifest or
/// state root: a future runtime adapter must verify both independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnCheckpointV1 {
    pub manifest_hash: Hash32,
    pub state_root: Hash32,
    pub reference_commitment: Hash32,
}

impl ExclusiveTurnCheckpointV1 {
    pub fn validate(&self) -> Result<(), ExclusiveTurnError> {
        if [self.manifest_hash, self.state_root, self.reference_commitment].contains(&Hash32::default()) {
            return Err(ExclusiveTurnError::Invalid("checkpoint digests must be nonzero"));
        }
        Ok(())
    }

    pub fn verify_reference_bytes(&self, bytes: &[u8]) -> Result<(), ExclusiveTurnError> {
        self.validate()?;
        if exclusive_turn_reference_commitment(bytes)? != self.reference_commitment {
            return Err(ExclusiveTurnError::Invalid("reference checkpoint bytes differ"));
        }
        Ok(())
    }
}

pub fn exclusive_turn_reference_commitment(bytes: &[u8]) -> Result<Hash32, ExclusiveTurnError> {
    if bytes.is_empty() || bytes.len() > MAX_EXCLUSIVE_TURN_REFERENCE_BYTES {
        return Err(ExclusiveTurnError::Invalid("reference checkpoint size out of bounds"));
    }
    Ok(Hash32::from_domain_bytes(REFERENCE_DOMAIN, bytes))
}

/// Format 1 fixes the roster for life, requires explicit online-overlap
/// handoff, and has no election, timeout takeover, membership or solo mode.
/// Initial epoch and fence are both 1; the identity binds this exact policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnGenesisV1 {
    pub format_version: u16,
    pub creation_nonce: [u8; 32],
    pub devices: [ExclusiveTurnDeviceV1; 2],
    pub initial_holder: PeerId,
    pub compatibility: Hash32,
    pub initial_checkpoint: ExclusiveTurnCheckpointV1,
}

impl ExclusiveTurnGenesisV1 {
    pub fn validate(&self) -> Result<(), ExclusiveTurnError> {
        if self.format_version != EXCLUSIVE_TURN_FORMAT_V1
            || self.creation_nonce == [0; 32]
            || self.compatibility == Hash32::default()
            || self.devices[0].peer_id >= self.devices[1].peer_id
            || self.devices.iter().any(|device| *device != ExclusiveTurnDeviceV1::from_key(device.public_key))
            || self.device(self.initial_holder).is_none()
        {
            return Err(ExclusiveTurnError::Invalid("invalid genesis policy or fixed device roster"));
        }
        self.initial_checkpoint.validate()
    }

    pub fn device(&self, peer: PeerId) -> Option<&ExclusiveTurnDeviceV1> {
        self.devices.iter().find(|device| device.peer_id == peer)
    }

    pub fn world_id(&self) -> Result<ExclusiveTurnWorldIdV1, ExclusiveTurnError> {
        self.validate()?;
        Ok(ExclusiveTurnWorldIdV1(Hash32::from_domain_bytes(GENESIS_DOMAIN, &encoded(self)?)))
    }

    pub fn approval_bytes(&self, peer: PeerId) -> Result<Vec<u8>, ExclusiveTurnError> {
        if self.device(peer).is_none() {
            return Err(ExclusiveTurnError::Invalid("approval signer not a founding device"));
        }
        signing(APPROVAL_DOMAIN, &(self.world_id()?, peer))
    }

    pub fn initial_grant_hash(&self) -> Result<Hash32, ExclusiveTurnError> {
        Ok(Hash32::from_domain_bytes(INITIAL_GRANT_DOMAIN, &encoded(&self.world_id()?)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnApprovalV1 {
    pub world_id: ExclusiveTurnWorldIdV1,
    pub device: PeerId,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnCreationV1 {
    pub genesis: ExclusiveTurnGenesisV1,
    /// Exactly one approval per fixed device, in canonical roster order.
    pub approvals: [ExclusiveTurnApprovalV1; 2],
}

/// A proposed handoff, not permission to host or release. Source is the
/// current holder; recipient is the other fixed device. The attempt is
/// strictly increasing for the source across its whole local journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnOfferV1 {
    pub world_id: ExclusiveTurnWorldIdV1,
    pub parent_grant: Hash32,
    pub base_epoch: u64,
    pub base_fence: u64,
    pub next_epoch: u64,
    pub next_fence: u64,
    pub source: PeerId,
    pub recipient: PeerId,
    pub attempt: u64,
    pub compatibility: Hash32,
    pub checkpoint: ExclusiveTurnCheckpointV1,
}

impl ExclusiveTurnOfferV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ExclusiveTurnError> {
        signing(OFFER_DOMAIN, self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnSignedOfferV1 {
    pub offer: ExclusiveTurnOfferV1,
    pub signature: Vec<u8>,
}

impl ExclusiveTurnSignedOfferV1 {
    pub fn offer_hash(&self) -> Result<Hash32, ExclusiveTurnError> {
        Ok(Hash32::from_domain_bytes(SIGNED_OFFER_DOMAIN, &encoded(self)?))
    }
}

/// Receipt only acknowledges exact staged bytes. It never grants a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnReceiptV1 {
    pub world_id: ExclusiveTurnWorldIdV1,
    pub offer_hash: Hash32,
    pub recipient: PeerId,
    pub signature: Vec<u8>,
}

impl ExclusiveTurnReceiptV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ExclusiveTurnError> {
        signing(RECEIPT_DOMAIN, &(self.world_id, self.offer_hash, self.recipient))
    }
}

/// The source signs the exact offer and recipient receipt. A future
/// durable journal must relinquish before exposing this signed record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnReleaseV1 {
    pub offer: ExclusiveTurnSignedOfferV1,
    pub receipt: ExclusiveTurnReceiptV1,
    pub signature: Vec<u8>,
}

impl ExclusiveTurnReleaseV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ExclusiveTurnError> {
        signing(RELEASE_DOMAIN, &(&self.offer, &self.receipt))
    }

    pub fn grant_hash(&self) -> Result<Hash32, ExclusiveTurnError> {
        Ok(Hash32::from_domain_bytes(GRANT_DOMAIN, &encoded(self)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveTurnHistoryV1 {
    pub creation: ExclusiveTurnCreationV1,
    pub releases: Vec<ExclusiveTurnReleaseV1>,
}

impl ExclusiveTurnHistoryV1 {
    /// Shape/size checks only. Call the independent cryptographic verifier
    /// against an independently pinned identity before trusting anything.
    pub fn validate_envelope(&self) -> Result<(), ExclusiveTurnError> {
        if self.releases.len() > MAX_EXCLUSIVE_TURN_RELEASES
            || self.creation.approvals.iter().any(|approval| approval.signature.len() != 64)
            || self.releases.iter().any(|release| {
                release.offer.signature.len() != 64
                    || release.receipt.signature.len() != 64
                    || release.signature.len() != 64
            })
        {
            return Err(ExclusiveTurnError::Invalid("exclusive-turn history exceeds record or signature bounds"));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ExclusiveTurnError> {
        self.validate_envelope()?;
        let bytes = encoded(self)?;
        if bytes.len() > MAX_EXCLUSIVE_TURN_HISTORY_BYTES {
            return Err(ExclusiveTurnError::Invalid("exclusive-turn history exceeds byte limit"));
        }
        Ok(bytes)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, ExclusiveTurnError> {
        if bytes.is_empty() || bytes.len() > MAX_EXCLUSIVE_TURN_HISTORY_BYTES {
            return Err(ExclusiveTurnError::Invalid("exclusive-turn history exceeds byte limit"));
        }
        let history: Self = postcard::from_bytes(bytes).map_err(ProtocolError::from)?;
        if history.canonical_bytes()? != bytes {
            return Err(ExclusiveTurnError::Invalid("noncanonical exclusive-turn history bytes"));
        }
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_canonical_history_codec_never_implies_signature_verification() {
        let mut devices = [ExclusiveTurnDeviceV1::from_key([1; 32]), ExclusiveTurnDeviceV1::from_key([2; 32])];
        devices.sort_by_key(|device| device.peer_id);
        let genesis = ExclusiveTurnGenesisV1 {
            format_version: EXCLUSIVE_TURN_FORMAT_V1,
            creation_nonce: [3; 32],
            devices,
            initial_holder: devices[0].peer_id,
            compatibility: Hash32([4; 32]),
            initial_checkpoint: ExclusiveTurnCheckpointV1 {
                manifest_hash: Hash32([5; 32]),
                state_root: Hash32([6; 32]),
                reference_commitment: exclusive_turn_reference_commitment(b"opaque fixture").unwrap(),
            },
        };
        let world_id = genesis.world_id().unwrap();
        // This is intentionally only structurally valid; no real signatures.
        let approvals =
            devices.map(|d| ExclusiveTurnApprovalV1 { world_id, device: d.peer_id, signature: vec![0; 64] });
        let mut history =
            ExclusiveTurnHistoryV1 { creation: ExclusiveTurnCreationV1 { genesis, approvals }, releases: vec![] };
        let encoded = history.canonical_bytes().unwrap();
        assert_eq!(ExclusiveTurnHistoryV1::decode_canonical(&encoded).unwrap(), history);
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(ExclusiveTurnHistoryV1::decode_canonical(&trailing).is_err());
        assert!(ExclusiveTurnHistoryV1::decode_canonical(&encoded[..encoded.len() - 1]).is_err());
        assert!(ExclusiveTurnHistoryV1::decode_canonical(&[]).is_err());
        assert!(ExclusiveTurnHistoryV1::decode_canonical(&vec![0; MAX_EXCLUSIVE_TURN_HISTORY_BYTES + 1]).is_err());
        // The first postcard varint is format version 1. An overlong
        // alternate encoding is not accepted even if a decoder parses it.
        assert_eq!(encoded[0], 1);
        let mut noncanonical = vec![0x81, 0x00];
        noncanonical.extend_from_slice(&encoded[1..]);
        assert!(ExclusiveTurnHistoryV1::decode_canonical(&noncanonical).is_err());
        history.creation.approvals[0].signature.push(0);
        assert!(history.canonical_bytes().is_err());
    }
}
