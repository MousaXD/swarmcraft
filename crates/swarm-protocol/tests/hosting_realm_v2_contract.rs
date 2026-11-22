//! Proposal-only V2 contract tests. The signature callback here is a test stub
//! and does NOT establish cryptographic acceptance, V1 quorum or live authority.

use swarm_protocol::{
    peer_id_from_public_key, AuthorityPolicyV1, ExpectedHostingAuthorityHeadV2, Hash32, HostingAuthorityOperationV2,
    MembershipPolicyV1, MembershipRecordV1, PeerId, ProposedHostingRealmGenesisV2, RuntimeCompatibilityManifestV1,
    SignedHostingAuthorityActionV2, SignedHostingMigrationProposalV2, SignedHostingMigrationVoteV2,
    SignedRealmHostingPolicyV2, SourceHistoryAnchorV2, WorldConfigV1, WorldGenesisV1, WorldHostingPolicyV1, WorldId,
    WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, HOSTING_POLICY_FORMAT_VERSION,
    HOSTING_REALM_FORMAT_VERSION_V2, HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2, PROTOCOL_VERSION,
};

fn accepts_test_signatures(_: &[u8; 32], _: &[u8], signature: &[u8]) -> bool {
    [7_u8, 8, 9, 10].iter().any(|byte| signature == [*byte; 64])
}

struct Fixture {
    genesis: WorldGenesisV1,
    config: WorldConfigV1,
    membership: MembershipRecordV1,
    realm: ProposedHostingRealmGenesisV2,
    policy: SignedRealmHostingPolicyV2,
    proposal: SignedHostingMigrationProposalV2,
    voter: WorldMemberV1,
}

impl Fixture {
    fn new() -> Self {
        let authority_public_key = [1; 32];
        let authority_peer_id = peer_id_from_public_key(&authority_public_key);
        let mut members: Vec<_> = [1_u8, 2, 3]
            .iter()
            .map(|key| {
                let public_key = [*key; 32];
                WorldMemberV1 {
                    peer_id: peer_id_from_public_key(&public_key),
                    public_key,
                    authority_eligible: true,
                    banned: false,
                }
            })
            .collect();
        members.sort_by_key(|member| member.peer_id);
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "26.1.2".into(),
            fabric_loader_version: "0.19.3".into(),
            compatibility_fingerprint: Hash32([5; 32]),
            creation_nonce: [6; 32],
            creator_public_key: authority_public_key,
            initial_membership: members.iter().map(|member| member.peer_id).collect(),
        };
        let source_world_id = genesis.world_id().unwrap();
        let config = WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: source_world_id,
            sequence: 1,
            previous_config_hash: None,
            compatibility: RuntimeCompatibilityManifestV1 {
                minecraft_version: "26.1.2".into(),
                loader_id: "fabric".into(),
                loader_version: "0.19.3".into(),
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
                name: "Source".into(),
                description: String::new(),
                tags: Vec::new(),
                icon_hash: None,
                approximate_region: None,
            },
            authority_peer_id,
            authority_public_key,
            signature: vec![8; 64],
        };
        let membership = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: source_world_id,
            epoch: 3,
            sequence: 1,
            previous_membership_hash: None,
            members,
            authority_peer_id,
            authority_public_key,
            signature: vec![8; 64],
        };
        let source = SourceHistoryAnchorV2 {
            source_world_id,
            source_config_sequence: config.sequence,
            source_config_hash: config.config_hash().unwrap(),
            source_membership_sequence: membership.sequence,
            source_membership_hash: membership.record_hash().unwrap(),
            source_snapshot_hash: Hash32([99; 32]),
            source_epoch: 3,
            source_fencing_token: 4,
        };
        let realm = ProposedHostingRealmGenesisV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            source,
            migration_nonce: [88; 32],
            required_authority_feature_version: HOSTING_REALM_REQUIRED_AUTHORITY_FEATURE_V2,
        };
        let policy = SignedRealmHostingPolicyV2 {
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
            signature: vec![7; 64],
        };
        let proposal = SignedHostingMigrationProposalV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            realm_genesis: realm,
            target_realm_id: realm.realm_id().unwrap(),
            initial_policy_hash: policy.record_hash().unwrap(),
            activation_epoch: 4,
            activation_fencing_token: 5,
            source_authority_peer_id: authority_peer_id,
            source_authority_public_key: authority_public_key,
            signature: vec![8; 64],
        };
        let voter = membership.members.iter().find(|member| member.public_key == [2; 32]).unwrap().clone();
        Self { genesis, config, membership, realm, policy, proposal, voter }
    }

    fn vote(&self) -> SignedHostingMigrationVoteV2 {
        SignedHostingMigrationVoteV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            target_realm_id: self.proposal.target_realm_id,
            migration_proposal_hash: self.proposal.proposal_hash().unwrap(),
            source_membership_hash: self.membership.record_hash().unwrap(),
            voter_peer_id: self.voter.peer_id,
            voter_public_key: self.voter.public_key,
            signature: vec![9; 64],
        }
    }

    fn action(&self) -> SignedHostingAuthorityActionV2 {
        SignedHostingAuthorityActionV2 {
            format_version: HOSTING_REALM_FORMAT_VERSION_V2,
            operation: HostingAuthorityOperationV2::RecoveryBallot,
            realm_id: self.realm.realm_id().unwrap(),
            committed_policy_hash: self.policy.record_hash().unwrap(),
            committed_migration_hash: self.proposal.proposal_hash().unwrap(),
            accepted_membership_hash: self.membership.record_hash().unwrap(),
            epoch: 4,
            fencing_token: 5,
            snapshot_hash: self.realm.source.source_snapshot_hash,
            candidate_peer_id: self.config.authority_peer_id,
            consent_revision: 1,
            selected_heap_mib: 2_048,
            signer_peer_id: self.voter.peer_id,
            signer_public_key: self.voter.public_key,
            nonce: [19; 32],
            signature: vec![10; 64],
        }
    }

    fn expected(&self) -> ExpectedHostingAuthorityHeadV2 {
        ExpectedHostingAuthorityHeadV2 {
            operation: HostingAuthorityOperationV2::RecoveryBallot,
            realm_id: self.realm.realm_id().unwrap(),
            committed_policy_hash: self.policy.record_hash().unwrap(),
            committed_migration_hash: self.proposal.proposal_hash().unwrap(),
            accepted_membership_hash: self.membership.record_hash().unwrap(),
            epoch: 4,
            fencing_token: 5,
            snapshot_hash: self.realm.source.source_snapshot_hash,
            candidate_peer_id: self.config.authority_peer_id,
            expected_consent_revision: 1,
            expected_selected_heap_mib: 2_048,
            expected_signer_peer_id: self.voter.peer_id,
            expected_nonce: [19; 32],
        }
    }
}

#[test]
fn distinct_v2_realm_preserves_source_v1_genesis_and_config_bytes() {
    let f = Fixture::new();
    let genesis_bytes = postcard::to_allocvec(&f.genesis).unwrap();
    let config_bytes = postcard::to_allocvec(&f.config).unwrap();
    let config_signature = f.config.signing_bytes().unwrap();
    let old_world_id: WorldId = f.genesis.world_id().unwrap();
    let new_realm = f.realm.realm_id().unwrap();

    assert_ne!(old_world_id, new_realm);
    assert_eq!(f.realm.source.source_world_id, old_world_id);
    assert_eq!(f.genesis.world_id().unwrap(), old_world_id);
    assert_eq!(postcard::to_allocvec(&f.genesis).unwrap(), genesis_bytes);
    assert_eq!(postcard::to_allocvec(&f.config).unwrap(), config_bytes);
    assert_eq!(f.config.signing_bytes().unwrap(), config_signature);
    assert!(f.realm.source.validate_against_accepted_v1(&f.genesis, &f.config, &f.membership).is_ok());
}

#[test]
fn migration_proposal_is_signed_but_does_not_grant_canonical_authority() {
    let f = Fixture::new();
    assert!(f
        .proposal
        .verify_against_accepted_source(&f.genesis, &f.config, &f.membership, &f.policy, accepts_test_signatures,)
        .is_ok());
    assert!(f
        .proposal
        .verify_against_accepted_source(&f.genesis, &f.config, &f.membership, &f.policy, |_, _, _| false,)
        .is_err());
    assert!(f.policy.verify_for_realm(&f.realm, f.config.authority_peer_id, |_, _, _| false).is_err());
    assert!(f.proposal.signing_bytes().unwrap().starts_with(b"swarmcraft/hosting-migration-proposal-sign/v2\0"));
    assert!(f.policy.signing_bytes().unwrap().starts_with(b"swarmcraft/hosting-realm-policy-sign/v2\0"));
}

#[test]
fn invalid_source_anchor_and_unsupported_version_reject_without_v1_mutation() {
    let f = Fixture::new();
    let mut bad = f.realm;
    bad.format_version = 1;
    assert!(bad.realm_id().is_err());
    bad = f.realm;
    bad.required_authority_feature_version = 1;
    assert!(bad.realm_id().is_err());
    bad = f.realm;
    bad.migration_nonce = [0; 32];
    assert!(bad.realm_id().is_err());

    bad = f.realm;
    bad.source.source_config_hash = Hash32([3; 32]);
    assert!(bad.source.validate_against_accepted_v1(&f.genesis, &f.config, &f.membership).is_err());
    bad = f.realm;
    bad.source.source_membership_hash = Hash32([4; 32]);
    assert!(bad.source.validate_against_accepted_v1(&f.genesis, &f.config, &f.membership).is_err());
    bad = f.realm;
    bad.source.source_snapshot_hash = Hash32::default();
    assert!(bad.source.validate_against_accepted_v1(&f.genesis, &f.config, &f.membership).is_err());
    bad = f.realm;
    bad.source.source_epoch = 1;
    assert!(bad.source.validate_against_accepted_v1(&f.genesis, &f.config, &f.membership).is_err());
}

#[test]
fn target_policy_is_not_a_v1_sidecar_and_history_cannot_fork_silently() {
    let f = Fixture::new();
    let mut bad = f.policy.clone();
    bad.realm_id = f.realm.source.source_world_id;
    assert!(bad.verify_for_realm(&f.realm, f.config.authority_peer_id, accepts_test_signatures).is_err());
    bad = f.policy.clone();
    bad.policy.minimum_server_heap_mib = 0;
    assert!(bad.validate_shape().is_err());
    bad = f.policy.clone();
    bad.sequence = 2;
    bad.previous_policy_hash = Some(f.policy.record_hash().unwrap());
    assert!(bad.validate_successor(&f.policy).is_ok());
    bad.previous_policy_hash = Some(Hash32([4; 32]));
    assert!(bad.validate_successor(&f.policy).is_err());
    bad = f.policy.clone();
    bad.sequence = 1;
    bad.previous_policy_hash = Some(f.policy.record_hash().unwrap());
    assert!(bad.validate_shape().is_err());
    bad = f.policy.clone();
    bad.signature.clear();
    assert!(bad.signing_bytes().is_ok());
    assert!(bad.validate_shape().is_err());
}

#[test]
fn migration_proposal_must_bind_source_authority_target_policy_and_fence() {
    let f = Fixture::new();
    let mut bad = f.proposal.clone();
    bad.target_realm_id = f.realm.source.source_world_id;
    assert!(bad.validate_shape().is_err());
    bad = f.proposal.clone();
    bad.initial_policy_hash = Hash32([4; 32]);
    assert!(bad
        .verify_against_accepted_source(&f.genesis, &f.config, &f.membership, &f.policy, accepts_test_signatures,)
        .is_err());
    bad = f.proposal.clone();
    bad.activation_epoch = f.realm.source.source_epoch;
    assert!(bad.validate_shape().is_err());
    bad = f.proposal.clone();
    bad.activation_fencing_token = f.realm.source.source_fencing_token;
    assert!(bad.validate_shape().is_err());
    bad = f.proposal.clone();
    bad.source_authority_public_key = [3; 32];
    assert!(bad.validate_shape().is_err());
    bad = f.proposal.clone();
    bad.signature.clear();
    assert!(bad.signing_bytes().is_ok());
    assert!(bad.validate_shape().is_err());
}

#[test]
fn v2_migration_votes_are_bound_to_source_membership_and_proposal() {
    let f = Fixture::new();
    let vote = f.vote();
    assert!(vote.verify_for_proposal(&f.proposal, &f.membership, accepts_test_signatures).is_ok());
    let mut bad = vote.clone();
    bad.migration_proposal_hash = Hash32([11; 32]);
    assert!(bad.verify_for_proposal(&f.proposal, &f.membership, accepts_test_signatures).is_err());
    bad = vote.clone();
    bad.source_membership_hash = Hash32([9; 32]);
    assert!(bad.verify_for_proposal(&f.proposal, &f.membership, accepts_test_signatures).is_err());
    bad = vote.clone();
    bad.voter_peer_id = PeerId([4; 32]);
    assert!(bad.verify_for_proposal(&f.proposal, &f.membership, accepts_test_signatures).is_err());
    bad = vote.clone();
    bad.signature.pop();
    assert!(bad.verify_for_proposal(&f.proposal, &f.membership, accepts_test_signatures).is_err());
    bad = vote.clone();
    assert!(bad.verify_for_proposal(&f.proposal, &f.membership, |_, _, _| false).is_err());
    bad.signature.clear();
    assert!(bad.signing_bytes().is_ok());
}

#[test]
fn signed_v2_authority_actions_reject_v1_downgrade_and_stale_context() {
    let f = Fixture::new();
    let action = f.action();
    let expected = f.expected();
    assert!(action.verify_for_head(&expected, accepts_test_signatures).is_ok());
    assert!(action.verify_for_head(&expected, |_, _, _| false).is_err());
    assert!(action.signing_bytes().unwrap().starts_with(b"swarmcraft/hosting-authority-action-sign/v2\0"));

    let mut bad = action.clone();
    bad.format_version = 1;
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.realm_id = f.realm.source.source_world_id;
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.operation = HostingAuthorityOperationV2::LeaseGrant;
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.candidate_peer_id = PeerId([17; 32]);
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.epoch += 1;
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.fencing_token += 1;
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.committed_policy_hash = Hash32([1; 32]);
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.committed_migration_hash = Hash32([2; 32]);
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.accepted_membership_hash = Hash32([3; 32]);
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.snapshot_hash = Hash32([4; 32]);
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.nonce = [20; 32];
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.consent_revision += 1;
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.selected_heap_mib += 512;
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.signer_public_key = [42; 32];
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
    bad = action.clone();
    bad.signature.clear();
    assert!(bad.signing_bytes().is_ok());
    assert!(bad.verify_for_head(&expected, accepts_test_signatures).is_err());
}

#[test]
fn verifier_owned_expectations_cannot_be_replaced_by_claimed_values() {
    let f = Fixture::new();
    let action = f.action();
    let mut expected = f.expected();

    expected.expected_consent_revision += 1;
    assert!(action.verify_for_head(&expected, accepts_test_signatures).is_err());
    expected = f.expected();
    expected.expected_selected_heap_mib += 512;
    assert!(action.verify_for_head(&expected, accepts_test_signatures).is_err());
    expected = f.expected();
    expected.expected_nonce[0] ^= 1;
    assert!(action.verify_for_head(&expected, accepts_test_signatures).is_err());
    expected = f.expected();
    expected.expected_signer_peer_id = PeerId([40; 32]);
    assert!(action.verify_for_head(&expected, accepts_test_signatures).is_err());

    let mut bad = action.clone();
    bad.selected_heap_mib = 0;
    assert!(bad.signing_bytes().is_err());
    bad = action.clone();
    bad.consent_revision = 0;
    assert!(bad.signing_bytes().is_err());
    bad = action;
    bad.nonce = [0; 32];
    assert!(bad.signing_bytes().is_err());
}
