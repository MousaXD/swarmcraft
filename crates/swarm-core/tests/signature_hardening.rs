use swarm_core::{verify_membership_signature, verify_signature, CoreError, PeerIdentity};
use swarm_protocol::{MembershipRecordV1, WorldId, WorldMemberV1, PROTOCOL_VERSION};

fn membership(identity: &PeerIdentity) -> MembershipRecordV1 {
    MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId([1; 32]),
        epoch: 3,
        sequence: 7,
        previous_membership_hash: None,
        members: vec![WorldMemberV1 {
            peer_id: identity.peer_id(),
            public_key: identity.public_key(),
            authority_eligible: true,
            banned: false,
        }],
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: Vec::new(),
    }
}

#[test]
fn signed_membership_rejects_post_signature_mutation() {
    let identity = PeerIdentity::from_secret_bytes([0x31; 32]);
    let mut record = membership(&identity);
    identity.sign_membership(&mut record).unwrap();
    verify_membership_signature(&record).unwrap();

    record.sequence += 1;
    assert!(matches!(verify_membership_signature(&record), Err(CoreError::SignatureInvalid)));
}

#[test]
fn malformed_signature_length_is_rejected_without_panicking() {
    let identity = PeerIdentity::from_secret_bytes([0x42; 32]);
    assert!(matches!(
        verify_signature(identity.peer_id(), identity.public_key(), b"signed material", &[0; 63]),
        Err(CoreError::SignatureInvalid)
    ));
}

#[test]
fn peer_identity_debug_does_not_expose_private_key_material() {
    let secret = [0x5a; 32];
    let identity = PeerIdentity::from_secret_bytes(secret);
    let debug = format!("{identity:?}");

    assert!(debug.contains("peer_id"));
    assert!(!debug.contains("signing_key"));
    assert!(!debug.contains(&format!("{secret:?}")));
}
