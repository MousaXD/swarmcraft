//! Signed participation and role isolation. No test implies Minecraft login.
use ed25519_dalek::{Signer, SigningKey};
use swarm_consensus::{ownerless::verify_ownerless_creation, ownerless_authority::*, ownerless_participation::*};
use swarm_protocol::*;

fn key(i: u8) -> SigningKey {
    SigningKey::from_bytes(&[i; 32])
}
fn participant(i: u8) -> OwnerlessParticipantV1 {
    OwnerlessParticipantV1::from_key(key(i).verifying_key().to_bytes())
}
fn validator(i: u8) -> OwnerlessValidatorV1 {
    OwnerlessValidatorV1::from_key(key(i).verifying_key().to_bytes())
}
fn genesis(public: bool) -> OwnerlessGenesisV1 {
    let mut validators = vec![validator(1), validator(2), validator(3)];
    validators.sort_by_key(|v| v.peer_id);
    OwnerlessGenesisV1 {
        format_version: if public { OWNERLESS_PUBLIC_FORMAT_V2 } else { OWNERLESS_FORMAT_V1 },
        name: "Participation reference".into(),
        creation_nonce: [21; 32],
        compatibility_fingerprint: Hash32([22; 32]),
        initial_state_commitment: ownerless_checkpoint_commitment(b"genesis").unwrap(),
        policy: if public { OWNERLESS_NO_OP_PUBLIC_V2 } else { OWNERLESS_NO_OP_V1 },
        validators,
    }
}
fn signed(genesis: OwnerlessGenesisV1) -> OwnerlessCreationV1 {
    let world_id = genesis.world_id().unwrap();
    let mut approvals: Vec<_> = (1..=3)
        .map(|i| {
            let validator = validator(i).peer_id;
            OwnerlessCreationApprovalV1 {
                world_id,
                validator,
                signature: key(i).sign(&genesis.approval_bytes(validator).unwrap()).to_bytes().to_vec(),
            }
        })
        .collect();
    approvals.sort_by_key(|v| v.validator);
    OwnerlessCreationV1 { genesis, approvals }
}
fn initial(public: bool) -> VerifiedOwnerlessHead {
    let creation = signed(genesis(public));
    verify_ownerless_history(
        creation.genesis.world_id().unwrap(),
        &OwnerlessAuthorityHistoryV1 { creation, records: vec![] },
    )
    .unwrap()
}
fn cert(
    head: &VerifiedOwnerlessHead,
    action: OwnerlessAuthorityActionV1,
    ids: &[u8],
) -> OwnerlessAuthorityCertificateV1 {
    let proposal = head.draft(action).unwrap();
    let mut votes: Vec<_> = ids
        .iter()
        .map(|i| {
            let validator = validator(*i).peer_id;
            OwnerlessAuthorityVoteV1 {
                proposal_hash: proposal.proposal_hash().unwrap(),
                validator,
                signature: key(*i).sign(&proposal.vote_bytes(validator).unwrap()).to_bytes().to_vec(),
            }
        })
        .collect();
    votes.sort_by_key(|v| v.validator);
    OwnerlessAuthorityCertificateV1 { proposal, votes }
}
fn apply(head: &VerifiedOwnerlessHead, action: OwnerlessAuthorityActionV1) -> VerifiedOwnerlessHead {
    apply_ownerless_certificate(head, &cert(head, action, &[1, 2])).unwrap()
}
fn challenge(
    head: &VerifiedOwnerlessHead,
    subject: u8,
    intent: OwnerlessParticipationIntentV1,
) -> OwnerlessParticipationChallengeV1 {
    ownerless_participation_challenge(head, participant(1), participant(subject), [41; 32], intent).unwrap()
}
fn response(challenge: &OwnerlessParticipationChallengeV1, signer: u8) -> OwnerlessParticipationResponseV1 {
    OwnerlessParticipationResponseV1 {
        challenge: challenge.clone(),
        signature: key(signer).sign(&challenge.response_bytes().unwrap()).to_bytes().to_vec(),
    }
}
fn request(challenge: &OwnerlessParticipationChallengeV1, signer: u8) -> OwnerlessParticipationRequestV1 {
    OwnerlessParticipationRequestV1 {
        challenge: challenge.clone(),
        signature: key(signer).sign(&challenge.request_bytes().unwrap()).to_bytes().to_vec(),
    }
}
fn budget(memory_mib: u32, cpu_millicores: u32) -> OwnerlessWorkerBudgetV1 {
    OwnerlessWorkerBudgetV1 { memory_mib, cpu_millicores }
}

#[test]
fn public_genesis_is_explicit_distinct_and_preserves_private_policy_hash() {
    let private = signed(genesis(false));
    let public = signed(genesis(true));
    let priv_id = private.genesis.world_id().unwrap();
    let pub_id = public.genesis.world_id().unwrap();
    assert_ne!(priv_id, pub_id);
    assert_eq!(
        private.genesis.policy_hash().unwrap().to_string(),
        "c694ae9067292c7ea3da7cc5a825e8ff0b370cccfa6267e45dc3a6c96f76ef4f"
    );
    assert_ne!(private.genesis.policy_hash().unwrap(), public.genesis.policy_hash().unwrap());
    assert!(verify_ownerless_creation(priv_id, &private).is_ok());
    assert!(verify_ownerless_creation(pub_id, &public).is_ok());
    assert!(verify_ownerless_creation(priv_id, &public).is_err());
    let mut downgrade = public.genesis.clone();
    downgrade.format_version = OWNERLESS_FORMAT_V1;
    assert!(downgrade.world_id().is_err());
    let mut upgraded = private.genesis.clone();
    upgraded.format_version = OWNERLESS_PUBLIC_FORMAT_V2;
    assert!(upgraded.world_id().is_err());
    let mut forged = public.clone();
    forged.approvals = private.approvals;
    for approval in &mut forged.approvals {
        approval.world_id = pub_id;
    }
    assert!(verify_ownerless_creation(pub_id, &forged).is_err());
    for modify in [
        |g: &mut OwnerlessGenesisV1| g.policy.player_op = true,
        |g: &mut OwnerlessGenesisV1| g.policy.founder_privileges = true,
        |g: &mut OwnerlessGenesisV1| g.policy.compute_grants_vote = true,
        |g: &mut OwnerlessGenesisV1| g.policy.solo_advancement = true,
        |g: &mut OwnerlessGenesisV1| g.policy.immutable_rules = false,
    ] {
        let mut invalid = public.genesis.clone();
        modify(&mut invalid);
        assert!(invalid.world_id().is_err());
    }
    assert_eq!(initial(false).participation_mode(), OwnerlessParticipationMode::InviteOnly);
    assert_eq!(initial(true).participation_mode(), OwnerlessParticipationMode::PublicAuthenticated);
}

#[test]
fn public_play_is_not_worker_eligibility_or_governance() {
    let head = initial(true);
    let play = challenge(&head, 4, OwnerlessParticipationIntentV1::Play);
    assert_eq!(
        verify_ownerless_participation(&head, &play, &response(&play, 4)).unwrap(),
        ReferenceParticipationEligibility::Player
    );
    assert!(verify_ownerless_participation(&head, &play, &response(&play, 3)).is_err());
    let offer = challenge(&head, 4, OwnerlessParticipationIntentV1::Volunteer { budget: budget(4096, 1000) });
    assert!(verify_ownerless_participation(&head, &offer, &response(&offer, 4)).is_err());
    assert!(head.draft(OwnerlessAuthorityActionV1::SetPlayer { player: participant(4), allowed: true }).is_err());
    let recover = cert(&head, OwnerlessAuthorityActionV1::Recover, &[1, 4]);
    assert!(apply_ownerless_certificate(&head, &recover).is_err());
    let eligible = OwnerlessAuthorityActionV1::SetWorker { worker: participant(4), maximum: Some(budget(4096, 2000)) };
    assert!(apply_ownerless_certificate(&head, &cert(&head, eligible.clone(), &[1])).is_err());
    let next = apply(&head, eligible);
    assert_eq!(next.validators(), head.validators());
    assert_eq!(next.checkpoint(), head.checkpoint());
    assert_eq!(next.fence(), head.fence() + 1);
    let offer = challenge(&next, 4, OwnerlessParticipationIntentV1::Volunteer { budget: budget(4096, 1000) });
    assert_eq!(
        verify_ownerless_participation(&next, &offer, &response(&offer, 4)).unwrap(),
        ReferenceParticipationEligibility::Volunteer { budget: budget(4096, 1000) }
    );
    let oversized = challenge(&next, 4, OwnerlessParticipationIntentV1::Volunteer { budget: budget(4097, 1000) });
    assert!(verify_ownerless_participation(&next, &oversized, &response(&oversized, 4)).is_err());
    assert!(apply_ownerless_certificate(&next, &cert(&next, OwnerlessAuthorityActionV1::Recover, &[1, 4])).is_err());
    let revoked = apply(&next, OwnerlessAuthorityActionV1::SetWorker { worker: participant(4), maximum: None });
    assert!(verify_ownerless_participation(&revoked, &offer, &response(&offer, 4)).is_err());
    let fresh = challenge(&revoked, 4, offer.intent);
    assert!(verify_ownerless_participation(&revoked, &fresh, &response(&fresh, 4)).is_err());
    let play = challenge(&revoked, 4, OwnerlessParticipationIntentV1::Play);
    assert!(verify_ownerless_participation(&revoked, &play, &response(&play, 4)).is_ok());
}

#[test]
fn private_invitation_is_independent_even_for_validators_and_eligible_workers() {
    let head = initial(false);
    for i in [1, 4] {
        let play = challenge(&head, i, OwnerlessParticipationIntentV1::Play);
        assert!(verify_ownerless_participation(&head, &play, &response(&play, i)).is_err());
    }
    let invited = apply(&head, OwnerlessAuthorityActionV1::SetPlayer { player: participant(4), allowed: true });
    let play = challenge(&invited, 4, OwnerlessParticipationIntentV1::Play);
    assert!(verify_ownerless_participation(&invited, &play, &response(&play, 4)).is_ok());
    let offered = challenge(&invited, 4, OwnerlessParticipationIntentV1::Volunteer { budget: budget(4096, 1000) });
    assert!(verify_ownerless_participation(&invited, &offered, &response(&offered, 4)).is_err());
    assert!(invited.draft(OwnerlessAuthorityActionV1::SetPlayer { player: participant(4), allowed: true }).is_err());
    let worker = apply(
        &invited,
        OwnerlessAuthorityActionV1::SetWorker { worker: participant(5), maximum: Some(budget(8192, 2000)) },
    );
    let play = challenge(&worker, 5, OwnerlessParticipationIntentV1::Play);
    assert!(verify_ownerless_participation(&worker, &play, &response(&play, 5)).is_err());
    let offer = challenge(&worker, 5, OwnerlessParticipationIntentV1::Volunteer { budget: budget(512, 250) });
    assert!(verify_ownerless_participation(&worker, &offer, &response(&offer, 5)).is_ok());
    let revoked = apply(&worker, OwnerlessAuthorityActionV1::SetPlayer { player: participant(4), allowed: false });
    assert!(revoked.invited_players().is_empty());
    assert_eq!(revoked.eligible_workers(), worker.eligible_workers());
    assert_eq!(revoked.validators(), head.validators());
    let play = challenge(&revoked, 4, OwnerlessParticipationIntentV1::Play);
    assert!(verify_ownerless_participation(&revoked, &play, &response(&play, 4)).is_err());
}

#[test]
fn signed_request_response_and_context_pins_refuse_tampering_and_role_replay() {
    let head = initial(true);
    let original = challenge(&head, 4, OwnerlessParticipationIntentV1::Play);
    assert!(verify_ownerless_participation_request(&head, &request(&original, 1)).is_ok());
    assert!(verify_ownerless_participation_request(&head, &request(&original, 2)).is_err());
    let signed = response(&original, 4);
    let other_world = initial(false);
    assert!(verify_ownerless_participation(&other_world, &original, &signed).is_err());
    let mut variants = vec![];
    let mut altered = original.clone();
    altered.nonce = [42; 32];
    variants.push(altered);
    let mut altered = original.clone();
    altered.verifier = participant(2);
    variants.push(altered);
    let mut altered = original.clone();
    altered.subject = participant(5);
    variants.push(altered);
    let mut altered = original.clone();
    altered.intent = OwnerlessParticipationIntentV1::Volunteer { budget: budget(512, 250) };
    variants.push(altered);
    let mut altered = original.clone();
    altered.head = Hash32([44; 32]);
    variants.push(altered);
    let mut altered = original.clone();
    altered.fence += 1;
    variants.push(altered);
    let mut altered = original.clone();
    altered.policy = Hash32([44; 32]);
    variants.push(altered);
    let mut altered = original.clone();
    altered.compatibility = Hash32([44; 32]);
    variants.push(altered);
    for altered in variants {
        assert!(verify_ownerless_participation(&head, &altered, &signed).is_err());
        let mut forged = signed.clone();
        forged.challenge = altered;
        assert!(verify_ownerless_participation(&head, &original, &forged).is_err());
    }
    let mut swapped = signed.clone();
    swapped.signature = request(&original, 1).signature;
    assert!(verify_ownerless_participation(&head, &original, &swapped).is_err());
    assert!(ownerless_participation_challenge(
        &head,
        participant(1),
        participant(4),
        [0; 32],
        OwnerlessParticipationIntentV1::Play
    )
    .is_err());
    let bad_key = OwnerlessParticipantV1::from_key([0; 32]);
    assert!(ownerless_participation_challenge(
        &head,
        participant(1),
        bad_key,
        [41; 32],
        OwnerlessParticipationIntentV1::Play
    )
    .is_err());
    for b in [budget(0, 0), budget(511, 250), budget(512, 0), budget(65537, 250), budget(512, 64001)] {
        assert!(head
            .draft(OwnerlessAuthorityActionV1::SetWorker { worker: participant(4), maximum: Some(b) })
            .is_err());
        assert!(ownerless_participation_challenge(
            &head,
            participant(1),
            participant(4),
            [41; 32],
            OwnerlessParticipationIntentV1::Volunteer { budget: b }
        )
        .is_err());
    }
    let recovered = apply(&head, OwnerlessAuthorityActionV1::Recover);
    assert!(verify_ownerless_participation_request(&recovered, &request(&original, 1)).is_err());
    assert!(verify_ownerless_participation(&recovered, &original, &signed).is_err());
}
