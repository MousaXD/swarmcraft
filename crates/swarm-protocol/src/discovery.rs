use serde::{Deserialize, Serialize};

use crate::{
    Hash32, MembershipPolicyV1, PeerId, ProtocolError, WorldId, WorldPresentationV1, WorldVisibilityV1,
    PROTOCOL_VERSION,
};

const WORLD_ANNOUNCEMENT_SIGN_DOMAIN: &[u8] = b"swarmcraft/world-announcement/v1\0";
const FRIEND_PRESENCE_SIGN_DOMAIN: &[u8] = b"swarmcraft/friend-presence/v1\0";

/// Bounded public compatibility material. Exact artifact requirements remain in
/// canonical world state and are intentionally not copied into discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryCompatibilityV1 {
    pub minecraft_version: String,
    pub loader_id: String,
    pub loader_version: String,
    pub fabric_adapter_version: String,
    pub compatibility_fingerprint: Hash32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DiscoveryFilterV1 {
    /// Case-insensitive free text matched against world name, description and tags.
    pub query: Option<String>,
    pub minecraft_version: Option<String>,
    pub loader_id: Option<String>,
    pub loader_version: Option<String>,
    pub tags: Vec<String>,
    pub approximate_region: Option<String>,
    /// Requested result cap. Receivers clamp this to their protocol maximum.
    pub limit: u16,
}

/// A short-lived, self-authenticating discovery projection of canonical world
/// state. It deliberately contains no membership list, invite secret, snapshot,
/// artifact URL, or machine-local runtime data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldAnnouncementV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub presentation: WorldPresentationV1,
    pub compatibility: DiscoveryCompatibilityV1,
    pub visibility: WorldVisibilityV1,
    pub membership_policy: MembershipPolicyV1,
    /// Sequence/hash of the canonical WorldConfigV1 projected by this record.
    pub config_sequence: u64,
    pub config_hash: Hash32,
    /// Current canonical authority generation that authorized publication.
    pub authority_epoch: u64,
    pub fencing_token: u64,
    /// Monotonic-within-authority publication sequence used for replay rejection.
    pub announcement_sequence: u64,
    pub issued_unix_ms: u64,
    pub expires_unix_ms: u64,
    pub announcer_peer_id: PeerId,
    pub announcer_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl WorldAnnouncementV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut presentation = self.presentation.clone();
        presentation.normalize();
        let unsigned = (
            self.protocol_version,
            self.world_id,
            presentation,
            &self.compatibility,
            self.visibility,
            self.membership_policy,
            self.config_sequence,
            self.config_hash,
            self.authority_epoch,
            self.fencing_token,
            self.announcement_sequence,
            self.issued_unix_ms,
            self.expires_unix_ms,
            self.announcer_peer_id,
            self.announcer_public_key,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(WORLD_ANNOUNCEMENT_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(WORLD_ANNOUNCEMENT_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    pub fn is_discoverable_visibility(&self) -> bool {
        matches!(self.visibility, WorldVisibilityV1::Public | WorldVisibilityV1::Unlisted)
    }
}

/// Challenge-bound liveness proof for a cryptographic friend identity. Shared
/// world information is never carried in this record; callers derive it from
/// their own locally authorized canonical state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendPresenceV1 {
    pub protocol_version: u16,
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub requester_peer_id: PeerId,
    pub nonce: [u8; 32],
    pub issued_unix_ms: u64,
    pub expires_unix_ms: u64,
    pub signature: Vec<u8>,
}

impl FriendPresenceV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.peer_id,
            self.public_key,
            self.requester_peer_id,
            self.nonce,
            self.issued_unix_ms,
            self.expires_unix_ms,
        );
        let encoded = postcard::to_allocvec(&unsigned)?;
        let mut bytes = Vec::with_capacity(FRIEND_PRESENCE_SIGN_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(FRIEND_PRESENCE_SIGN_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }

    pub fn protocol_is_supported(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announcement() -> WorldAnnouncementV1 {
        WorldAnnouncementV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            presentation: WorldPresentationV1 {
                name: "A world".into(),
                description: "test".into(),
                tags: vec!["survival".into(), "friends".into()],
                icon_hash: None,
                approximate_region: Some("me-central".into()),
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
            config_sequence: 3,
            config_hash: Hash32([3; 32]),
            authority_epoch: 4,
            fencing_token: 5,
            announcement_sequence: 6,
            issued_unix_ms: 10,
            expires_unix_ms: 20,
            announcer_peer_id: PeerId([4; 32]),
            announcer_public_key: [5; 32],
            signature: Vec::new(),
        }
    }

    #[test]
    fn tag_order_does_not_change_announcement_signing_bytes() {
        let a = announcement();
        let mut b = a.clone();
        b.presentation.tags.reverse();
        assert_eq!(a.signing_bytes().unwrap(), b.signing_bytes().unwrap());
    }

    #[test]
    fn private_visibility_is_not_discoverable() {
        let mut value = announcement();
        value.visibility = WorldVisibilityV1::Private;
        assert!(!value.is_discoverable_visibility());
        value.visibility = WorldVisibilityV1::Unlisted;
        assert!(value.is_discoverable_visibility());
    }
}
