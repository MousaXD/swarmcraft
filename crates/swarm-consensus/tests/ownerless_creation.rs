use ed25519_dalek::{Signer, SigningKey};
use swarm_consensus::ownerless::{validate_ownerless_genesis, verify_ownerless_approval, verify_ownerless_creation};
use swarm_protocol::{
    Hash32, OwnerlessCreationApprovalV1, OwnerlessCreationV1, OwnerlessGenesisV1, OwnerlessValidatorV1,
    OwnerlessWorldIdV1, WorldId, OWNERLESS_FORMAT_V1, OWNERLESS_NO_OP_V1,
};

fn fixture() -> (OwnerlessGenesisV1, Vec<SigningKey>) {
    let mut keys: Vec<_> = (1..=3).map(|n| SigningKey::from_bytes(&[n; 32])).collect();
    keys.sort_by_key(|key| OwnerlessValidatorV1::from_key(key.verifying_key().to_bytes()).peer_id);
    let genesis = OwnerlessGenesisV1 {
        format_version: OWNERLESS_FORMAT_V1,
        name: "Immutable no-OP".into(),
        creation_nonce: [9; 32],
        compatibility_fingerprint: Hash32([10; 32]),
        initial_state_commitment: Hash32([11; 32]),
        policy: OWNERLESS_NO_OP_V1,
        validators: keys.iter().map(|key| OwnerlessValidatorV1::from_key(key.verifying_key().to_bytes())).collect(),
    };
    (genesis, keys)
}

fn signed(genesis: OwnerlessGenesisV1, keys: &[SigningKey]) -> OwnerlessCreationV1 {
    let world_id = genesis.world_id().unwrap();
    let approvals = keys
        .iter()
        .map(|key| {
            let validator = OwnerlessValidatorV1::from_key(key.verifying_key().to_bytes()).peer_id;
            OwnerlessCreationApprovalV1 {
                world_id,
                validator,
                signature: key.sign(&genesis.approval_bytes(validator).unwrap()).to_bytes().to_vec(),
            }
        })
        .collect();
    OwnerlessCreationV1 { genesis, approvals }
}

#[test]
fn all_three_approvals_bind_immutable_no_op_creation() {
    let (genesis, keys) = fixture();
    let world = genesis.world_id().unwrap();
    let creation = signed(genesis, &keys);
    let verified = verify_ownerless_creation(world, &creation).unwrap();
    assert_eq!(verified.world_id(), world);
    assert_eq!(verified.policy_hash(), creation.genesis.policy_hash().unwrap());
    assert_eq!(verified.maintenance_threshold(), 2);
    for approval in &creation.approvals {
        verify_ownerless_approval(&creation.genesis, world, approval).unwrap();
    }
    assert_eq!(world.to_string().parse::<OwnerlessWorldIdV1>().unwrap(), world);
    assert!(world.to_string().parse::<WorldId>().is_err());
    assert!("scworld:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        .parse::<OwnerlessWorldIdV1>()
        .is_err());
}

#[test]
fn founder_or_majority_alone_cannot_create_and_signatures_cannot_be_amplified() {
    let (genesis, keys) = fixture();
    let world = genesis.world_id().unwrap();
    let valid = signed(genesis, &keys);
    for n in 0..3 {
        let mut partial = valid.clone();
        partial.approvals.truncate(n);
        assert!(verify_ownerless_creation(world, &partial).is_err());
    }
    let mut duplicate = valid.clone();
    duplicate.approvals[1] = duplicate.approvals[0].clone();
    assert!(verify_ownerless_creation(world, &duplicate).is_err());
    let mut reorder = valid.clone();
    reorder.approvals.swap(0, 1);
    assert!(verify_ownerless_creation(world, &reorder).is_err());
    let mut forged = valid.clone();
    forged.approvals[1].signature = forged.approvals[0].signature.clone();
    assert!(verify_ownerless_creation(world, &forged).is_err());
    let mut corrupt = valid.clone();
    corrupt.approvals[0].signature[0] ^= 1;
    assert!(verify_ownerless_creation(world, &corrupt).is_err());
}

#[test]
fn rules_are_fixed_even_if_all_keys_would_sign() {
    let (genesis, _) = fixture();
    for mutate in [
        |g: &mut OwnerlessGenesisV1| g.policy.founder_privileges = true,
        |g: &mut OwnerlessGenesisV1| g.policy.player_op = true,
        |g: &mut OwnerlessGenesisV1| g.policy.rcon = true,
        |g: &mut OwnerlessGenesisV1| g.policy.privileged_console = true,
        |g: &mut OwnerlessGenesisV1| g.policy.command_blocks = true,
        |g: &mut OwnerlessGenesisV1| g.policy.solo_advancement = true,
        |g: &mut OwnerlessGenesisV1| g.policy.compute_grants_vote = true,
        |g: &mut OwnerlessGenesisV1| g.policy.authenticated_invite_only = false,
        |g: &mut OwnerlessGenesisV1| g.policy.immutable_rules = false,
    ] {
        let mut bad = genesis.clone();
        mutate(&mut bad);
        assert!(bad.world_id().is_err());
    }
}

#[test]
fn genesis_pins_prevent_replay_and_roster_substitution() {
    let (genesis, keys) = fixture();
    let world = genesis.world_id().unwrap();
    let valid = signed(genesis, &keys);
    for mutate in [
        |g: &mut OwnerlessGenesisV1| g.name = "Other world".into(),
        |g: &mut OwnerlessGenesisV1| g.creation_nonce = [1; 32],
        |g: &mut OwnerlessGenesisV1| g.compatibility_fingerprint = Hash32([2; 32]),
        |g: &mut OwnerlessGenesisV1| g.initial_state_commitment = Hash32([3; 32]),
        |g: &mut OwnerlessGenesisV1| {
            g.validators[0] =
                OwnerlessValidatorV1::from_key(SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes());
            g.validators.sort_by_key(|v| v.peer_id);
        },
    ] {
        let mut replay = valid.clone();
        mutate(&mut replay.genesis);
        let changed_world = replay.genesis.world_id().unwrap();
        assert_ne!(changed_world, world);
        assert!(verify_ownerless_creation(world, &replay).is_err());
        for a in &mut replay.approvals {
            a.world_id = changed_world;
        }
        assert!(verify_ownerless_creation(changed_world, &replay).is_err());
    }
}

#[test]
fn bounded_canonical_roster_and_weak_keys_refused() {
    let (genesis, _) = fixture();
    let mut duplicate = genesis.clone();
    duplicate.validators[0] = duplicate.validators[1];
    assert!(validate_ownerless_genesis(&duplicate).is_err());
    let mut reorder = genesis.clone();
    reorder.validators.swap(0, 1);
    assert!(validate_ownerless_genesis(&reorder).is_err());
    let mut wrong_id = genesis.clone();
    wrong_id.validators[0].public_key = [0; 32];
    assert!(validate_ownerless_genesis(&wrong_id).is_err());
    let mut weak = genesis.clone();
    let mut identity_point = [0; 32];
    identity_point[0] = 1;
    weak.validators[0] = OwnerlessValidatorV1::from_key(identity_point);
    weak.validators.sort_by_key(|v| v.peer_id);
    assert!(weak.validate().is_ok());
    assert!(validate_ownerless_genesis(&weak).is_err());
    let mut too_many = genesis.clone();
    too_many.validators = (1..=16)
        .map(|n| OwnerlessValidatorV1::from_key(SigningKey::from_bytes(&[n; 32]).verifying_key().to_bytes()))
        .collect();
    too_many.validators.sort_by_key(|v| v.peer_id);
    assert!(validate_ownerless_genesis(&too_many).is_err());
    let mut two = genesis.clone();
    two.validators.pop();
    assert!(validate_ownerless_genesis(&two).is_err());
}

#[test]
fn outsider_has_no_vote_and_creation_is_distinct_from_maintenance() {
    let (genesis, keys) = fixture();
    let outsider = OwnerlessValidatorV1::from_key(SigningKey::from_bytes(&[42; 32]).verifying_key().to_bytes());
    assert!(genesis.validator(outsider.peer_id).is_none());
    assert!(genesis.approval_bytes(outsider.peer_id).is_err());
    let valid = signed(genesis, &keys);
    assert_eq!(valid.genesis.maintenance_threshold().unwrap(), 2);
    // A threshold is merely a policy value. Even these two approvals do not
    // authorize creation, recovery, a runtime, or any canonical state change.
    let mut two = valid.clone();
    two.approvals.pop();
    assert!(verify_ownerless_creation(valid.genesis.world_id().unwrap(), &two).is_err());
}

#[test]
fn mixed_torsion_alias_is_not_an_extra_validator() {
    // Adding the order-two point (0,-1) maps (x,y) to (-x,-y).
    // Produce that encoding from an ordinary public key, without a
    // new dependency or a signing secret in the protocol contract.
    let (mut genesis, keys) = fixture();
    let mut y = keys[0].verifying_key().to_bytes();
    let sign = y[31] & 0x80;
    y[31] &= 0x7f;
    let mut p = [0xffu8; 32];
    p[0] = 0xed;
    p[31] = 0x7f;
    let mut borrow = 0i16;
    for i in 0..32 {
        let n = i16::from(p[i]) - i16::from(y[i]) - borrow;
        p[i] = n.rem_euclid(256) as u8;
        borrow = i16::from(n < 0);
    }
    assert_eq!(borrow, 0);
    p[31] |= sign ^ 0x80;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&p).unwrap();
    assert!(!key.is_weak());
    assert!(!key.to_edwards().is_torsion_free());
    genesis.validators.push(OwnerlessValidatorV1::from_key(p));
    genesis.validators.sort_by_key(|v| v.peer_id);
    assert!(genesis.validate().is_ok());
    assert!(validate_ownerless_genesis(&genesis).is_err());
}
