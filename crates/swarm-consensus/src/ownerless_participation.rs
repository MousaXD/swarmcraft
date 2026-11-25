//! #67 reference role eligibility. No live quorum, Minecraft authentication,
//! resource reservation, validator admission, or canonical execution permit.
use swarm_protocol::{
    OwnerlessError, OwnerlessParticipantV1, OwnerlessParticipationChallengeV1, OwnerlessParticipationIntentV1,
    OwnerlessParticipationMode, OwnerlessParticipationRequestV1, OwnerlessParticipationResponseV1,
    OwnerlessWorkerBudgetV1,
};

use crate::ownerless::{validate_ownerless_public_key, verify_signature};
use crate::ownerless_authority::VerifiedOwnerlessHead;

pub(crate) fn validate_participant(participant: &OwnerlessParticipantV1) -> Result<(), OwnerlessError> {
    if *participant != OwnerlessParticipantV1::from_key(participant.public_key) {
        return Err(OwnerlessError::Invalid("participant identity key mismatch"));
    }
    validate_ownerless_public_key(&participant.public_key)
}

pub fn ownerless_participation_challenge(
    head: &VerifiedOwnerlessHead,
    verifier: OwnerlessParticipantV1,
    subject: OwnerlessParticipantV1,
    nonce: [u8; 32],
    intent: OwnerlessParticipationIntentV1,
) -> Result<OwnerlessParticipationChallengeV1, OwnerlessError> {
    let challenge = OwnerlessParticipationChallengeV1 {
        world: head.world(),
        head: head.hash(),
        fence: head.fence(),
        policy: head.policy(),
        compatibility: head.compatibility(),
        verifier,
        subject,
        nonce,
        intent,
    };
    validate_ownerless_participation_challenge(head, &challenge)?;
    Ok(challenge)
}

fn validate_ownerless_participation_challenge(
    head: &VerifiedOwnerlessHead,
    challenge: &OwnerlessParticipationChallengeV1,
) -> Result<(), OwnerlessError> {
    if challenge.world != head.world()
        || challenge.head != head.hash()
        || challenge.fence != head.fence()
        || challenge.policy != head.policy()
        || challenge.compatibility != head.compatibility()
        || challenge.nonce == [0; 32]
    {
        return Err(OwnerlessError::Invalid("participation challenge differs from accepted head or nonce"));
    }
    validate_participant(&challenge.verifier)?;
    validate_participant(&challenge.subject)?;
    if let OwnerlessParticipationIntentV1::Volunteer { budget } = challenge.intent {
        budget.validate()?;
    }
    Ok(())
}

pub fn verify_ownerless_participation_request(
    head: &VerifiedOwnerlessHead,
    request: &OwnerlessParticipationRequestV1,
) -> Result<(), OwnerlessError> {
    let challenge = &request.challenge;
    validate_ownerless_participation_challenge(head, challenge)?;
    if !verify_signature(&challenge.verifier.public_key, &challenge.request_bytes()?, &request.signature) {
        return Err(OwnerlessError::Signature);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceParticipationEligibility {
    Player,
    Volunteer { budget: OwnerlessWorkerBudgetV1 },
}

/// The caller supplies its own issued challenge, not an untrusted claimed
/// expectation. Successful callers MUST consume that challenge before using
/// the result. This historical reference check is not a live authority grant.
pub fn verify_ownerless_participation(
    head: &VerifiedOwnerlessHead,
    expected: &OwnerlessParticipationChallengeV1,
    response: &OwnerlessParticipationResponseV1,
) -> Result<ReferenceParticipationEligibility, OwnerlessError> {
    validate_ownerless_participation_challenge(head, expected)?;
    if response.challenge != *expected {
        return Err(OwnerlessError::Invalid("participation response differs from issued challenge"));
    }
    if !verify_signature(&expected.subject.public_key, &expected.response_bytes()?, &response.signature) {
        return Err(OwnerlessError::Signature);
    }
    match expected.intent {
        OwnerlessParticipationIntentV1::Play => {
            if head.participation_mode() == OwnerlessParticipationMode::InviteOnly
                && !head.invited_players().contains(&expected.subject)
            {
                return Err(OwnerlessError::Invalid("player has no accepted private invitation"));
            }
            Ok(ReferenceParticipationEligibility::Player)
        }
        OwnerlessParticipationIntentV1::Volunteer { budget } => {
            let maximum = head
                .eligible_workers()
                .iter()
                .find(|w| w.worker == expected.subject)
                .ok_or(OwnerlessError::Invalid("worker has no accepted eligibility"))?
                .maximum;
            if !maximum.contains(budget) {
                return Err(OwnerlessError::Invalid("volunteer offer exceeds accepted worker cap"));
            }
            Ok(ReferenceParticipationEligibility::Volunteer { budget })
        }
    }
}
