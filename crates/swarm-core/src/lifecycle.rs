use crate::{verify_signature, CoreError, PeerIdentity};
use swarm_protocol::{JoinRequestV1, LeaveRequestV1, SleepRecordV1};

impl PeerIdentity {
    pub fn sign_join_request(&self, request: &mut JoinRequestV1) -> Result<(), CoreError> {
        request.joining_member.peer_id = self.peer_id();
        request.joining_member.public_key = self.public_key();
        request.validate_semantics()?;
        request.signature.clear();
        request.signature = self.sign(&request.signing_bytes()?);
        Ok(())
    }

    pub fn sign_leave_request(&self, request: &mut LeaveRequestV1) -> Result<(), CoreError> {
        request.leaving_peer_id = self.peer_id();
        request.leaving_public_key = self.public_key();
        request.validate_semantics()?;
        request.signature.clear();
        request.signature = self.sign(&request.signing_bytes()?);
        Ok(())
    }

    pub fn sign_sleep_record(&self, record: &mut SleepRecordV1) -> Result<(), CoreError> {
        record.authority_peer_id = self.peer_id();
        record.authority_public_key = self.public_key();
        record.validate_semantics()?;
        record.signature.clear();
        record.signature = self.sign(&record.signing_bytes()?);
        Ok(())
    }
}

pub fn verify_join_request_signature(request: &JoinRequestV1) -> Result<(), CoreError> {
    request.validate_semantics()?;
    verify_signature(
        request.joining_member.peer_id,
        request.joining_member.public_key,
        &request.signing_bytes()?,
        &request.signature,
    )
}

pub fn verify_leave_request_signature(request: &LeaveRequestV1) -> Result<(), CoreError> {
    request.validate_semantics()?;
    verify_signature(request.leaving_peer_id, request.leaving_public_key, &request.signing_bytes()?, &request.signature)
}

pub fn verify_sleep_record_signature(record: &SleepRecordV1) -> Result<(), CoreError> {
    record.validate_semantics()?;
    verify_signature(record.authority_peer_id, record.authority_public_key, &record.signing_bytes()?, &record.signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random_nonce;
    use swarm_protocol::{Hash32, InviteV1, PeerId, WorldGenesisV1, WorldId, WorldMemberV1, PROTOCOL_VERSION};

    #[test]
    fn join_request_proves_joining_key_control() {
        let inviter = PeerIdentity::generate();
        let joining = PeerIdentity::generate();
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([3; 32]),
            creation_nonce: random_nonce(),
            creator_public_key: inviter.public_key(),
            initial_membership: vec![inviter.peer_id()],
        };
        let world = genesis.world_id().unwrap();
        let mut invite = InviteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            display_name: "world".into(),
            genesis,
            inviter_peer_id: inviter.peer_id(),
            inviter_public_key: inviter.public_key(),
            bootstrap_addrs: Vec::new(),
            expires_unix_ms: u64::MAX,
            nonce: random_nonce(),
            signature: Vec::new(),
        };
        inviter.sign_invite(&mut invite).unwrap();
        let mut request = JoinRequestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            invite,
            joining_member: WorldMemberV1 {
                peer_id: PeerId([0; 32]),
                public_key: [0; 32],
                authority_eligible: true,
                banned: false,
            },
            nonce: random_nonce(),
            signature: Vec::new(),
        };
        joining.sign_join_request(&mut request).unwrap();
        verify_join_request_signature(&request).unwrap();
    }

    #[test]
    fn leave_request_rejects_modified_membership_hash() {
        let peer = PeerIdentity::generate();
        let mut request = LeaveRequestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            membership_hash: Hash32([2; 32]),
            leaving_peer_id: peer.peer_id(),
            leaving_public_key: peer.public_key(),
            nonce: random_nonce(),
            signature: Vec::new(),
        };
        peer.sign_leave_request(&mut request).unwrap();
        verify_leave_request_signature(&request).unwrap();
        request.membership_hash = Hash32([3; 32]);
        assert!(verify_leave_request_signature(&request).is_err());
    }

    #[test]
    fn sleep_record_signature_rejects_modified_generation() {
        let authority = PeerIdentity::generate();
        let mut record = SleepRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            latest_snapshot_hash: Hash32([2; 32]),
            epoch: 5,
            fencing_token: 8,
            authority_peer_id: authority.peer_id(),
            authority_public_key: authority.public_key(),
            signature: Vec::new(),
        };
        authority.sign_sleep_record(&mut record).unwrap();
        verify_sleep_record_signature(&record).unwrap();
        record.fencing_token += 1;
        assert!(verify_sleep_record_signature(&record).is_err());
    }
}
