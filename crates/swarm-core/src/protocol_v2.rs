use crate::{random_nonce, verify_signature, CoreError, PeerIdentity};
use swarm_protocol::{
    Hash32, RecoveryBallotV1, RecoveryVoteV1, SoloBranchV1, WorldConfigV1, WorldGenesisV1, WorldId, PROTOCOL_VERSION,
};

pub fn create_world_genesis_with_fingerprint(
    identity: &PeerIdentity,
    minecraft_version: String,
    fabric_loader_version: String,
    compatibility_fingerprint: Hash32,
) -> Result<(WorldId, WorldGenesisV1), CoreError> {
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version,
        fabric_loader_version,
        compatibility_fingerprint,
        creation_nonce: random_nonce(),
        creator_public_key: identity.public_key(),
        initial_membership: vec![identity.peer_id()],
    };
    Ok((genesis.world_id()?, genesis))
}

pub fn sign_recovery_ballot(identity: &PeerIdentity, ballot: &mut RecoveryBallotV1) -> Result<(), CoreError> {
    ballot.candidate_peer_id = identity.peer_id();
    ballot.candidate_public_key = identity.public_key();
    ballot.signature.clear();
    ballot.signature = identity.sign(&ballot.signing_bytes()?);
    Ok(())
}

pub fn verify_recovery_ballot_signature(ballot: &RecoveryBallotV1) -> Result<(), CoreError> {
    verify_signature(ballot.candidate_peer_id, ballot.candidate_public_key, &ballot.signing_bytes()?, &ballot.signature)
}

pub fn sign_recovery_vote(identity: &PeerIdentity, vote: &mut RecoveryVoteV1) -> Result<(), CoreError> {
    vote.voter_peer_id = identity.peer_id();
    vote.voter_public_key = identity.public_key();
    vote.signature.clear();
    vote.signature = identity.sign(&vote.signing_bytes()?);
    Ok(())
}

pub fn verify_recovery_vote_signature(vote: &RecoveryVoteV1) -> Result<(), CoreError> {
    verify_signature(vote.voter_peer_id, vote.voter_public_key, &vote.signing_bytes()?, &vote.signature)
}

pub fn sign_world_config(identity: &PeerIdentity, config: &mut WorldConfigV1) -> Result<(), CoreError> {
    config.authority_peer_id = identity.peer_id();
    config.authority_public_key = identity.public_key();
    config.signature.clear();
    config.signature = identity.sign(&config.signing_bytes()?);
    Ok(())
}

pub fn verify_world_config_signature(config: &WorldConfigV1) -> Result<(), CoreError> {
    verify_signature(config.authority_peer_id, config.authority_public_key, &config.signing_bytes()?, &config.signature)
}

pub fn sign_solo_branch(identity: &PeerIdentity, branch: &mut SoloBranchV1) -> Result<(), CoreError> {
    branch.authority_peer_id = identity.peer_id();
    branch.authority_public_key = identity.public_key();
    branch.signature.clear();
    branch.signature = identity.sign(&branch.signing_bytes()?);
    Ok(())
}

pub fn verify_solo_branch_signature(branch: &SoloBranchV1) -> Result<(), CoreError> {
    verify_signature(branch.authority_peer_id, branch.authority_public_key, &branch.signing_bytes()?, &branch.signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{PeerId, WorldId};

    #[test]
    fn recovery_ballot_signature_binds_round() {
        let identity = PeerIdentity::from_secret_bytes([7; 32]);
        let mut ballot = RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            base_epoch: 4,
            base_fencing_token: 8,
            target_epoch: 5,
            target_fencing_token: 9,
            round: 2,
            candidate_peer_id: PeerId::default(),
            candidate_public_key: [0; 32],
            base_snapshot_hash: Hash32([2; 32]),
            base_state_hash: Hash32([3; 32]),
            membership_hash: Hash32([4; 32]),
            signature: Vec::new(),
        };
        sign_recovery_ballot(&identity, &mut ballot).unwrap();
        verify_recovery_ballot_signature(&ballot).unwrap();
        ballot.round += 1;
        assert!(verify_recovery_ballot_signature(&ballot).is_err());
    }

    #[test]
    fn explicit_compatibility_fingerprint_is_embedded_in_genesis() {
        let identity = PeerIdentity::from_secret_bytes([8; 32]);
        let fingerprint = Hash32([5; 32]);
        let (_, genesis) =
            create_world_genesis_with_fingerprint(&identity, "1.21.8".into(), "0.17.2".into(), fingerprint).unwrap();
        assert_eq!(genesis.compatibility_fingerprint, fingerprint);
    }
}
