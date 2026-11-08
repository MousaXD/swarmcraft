use swarm_protocol::{
    evaluate_hosting_contribution_v1, peer_id_from_public_key, AuthorityPolicyV1, ExpectedHostContributionClaimV1,
    Hash32, HostingAdmissionDenialV1 as Denial, HostingContributionAdmissionV1, HostingPolicyActivationV1,
    LocalHostingConsentV1, MembershipPolicyV1, PeerId, RuntimeCompatibilityManifestV1, SignedHostContributionClaimV1,
    SignedWorldHostingPolicyV1, WorldConfigV1, WorldHostingPolicyV1, WorldId, WorldMemberV1, WorldPresentationV1,
    WorldVisibilityV1, HOSTING_POLICY_FORMAT_VERSION, MIN_HOSTING_HEAP_MIB, PROTOCOL_VERSION,
};

fn config() -> WorldConfigV1 {
    let authority_public_key = [1; 32];
    WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId([3; 32]),
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
            name: "Legacy world".into(),
            description: String::new(),
            tags: vec![],
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: peer_id_from_public_key(&authority_public_key),
        authority_public_key,
        signature: vec![8; 64],
    }
}

fn policy(config: &WorldConfigV1) -> SignedWorldHostingPolicyV1 {
    SignedWorldHostingPolicyV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: config.world_id,
        config_sequence: config.sequence,
        config_hash: config.config_hash().unwrap(),
        sequence: 1,
        previous_policy_hash: None,
        policy: WorldHostingPolicyV1 {
            format_version: HOSTING_POLICY_FORMAT_VERSION,
            allow_member_contributions: true,
            minimum_server_heap_mib: 1_024,
        },
        authority_peer_id: config.authority_peer_id,
        authority_public_key: config.authority_public_key,
        signature: vec![7; 64],
    }
}

fn member() -> WorldMemberV1 {
    let public_key = [2; 32];
    WorldMemberV1 { peer_id: peer_id_from_public_key(&public_key), public_key, authority_eligible: true, banned: false }
}

fn consent(world_id: WorldId, peer_id: PeerId) -> LocalHostingConsentV1 {
    LocalHostingConsentV1 {
        format_version: HOSTING_POLICY_FORMAT_VERSION,
        world_id,
        peer_id,
        revision: 1,
        enabled: true,
        max_server_heap_mib: 4_096,
    }
}

// Test-only stand-in: this is NOT cryptographic signature verification.
// Production callers must use swarm-core Ed25519 verification and canonical activation.
fn test_signature(_: &[u8; 32], _: &[u8], signature: &[u8]) -> bool {
    signature == [7; 64]
}

struct Fixture {
    config: WorldConfigV1,
    policy: SignedWorldHostingPolicyV1,
    member: WorldMemberV1,
    consent: LocalHostingConsentV1,
}

impl Fixture {
    fn new() -> Self {
        let config = config();
        let policy = policy(&config);
        let member = member();
        let consent = consent(config.world_id, member.peer_id);
        Self { config, policy, member, consent }
    }

    fn admission(&self) -> HostingContributionAdmissionV1<'_> {
        HostingContributionAdmissionV1 {
            accepted_config: &self.config,
            policy: Some(&self.policy),
            activation: HostingPolicyActivationV1::Enforced {
                accepted_policy_hash: self.policy.record_hash().unwrap(),
            },
            member: &self.member,
            local_consent: Some(&self.consent),
            expected_consent_revision: self.consent.revision,
            server_heap_mib: 2_048,
            accepted_membership_verified: true,
            negotiated_authority_capability_enforced: true,
            runtime_compatible: true,
            snapshot_complete: true,
            runtime_budget_verified: true,
            quorum_and_fencing_verified: true,
        }
    }
}

#[test]
fn old_signed_world_config_bytes_and_identity_stay_unmodified() {
    let config = config();
    let original = postcard::to_allocvec(&config).unwrap();
    let hash = config.config_hash().unwrap();
    let sign_bytes = config.signing_bytes().unwrap();
    let policy = policy(&config);
    assert!(policy.validate_shape().is_ok());
    let restored: WorldConfigV1 = postcard::from_bytes(&original).unwrap();
    assert_eq!(restored, config);
    assert_eq!(config.config_hash().unwrap(), hash);
    assert_eq!(config.signing_bytes().unwrap(), sign_bytes);
    assert_eq!(postcard::to_allocvec(&config).unwrap(), original);
}

#[test]
fn new_records_roundtrip_and_have_distinct_canonical_domains() {
    let p = policy(&config());
    let bytes = postcard::to_allocvec(&p).unwrap();
    let restored: SignedWorldHostingPolicyV1 = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(restored.record_hash().unwrap(), p.record_hash().unwrap());
    assert!(p.signing_bytes().unwrap().starts_with(b"swarmcraft/hosting-policy-sign/v1\0"));
    assert_ne!(p.record_hash().unwrap(), p.config_hash);
    assert!(postcard::from_bytes::<SignedWorldHostingPolicyV1>(&bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn invalid_and_unknown_policy_formats_fail_closed() {
    let mut p = policy(&config());
    p.policy.format_version = 2;
    assert!(p.validate_shape().is_err());
    p.policy.format_version = HOSTING_POLICY_FORMAT_VERSION;
    p.policy.minimum_server_heap_mib = MIN_HOSTING_HEAP_MIB - 1;
    assert!(p.validate_shape().is_err());
    p.policy.minimum_server_heap_mib = 1_024;
    p.policy.allow_member_contributions = false;
    assert!(p.validate_shape().is_err());
    p.policy.minimum_server_heap_mib = 0;
    assert!(p.validate_shape().is_ok());
    p.protocol_version += 1;
    assert!(p.validate_shape().is_err());
    p.protocol_version = PROTOCOL_VERSION;
    p.signature.clear();
    assert!(p.validate_shape().is_err());
}

#[test]
fn signature_and_config_binding_are_mandatory() {
    let f = Fixture::new();
    assert!(f.policy.verify_for_config(&f.config, test_signature).is_ok());
    assert!(f.policy.verify_for_config(&f.config, |_, _, _| false).is_err());
    let mut p = f.policy.clone();
    p.signature[0] = 9;
    assert!(p.verify_for_config(&f.config, test_signature).is_err());
    p = f.policy.clone();
    p.config_hash = Hash32([33; 32]);
    assert!(p.verify_for_config(&f.config, test_signature).is_err());
    p = f.policy.clone();
    p.world_id = WorldId([25; 32]);
    assert!(p.verify_for_config(&f.config, test_signature).is_err());
    p = f.policy.clone();
    p.authority_peer_id = member().peer_id;
    assert!(p.verify_for_config(&f.config, test_signature).is_err());
}

#[test]
fn policy_history_requires_strict_hash_link_and_monotonic_sequence() {
    let f = Fixture::new();
    assert!(f.policy.validate_successor(None).is_ok());
    let mut next = f.policy.clone();
    next.sequence = 2;
    next.previous_policy_hash = Some(f.policy.record_hash().unwrap());
    assert!(next.validate_successor(Some(&f.policy)).is_ok());
    next.previous_policy_hash = Some(Hash32([9; 32]));
    assert!(next.validate_successor(Some(&f.policy)).is_err());
    next.previous_policy_hash = Some(f.policy.record_hash().unwrap());
    next.sequence = 1;
    assert!(next.validate_successor(Some(&f.policy)).is_err());
    next.sequence = 2;
    next.config_sequence = 0;
    assert!(next.validate_successor(Some(&f.policy)).is_err());
    assert!(next.validate_successor(None).is_err());
}

#[test]
fn old_or_missing_policy_never_automatically_authorizes_contribution() {
    let f = Fixture::new();
    let mut a = f.admission();
    a.activation = HostingPolicyActivationV1::NotEnforced;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::PolicyNotActivated));
    a = f.admission();
    a.policy = None;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::PolicyMissingOrStale));
    a = f.admission();
    a.activation = HostingPolicyActivationV1::Enforced { accepted_policy_hash: Hash32([2; 32]) };
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::PolicyMissingOrStale));
}

#[test]
fn joining_and_membership_never_imply_hosting_consent() {
    let f = Fixture::new();
    let mut a = f.admission();
    a.local_consent = None;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::LocalConsentMissingOrRevoked));

    let mut player_only = f.consent;
    player_only.enabled = false;
    player_only.max_server_heap_mib = 0;
    a.local_consent = Some(&player_only);
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::LocalConsentMissingOrRevoked));

    let ineligible = WorldMemberV1 { authority_eligible: false, ..f.member.clone() };
    a = f.admission();
    a.member = &ineligible;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::MemberNotEligible));

    let banned = WorldMemberV1 { banned: true, ..f.member.clone() };
    a.member = &banned;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::MemberNotEligible));
}

#[test]
fn consent_is_per_world_per_peer_local_and_monotonically_revocable() {
    let f = Fixture::new();
    let mut a = f.admission();
    let wrong_world = LocalHostingConsentV1 { world_id: WorldId([4; 32]), ..f.consent };
    a.local_consent = Some(&wrong_world);
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::ConsentStaleOrInvalid));

    let wrong_peer = LocalHostingConsentV1 { peer_id: PeerId([1; 32]), ..f.consent };
    a.local_consent = Some(&wrong_peer);
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::ConsentStaleOrInvalid));
    a = f.admission();
    a.expected_consent_revision = 0;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::ConsentStaleOrInvalid));

    let mut revoked = f.consent;
    revoked.revision += 1;
    revoked.enabled = false;
    revoked.max_server_heap_mib = 0;
    assert!(revoked.supersedes(&f.consent).is_ok());
    assert!(f.consent.supersedes(&revoked).is_err());
    a = f.admission();
    a.local_consent = Some(&revoked);
    a.expected_consent_revision = revoked.revision;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::LocalConsentMissingOrRevoked));
}

#[test]
fn heap_policy_and_local_ceiling_both_limit_contributions() {
    let f = Fixture::new();
    let mut a = f.admission();
    assert!(evaluate_hosting_contribution_v1(&a, test_signature).is_ok());
    a.server_heap_mib = 512;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::HeapOutsideConsentOrPolicy));
    a.server_heap_mib = 8_192;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::HeapOutsideConsentOrPolicy));
    let mut bad = f.consent;
    bad.max_server_heap_mib = 1;
    a = f.admission();
    a.local_consent = Some(&bad);
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::ConsentStaleOrInvalid));
}

#[test]
fn compatibility_runtime_replication_and_quorum_are_separate_gates() {
    let f = Fixture::new();
    let mut a = f.admission();
    a.negotiated_authority_capability_enforced = false;
    assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::CapabilityNotEnforced));
    for gate in 0..5 {
        a = f.admission();
        match gate {
            0 => a.accepted_membership_verified = false,
            1 => a.runtime_compatible = false,
            2 => a.snapshot_complete = false,
            3 => a.runtime_budget_verified = false,
            _ => a.quorum_and_fencing_verified = false,
        }
        assert_eq!(evaluate_hosting_contribution_v1(&a, test_signature), Err(Denial::IncompleteAuthorityProof));
    }
}

#[test]
fn forged_or_malformed_policy_never_promotes_a_peer() {
    let f = Fixture::new();
    let mut a = f.admission();
    assert_eq!(evaluate_hosting_contribution_v1(&a, |_, _, _| false), Err(Denial::PolicyInvalidOrUnverified));
    let mut bad = f.policy.clone();
    bad.authority_public_key = [77; 32];
    a.policy = Some(&bad);
    a.activation = HostingPolicyActivationV1::Enforced { accepted_policy_hash: Hash32([9; 32]) };
    assert!(evaluate_hosting_contribution_v1(&a, test_signature).is_err());
}

fn signed_host_claim(f: &Fixture) -> SignedHostContributionClaimV1 {
    SignedHostContributionClaimV1 {
        format_version: HOSTING_POLICY_FORMAT_VERSION,
        world_id: f.config.world_id,
        peer_id: f.member.peer_id,
        public_key: f.member.public_key,
        accepted_policy_hash: f.policy.record_hash().unwrap(),
        epoch: 7,
        fencing_token: 12,
        snapshot_hash: Hash32([6; 32]),
        consent_revision: f.consent.revision,
        selected_heap_mib: 2048,
        sampled_at_unix_ms: 1000,
        expires_at_unix_ms: 2000,
        challenge_nonce: [42; 32],
        signature: vec![7; 64],
    }
}

fn expected_host_claim(claim: &SignedHostContributionClaimV1) -> ExpectedHostContributionClaimV1 {
    ExpectedHostContributionClaimV1 {
        world_id: claim.world_id,
        peer_id: claim.peer_id,
        policy_hash: claim.accepted_policy_hash,
        epoch: claim.epoch,
        fencing_token: claim.fencing_token,
        snapshot_hash: claim.snapshot_hash,
        challenge_nonce: claim.challenge_nonce,
        now_unix_ms: 1500,
    }
}

#[test]
fn challenge_bound_host_claim_is_not_a_legacy_hello_or_authority_token() {
    let f = Fixture::new();
    let claim = signed_host_claim(&f);
    let expected = expected_host_claim(&claim);
    assert!(claim.signing_bytes().unwrap().starts_with(b"swarmcraft/host-contribution-claim/v1\0"));
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_ok());
    assert!(claim.verify_for_challenge(&expected, |_, _, _| false, |_| true).is_err());
    let bytes = postcard::to_allocvec(&claim).unwrap();
    assert_eq!(postcard::from_bytes::<SignedHostContributionClaimV1>(&bytes).unwrap(), claim);
}

#[test]
fn signed_host_claim_is_single_use_even_before_expiration() {
    use std::collections::BTreeSet;

    let f = Fixture::new();
    let claim = signed_host_claim(&f);
    let expected = expected_host_claim(&claim);
    let mut pending = BTreeSet::from([claim.challenge_nonce]);
    assert!(claim.verify_for_challenge(&expected, test_signature, |nonce| pending.remove(nonce)).is_ok());
    assert_eq!(pending.len(), 0);
    assert!(claim.verify_for_challenge(&expected, test_signature, |nonce| pending.remove(nonce)).is_err());
    let mut pending = BTreeSet::from([claim.challenge_nonce]);
    assert!(claim.verify_for_challenge(&expected, |_, _, _| false, |nonce| pending.remove(nonce)).is_err());
    // Forged signatures must never consume a valid outstanding challenge.
    assert!(pending.contains(&claim.challenge_nonce));
}

#[test]
fn host_claim_must_bind_challenge_policy_head_peer_and_epoch() {
    let f = Fixture::new();
    let claim = signed_host_claim(&f);
    let mut expected = expected_host_claim(&claim);
    expected.challenge_nonce[0] ^= 1;
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
    expected = expected_host_claim(&claim);
    expected.policy_hash = Hash32([9; 32]);
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
    expected = expected_host_claim(&claim);
    expected.peer_id = PeerId([9; 32]);
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
    expected = expected_host_claim(&claim);
    expected.world_id = WorldId([9; 32]);
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
    expected = expected_host_claim(&claim);
    expected.epoch += 1;
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
    expected = expected_host_claim(&claim);
    expected.fencing_token += 1;
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
    expected = expected_host_claim(&claim);
    expected.snapshot_hash = Hash32([4; 32]);
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
}

#[test]
fn host_claim_expiration_replay_and_malformed_signed_data_fail_closed() {
    let f = Fixture::new();
    let mut claim = signed_host_claim(&f);
    let mut expected = expected_host_claim(&claim);

    expected.now_unix_ms = claim.expires_at_unix_ms + 1;
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
    expected.now_unix_ms = claim.sampled_at_unix_ms - 1;
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());

    expected = expected_host_claim(&claim);
    claim.format_version = HOSTING_POLICY_FORMAT_VERSION + 1;
    assert!(claim.verify_for_challenge(&expected, test_signature, |_| true).is_err());
    claim.format_version = HOSTING_POLICY_FORMAT_VERSION;
    claim.expires_at_unix_ms = claim.sampled_at_unix_ms + 30_001;
    assert!(claim.validate_shape().is_err());
    claim.expires_at_unix_ms = claim.sampled_at_unix_ms;
    assert!(claim.validate_shape().is_err());
    claim.expires_at_unix_ms = claim.sampled_at_unix_ms - 1;
    assert!(claim.validate_shape().is_err());
    claim = signed_host_claim(&f);
    claim.consent_revision = 0;
    assert!(claim.validate_shape().is_err());
    claim = signed_host_claim(&f);
    claim.selected_heap_mib = 0;
    assert!(claim.validate_shape().is_err());
    claim = signed_host_claim(&f);
    claim.signature.pop();
    assert!(claim.validate_shape().is_err());
    claim = signed_host_claim(&f);
    claim.public_key[0] ^= 1;
    assert!(claim.validate_shape().is_err());
}
