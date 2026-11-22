//! Real Ed25519 negative/positive coverage for the inert V2 migration quorum.
//! Neither signature validity nor majority alone proves a canonical source
//! checkpoint, durable cutover or the right to start a Minecraft authority.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use swarm_protocol::{
    peer_id_from_public_key, AuthorityPolicyV1, Hash32, HostingMigrationSourceContextV2, HostingMigrationVoterSetV2,
    MembershipPolicyV1, MembershipRecordV1, ProposedHostingRealmGenesisV2, RuntimeCompatibilityManifestV1,
    SignedHostingMigrationProposalV2, SignedHostingMigrationQuorumV2, SignedHostingMigrationVoteV2,
    SignedRealmHostingPolicyV2, SourceHistoryAnchorV2, WorldConfigV1, WorldGenesisV1, WorldHostingPolicyV1,
    WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, HOSTING_POLICY_FORMAT_VERSION,
    HOSTING_REALM_FORMAT_VERSION_V2, HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2, PROTOCOL_VERSION,
};

fn sign(key: &SigningKey, bytes: &[u8]) -> Vec<u8> {
    key.sign(bytes).to_bytes().to_vec()
}

fn real_verify(key: &[u8; 32], bytes: &[u8], sig: &[u8]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(key) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(sig) else {
        return false;
    };
    key.verify(bytes, &sig).is_ok()
}

fn member(key: &SigningKey, authority_eligible: bool) -> WorldMemberV1 {
    let public_key = key.verifying_key().to_bytes();
    WorldMemberV1 { peer_id: peer_id_from_public_key(&public_key), public_key, authority_eligible, banned: false }
}

struct Fixture {
    keys: Vec<SigningKey>,
    genesis: WorldGenesisV1,
    config: WorldConfigV1,
    membership: MembershipRecordV1,
    realm: ProposedHostingRealmGenesisV2,
    policy: SignedRealmHostingPolicyV2,
    proposal: SignedHostingMigrationProposalV2,
}

impl Fixture {
    fn new() -> Self {
        let keys: Vec<_> = (1..=4).map(|index| SigningKey::from_bytes(&[index; 32])).collect();
        let mut members: Vec<_> = keys[..3].iter().map(|key| member(key, true)).collect();
        members.sort_by_key(|member| member.peer_id);
        let authority_public_key = keys[0].verifying_key().to_bytes();
        let authority_peer_id = peer_id_from_public_key(&authority_public_key);
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([5; 32]),
            creation_nonce: [6; 32],
            creator_public_key: authority_public_key,
            initial_membership: members.iter().map(|member| member.peer_id).collect(),
        };
        let world_id = genesis.world_id().unwrap();
        let mut config = WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id,
            sequence: 1,
            previous_config_hash: None,
            compatibility: RuntimeCompatibilityManifestV1 {
                minecraft_version: "1.21.8".into(),
                loader_id: "fabric".into(),
                loader_version: "0.17.2".into(),
                swarmcraft_protocol_version: PROTOCOL_VERSION,
                fabric_adapter_version: "0.5.0".into(),
                required_server_mods: Vec::new(),
                required_client_mods: Vec::new(),
                datapacks: Vec::new(),
            },
            visibility: WorldVisibilityV1::Private,
            authority_policy: AuthorityPolicyV1 { allow_solo_advancement: false, preferred_replication_factor: 2 },
            membership_policy: MembershipPolicyV1::InviteOnly,
            presentation: WorldPresentationV1 {
                name: "Crypto V1 source".into(),
                description: String::new(),
                tags: Vec::new(),
                icon_hash: None,
                approximate_region: None,
            },
            authority_peer_id,
            authority_public_key,
            signature: Vec::new(),
        };
        config.signature = sign(&keys[0], &config.signing_bytes().unwrap());
        let mut membership = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id,
            epoch: 3,
            sequence: 1,
            previous_membership_hash: None,
            members,
            authority_peer_id,
            authority_public_key,
            signature: Vec::new(),
        };
        membership.signature = sign(&keys[0], &membership.signing_bytes().unwrap());
        let source = SourceHistoryAnchorV2 {
            source_world_id: world_id,
            source_config_sequence: config.sequence,
            source_config_hash: config.config_hash().unwrap(),
            source_membership_sequence: membership.sequence,
            source_membership_hash: membership.record_hash().unwrap(),
            source_snapshot_hash: Hash32([99; 32]),
            source_epoch: membership.epoch,
            source_fencing_token: 4,
        };
        let realm = ProposedHostingRealmGenesisV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            source,
            migration_nonce: [88; 32],
            required_authority_feature_version: HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2,
        };
        let mut policy = SignedRealmHostingPolicyV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            realm_id: realm.realm_id().unwrap(),
            sequence: 1,
            previous_policy_hash: None,
            policy: WorldHostingPolicyV1 {
                format_version: HOSTING_POLICY_FORMAT_VERSION,
                allow_member_contributions: true,
                minimum_server_heap_mib: 1_024,
            },
            signer_peer_id: authority_peer_id,
            signer_public_key: authority_public_key,
            signature: Vec::new(),
        };
        policy.signature = sign(&keys[0], &policy.signing_bytes().unwrap());
        let mut proposal = SignedHostingMigrationProposalV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            realm_genesis: realm,
            target_realm_id: realm.realm_id().unwrap(),
            initial_policy_hash: policy.record_hash().unwrap(),
            activation_epoch: 4,
            activation_fencing_token: 5,
            source_authority_peer_id: authority_peer_id,
            source_authority_public_key: authority_public_key,
            signature: Vec::new(),
        };
        proposal.signature = sign(&keys[0], &proposal.signing_bytes().unwrap());
        Self { keys, genesis, config, membership, realm, policy, proposal }
    }

    fn resign_for_membership(&mut self) {
        self.membership.signature = sign(&self.keys[0], &self.membership.signing_bytes().unwrap());
        self.realm.source.source_membership_sequence = self.membership.sequence;
        self.realm.source.source_membership_hash = self.membership.record_hash().unwrap();
        self.policy.realm_id = self.realm.realm_id().unwrap();
        self.policy.signature = sign(&self.keys[0], &self.policy.signing_bytes().unwrap());
        self.proposal.realm_genesis = self.realm;
        self.proposal.target_realm_id = self.realm.realm_id().unwrap();
        self.proposal.initial_policy_hash = self.policy.record_hash().unwrap();
        self.proposal.signature = sign(&self.keys[0], &self.proposal.signing_bytes().unwrap());
    }

    fn vote(&self, index: usize) -> SignedHostingMigrationVoteV2 {
        let public_key = self.keys[index].verifying_key().to_bytes();
        let mut vote = SignedHostingMigrationVoteV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            target_realm_id: self.proposal.target_realm_id,
            migration_proposal_hash: self.proposal.proposal_hash().unwrap(),
            source_membership_hash: self.membership.record_hash().unwrap(),
            voter_peer_id: peer_id_from_public_key(&public_key),
            voter_public_key: public_key,
            signature: Vec::new(),
        };
        vote.signature = sign(&self.keys[index], &vote.signing_bytes().unwrap());
        vote
    }

    fn certificate(&self, indices: &[usize]) -> SignedHostingMigrationQuorumV2 {
        let mut votes: Vec<_> = indices.iter().map(|&index| self.vote(index)).collect();
        votes.sort_by_key(|vote| vote.voter_peer_id);
        SignedHostingMigrationQuorumV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            migration_proposal_hash: self.proposal.proposal_hash().unwrap(),
            source_membership_hash: self.membership.record_hash().unwrap(),
            votes,
        }
    }

    fn verify(&self, cert: &SignedHostingMigrationQuorumV2, old: Option<&MembershipRecordV1>) -> bool {
        cert.verify_for_accepted_source(
            &HostingMigrationSourceContextV2 {
                genesis: &self.genesis,
                config: &self.config,
                current: &self.membership,
                voter_set: match old {
                    Some(previous) => HostingMigrationVoterSetV2::PendingJoint { previous },
                    None => HostingMigrationVoterSetV2::AcceptedStable,
                },
                initial_policy: &self.policy,
                proposal: &self.proposal,
            },
            real_verify,
        )
        .is_ok()
    }
}

#[test]
fn crypto_majority_is_real_and_cross_signed_records_cannot_be_forged() {
    let f = Fixture::new();
    let cert = f.certificate(&[0, 1]);
    assert!(f.verify(&cert, None));
    assert!(!f.verify(&f.certificate(&[0]), None));

    let mut bad = cert.clone();
    bad.votes[0].signature = bad.votes[1].signature.clone();
    assert!(!f.verify(&bad, None));
    bad = cert.clone();
    bad.votes[0].signature[0] ^= 1;
    assert!(!f.verify(&bad, None));
    bad = cert.clone();
    bad.votes[0].source_membership_hash = Hash32([2; 32]);
    assert!(!f.verify(&bad, None));
    bad = cert.clone();
    bad.votes[0].target_realm_id = f.config.world_id;
    assert!(!f.verify(&bad, None));
    bad = cert.clone();
    bad.votes[0].signature.clear();
    assert!(!f.verify(&bad, None));

    let mut tampered = Fixture::new();
    tampered.config.signature[0] ^= 1;
    assert!(!tampered.verify(&cert, None));
    let mut tampered = Fixture::new();
    tampered.membership.signature[0] ^= 1;
    assert!(!tampered.verify(&cert, None));
    let mut tampered = Fixture::new();
    tampered.proposal.signature[0] ^= 1;
    assert!(!tampered.verify(&cert, None));
}

#[test]
fn crypto_joint_quorum_requires_both_member_sets_and_player_only_can_vote() {
    let mut f = Fixture::new();
    let old = f.membership.clone();
    f.membership.sequence = 2;
    f.membership.previous_membership_hash = Some(old.record_hash().unwrap());
    // Remove key 2 and add key 4. Key 3 is explicitly player-only but
    // remains an ordinary voting member in BOTH sets.
    f.membership.members.retain(|record| record.peer_id != member(&f.keys[1], true).peer_id);
    f.membership.members.push(member(&f.keys[3], false));
    for record in &mut f.membership.members {
        if record.peer_id == member(&f.keys[2], true).peer_id {
            record.authority_eligible = false;
        }
    }
    f.membership.members.sort_by_key(|record| record.peer_id);
    f.resign_for_membership();

    assert!(!f.verify(&f.certificate(&[0, 1]), Some(&old))); // old-only majority
    assert!(!f.verify(&f.certificate(&[0, 3]), Some(&old))); // new-only majority
    assert!(f.verify(&f.certificate(&[0, 2]), Some(&old))); // includes player-only voter
    assert!(f.verify(&f.certificate(&[0, 1, 3]), Some(&old))); // separate majorities
    // Already accepted stable memberships use current votes; the caller
    // must independently prove there is no pending source transition.
    assert!(f.verify(&f.certificate(&[0, 2]), None));

    let mut forged_old = old.clone();
    forged_old.signature[0] ^= 1;
    assert!(!f.verify(&f.certificate(&[0, 2]), Some(&forged_old)));
    forged_old = old.clone();
    forged_old.epoch += 1;
    assert!(!f.verify(&f.certificate(&[0, 2]), Some(&forged_old)));
}

#[test]
fn crypto_completed_historical_membership_uses_current_stable_voters() {
    let mut f = Fixture::new();
    let preceding = f.membership.clone();
    f.membership.sequence += 1;
    f.membership.previous_membership_hash = Some(preceding.record_hash().unwrap());
    f.resign_for_membership();
    // Existing member voters 0 and 1 form a majority of the fully accepted
    // current set. The previously accepted historical set does not re-vote.
    // Only the caller's independently verified canonical stable-state proof
    // may authorize using AcceptedStable here.
    let cert = f.certificate(&[0, 1]);
    assert!(f.verify(&cert, None));
    assert!(f.verify(&cert, Some(&preceding)));
}

#[test]
fn crypto_source_authority_must_remain_eligible_and_unbanned() {
    let mut banned = Fixture::new();
    let authority = banned.membership.authority_peer_id;
    banned.membership.members.iter_mut().find(|m| m.peer_id == authority).unwrap().banned = true;
    banned.resign_for_membership();
    let banned_cert = banned.certificate(&[1, 2]);
    assert!(!banned.verify(&banned_cert, None));

    let mut ineligible = Fixture::new();
    let authority = ineligible.membership.authority_peer_id;
    ineligible.membership.members.iter_mut().find(|m| m.peer_id == authority).unwrap().authority_eligible = false;
    ineligible.resign_for_membership();
    let ineligible_cert = ineligible.certificate(&[1, 2]);
    assert!(!ineligible.verify(&ineligible_cert, None));
}
