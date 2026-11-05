use swarm_core::{
    lifecycle::{verify_join_request_signature, verify_leave_request_signature, verify_sleep_record_signature},
    verify_invite_signature, verify_lease_signature, verify_membership_signature, verify_recovery_ballot_signature,
    verify_recovery_vote_signature, verify_snapshot_signature, verify_solo_branch_signature, verify_transfer_signature,
    verify_world_config_signature, PeerIdentity,
};
use swarm_protocol::{
    snapshot_state_root, ArtifactSideV1, AuthorityLeaseGrantV1, AuthorityPolicyV1, AuthorityTransferV1, BlobDescriptor,
    BlobEncoding, EpochMode, EpochRecordV1, Hash32, InviteV1, JoinRequestV1, LeaveRequestV1, MembershipPolicyV1,
    MembershipRecordV1, MembershipVoteV1, RecoveryBallotV1, RecoveryVoteV1, RuntimeCompatibilityManifestV1,
    SleepRecordV1, SnapshotEntry, SnapshotManifestV1, SoloBranchV1, TransferPhase, WorldConfigV1, WorldGenesisV1,
    WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
};

fn unsupported() -> u16 {
    PROTOCOL_VERSION.checked_add(1).unwrap()
}

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn supported_genesis(identity: &PeerIdentity) -> (WorldGenesisV1, swarm_protocol::WorldId) {
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "1.21.8".into(),
        fabric_loader_version: "0.17.2".into(),
        compatibility_fingerprint: Hash32([4; 32]),
        creation_nonce: [5; 32],
        creator_public_key: identity.public_key(),
        initial_membership: vec![identity.peer_id()],
    };
    let world = genesis.world_id().unwrap();
    (genesis, world)
}

#[test]
fn every_state_bearing_signed_record_family_rejects_a_resigned_unsupported_version() {
    let authority = PeerIdentity::from_secret_bytes([7; 32]);
    let voter = PeerIdentity::from_secret_bytes([8; 32]);
    let (genesis, world) = supported_genesis(&authority);

    let entries = vec![SnapshotEntry {
        path: "level.dat".into(),
        blob: BlobDescriptor {
            hash: Hash32([1; 32]),
            uncompressed_size: 1,
            encoded_size: 1,
            encoding: BlobEncoding::Raw,
        },
    }];
    let mut snapshot = SnapshotManifestV1 {
        protocol_version: unsupported(),
        world_id: world,
        snapshot_number: 1,
        epoch: 0,
        sequence: 1,
        previous_snapshot_hash: None,
        state_root: snapshot_state_root(&entries).unwrap(),
        entries,
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    snapshot.signature = authority.sign(&snapshot.signing_bytes().unwrap());
    assert!(verify_snapshot_signature(&snapshot).is_err());

    let mut membership = MembershipRecordV1 {
        protocol_version: unsupported(),
        world_id: world,
        epoch: 0,
        sequence: 0,
        previous_membership_hash: None,
        members: vec![member(&authority)],
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    membership.signature = authority.sign(&membership.signing_bytes().unwrap());
    assert!(verify_membership_signature(&membership).is_err());

    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: "1.21.8".into(),
        loader_id: "fabric".into(),
        loader_version: "0.17.2".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: "0.2.0".into(),
        required_server_mods: vec![swarm_protocol::ArtifactRequirementV1 {
            artifact_id: "example".into(),
            version: "1.0.0".into(),
            artifact_hash: Hash32([2; 32]),
            side: ArtifactSideV1::Server,
            provider_hint: Some("modrinth:example".into()),
        }],
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let mut config = WorldConfigV1 {
        protocol_version: unsupported(),
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 2 },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: "test".into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    config.signature = authority.sign(&config.signing_bytes().unwrap());
    assert!(verify_world_config_signature(&config).is_err());

    let mut epoch = EpochRecordV1 {
        protocol_version: unsupported(),
        world_id: world,
        epoch_number: 1,
        previous_epoch_hash: None,
        base_state_hash: Hash32([3; 32]),
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: 1,
        reason: "unsupported-version probe".into(),
        signature: Vec::new(),
    };
    epoch.signature = authority.sign(&epoch.signing_bytes().unwrap());
    assert!(epoch.validate_semantics().is_err());

    let mut transfer = AuthorityTransferV1 {
        protocol_version: unsupported(),
        world_id: world,
        from_peer_id: authority.peer_id(),
        to_peer_id: voter.peer_id(),
        base_snapshot_hash: Hash32([3; 32]),
        next_epoch: 1,
        next_fencing_token: 1,
        phase: TransferPhase::Prepared,
        signer_peer_id: authority.peer_id(),
        signer_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    transfer.signature = authority.sign(&transfer.signing_bytes().unwrap());
    assert!(verify_transfer_signature(&transfer).is_err());

    let mut lease = AuthorityLeaseGrantV1 {
        protocol_version: unsupported(),
        world_id: world,
        epoch: 1,
        fencing_token: 1,
        lease_duration_ms: 5_000,
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        nonce: [6; 32],
        signature: Vec::new(),
    };
    lease.signature = authority.sign(&lease.signing_bytes().unwrap());
    assert!(verify_lease_signature(&lease).is_err());

    let mut sleep = SleepRecordV1 {
        protocol_version: unsupported(),
        world_id: world,
        latest_snapshot_hash: Hash32([3; 32]),
        epoch: 1,
        fencing_token: 1,
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    sleep.signature = authority.sign(&sleep.signing_bytes().unwrap());
    assert!(verify_sleep_record_signature(&sleep).is_err());

    let mut ballot = RecoveryBallotV1 {
        protocol_version: unsupported(),
        world_id: world,
        base_epoch: 1,
        base_fencing_token: 1,
        target_epoch: 2,
        target_fencing_token: 2,
        round: 1,
        candidate_peer_id: authority.peer_id(),
        candidate_public_key: authority.public_key(),
        base_snapshot_hash: Hash32([3; 32]),
        base_state_hash: Hash32([4; 32]),
        membership_hash: Hash32([5; 32]),
        signature: Vec::new(),
    };
    ballot.signature = authority.sign(&ballot.signing_bytes().unwrap());
    assert!(verify_recovery_ballot_signature(&ballot).is_err());

    let mut vote = RecoveryVoteV1 {
        protocol_version: unsupported(),
        world_id: world,
        ballot_hash: ballot.ballot_hash().unwrap(),
        base_epoch: ballot.base_epoch,
        target_epoch: ballot.target_epoch,
        round: ballot.round,
        candidate_peer_id: authority.peer_id(),
        voter_peer_id: voter.peer_id(),
        voter_public_key: voter.public_key(),
        signature: Vec::new(),
    };
    vote.signature = voter.sign(&vote.signing_bytes().unwrap());
    assert!(verify_recovery_vote_signature(&vote).is_err());

    let mut solo = SoloBranchV1 {
        protocol_version: unsupported(),
        world_id: world,
        base_snapshot_hash: Hash32([3; 32]),
        base_epoch: 1,
        head_snapshot_hash: Hash32([4; 32]),
        head_epoch: 1,
        head_sequence: 2,
        state_hash: Hash32([5; 32]),
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    solo.signature = authority.sign(&solo.signing_bytes().unwrap());
    assert!(verify_solo_branch_signature(&solo).is_err());

    let mut invite = InviteV1 {
        protocol_version: unsupported(),
        world_id: world,
        display_name: "test".into(),
        genesis: genesis.clone(),
        inviter_peer_id: authority.peer_id(),
        inviter_public_key: authority.public_key(),
        bootstrap_addrs: Vec::new(),
        expires_unix_ms: u64::MAX,
        nonce: [9; 32],
        signature: Vec::new(),
    };
    invite.signature = authority.sign(&invite.signing_bytes().unwrap());
    assert!(verify_invite_signature(&invite).is_err());

    let mut join = JoinRequestV1 {
        protocol_version: unsupported(),
        world_id: world,
        invite: InviteV1 { protocol_version: PROTOCOL_VERSION, ..invite.clone() },
        joining_member: member(&voter),
        nonce: [10; 32],
        signature: Vec::new(),
    };
    join.signature = voter.sign(&join.signing_bytes().unwrap());
    assert!(verify_join_request_signature(&join).is_err());

    let mut leave = LeaveRequestV1 {
        protocol_version: unsupported(),
        world_id: world,
        membership_hash: Hash32([6; 32]),
        leaving_peer_id: voter.peer_id(),
        leaving_public_key: voter.public_key(),
        nonce: [11; 32],
        signature: Vec::new(),
    };
    leave.signature = voter.sign(&leave.signing_bytes().unwrap());
    assert!(verify_leave_request_signature(&leave).is_err());

    let mut membership_vote = MembershipVoteV1 {
        protocol_version: unsupported(),
        world_id: world,
        previous_membership_hash: Hash32([1; 32]),
        proposed_membership_hash: Hash32([2; 32]),
        proposed_sequence: 1,
        voter_peer_id: voter.peer_id(),
        voter_public_key: voter.public_key(),
        signature: Vec::new(),
    };
    membership_vote.signature = voter.sign(&membership_vote.signing_bytes().unwrap());
    assert!(membership_vote.validate_semantics().is_err());
}
