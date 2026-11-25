use super::*;
use swarm_core::{sign_world_config, PeerIdentity};
use swarm_protocol::{
    AuthorityPolicyV1, EpochMode, EpochRecordV1, Hash32, MembershipPolicyV1, MembershipRecordV1,
    RuntimeCompatibilityManifestV1, WorldConfigV1, WorldGenesisV1, WorldMemberV1, WorldPresentationV1,
    WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, WorldMetadataV1};

struct Fixture {
    _temp: tempfile::TempDir,
    storage: Storage,
    authority: PeerIdentity,
    member: PeerIdentity,
    world: WorldId,
    compatibility: RuntimeCompatibilityManifestV1,
}

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path()).unwrap();
    let authority = PeerIdentity::from_secret_bytes([41; 32]);
    let member_identity = PeerIdentity::from_secret_bytes([42; 32]);
    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: "1.21.8".into(),
        loader_id: "fabric".into(),
        loader_version: "0.17.2".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: "0.5.0".into(),
        required_server_mods: Vec::new(),
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let mut initial_membership = vec![authority.peer_id(), member_identity.peer_id()];
    initial_membership.sort();
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "1.21.8".into(),
        fabric_loader_version: "0.17.2".into(),
        compatibility_fingerprint: compatibility.fingerprint().unwrap(),
        creation_nonce: [43; 32],
        creator_public_key: authority.public_key(),
        initial_membership,
    };
    let world = genesis.world_id().unwrap();
    storage
        .create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "daemon acceptance".into(),
            world_id: world,
            genesis,
        })
        .unwrap();

    let mut members = vec![member(&authority), member(&member_identity)];
    members.sort_by_key(|value| value.peer_id);
    storage
        .save_world_descriptor(&WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: compatibility.fingerprint().unwrap(),
            members: members.clone(),
            preferred_replication_factor: 2,
        })
        .unwrap();
    let mut membership = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch: 0,
        sequence: 0,
        previous_membership_hash: None,
        members,
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    authority.sign_membership(&mut membership).unwrap();
    storage.save_membership_record(&membership).unwrap();
    let mut initial_epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 0,
        previous_epoch_hash: None,
        base_state_hash: Hash32([44; 32]),
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: 0,
        reason: "fixture".into(),
        signature: Vec::new(),
    };
    initial_epoch.signature = authority.sign(&initial_epoch.signing_bytes().unwrap());
    storage.save_epoch_record(&initial_epoch).unwrap();

    Fixture { _temp: temp, storage, authority, member: member_identity, world, compatibility }
}

fn config(fixture: &Fixture, signer: &PeerIdentity) -> WorldConfigV1 {
    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        sequence: 1,
        previous_config_hash: None,
        compatibility: fixture.compatibility.clone(),
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 2 },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: "daemon acceptance".into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: signer.peer_id(),
        authority_public_key: signer.public_key(),
        signature: Vec::new(),
    };
    sign_world_config(signer, &mut config).unwrap();
    config
}

fn signed_join(
    fixture: &Fixture,
    expires_unix_ms: u64,
) -> (swarm_protocol::JoinRequestV1, MembershipProposalV1, MembershipVoteV1) {
    let joining = PeerIdentity::from_secret_bytes([69; 32]);
    let mut invite = swarm_protocol::InviteV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        display_name: "test".into(),
        genesis: fixture.storage.load_world(fixture.world).unwrap().genesis,
        inviter_peer_id: fixture.authority.peer_id(),
        inviter_public_key: fixture.authority.public_key(),
        bootstrap_addrs: Vec::new(),
        expires_unix_ms,
        nonce: [70; 32],
        signature: Vec::new(),
    };
    fixture.authority.sign_invite(&mut invite).unwrap();
    let mut request = swarm_protocol::JoinRequestV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        invite,
        joining_member: member(&joining),
        nonce: [71; 32],
        signature: Vec::new(),
    };
    joining.sign_join_request(&mut request).unwrap();
    let previous = fixture.storage.load_membership_record(fixture.world).unwrap();
    let mut proposed = previous.clone();
    proposed.sequence += 1;
    proposed.previous_membership_hash = Some(previous.record_hash().unwrap());
    proposed.members.push(request.joining_member.clone());
    proposed.members.sort_by_key(|member| member.peer_id);
    fixture.authority.sign_membership(&mut proposed).unwrap();
    let proposal = MembershipProposalV1 { previous, proposed };
    let vote = sign_membership_vote(&fixture.authority, &proposal).unwrap();
    (request, proposal, vote)
}

#[test]
fn revoked_invitation_denies_fresh_prepare_after_restart() {
    let fixture = fixture();
    let (request, proposal, vote) = signed_join(&fixture, unix_millis().unwrap() + 60_000);
    assert!(fixture.storage.revoke_invite(&request.invite).unwrap());
    assert!(!fixture.storage.revoke_invite(&request.invite).unwrap());
    let reopened = Storage::open(fixture._temp.path()).unwrap();
    assert!(reopened.is_invite_revoked(&request.invite).unwrap());
    assert!(matches!(
        reopened.promise_join_membership_proposal(&proposal, &vote, &request),
        Err(StorageError::InviteRevoked)
    ));
    assert!(load_optional_membership_promise(&reopened, fixture.world).unwrap().is_none());
    assert_eq!(reopened.load_membership_record(fixture.world).unwrap(), proposal.previous);
}

#[test]
fn expired_invitation_cannot_prepare_and_revocation_cannot_erase_a_promise() {
    let fixture = fixture();
    let (expired, proposal, vote) = signed_join(&fixture, unix_millis().unwrap() - 1);
    assert!(matches!(
        fixture.storage.promise_join_membership_proposal(&proposal, &vote, &expired),
        Err(StorageError::InviteExpired)
    ));
    let (valid, proposal, vote) = signed_join(&fixture, unix_millis().unwrap() + 60_000);
    assert_eq!(
        fixture.storage.promise_join_membership_proposal(&proposal, &vote, &valid).unwrap(),
        MembershipPromiseResult::Accepted
    );
    assert!(matches!(fixture.storage.revoke_invite(&valid.invite), Err(StorageError::InviteRevocationPrepared)));
    let reopened = Storage::open(fixture._temp.path()).unwrap();
    assert!(!reopened.is_invite_revoked(&valid.invite).unwrap());
    assert_eq!(reopened.load_membership_promise(fixture.world).unwrap().proposal, proposal);
}

#[test]
fn corrupt_membership_prepare_blocks_authority_and_recovery_transitions() {
    let fixture = fixture();
    assert!(ensure_no_membership_prepare(&fixture.storage, fixture.world).is_ok());

    let path = fixture.storage.world_dir(fixture.world).join("metadata/membership-promise.postcard");
    let corrupted = b"invalid membership prepare";
    std::fs::write(&path, corrupted).unwrap();

    assert!(ensure_no_membership_prepare(&fixture.storage, fixture.world).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), corrupted);
}

#[test]
fn daemon_rejects_valid_non_authority_world_config() {
    let fixture = fixture();
    let malicious = config(&fixture, &fixture.member);
    verify_world_config_signature(&malicious).unwrap();
    assert!(authorize_world_config_current_authority(&fixture.storage, fixture.member.peer_id(), &malicious).is_err());

    let legitimate = config(&fixture, &fixture.authority);
    assert!(
        authorize_world_config_current_authority(&fixture.storage, fixture.authority.peer_id(), &legitimate).is_ok()
    );
}

#[test]
fn daemon_rejects_wrong_parent_and_same_sequence_snapshot_conflicts() {
    let fixture = fixture();
    let source = fixture._temp.path().join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("level.dat"), b"one").unwrap();
    let mut current = fixture
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world: fixture.world,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: fixture.authority.peer_id(),
                authority_public_key: fixture.authority.public_key(),
            },
        )
        .unwrap();
    fixture.authority.sign_snapshot(&mut current).unwrap();
    fixture.storage.commit_snapshot(&current).unwrap();
    assert!(authorize_manifest(&fixture.storage, fixture.authority.peer_id(), &current).is_ok());

    let mut wrong_parent = current.clone();
    wrong_parent.snapshot_number = 2;
    wrong_parent.sequence = 2;
    wrong_parent.previous_snapshot_hash = Some(Hash32([99; 32]));
    wrong_parent.signature.clear();
    fixture.authority.sign_snapshot(&mut wrong_parent).unwrap();
    assert!(authorize_manifest(&fixture.storage, fixture.authority.peer_id(), &wrong_parent).is_err());

    let mut same_sequence_conflict = current.clone();
    same_sequence_conflict.snapshot_number = 2;
    same_sequence_conflict.previous_snapshot_hash = Some(current.manifest_hash().unwrap());
    same_sequence_conflict.signature.clear();
    fixture.authority.sign_snapshot(&mut same_sequence_conflict).unwrap();
    assert!(authorize_manifest(&fixture.storage, fixture.authority.peer_id(), &same_sequence_conflict).is_err());
}

#[test]
fn recovery_candidate_requires_runtime_mod_and_conflict_readiness() {
    let fixture = fixture();
    let fingerprint = fixture.compatibility.fingerprint().unwrap();
    let mut capability = HostCapabilityV1 {
        world_id: fixture.world,
        compatibility_fingerprint: fingerprint,
        runtime: HostRuntimeReadinessV1::Ready,
        server_mods: ServerModsReadinessV1::Ready,
        conflict_free: true,
        recovery_quorum_without_authority: true,
    };
    assert!(host_capability_ready(&capability, fixture.world, fingerprint));

    capability.runtime = HostRuntimeReadinessV1::Unverified;
    assert!(!host_capability_ready(&capability, fixture.world, fingerprint));
    capability.runtime = HostRuntimeReadinessV1::Ready;
    capability.server_mods = ServerModsReadinessV1::Missing;
    assert!(!host_capability_ready(&capability, fixture.world, fingerprint));
    capability.server_mods = ServerModsReadinessV1::Ready;
    capability.conflict_free = false;
    assert!(!host_capability_ready(&capability, fixture.world, fingerprint));
    capability.conflict_free = true;
    capability.compatibility_fingerprint = Hash32([91; 32]);
    assert!(!host_capability_ready(&capability, fixture.world, fingerprint));
}

#[test]
fn quorum_wake_is_bound_to_the_signed_sleep_generation_and_exact_snapshot() {
    let fixture = fixture();
    let source = fixture._temp.path().join("sleep-source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("level.dat"), b"sleep-boundary").unwrap();
    let mut latest = fixture
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world: fixture.world,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: fixture.authority.peer_id(),
                authority_public_key: fixture.authority.public_key(),
            },
        )
        .unwrap();
    fixture.authority.sign_snapshot(&mut latest).unwrap();
    fixture.storage.commit_snapshot(&latest).unwrap();
    let epoch = fixture.storage.load_epoch_record(fixture.world).unwrap();
    let mut sleep = SleepRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        latest_snapshot_hash: latest.manifest_hash().unwrap(),
        epoch: epoch.epoch_number,
        fencing_token: epoch.fencing_token,
        authority_peer_id: fixture.authority.peer_id(),
        authority_public_key: fixture.authority.public_key(),
        signature: Vec::new(),
    };
    fixture.authority.sign_sleep_record(&mut sleep).unwrap();

    assert!(validate_sleep_recovery_base(&sleep, &epoch, &latest).is_ok());

    let mut stale_generation = sleep.clone();
    stale_generation.fencing_token = stale_generation.fencing_token.saturating_add(1);
    fixture.authority.sign_sleep_record(&mut stale_generation).unwrap();
    assert!(validate_sleep_recovery_base(&stale_generation, &epoch, &latest).is_err());

    let mut wrong_snapshot = sleep;
    wrong_snapshot.latest_snapshot_hash = Hash32([92; 32]);
    fixture.authority.sign_sleep_record(&mut wrong_snapshot).unwrap();
    assert!(validate_sleep_recovery_base(&wrong_snapshot, &epoch, &latest).is_err());
}

fn signed_recovery_ballot_and_vote(fixture: &Fixture) -> (RecoveryBallotV1, RecoveryVoteV1) {
    let membership = fixture.storage.load_membership_record(fixture.world).unwrap();
    let mut ballot = RecoveryBallotV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        base_epoch: 0,
        base_fencing_token: 0,
        target_epoch: 1,
        target_fencing_token: 1,
        round: 7,
        candidate_peer_id: fixture.authority.peer_id(),
        candidate_public_key: fixture.authority.public_key(),
        base_snapshot_hash: Hash32([67; 32]),
        base_state_hash: Hash32([44; 32]),
        membership_hash: membership.record_hash().unwrap(),
        signature: Vec::new(),
    };
    sign_recovery_ballot(&fixture.authority, &mut ballot).unwrap();
    let mut vote = RecoveryVoteV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        ballot_hash: ballot.ballot_hash().unwrap(),
        base_epoch: ballot.base_epoch,
        target_epoch: ballot.target_epoch,
        round: ballot.round,
        candidate_peer_id: ballot.candidate_peer_id,
        voter_peer_id: fixture.authority.peer_id(),
        voter_public_key: fixture.authority.public_key(),
        signature: Vec::new(),
    };
    sign_recovery_vote(&fixture.authority, &mut vote).unwrap();
    (ballot, vote)
}

#[test]
fn accepted_recovery_retry_consumes_only_the_certified_predecessor_sleep() {
    let fixture = fixture();
    let storage = &fixture.storage;
    let world = fixture.world;
    let source = fixture._temp.path().join("retry-boundary");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("level.dat"), b"before recovery").unwrap();
    let mut base = storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: fixture.authority.peer_id(),
                authority_public_key: fixture.authority.public_key(),
            },
        )
        .unwrap();
    fixture.authority.sign_snapshot(&mut base).unwrap();
    storage.commit_snapshot(&base).unwrap();
    let prior = storage.load_epoch_record(world).unwrap();
    let mut sleep = SleepRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        latest_snapshot_hash: base.manifest_hash().unwrap(),
        epoch: 0,
        fencing_token: 0,
        authority_peer_id: prior.authority_peer_id,
        authority_public_key: prior.authority_public_key,
        signature: Vec::new(),
    };
    fixture.authority.sign_sleep_record(&mut sleep).unwrap();
    storage.save_sleep_record(&sleep).unwrap();
    let (mut ballot, mut vote) = signed_recovery_ballot_and_vote(&fixture);
    ballot.base_snapshot_hash = base.manifest_hash().unwrap();
    ballot.base_state_hash = base.state_root;
    sign_recovery_ballot(&fixture.authority, &mut ballot).unwrap();
    vote.ballot_hash = ballot.ballot_hash().unwrap();
    sign_recovery_vote(&fixture.authority, &mut vote).unwrap();
    let mut second_vote = vote.clone();
    second_vote.voter_peer_id = fixture.member.peer_id();
    second_vote.voter_public_key = fixture.member.public_key();
    sign_recovery_vote(&fixture.member, &mut second_vote).unwrap();
    let certificate = RecoveryCertificateV1 { ballot, votes: vec![vote, second_vote] };
    let mut epoch = EpochRecordV1 {
        epoch_number: 1,
        fencing_token: 1,
        previous_epoch_hash: Some(epoch_record_hash(&prior).unwrap()),
        mode: EpochMode::Recovery,
        base_state_hash: base.state_root,
        reason: "certified retry".into(),
        signature: Vec::new(),
        ..prior
    };
    epoch.signature = fixture.authority.sign(&epoch.signing_bytes().unwrap());
    storage.save_recovery_certificate(&certificate).unwrap();
    storage.save_epoch_record(&epoch).unwrap();
    let mut membership = storage.load_membership_record(world).unwrap();
    membership.previous_membership_hash = Some(membership.record_hash().unwrap());
    membership.epoch = 1;
    membership.sequence += 1;
    fixture.authority.sign_membership(&mut membership).unwrap();
    storage.save_membership_record(&membership).unwrap();
    std::fs::write(source.join("level.dat"), b"after recovery").unwrap();
    let mut head = storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world,
                snapshot_number: 2,
                epoch: 1,
                sequence: 2,
                previous_snapshot_hash: Some(base.manifest_hash().unwrap()),
                authority_peer_id: fixture.authority.peer_id(),
                authority_public_key: fixture.authority.public_key(),
            },
        )
        .unwrap();
    fixture.authority.sign_snapshot(&mut head).unwrap();
    storage.commit_snapshot(&head).unwrap();

    validate_accepted_recovery_replay(storage, fixture.authority.peer_id(), &epoch, &certificate).unwrap();
    assert!(validate_accepted_recovery_replay(storage, fixture.member.peer_id(), &epoch, &certificate).is_err());
    let mut wrong = certificate.clone();
    wrong.votes.pop();
    assert!(validate_accepted_recovery_replay(storage, fixture.authority.peer_id(), &epoch, &wrong).is_err());
    assert!(finish_recovery_retry_sleep_cleanup(storage, &epoch, &certificate).unwrap());
    assert!(launch_guard::load_sleep_record_fail_closed(storage, world).unwrap().is_none());
    sleep.epoch = 1;
    sleep.fencing_token = 1;
    sleep.latest_snapshot_hash = head.manifest_hash().unwrap();
    fixture.authority.sign_sleep_record(&mut sleep).unwrap();
    storage.save_sleep_record(&sleep).unwrap();
    assert!(!finish_recovery_retry_sleep_cleanup(storage, &epoch, &certificate).unwrap());
    assert_eq!(storage.load_sleep_record(world).unwrap(), sleep);
}

#[test]
fn cached_recovery_ballot_must_match_readable_durable_promise_before_certification() {
    let fixture = fixture();
    let (ballot, vote) = signed_recovery_ballot_and_vote(&fixture);
    assert!(matches!(
        fixture.storage.promise_recovery_ballot(&ballot, &vote).unwrap(),
        RecoveryPromiseResult::Accepted
    ));
    assert!(ensure_active_recovery_promise(&fixture.storage, &ballot).is_ok());
    assert!(ensure_local_recovery_promise_allows_certificate(&fixture.storage, &ballot).is_ok());

    let mut conflicting = ballot.clone();
    conflicting.base_snapshot_hash = Hash32([92; 32]);
    conflicting.signature.clear();
    sign_recovery_ballot(&fixture.authority, &mut conflicting).unwrap();
    assert!(ensure_local_recovery_promise_allows_certificate(&fixture.storage, &conflicting).is_err());

    let promise_path = fixture.storage.world_dir(fixture.world).join("metadata/recovery-promise.postcard");
    let corrupt = b"invalid durable recovery vote after quorum was in memory";
    std::fs::write(&promise_path, corrupt).unwrap();
    assert!(load_optional_recovery_promise(&fixture.storage, fixture.world).is_err());
    assert!(ensure_active_recovery_promise(&fixture.storage, &ballot).is_err());
    assert!(ensure_local_recovery_promise_allows_certificate(&fixture.storage, &ballot).is_err());
    assert_eq!(std::fs::read(&promise_path).unwrap(), corrupt);
    assert!(fixture.storage.load_recovery_certificate(fixture.world).is_err());
    assert_eq!(fixture.storage.load_epoch_record(fixture.world).unwrap().epoch_number, 0);
}

#[cfg(unix)]
#[test]
fn dangling_durable_recovery_promise_rejects_inbound_certificate_and_cached_ballot() {
    use std::os::unix::fs::symlink;

    let fixture = fixture();
    let (ballot, vote) = signed_recovery_ballot_and_vote(&fixture);
    fixture.storage.promise_recovery_ballot(&ballot, &vote).unwrap();
    let promise_path = fixture.storage.world_dir(fixture.world).join("metadata/recovery-promise.postcard");
    std::fs::remove_file(&promise_path).unwrap();
    symlink(fixture._temp.path().join("missing-vote"), &promise_path).unwrap();

    assert!(ensure_active_recovery_promise(&fixture.storage, &ballot).is_err());
    assert!(ensure_local_recovery_promise_allows_certificate(&fixture.storage, &ballot).is_err());
    assert!(std::fs::symlink_metadata(&promise_path).unwrap().file_type().is_symlink());
    assert_eq!(fixture.storage.load_epoch_record(fixture.world).unwrap().epoch_number, 0);
}

#[test]
fn corrupt_accepted_epoch_cannot_be_treated_as_missing_before_recovery_control_writes() {
    let fixture = fixture();
    assert_eq!(load_optional_epoch_record(&fixture.storage, fixture.world).unwrap().unwrap().epoch_number, 0);

    let epoch_path = fixture.storage.world_dir(fixture.world).join("metadata/epoch.postcard");
    let corrupt = b"invalid accepted epoch";
    std::fs::write(&epoch_path, corrupt).unwrap();
    assert!(load_optional_epoch_record(&fixture.storage, fixture.world).is_err());
    assert_eq!(std::fs::read(&epoch_path).unwrap(), corrupt);
    assert!(fixture.storage.load_recovery_certificate(fixture.world).is_err());

    std::fs::remove_file(&epoch_path).unwrap();
    assert!(load_optional_epoch_record(&fixture.storage, fixture.world).unwrap().is_none());
}

#[test]
fn corrupt_epoch_blocks_config_snapshot_and_transfer_preflight() {
    let fixture = fixture();
    let source = fixture._temp.path().join("epoch-corruption-snapshots");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("level.dat"), b"accepted-state").unwrap();
    let mut first = fixture
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world: fixture.world,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: fixture.authority.peer_id(),
                authority_public_key: fixture.authority.public_key(),
            },
        )
        .unwrap();
    fixture.authority.sign_snapshot(&mut first).unwrap();
    fixture.storage.commit_snapshot(&first).unwrap();

    let mut transfer = swarm_protocol::AuthorityTransferV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        from_peer_id: fixture.authority.peer_id(),
        to_peer_id: fixture.member.peer_id(),
        base_snapshot_hash: first.manifest_hash().unwrap(),
        next_epoch: 1,
        next_fencing_token: 1,
        phase: TransferPhase::Prepared,
        signer_peer_id: fixture.authority.peer_id(),
        signer_public_key: fixture.authority.public_key(),
        signature: Vec::new(),
    };
    fixture.authority.sign_transfer(&mut transfer).unwrap();

    std::fs::write(source.join("level.dat"), b"next-replicated-state").unwrap();
    let mut second = fixture
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world: fixture.world,
                snapshot_number: 2,
                epoch: 0,
                sequence: 2,
                previous_snapshot_hash: Some(first.manifest_hash().unwrap()),
                authority_peer_id: fixture.authority.peer_id(),
                authority_public_key: fixture.authority.public_key(),
            },
        )
        .unwrap();
    fixture.authority.sign_snapshot(&mut second).unwrap();

    let legitimate = config(&fixture, &fixture.authority);
    assert!(
        authorize_world_config_current_authority(&fixture.storage, fixture.authority.peer_id(), &legitimate).is_ok()
    );
    assert!(authorize_manifest(&fixture.storage, fixture.authority.peer_id(), &second).is_ok());
    assert!(validate_transfer(&fixture.storage, &transfer).is_ok());
    let descriptor = fixture.storage.load_world_descriptor(fixture.world).unwrap();
    assert!(validate_persisted_transfer(&fixture.storage, &descriptor, &transfer).unwrap());

    let epoch_path = fixture.storage.world_dir(fixture.world).join("metadata/epoch.postcard");
    let valid_epoch = fixture.storage.load_epoch_record(fixture.world).unwrap();
    let corrupt = b"corrupt accepted authority epoch";
    std::fs::write(&epoch_path, corrupt).unwrap();

    assert!(
        authorize_world_config_current_authority(&fixture.storage, fixture.authority.peer_id(), &legitimate).is_err()
    );
    assert!(authorize_manifest(&fixture.storage, fixture.authority.peer_id(), &second).is_err());
    assert!(validate_transfer(&fixture.storage, &transfer).is_err());
    assert_eq!(std::fs::read(&epoch_path).unwrap(), corrupt);
    assert!(fixture.storage.load_transfer_record(fixture.world).is_err());

    // Well-formed but unsigned/altered records are just as unsafe as
    // unreadable bytes. Never infer a valid authority from storage semantics.
    let mut unsigned_epoch = valid_epoch;
    unsigned_epoch.signature[0] ^= 1;
    std::fs::write(&epoch_path, postcard::to_allocvec(&unsigned_epoch).unwrap()).unwrap();
    assert!(fixture.storage.load_epoch_record(fixture.world).is_ok());
    assert!(validate_transfer(&fixture.storage, &transfer).is_err());
    assert!(validate_persisted_transfer(&fixture.storage, &descriptor, &transfer).is_err());

    // Missing is not an implicit signed bootstrap authority for handoff.
    std::fs::remove_file(&epoch_path).unwrap();
    assert!(validate_transfer(&fixture.storage, &transfer).is_err());
    assert!(validate_persisted_transfer(&fixture.storage, &descriptor, &transfer).is_err());
}

#[test]
fn corrupt_previous_transfer_cannot_be_replaced_by_new_prepared_transfer() {
    let fixture = fixture();
    let source = fixture._temp.path().join("transfer-corruption-source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("level.dat"), b"transfer-canonical-base").unwrap();
    let mut first = fixture
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world: fixture.world,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: fixture.authority.peer_id(),
                authority_public_key: fixture.authority.public_key(),
            },
        )
        .unwrap();
    fixture.authority.sign_snapshot(&mut first).unwrap();
    fixture.storage.commit_snapshot(&first).unwrap();

    let mut transfer = swarm_protocol::AuthorityTransferV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        from_peer_id: fixture.authority.peer_id(),
        to_peer_id: fixture.member.peer_id(),
        base_snapshot_hash: first.manifest_hash().unwrap(),
        next_epoch: 1,
        next_fencing_token: 1,
        phase: TransferPhase::Prepared,
        signer_peer_id: fixture.authority.peer_id(),
        signer_public_key: fixture.authority.public_key(),
        signature: Vec::new(),
    };
    fixture.authority.sign_transfer(&mut transfer).unwrap();
    fixture.storage.save_transfer_record(&transfer).unwrap();

    let transfer_path = fixture.storage.world_dir(fixture.world).join("metadata/transfer.postcard");
    let corrupt = b"corrupt previously prepared authority transfer";
    std::fs::write(&transfer_path, corrupt).unwrap();
    assert!(validate_transfer(&fixture.storage, &transfer).is_err());
    assert_eq!(std::fs::read(&transfer_path).unwrap(), corrupt);
}

#[test]
fn inbound_recovery_lease_and_sleep_are_fenced_by_durable_signed_transfer() {
    let fixture = fixture();
    let mut current = fixture.storage.load_epoch_record(fixture.world).unwrap();
    let original_hash = epoch_record_hash(&current).unwrap();
    current.epoch_number = 1;
    current.fencing_token = 1;
    current.previous_epoch_hash = Some(original_hash);
    current.signature = fixture.authority.sign(&current.signing_bytes().unwrap());
    fixture.storage.save_epoch_record(&current).unwrap();

    let source = fixture._temp.path().join("inbound-transfer-fence-source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("level.dat"), b"accepted-transfer-snapshot").unwrap();
    let mut latest = fixture
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world: fixture.world,
                snapshot_number: 1,
                epoch: current.epoch_number,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: fixture.authority.peer_id(),
                authority_public_key: fixture.authority.public_key(),
            },
        )
        .unwrap();
    fixture.authority.sign_snapshot(&mut latest).unwrap();
    fixture.storage.commit_snapshot(&latest).unwrap();
    let checkpoint_hash = latest.manifest_hash().unwrap();

    assert!(ensure_no_competing_authority_during_transfer(&fixture.storage, fixture.world, None).is_ok());

    let mut transfer = AuthorityTransferV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        from_peer_id: fixture.authority.peer_id(),
        to_peer_id: fixture.member.peer_id(),
        base_snapshot_hash: checkpoint_hash,
        next_epoch: current.epoch_number + 1,
        next_fencing_token: current.fencing_token + 1,
        phase: TransferPhase::Prepared,
        signer_peer_id: fixture.authority.peer_id(),
        signer_public_key: fixture.authority.public_key(),
        signature: Vec::new(),
    };
    fixture.authority.sign_transfer(&mut transfer).unwrap();
    fixture.storage.save_transfer_record(&transfer).unwrap();
    let mut successor = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: fixture.world,
        epoch_number: transfer.next_epoch,
        previous_epoch_hash: Some(epoch_record_hash(&current).unwrap()),
        base_state_hash: latest.state_root,
        authority_peer_id: fixture.member.peer_id(),
        authority_public_key: fixture.member.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: transfer.next_fencing_token,
        reason: "signed transfer acceptance".into(),
        signature: Vec::new(),
    };
    successor.signature = fixture.member.sign(&successor.signing_bytes().unwrap());
    for phase in [TransferPhase::Prepared, TransferPhase::Accepted, TransferPhase::Committed] {
        transfer.phase = phase;
        let signer = if phase == TransferPhase::Accepted { &fixture.member } else { &fixture.authority };
        transfer.signer_peer_id = signer.peer_id();
        transfer.signer_public_key = signer.public_key();
        signer.sign_transfer(&mut transfer).unwrap();
        fixture.storage.save_transfer_record(&transfer).unwrap();
        assert!(ensure_no_competing_authority_during_transfer(&fixture.storage, fixture.world, None).is_err());
        let inbound_epoch =
            ensure_no_competing_authority_during_transfer(&fixture.storage, fixture.world, Some(&successor));
        assert_eq!(inbound_epoch.is_ok(), phase == TransferPhase::Committed);
        let mut competing = successor.clone();
        competing.authority_peer_id = fixture.authority.peer_id();
        competing.authority_public_key = fixture.authority.public_key();
        assert!(
            ensure_no_competing_authority_during_transfer(&fixture.storage, fixture.world, Some(&competing),).is_err()
        );
    }
    fixture.storage.save_epoch_record(&successor).unwrap();
    // Signed successor epoch now releases the transfer fence.
    assert!(ensure_no_competing_authority_during_transfer(&fixture.storage, fixture.world, None).is_ok());
}

#[test]
fn tampered_durable_recovery_signatures_block_cached_and_inbound_authority() {
    let fixture = fixture();
    let (ballot, vote) = signed_recovery_ballot_and_vote(&fixture);
    fixture.storage.promise_recovery_ballot(&ballot, &vote).unwrap();
    let original = fixture.storage.load_recovery_promise(fixture.world).unwrap();
    let path = fixture.storage.world_dir(fixture.world).join("metadata/recovery-promise.postcard");

    let mut broken_vote = original.clone();
    broken_vote.vote.signature[0] ^= 0x55;
    std::fs::write(&path, postcard::to_allocvec(&broken_vote).unwrap()).unwrap();
    assert!(ensure_active_recovery_promise(&fixture.storage, &ballot).is_err());
    assert!(ensure_local_recovery_promise_allows_certificate(&fixture.storage, &ballot).is_err());

    let mut broken_ballot = original;
    broken_ballot.ballot.signature[0] ^= 0x55;
    std::fs::write(&path, postcard::to_allocvec(&broken_ballot).unwrap()).unwrap();
    assert!(ensure_active_recovery_promise(&fixture.storage, &ballot).is_err());
    assert!(ensure_local_recovery_promise_allows_certificate(&fixture.storage, &ballot).is_err());
    assert_eq!(fixture.storage.load_epoch_record(fixture.world).unwrap().epoch_number, 0);
    assert!(fixture.storage.load_recovery_certificate(fixture.world).is_err());
}
