use swarm_cli::discovery::validate_fresh_discovery_candidate;
use swarm_consensus::{membership_vote_for, validate_discovery_freshness_quorum};
use swarm_core::{sign_discovery_freshness_vote, sign_world_announcement, DiscoveryFreshnessReplayGuard, PeerIdentity};
use swarm_protocol::{
    DiscoveryCanonicalHeadV1, DiscoveryCompatibilityV1, DiscoveryFreshnessChallengeV1, DiscoveryMembershipProofV1,
    Hash32, MembershipCertificateV1, MembershipPolicyV1, MembershipProposalV1, MembershipRecordV1, PeerId,
    WorldAnnouncementV1, WorldGenesisV1, WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
};

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn signed_membership(
    authority: &PeerIdentity,
    world: swarm_protocol::WorldId,
    epoch: u64,
    sequence: u64,
    previous: Option<Hash32>,
    mut members: Vec<WorldMemberV1>,
) -> MembershipRecordV1 {
    members.sort_by_key(|value| value.peer_id);
    let mut record = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch,
        sequence,
        previous_membership_hash: previous,
        members,
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    authority.sign_membership(&mut record).unwrap();
    record
}

fn fixture(
    count: usize,
) -> (Vec<PeerIdentity>, WorldAnnouncementV1, DiscoveryMembershipProofV1, DiscoveryFreshnessChallengeV1) {
    let identities = (1..=count).map(|id| PeerIdentity::from_secret_bytes([id as u8; 32])).collect::<Vec<_>>();
    let mut initial_members = identities.iter().map(member).collect::<Vec<_>>();
    initial_members.sort_by_key(|value| value.peer_id);
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "1.21.8".into(),
        fabric_loader_version: "0.17.2".into(),
        compatibility_fingerprint: Hash32([9; 32]),
        creation_nonce: [7; 32],
        creator_public_key: identities[0].public_key(),
        initial_membership: initial_members.iter().map(|value| value.peer_id).collect(),
    };
    let world = genesis.world_id().unwrap();
    let current = signed_membership(&identities[0], world, 3, 0, None, initial_members);
    let mut announcement = WorldAnnouncementV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        presentation: WorldPresentationV1 {
            name: "Fresh world".into(),
            description: String::new(),
            tags: vec!["survival".into()],
            icon_hash: None,
            approximate_region: None,
        },
        compatibility: DiscoveryCompatibilityV1 {
            minecraft_version: "1.21.8".into(),
            loader_id: "fabric".into(),
            loader_version: "0.17.2".into(),
            fabric_adapter_version: "0.5.0".into(),
            compatibility_fingerprint: Hash32([9; 32]),
        },
        visibility: WorldVisibilityV1::Public,
        membership_policy: MembershipPolicyV1::InviteOnly,
        config_sequence: 4,
        config_hash: Hash32([10; 32]),
        membership_sequence: current.sequence,
        membership_hash: current.record_hash().unwrap(),
        authority_epoch: 3,
        fencing_token: 8,
        canonical_head: Some(DiscoveryCanonicalHeadV1 {
            snapshot_number: 12,
            manifest_hash: Hash32([11; 32]),
            epoch: 3,
            sequence: 22,
        }),
        announcement_sequence: 1,
        issued_unix_ms: 1_000,
        expires_unix_ms: 50_000,
        announcer_peer_id: identities[0].peer_id(),
        announcer_public_key: identities[0].public_key(),
        signature: Vec::new(),
    };
    sign_world_announcement(&identities[0], &mut announcement).unwrap();
    let proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis,
        initial_membership: current.clone(),
        membership_certificates: Vec::new(),
        current_membership: current,
        pending_membership: None,
    };
    let verifier = PeerIdentity::from_secret_bytes([99; 32]);
    let challenge = DiscoveryFreshnessChallengeV1 {
        protocol_version: PROTOCOL_VERSION,
        verifier_peer_id: verifier.peer_id(),
        nonce: [55; 32],
        world_id: world,
        announcement_hash: announcement.announcement_hash().unwrap(),
        membership_sequence: announcement.membership_sequence,
        membership_hash: announcement.membership_hash,
        pending_membership_proposal_hash: None,
        authority_peer_id: announcement.announcer_peer_id,
        authority_epoch: announcement.authority_epoch,
        fencing_token: announcement.fencing_token,
        config_sequence: announcement.config_sequence,
        config_hash: announcement.config_hash,
        canonical_head: announcement.canonical_head,
        issued_unix_ms: 2_000,
        expires_unix_ms: 10_000,
    };
    (identities, announcement, proof, challenge)
}

fn votes<'a>(
    ids: impl IntoIterator<Item = &'a PeerIdentity>,
    challenge: &DiscoveryFreshnessChallengeV1,
) -> Vec<swarm_protocol::DiscoveryFreshnessVoteV1> {
    let mut result =
        ids.into_iter().map(|id| sign_discovery_freshness_vote(id, challenge).unwrap()).collect::<Vec<_>>();
    result.sort_by_key(|vote| vote.voter_peer_id);
    result
}

#[test]
fn current_quorum_accepts_and_replay_wrong_head_epoch_membership_world_and_verifier_fail() {
    let (ids, announcement, proof, challenge) = fixture(3);
    let verifier = challenge.verifier_peer_id;
    let valid_votes = votes(ids[..2].iter(), &challenge);
    let mut replay = DiscoveryFreshnessReplayGuard::default();
    validate_fresh_discovery_candidate(
        &announcement,
        &proof,
        &challenge,
        &valid_votes,
        verifier,
        challenge.nonce,
        3_000,
        &mut replay,
    )
    .unwrap();
    assert!(validate_fresh_discovery_candidate(
        &announcement,
        &proof,
        &challenge,
        &valid_votes,
        verifier,
        challenge.nonce,
        3_000,
        &mut replay,
    )
    .is_err());

    for mutate in 0..7 {
        let mut bad = challenge.clone();
        match mutate {
            0 => bad.world_id = swarm_protocol::WorldId([42; 32]),
            1 => bad.membership_hash = Hash32([42; 32]),
            2 => bad.membership_sequence += 1,
            3 => bad.authority_epoch += 1,
            4 => bad.fencing_token += 1,
            5 => bad.canonical_head.as_mut().unwrap().manifest_hash = Hash32([42; 32]),
            _ => bad.verifier_peer_id = PeerId([42; 32]),
        }
        let bad_votes = votes(ids[..2].iter(), &bad);
        let mut guard = DiscoveryFreshnessReplayGuard::default();
        assert!(validate_fresh_discovery_candidate(
            &announcement,
            &proof,
            &bad,
            &bad_votes,
            verifier,
            challenge.nonce,
            3_000,
            &mut guard,
        )
        .is_err());
    }
}

#[test]
fn unrelated_self_signed_attacker_and_removed_or_banned_signers_do_not_form_current_quorum() {
    let (ids, mut announcement, mut proof, challenge) = fixture(3);
    let attacker = PeerIdentity::from_secret_bytes([88; 32]);
    announcement.announcer_peer_id = attacker.peer_id();
    announcement.announcer_public_key = attacker.public_key();
    sign_world_announcement(&attacker, &mut announcement).unwrap();
    let mut guard = DiscoveryFreshnessReplayGuard::default();
    assert!(validate_fresh_discovery_candidate(
        &announcement,
        &proof,
        &challenge,
        &votes(ids[..2].iter(), &challenge),
        challenge.verifier_peer_id,
        challenge.nonce,
        3_000,
        &mut guard,
    )
    .is_err());

    proof.current_membership.members[1].banned = true;
    let mut banned_votes = votes([&ids[0], &ids[1]], &challenge);
    banned_votes.sort_by_key(|vote| vote.voter_peer_id);
    assert!(validate_discovery_freshness_quorum(&proof, &banned_votes).is_err());
}

#[test]
fn joint_transition_requires_both_old_and_new_quorums_and_stale_old_side_cannot_certify() {
    let (mut ids, announcement, mut proof, _) = fixture(3);
    ids.push(PeerIdentity::from_secret_bytes([4; 32]));
    ids.push(PeerIdentity::from_secret_bytes([5; 32]));
    let previous = proof.current_membership.clone();
    let mut proposed_members = ids.iter().map(member).collect::<Vec<_>>();
    proposed_members.sort_by_key(|value| value.peer_id);
    let proposed = signed_membership(
        &ids[0],
        announcement.world_id,
        previous.epoch,
        1,
        Some(previous.record_hash().unwrap()),
        proposed_members,
    );
    let proposal = MembershipProposalV1 { previous: previous.clone(), proposed };
    proof.pending_membership = Some(proposal.clone());
    let mut challenge = DiscoveryFreshnessChallengeV1 {
        protocol_version: PROTOCOL_VERSION,
        verifier_peer_id: PeerIdentity::from_secret_bytes([99; 32]).peer_id(),
        nonce: [77; 32],
        world_id: announcement.world_id,
        announcement_hash: announcement.announcement_hash().unwrap(),
        membership_sequence: announcement.membership_sequence,
        membership_hash: announcement.membership_hash,
        pending_membership_proposal_hash: Some(proposal.proposal_hash().unwrap()),
        authority_peer_id: announcement.announcer_peer_id,
        authority_epoch: announcement.authority_epoch,
        fencing_token: announcement.fencing_token,
        config_sequence: announcement.config_sequence,
        config_hash: announcement.config_hash,
        canonical_head: announcement.canonical_head,
        issued_unix_ms: 2_000,
        expires_unix_ms: 10_000,
    };
    let old_only = votes(ids[..2].iter(), &challenge);
    assert!(validate_discovery_freshness_quorum(&proof, &old_only).is_err());
    let joint = votes([&ids[0], &ids[1], &ids[3]], &challenge);
    validate_discovery_freshness_quorum(&proof, &joint).unwrap();

    challenge.nonce = [78; 32];
    let stale_old_partition = votes(ids[..1].iter(), &challenge);
    assert!(validate_discovery_freshness_quorum(&proof, &stale_old_partition).is_err());
}

#[test]
fn truncated_membership_change_chain_and_noncanonical_vote_collection_fail_closed() {
    let (mut ids, mut announcement, mut proof, mut challenge) = fixture(3);
    ids.push(PeerIdentity::from_secret_bytes([4; 32]));
    ids.push(PeerIdentity::from_secret_bytes([5; 32]));
    let previous = proof.current_membership.clone();
    let mut proposed_members = ids.iter().map(member).collect::<Vec<_>>();
    proposed_members.sort_by_key(|value| value.peer_id);
    let proposed = signed_membership(
        &ids[0],
        announcement.world_id,
        previous.epoch,
        1,
        Some(previous.record_hash().unwrap()),
        proposed_members,
    );
    let proposal = MembershipProposalV1 { previous: previous.clone(), proposed: proposed.clone() };
    let mut membership_votes = [0usize, 1, 3]
        .into_iter()
        .map(|index| {
            let mut vote = membership_vote_for(&proposal, ids[index].peer_id(), ids[index].public_key()).unwrap();
            vote.signature = ids[index].sign(&vote.signing_bytes().unwrap());
            vote
        })
        .collect::<Vec<_>>();
    membership_votes.sort_by_key(|vote| vote.voter_peer_id);
    proof.membership_certificates.push(MembershipCertificateV1 { proposal, votes: membership_votes });
    proof.current_membership = proposed.clone();
    announcement.membership_sequence = proposed.sequence;
    announcement.membership_hash = proposed.record_hash().unwrap();
    sign_world_announcement(&ids[0], &mut announcement).unwrap();
    challenge.announcement_hash = announcement.announcement_hash().unwrap();
    challenge.membership_sequence = announcement.membership_sequence;
    challenge.membership_hash = announcement.membership_hash;
    let valid = votes([&ids[0], &ids[1], &ids[3]], &challenge);
    let mut guard = DiscoveryFreshnessReplayGuard::default();
    validate_fresh_discovery_candidate(
        &announcement,
        &proof,
        &challenge,
        &valid,
        challenge.verifier_peer_id,
        challenge.nonce,
        3_000,
        &mut guard,
    )
    .unwrap();

    let mut truncated = proof.clone();
    truncated.membership_certificates.clear();
    assert!(swarm_consensus::validate_discovery_membership_proof_shape(&truncated).is_err());

    let mut duplicated = valid.clone();
    duplicated.push(valid[0].clone());
    assert!(validate_discovery_freshness_quorum(&proof, &duplicated).is_err());
    let mut reordered = valid.clone();
    reordered.reverse();
    assert!(validate_discovery_freshness_quorum(&proof, &reordered).is_err());
}
