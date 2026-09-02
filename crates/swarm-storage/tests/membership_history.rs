use swarm_protocol::{
    Hash32, MembershipRecordV1, PeerId, WorldDescriptorV1, WorldGenesisV1, WorldMemberV1, PROTOCOL_VERSION,
    STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{Storage, WorldMetadataV1};

fn member(peer: u8) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: PeerId([peer; 32]),
        public_key: [peer; 32],
        authority_eligible: true,
        banned: false,
    }
}

fn fixture() -> (tempfile::TempDir, Storage, swarm_protocol::WorldId) {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path()).unwrap();
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "1.21.8".into(),
        fabric_loader_version: "0.17.2".into(),
        compatibility_fingerprint: Hash32([4; 32]),
        creation_nonce: [5; 32],
        creator_public_key: [1; 32],
        initial_membership: vec![PeerId([1; 32]), PeerId([2; 32])],
    };
    let world = genesis.world_id().unwrap();
    storage
        .create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "membership history".into(),
            world_id: world,
            genesis,
        })
        .unwrap();
    storage
        .save_world_descriptor(&WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: Hash32([4; 32]),
            members: vec![member(1), member(2)],
            preferred_replication_factor: 2,
        })
        .unwrap();
    (temp, storage, world)
}

fn record(
    world: swarm_protocol::WorldId,
    sequence: u64,
    previous_membership_hash: Option<Hash32>,
) -> MembershipRecordV1 {
    MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch: 0,
        sequence,
        previous_membership_hash,
        members: vec![member(1), member(2)],
        authority_peer_id: PeerId([1; 32]),
        authority_public_key: [1; 32],
        signature: vec![1; 64],
    }
}

#[test]
fn membership_requires_exact_direct_parent_and_sequence() {
    let (_temp, storage, world) = fixture();
    let initial = record(world, 0, None);
    storage.save_membership_record(&initial).unwrap();

    let wrong_parent = record(world, 1, Some(Hash32([99; 32])));
    assert!(storage.save_membership_record(&wrong_parent).is_err());
    assert_eq!(storage.load_membership_record(world).unwrap(), initial);

    let skipped = record(world, 2, Some(initial.record_hash().unwrap()));
    assert!(storage.save_membership_record(&skipped).is_err());
    assert_eq!(storage.load_membership_record(world).unwrap(), initial);

    let direct = record(world, 1, Some(initial.record_hash().unwrap()));
    storage.save_membership_record(&direct).unwrap();
    assert_eq!(storage.load_membership_record(world).unwrap(), direct);
}

#[test]
fn membership_rejects_noncanonical_member_order_and_duplicates() {
    let (_temp, storage, world) = fixture();

    let mut reordered = record(world, 0, None);
    reordered.members.reverse();
    assert!(storage.save_membership_record(&reordered).is_err());

    let mut duplicated = record(world, 0, None);
    duplicated.members.push(member(2));
    assert!(storage.save_membership_record(&duplicated).is_err());
}
