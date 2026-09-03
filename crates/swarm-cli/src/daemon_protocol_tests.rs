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
