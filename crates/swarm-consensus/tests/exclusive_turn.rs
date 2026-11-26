//! Real signature contract tests for #8. These do not simulate a durable
//! writer, a Minecraft checkpoint, or a live permission to host.
use ed25519_dalek::{Signer, SigningKey};
use swarm_consensus::exclusive_turn::*;
use swarm_protocol::*;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn device(key: &SigningKey) -> ExclusiveTurnDeviceV1 {
    ExclusiveTurnDeviceV1::from_key(key.verifying_key().to_bytes())
}

fn checkpoint(bytes: &[u8]) -> ExclusiveTurnCheckpointV1 {
    ExclusiveTurnCheckpointV1 {
        manifest_hash: Hash32::from_domain_bytes(b"test-only manifest", bytes),
        state_root: Hash32::from_domain_bytes(b"test-only state", bytes),
        reference_commitment: exclusive_turn_reference_commitment(bytes).unwrap(),
    }
}

fn fixture(nonce: u8) -> ([SigningKey; 2], ExclusiveTurnHistoryV1) {
    let mut keys = [key(11), key(22)];
    keys.sort_by_key(|key| device(key).peer_id);
    let genesis = ExclusiveTurnGenesisV1 {
        format_version: EXCLUSIVE_TURN_FORMAT_V1,
        creation_nonce: [nonce; 32],
        devices: [device(&keys[0]), device(&keys[1])],
        initial_holder: device(&keys[0]).peer_id,
        compatibility: Hash32([88; 32]),
        initial_checkpoint: checkpoint(b"initial reference state"),
    };
    let world_id = genesis.world_id().unwrap();
    let approvals = std::array::from_fn(|i| ExclusiveTurnApprovalV1 {
        world_id,
        device: genesis.devices[i].peer_id,
        signature: keys[i].sign(&genesis.approval_bytes(genesis.devices[i].peer_id).unwrap()).to_bytes().to_vec(),
    });
    (keys, ExclusiveTurnHistoryV1 { creation: ExclusiveTurnCreationV1 { genesis, approvals }, releases: vec![] })
}

fn head(history: &ExclusiveTurnHistoryV1) -> VerifiedExclusiveTurnHead {
    verify_exclusive_turn_history(history.creation.genesis.world_id().unwrap(), history).unwrap()
}

fn sign_offer(offer: ExclusiveTurnOfferV1, source: &SigningKey) -> ExclusiveTurnSignedOfferV1 {
    let signature = source.sign(&offer.signing_bytes().unwrap()).to_bytes().to_vec();
    ExclusiveTurnSignedOfferV1 { offer, signature }
}

fn receipt(offer: &ExclusiveTurnSignedOfferV1, target: &SigningKey) -> ExclusiveTurnReceiptV1 {
    let mut receipt = ExclusiveTurnReceiptV1 {
        world_id: offer.offer.world_id,
        offer_hash: offer.offer_hash().unwrap(),
        recipient: device(target).peer_id,
        signature: vec![],
    };
    receipt.signature = target.sign(&receipt.signing_bytes().unwrap()).to_bytes().to_vec();
    receipt
}

fn release(
    offer: ExclusiveTurnSignedOfferV1,
    receipt: ExclusiveTurnReceiptV1,
    source: &SigningKey,
) -> ExclusiveTurnReleaseV1 {
    let mut release = ExclusiveTurnReleaseV1 { offer, receipt, signature: vec![] };
    release.signature = source.sign(&release.signing_bytes().unwrap()).to_bytes().to_vec();
    release
}

fn transfer(
    head: &VerifiedExclusiveTurnHead,
    attempt: u64,
    bytes: &[u8],
    source: &SigningKey,
    target: &SigningKey,
) -> ExclusiveTurnReleaseV1 {
    let offer = sign_offer(head.draft_offer(attempt, checkpoint(bytes)).unwrap(), source);
    let receipt = receipt(&offer, target);
    release(offer, receipt, source)
}

#[test]
fn fixed_policy_creation_requires_two_distinct_canonical_keys_and_exact_approvals() {
    let (_, history) = fixture(1);
    let world = history.creation.genesis.world_id().unwrap();
    let initial = head(&history);
    assert_eq!(initial.world(), world);
    assert_eq!(initial.epoch(), 1);
    assert_eq!(initial.fence(), 1);
    assert_eq!(initial.holder(), history.creation.genesis.initial_holder);
    assert_eq!(initial.checkpoint(), checkpoint(b"initial reference state"));
    assert_eq!(world.to_string().parse::<ExclusiveTurnWorldIdV1>().unwrap(), world);
    assert!(world.to_string().parse::<WorldId>().is_err());
    assert!(world.to_string().parse::<OwnerlessWorldIdV1>().is_err());
    assert!(format!("scworld:{}", Hash32([1; 32])).parse::<ExclusiveTurnWorldIdV1>().is_err());

    let (_, other) = fixture(2);
    assert!(verify_exclusive_turn_creation(world, &other.creation).is_err());
    let mut bad = history.creation.clone();
    bad.approvals[1] = bad.approvals[0].clone();
    assert!(verify_exclusive_turn_creation(world, &bad).is_err());
    let mut bad = history.creation.clone();
    bad.approvals.swap(0, 1);
    assert!(verify_exclusive_turn_creation(world, &bad).is_err());
    let mut bad = history.creation.clone();
    bad.approvals[1].signature[0] ^= 1;
    assert!(verify_exclusive_turn_creation(world, &bad).is_err());
    let mut bad = history.creation.clone();
    bad.approvals[1].signature.clear();
    assert!(verify_exclusive_turn_creation(world, &bad).is_err());
    let mut bad = history.creation.clone();
    bad.genesis.devices[1] = bad.genesis.devices[0];
    assert!(bad.genesis.world_id().is_err());
    let mut bad = history.creation.clone();
    bad.genesis.devices.swap(0, 1);
    assert!(bad.genesis.world_id().is_err());
    let mut bad = history.creation.clone();
    bad.genesis.devices[0].peer_id = device(&key(90)).peer_id;
    assert!(bad.genesis.world_id().is_err());
    let mut bad = history.creation.clone();
    bad.genesis.initial_holder = device(&key(90)).peer_id;
    assert!(bad.genesis.world_id().is_err());
    let mut bad = history.creation.clone();
    bad.genesis.format_version = EXCLUSIVE_TURN_SAVE_FORMAT_V2 + 1;
    assert!(bad.genesis.world_id().is_err());
    let mut bad = history.creation.clone();
    bad.genesis.compatibility = Hash32::default();
    assert!(bad.genesis.world_id().is_err());
    let mut bad = history.creation.clone();
    bad.genesis.creation_nonce = [0; 32];
    assert!(bad.genesis.world_id().is_err());

    // Both the roster identity binding and strong-key checks are required.
    let mut weak = history.creation.clone();
    weak.genesis.devices[1] = ExclusiveTurnDeviceV1::from_key([0; 32]);
    weak.genesis.devices.sort_by_key(|device| device.peer_id);
    weak.genesis.initial_holder = weak.genesis.devices[0].peer_id;
    let weak_world = weak.genesis.world_id().unwrap();
    for (approval, device) in weak.approvals.iter_mut().zip(weak.genesis.devices) {
        approval.world_id = weak_world;
        approval.device = device.peer_id;
    }
    // Ensure the genuine device's approval is valid so rejection reaches
    // the weak-key check even if the weak device sorts second.
    let genuine = fixture(1).0[0].clone();
    for approval in &mut weak.approvals {
        if approval.device == device(&genuine).peer_id {
            approval.signature =
                genuine.sign(&weak.genesis.approval_bytes(approval.device).unwrap()).to_bytes().to_vec();
        }
    }
    assert!(matches!(
        verify_exclusive_turn_creation(weak_world, &weak),
        Err(ExclusiveTurnError::Invalid("weak or noncanonical device public key"))
            | Err(ExclusiveTurnError::Invalid("invalid device public key"))
    ));
}

#[test]
fn alternating_releases_bind_checkpoint_and_independent_attempt_floors() {
    let (keys, mut history) = fixture(3);
    let initial = head(&history);
    let a = device(&keys[0]).peer_id;
    let b = device(&keys[1]).peer_id;
    let first = transfer(&initial, 5, b"checkpoint one", &keys[0], &keys[1]);
    verify_exclusive_turn_offer(&initial, &first.offer).unwrap();
    verify_exclusive_turn_receipt(&initial, &first.offer, &first.receipt).unwrap();
    let mut no_release = first.clone();
    no_release.signature.clear();
    assert!(apply_exclusive_turn_release(&initial, &no_release).is_err());
    let next = apply_exclusive_turn_release(&initial, &first).unwrap();
    assert_eq!(next.holder(), b);
    assert_eq!(next.epoch(), 2);
    assert_eq!(next.fence(), 2);
    assert_eq!(next.grant(), first.grant_hash().unwrap());
    assert_eq!(next.checkpoint(), checkpoint(b"checkpoint one"));
    assert_eq!(next.attempt_floor(a).unwrap(), 5);
    assert_eq!(next.attempt_floor(b).unwrap(), 0);
    assert!(next.attempt_floor(device(&key(90)).peer_id).is_err());
    assert!(apply_exclusive_turn_release(&next, &first).is_err());
    history.releases.push(first);
    let second = transfer(&next, 1, b"checkpoint two", &keys[1], &keys[0]);
    let again = apply_exclusive_turn_release(&next, &second).unwrap();
    history.releases.push(second);
    assert_eq!(again.holder(), a);
    assert_eq!(again.epoch(), 3);
    assert_eq!(again.fence(), 3);
    assert_eq!(again.attempt_floor(a).unwrap(), 5);
    assert_eq!(again.attempt_floor(b).unwrap(), 1);
    assert!(again.draft_offer(5, checkpoint(b"stale attempt")).is_err());
    assert!(again.draft_offer(0, checkpoint(b"zero attempt")).is_err());
    history.releases.push(transfer(&again, 6, b"checkpoint three", &keys[0], &keys[1]));
    let last = head(&history);
    assert_eq!(last.epoch(), 4);
    assert_eq!(last.fence(), 4);
    assert_eq!(last.holder(), b);
    assert_eq!(last.compatibility(), initial.compatibility());
    assert_eq!(last.devices(), initial.devices());
    assert_eq!(last.world(), initial.world());
    assert_eq!(last.attempt_floor(a).unwrap(), 6);
    assert_eq!(last.attempt_floor(b).unwrap(), 1);
    last.checkpoint().verify_reference_bytes(b"checkpoint three").unwrap();
    assert!(last.checkpoint().verify_reference_bytes(b"checkpoint two").is_err());
    assert!(last.checkpoint().verify_reference_bytes(b"").is_err());
    assert!(exclusive_turn_reference_commitment(&vec![1; MAX_EXCLUSIVE_TURN_REFERENCE_BYTES + 1]).is_err());
}

#[test]
fn save_policy_is_committed_to_world_identity_and_survives_release_history() {
    let (keys, opaque) = fixture(17);
    let original = head(&opaque);
    assert!(!original.save_backed());
    let mut save = opaque.clone();
    save.creation.genesis.format_version = EXCLUSIVE_TURN_SAVE_FORMAT_V2;
    let world = save.creation.genesis.world_id().unwrap();
    assert_ne!(world, original.world());
    assert_ne!(save.creation.genesis.initial_grant_hash().unwrap(), original.grant());
    // No old approval or opaque grant can be reused for the new policy.
    assert!(verify_exclusive_turn_history(world, &save).is_err());
    for (i, approval) in save.creation.approvals.iter_mut().enumerate() {
        approval.world_id = world;
        approval.signature =
            keys[i].sign(&save.creation.genesis.approval_bytes(approval.device).unwrap()).to_bytes().to_vec();
    }
    let base = head(&save);
    assert!(base.save_backed());
    let release = transfer(&base, 1, b"record-contract-only fixture", &keys[0], &keys[1]);
    assert!(apply_exclusive_turn_release(&original, &release).is_err());
    save.releases.push(release);
    let next = head(&save);
    assert!(next.save_backed());
    assert_eq!(next.epoch(), 2);
    let mut downgraded = save.clone();
    downgraded.creation.genesis.format_version = EXCLUSIVE_TURN_FORMAT_V1;
    assert!(verify_exclusive_turn_history(world, &downgraded).is_err());
    assert!(verify_exclusive_turn_history(original.world(), &downgraded).is_err());
    assert!(exclusive_turn_history_extends(original.world(), &opaque, &save).is_err());
}

#[test]
fn every_handoff_binding_and_role_is_verified_before_release_acceptance() {
    let (keys, history) = fixture(4);
    let base = head(&history);
    let valid = transfer(&base, 1, b"exact next bytes", &keys[0], &keys[1]);
    let (_, other) = fixture(5);
    let other_world = head(&other).world();
    let mut wrong_world = valid.offer.offer.clone();
    wrong_world.world_id = other_world;
    let signed = sign_offer(wrong_world, &keys[0]);
    let ack = receipt(&signed, &keys[1]);
    assert!(apply_exclusive_turn_release(&base, &release(signed, ack, &keys[0])).is_err());
    let mutations: [fn(&mut ExclusiveTurnOfferV1); 10] = [
        |o| o.parent_grant = Hash32([7; 32]),
        |o| o.base_epoch += 1,
        |o| o.base_fence += 1,
        |o| o.next_epoch += 1,
        |o| o.next_fence += 1,
        |o| o.source = o.recipient,
        |o| o.recipient = o.source,
        |o| o.attempt = 0,
        |o| o.compatibility = Hash32([7; 32]),
        |o| o.checkpoint.state_root = Hash32::default(),
    ];
    for mutate in mutations {
        let mut tampered = valid.clone();
        mutate(&mut tampered.offer.offer);
        assert!(apply_exclusive_turn_release(&base, &tampered).is_err());
        let signed = sign_offer(tampered.offer.offer, &keys[0]);
        let ack = receipt(&signed, &keys[1]);
        assert!(apply_exclusive_turn_release(&base, &release(signed, ack, &keys[0])).is_err());
    }
    let mut tampered = valid.clone();
    tampered.offer.offer.checkpoint = checkpoint(b"different valid digests");
    assert!(apply_exclusive_turn_release(&base, &tampered).is_err());
    // Even a valid newly signed offer needs a new exact recipient receipt.
    tampered.offer = sign_offer(tampered.offer.offer, &keys[0]);
    let changed = release(tampered.offer, valid.receipt.clone(), &keys[0]);
    assert!(apply_exclusive_turn_release(&base, &changed).is_err());
    let outsider = key(91);
    let forged_offer = sign_offer(valid.offer.offer.clone(), &outsider);
    assert!(verify_exclusive_turn_offer(&base, &forged_offer).is_err());
    let wrong_recipient = receipt(&valid.offer, &keys[0]);
    assert!(verify_exclusive_turn_receipt(&base, &valid.offer, &wrong_recipient).is_err());
    let mut forged_receipt = valid.receipt.clone();
    forged_receipt.signature = outsider.sign(&forged_receipt.signing_bytes().unwrap()).to_bytes().to_vec();
    assert!(apply_exclusive_turn_release(&base, &release(valid.offer.clone(), forged_receipt, &keys[0])).is_err());
    let forged_release = release(valid.offer.clone(), valid.receipt.clone(), &keys[1]);
    assert!(apply_exclusive_turn_release(&base, &forged_release).is_err());
    // Cross-domain signatures cannot be reused as a release.
    let mut wrong_domain = valid.clone();
    wrong_domain.signature = wrong_domain.offer.signature.clone();
    assert!(apply_exclusive_turn_release(&base, &wrong_domain).is_err());
    let mut wrong_domain = valid.clone();
    wrong_domain.receipt.signature = wrong_domain.offer.signature.clone();
    assert!(apply_exclusive_turn_release(&base, &wrong_domain).is_err());
    assert!(apply_exclusive_turn_release(&base, &valid).is_ok());
}

#[test]
fn exact_prefix_reconciliation_rejects_valid_conflicts_and_rollback() {
    let (keys, initial) = fixture(6);
    let base = head(&initial);
    let world = base.world();
    let mut left = initial.clone();
    let mut right = initial.clone();
    left.releases.push(transfer(&base, 1, b"left", &keys[0], &keys[1]));
    right.releases.push(transfer(&base, 2, b"right", &keys[0], &keys[1]));
    // A deliberately equivocating holder can sign two valid chains. CFT
    // requires durable non-equivocation; the verifier will never choose
    // one conflicting already-accepted history over the other.
    assert_ne!(head(&left).grant(), head(&right).grant());
    assert!(exclusive_turn_history_extends(world, &left, &right).is_err());
    assert!(exclusive_turn_history_extends(world, &right, &left).is_err());
    assert!(exclusive_turn_history_extends(world, &left, &initial).is_err());
    assert_eq!(exclusive_turn_history_extends(world, &initial, &left).unwrap(), head(&left));
    assert_eq!(exclusive_turn_history_extends(world, &left, &left).unwrap(), head(&left));
    let next = head(&left);
    let mut extension = left.clone();
    extension.releases.push(transfer(&next, 8, b"back", &keys[1], &keys[0]));
    assert_eq!(exclusive_turn_history_extends(world, &left, &extension).unwrap(), head(&extension));
    assert!(exclusive_turn_history_extends(world, &right, &extension).is_err());
}

#[test]
fn history_and_attempt_exhaustion_never_wrap_or_reopen_an_old_grant() {
    let (keys, mut history) = fixture(7);
    let mut current = head(&history);
    for index in 0..MAX_EXCLUSIVE_TURN_RELEASES {
        let source = index % 2;
        let record = transfer(&current, index as u64 + 1, &index.to_le_bytes(), &keys[source], &keys[1 - source]);
        current = apply_exclusive_turn_release(&current, &record).unwrap();
        history.releases.push(record);
    }
    assert_eq!(head(&history), current);
    assert!(current.draft_offer(u64::MAX, checkpoint(b"out of history")).is_err());
    history.releases.push(history.releases[0].clone());
    assert!(verify_exclusive_turn_history(current.world(), &history).is_err());

    let (keys, mut history) = fixture(8);
    let initial = head(&history);
    history.releases.push(transfer(&initial, u64::MAX, b"last source attempt", &keys[0], &keys[1]));
    let next = head(&history);
    history.releases.push(transfer(&next, 1, b"return", &keys[1], &keys[0]));
    let again = head(&history);
    assert_eq!(again.attempt_floor(again.holder()).unwrap(), u64::MAX);
    assert!(again.draft_offer(u64::MAX, checkpoint(b"no more attempts")).is_err());
}
