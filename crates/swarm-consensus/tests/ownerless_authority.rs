//! Cryptographic contract tests; these deliberately do not claim BFT safety.
use ed25519_dalek::{Signer, SigningKey};
use swarm_consensus::ownerless_authority::*;
use swarm_protocol::*;

fn keys(n: u8) -> Vec<SigningKey> {
    (1..=n).map(|i| SigningKey::from_bytes(&[i; 32])).collect()
}
fn validator(key: &SigningKey) -> OwnerlessValidatorV1 {
    OwnerlessValidatorV1::from_key(key.verifying_key().to_bytes())
}
fn roster(keys: &[SigningKey]) -> Vec<OwnerlessValidatorV1> {
    let mut v: Vec<_> = keys.iter().map(validator).collect();
    v.sort_by_key(|v| v.peer_id);
    v
}
fn history(keys: &[SigningKey]) -> OwnerlessAuthorityHistoryV1 {
    let genesis = OwnerlessGenesisV1 {
        format_version: OWNERLESS_FORMAT_V1,
        name: "Reference realm".into(),
        creation_nonce: [21; 32],
        compatibility_fingerprint: Hash32([22; 32]),
        initial_state_commitment: ownerless_checkpoint_commitment(b"genesis").unwrap(),
        policy: OWNERLESS_NO_OP_V1,
        validators: roster(keys),
    };
    let world_id = genesis.world_id().unwrap();
    let mut approvals: Vec<_> = keys
        .iter()
        .map(|key| {
            let validator = validator(key).peer_id;
            OwnerlessCreationApprovalV1 {
                world_id,
                validator,
                signature: key.sign(&genesis.approval_bytes(validator).unwrap()).to_bytes().to_vec(),
            }
        })
        .collect();
    approvals.sort_by_key(|v| v.validator);
    OwnerlessAuthorityHistoryV1 { creation: OwnerlessCreationV1 { genesis, approvals }, records: vec![] }
}
fn head(history: &OwnerlessAuthorityHistoryV1) -> VerifiedOwnerlessHead {
    verify_ownerless_history(history.creation.genesis.world_id().unwrap(), history).unwrap()
}
fn vote(proposal: &OwnerlessAuthorityProposalV1, key: &SigningKey) -> OwnerlessAuthorityVoteV1 {
    let validator = validator(key).peer_id;
    OwnerlessAuthorityVoteV1 {
        proposal_hash: proposal.proposal_hash().unwrap(),
        validator,
        signature: key.sign(&proposal.vote_bytes(validator).unwrap()).to_bytes().to_vec(),
    }
}
fn certify(proposal: &OwnerlessAuthorityProposalV1, keys: &[SigningKey]) -> OwnerlessAuthorityCertificateV1 {
    let mut votes: Vec<_> = keys.iter().map(|k| vote(proposal, k)).collect();
    votes.sort_by_key(|v| v.validator);
    OwnerlessAuthorityCertificateV1 { proposal: proposal.clone(), votes }
}

#[test]
fn surviving_majority_advances_and_recovers_exact_checkpoint_without_founder() {
    let k = keys(4);
    let mut h = history(&k[..3]);
    let initial = head(&h);
    let commitment = ownerless_checkpoint_commitment(b"next").unwrap();
    let p = initial.draft(OwnerlessAuthorityActionV1::Checkpoint { commitment }).unwrap();
    assert!(apply_ownerless_certificate(&initial, &certify(&p, &k[1..2])).is_err());
    assert!(apply_ownerless_certificate(&initial, &certify(&p, &[k[1].clone(), k[3].clone()])).is_err());
    h.records.push(certify(&p, &k[1..3]));
    let accepted = head(&h);
    assert_eq!(accepted.sequence(), 1);
    assert_eq!(accepted.fence(), 1);
    assert_eq!(accepted.checkpoint(), commitment);
    let recovery = accepted.draft(OwnerlessAuthorityActionV1::Recover).unwrap();
    h.records.push(certify(&recovery, &k[1..3]));
    let recovered = head(&h);
    assert_eq!(recovered.sequence(), 2);
    assert_eq!(recovered.fence(), 2);
    assert_eq!(recovered.checkpoint(), commitment);
    assert_eq!(recovered.world(), initial.world());
    assert_eq!(recovered.policy(), initial.policy());
    assert!(apply_ownerless_certificate(&recovered, &certify(&p, &k[1..3])).is_err());
    assert!(apply_ownerless_certificate(&recovered, &certify(&recovery, &k[1..3])).is_err());
}

#[test]
fn joint_majorities_and_every_new_validator_are_required() {
    let k = keys(6);
    let mut h = history(&k[..3]);
    let old = head(&h);
    let new = vec![k[2].clone(), k[3].clone(), k[4].clone(), k[5].clone()];
    let p = old.draft(OwnerlessAuthorityActionV1::Reconfigure { validators: roster(&new) }).unwrap();
    // New majority without old majority.
    assert!(apply_ownerless_certificate(&old, &certify(&p, &k[3..])).is_err());
    // Old majority but missing proposed majority.
    assert!(apply_ownerless_certificate(&old, &certify(&p, &k[..3])).is_err());
    // Both majorities, but one newly enrolled validator did not consent.
    assert!(apply_ownerless_certificate(&old, &certify(&p, &k[1..5])).is_err());
    let outsider = k[3].clone();
    let checkpoint = old.draft(OwnerlessAuthorityActionV1::Checkpoint { commitment: Hash32([90; 32]) }).unwrap();
    assert!(verify_ownerless_vote(&old, &checkpoint, &vote(&checkpoint, &outsider)).is_err());
    h.records.push(certify(&p, &k[1..]));
    let next = head(&h);
    assert_eq!(next.validators(), roster(&new));
    assert_eq!(next.fence(), 2);
    assert_eq!(next.checkpoint(), old.checkpoint());
    let p2 = next.draft(OwnerlessAuthorityActionV1::Recover).unwrap();
    assert!(verify_ownerless_vote(&next, &p2, &vote(&p2, &k[1])).is_err());
    assert!(apply_ownerless_certificate(&next, &certify(&p2, &k[2..4])).is_err());
    assert!(apply_ownerless_certificate(&next, &certify(&p2, &k[2..5])).is_ok());
}

#[test]
fn tampering_and_signature_replay_are_rejected_even_with_a_majority() {
    let k = keys(3);
    let h = history(&k);
    let base = head(&h);
    let p = base.draft(OwnerlessAuthorityActionV1::Recover).unwrap();
    let valid = certify(&p, &k[..2]);
    let mutations: [fn(&mut OwnerlessAuthorityProposalV1); 7] = [
        |p| p.sequence += 1,
        |p| p.parent = Hash32([1; 32]),
        |p| p.policy_hash = Hash32([1; 32]),
        |p| p.compatibility = Hash32([1; 32]),
        |p| p.base_fence += 1,
        |p| p.base_checkpoint = Hash32([1; 32]),
        |p| p.action = OwnerlessAuthorityActionV1::Checkpoint { commitment: Hash32([1; 32]) },
    ];
    for (i, mutate) in mutations.into_iter().enumerate() {
        let mut bad = valid.clone();
        mutate(&mut bad.proposal);
        assert!(apply_ownerless_certificate(&base, &bad).is_err());
        // A legitimately signed changed action is possible, but never
        // legitimately signed changed ancestry/policy/compatibility.
        if i != 6 {
            assert!(apply_ownerless_certificate(&base, &certify(&bad.proposal, &k[..2])).is_err());
        }
    }
    let mut dup = valid.clone();
    dup.votes[1] = dup.votes[0].clone();
    assert!(apply_ownerless_certificate(&base, &dup).is_err());
    let mut unsorted = valid.clone();
    unsorted.votes.reverse();
    assert!(apply_ownerless_certificate(&base, &unsorted).is_err());
    let mut forged = valid.clone();
    forged.votes[0].signature[1] ^= 1;
    assert!(apply_ownerless_certificate(&base, &forged).is_err());
    let mut other = h.clone();
    other.creation.genesis.creation_nonce = [31; 32];
    assert!(verify_ownerless_history(other.creation.genesis.world_id().unwrap(), &other).is_err());
}

#[test]
fn witness_subsets_are_interchangeable_but_conflicting_accepted_ancestry_is_not() {
    let k = keys(3);
    let h = history(&k);
    let base = head(&h);
    let p = base.draft(OwnerlessAuthorityActionV1::Recover).unwrap();
    let mut a = h.clone();
    a.records.push(certify(&p, &k[..2]));
    let mut b = h.clone();
    b.records.push(certify(&p, &k[1..]));
    assert_eq!(head(&a), head(&b));
    assert!(ownerless_history_extends(&a, &b));
    assert!(!ownerless_history_extends(&a, &h));
    let fork = base.draft(OwnerlessAuthorityActionV1::Checkpoint { commitment: Hash32([77; 32]) }).unwrap();
    let mut c = h.clone();
    // Deliberate equivocation of fixture signing keys demonstrates why
    // signature verification alone is not BFT or proof of a unique head.
    c.records.push(certify(&fork, &k[..2]));
    assert_ne!(head(&a), head(&c));
    assert!(!ownerless_history_extends(&a, &c));
    assert!(!ownerless_history_extends(&c, &a));
    assert!(ownerless_history_extends(&h, &a));
}

#[test]
fn bounded_rosters_and_checkpoint_documents_fail_closed() {
    let k = keys(16);
    let h = history(&k[..3]);
    let base = head(&h);
    let change = |validators| base.draft(OwnerlessAuthorityActionV1::Reconfigure { validators });
    assert!(change(roster(&k[..2])).is_err());
    assert!(change(roster(&k[..3])).is_err());
    assert!(change(roster(&k)).is_err());
    let mut duplicate = roster(&k[1..4]);
    duplicate[1] = duplicate[0];
    assert!(change(duplicate).is_err());
    let mut weak = roster(&k[1..4]);
    weak[0] = OwnerlessValidatorV1::from_key([0; 32]);
    weak.sort_by_key(|v| v.peer_id);
    assert!(change(weak).is_err());
    assert!(base.draft(OwnerlessAuthorityActionV1::Checkpoint { commitment: base.checkpoint() }).is_err());
    assert!(base.draft(OwnerlessAuthorityActionV1::Checkpoint { commitment: Hash32::default() }).is_err());
    assert!(ownerless_checkpoint_commitment(b"").is_err());
    assert!(ownerless_checkpoint_commitment(&vec![0; MAX_OWNERLESS_CHECKPOINT_BYTES + 1]).is_err());
    let mut oversized = h.clone();
    let p = base.draft(OwnerlessAuthorityActionV1::Recover).unwrap();
    oversized.records = vec![certify(&p, &k[..2]); MAX_OWNERLESS_HISTORY_RECORDS + 1];
    assert!(verify_ownerless_history(base.world(), &oversized).is_err());
}

#[test]
fn ownerless_observation_threshold_and_nonce_are_not_governance_votes() {
    use swarm_consensus::ownerless_observation::*;
    let k = keys(4);
    let h = history(&k[..3]);
    let initial = head(&h);
    let verifier = validator(&k[1]).peer_id;
    let context = observation_context(&initial, verifier, [43; 32]).unwrap();
    let sign = |context: OwnerlessObservationV1, key: &SigningKey| OwnerlessObservationRequestV1 {
        signature: key.sign(&context.request_bytes().unwrap()).to_bytes().to_vec(),
        context,
    };
    let request = sign(context.clone(), &k[1]);
    let respond = |request: &OwnerlessObservationRequestV1, key: &SigningKey| {
        let validator = validator(key).peer_id;
        OwnerlessObservationResponseV1 {
            context_hash: request.context.context_hash().unwrap(),
            validator,
            signature: key.sign(&request.context.response_bytes(validator).unwrap()).to_bytes().to_vec(),
        }
    };
    let local = respond(&request, &k[1]);
    let remote = respond(&request, &k[2]);
    assert!(verify_observation_request(&initial, &request).is_ok());
    assert!(verify_observation_majority(&initial, &request, &[local.clone(), remote.clone()]).is_ok());
    assert!(verify_observation_majority(&initial, &request, std::slice::from_ref(&local)).is_err());
    assert!(verify_observation_majority(&initial, &request, &[local.clone(), local.clone()]).is_err());
    assert!(verify_observation_majority(&initial, &request, &[remote.clone(), respond(&request, &k[0])]).is_err());
    assert!(verify_observation_majority(&initial, &request, &[local.clone(), respond(&request, &k[3])]).is_err());
    assert!(verify_observation_response(&initial, &request, validator(&k[0]).peer_id, &remote).is_err());

    let fresh = sign(observation_context(&initial, verifier, [44; 32]).unwrap(), &k[1]);
    assert!(verify_observation_response(&initial, &fresh, remote.validator, &remote).is_err());
    assert!(observation_context(&initial, verifier, [0; 32]).is_err());
    assert!(observation_context(&initial, validator(&k[3]).peer_id, [43; 32]).is_err());
    let mut altered = context.clone();
    altered.world = format!("scownerless:{}", Hash32([1; 32])).parse().unwrap();
    let mut wrong = vec![altered];
    let mut altered = context.clone();
    altered.head = Hash32([1; 32]);
    wrong.push(altered);
    let mut altered = context.clone();
    altered.sequence += 1;
    wrong.push(altered);
    let mut altered = context.clone();
    altered.fence += 1;
    wrong.push(altered);
    let mut altered = context.clone();
    altered.checkpoint = Hash32([1; 32]);
    wrong.push(altered);
    let mut altered = context.clone();
    altered.policy = Hash32([1; 32]);
    wrong.push(altered);
    let mut altered = context.clone();
    altered.compatibility = Hash32([1; 32]);
    wrong.push(altered);
    for altered in wrong {
        assert!(verify_observation_request(&initial, &sign(altered, &k[1])).is_err());
    }
    assert!(verify_observation_request(&initial, &sign(context, &k[2])).is_err());
    let mut forged = remote.clone();
    forged.signature = request.signature.clone();
    assert!(verify_observation_response(&initial, &request, remote.validator, &forged).is_err());

    let proposal = initial.draft(OwnerlessAuthorityActionV1::Recover).unwrap();
    let next = apply_ownerless_certificate(&initial, &certify(&proposal, &k[1..3])).unwrap();
    assert!(verify_observation_majority(&next, &request, &[local.clone(), remote.clone()]).is_err());
    let governance = OwnerlessAuthorityVoteV1 {
        proposal_hash: proposal.proposal_hash().unwrap(),
        validator: remote.validator,
        signature: remote.signature,
    };
    assert!(verify_ownerless_vote(&initial, &proposal, &governance).is_err());
}

#[test]
fn ownerless_observation_removed_member_cannot_attest_under_new_roster() {
    use swarm_consensus::ownerless_observation::*;
    let k = keys(4);
    let h = history(&k[..3]);
    let initial = head(&h);
    let next_roster = roster(&k[1..4]);
    let proposal = initial.draft(OwnerlessAuthorityActionV1::Reconfigure { validators: next_roster }).unwrap();
    let next = apply_ownerless_certificate(&initial, &certify(&proposal, &k[1..4])).unwrap();
    let removed = validator(&k[0]).peer_id;
    assert!(observation_context(&next, removed, [2; 32]).is_err());
    let verifier = validator(&k[1]).peer_id;
    let context = observation_context(&next, verifier, [2; 32]).unwrap();
    let request = OwnerlessObservationRequestV1 {
        signature: k[1].sign(&context.request_bytes().unwrap()).to_bytes().to_vec(),
        context,
    };
    let response = OwnerlessObservationResponseV1 {
        context_hash: request.context.context_hash().unwrap(),
        validator: removed,
        signature: k[0].sign(&request.context.response_bytes(removed).unwrap()).to_bytes().to_vec(),
    };
    assert!(verify_observation_response(&next, &request, removed, &response).is_err());
}
