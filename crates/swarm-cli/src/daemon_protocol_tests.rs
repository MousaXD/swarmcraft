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
    storage
        .save_epoch_record(&EpochRecordV1 {
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
        })
        .unwrap();

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

    let epoch_path = fixture.storage.world_dir(fixture.world).join("metadata/epoch.postcard");
    let corrupt = b"corrupt accepted authority epoch";
    std::fs::write(&epoch_path, corrupt).unwrap();

    assert!(
        authorize_world_config_current_authority(&fixture.storage, fixture.authority.peer_id(), &legitimate).is_err()
    );
    assert!(authorize_manifest(&fixture.storage, fixture.authority.peer_id(), &second).is_err());
    assert!(validate_transfer(&fixture.storage, &transfer).is_err());
    assert_eq!(std::fs::read(&epoch_path).unwrap(), corrupt);
    assert!(fixture.storage.load_transfer_record(fixture.world).is_err());
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
