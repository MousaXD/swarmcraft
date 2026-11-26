//! Checks an independently pinned historical head and nonce-bound signatures.
//! The caller MUST own the nonce, enforce its own monotonic collection deadline,
//! check actual local journals, and never interpret a sample as a lease.
use crate::{ownerless::verify_signature, ownerless_authority::VerifiedOwnerlessHead};
use std::collections::BTreeSet;
use swarm_protocol::{
    OwnerlessError, OwnerlessObservationRequestV1, OwnerlessObservationResponseV1, OwnerlessObservationV1, PeerId,
    MAX_OWNERLESS_VALIDATORS,
};

pub fn observation_context(
    head: &VerifiedOwnerlessHead,
    verifier: PeerId,
    nonce: [u8; 32],
) -> Result<OwnerlessObservationV1, OwnerlessError> {
    if nonce == [0; 32] || !head.validators().iter().any(|v| v.peer_id == verifier) {
        return Err(OwnerlessError::Invalid("invalid observation nonce or verifier membership"));
    }
    Ok(OwnerlessObservationV1 {
        world: head.world(),
        head: head.hash(),
        sequence: head.sequence(),
        fence: head.fence(),
        checkpoint: head.checkpoint(),
        policy: head.policy(),
        compatibility: head.compatibility(),
        verifier,
        nonce,
    })
}

pub fn verify_observation_request(
    head: &VerifiedOwnerlessHead,
    request: &OwnerlessObservationRequestV1,
) -> Result<(), OwnerlessError> {
    let context = &request.context;
    if *context != observation_context(head, context.verifier, context.nonce)? {
        return Err(OwnerlessError::Invalid("observation differs from exact accepted head"));
    }
    let voter = head
        .validators()
        .iter()
        .find(|v| v.peer_id == context.verifier)
        .ok_or(OwnerlessError::Invalid("unknown observation verifier"))?;
    if !verify_signature(&voter.public_key, &context.request_bytes()?, &request.signature) {
        return Err(OwnerlessError::Signature);
    }
    Ok(())
}

pub fn verify_observation_response(
    head: &VerifiedOwnerlessHead,
    request: &OwnerlessObservationRequestV1,
    expected_validator: PeerId,
    response: &OwnerlessObservationResponseV1,
) -> Result<(), OwnerlessError> {
    verify_observation_request(head, request)?;
    if response.validator != expected_validator || response.context_hash != request.context.context_hash()? {
        return Err(OwnerlessError::Invalid("observation response differs from targeted validator or context"));
    }
    let voter = head
        .validators()
        .iter()
        .find(|v| v.peer_id == expected_validator)
        .ok_or(OwnerlessError::Invalid("unknown observation responder"))?;
    if !verify_signature(&voter.public_key, &request.context.response_bytes(expected_validator)?, &response.signature) {
        return Err(OwnerlessError::Signature);
    }
    Ok(())
}

/// This is only a mathematical signature threshold over one historical context.
/// It does not determine whether the history remains current, now or later.
pub fn verify_observation_majority(
    head: &VerifiedOwnerlessHead,
    request: &OwnerlessObservationRequestV1,
    responses: &[OwnerlessObservationResponseV1],
) -> Result<(), OwnerlessError> {
    verify_observation_request(head, request)?;
    if responses.len() < head.threshold() || responses.len() > MAX_OWNERLESS_VALIDATORS {
        return Err(OwnerlessError::Invalid("missing observation majority or excessive responses"));
    }
    let mut seen = BTreeSet::new();
    for response in responses {
        if !seen.insert(response.validator) {
            return Err(OwnerlessError::Invalid("duplicate observation responder"));
        }
        verify_observation_response(head, request, response.validator, response)?;
    }
    if !seen.contains(&request.context.verifier) {
        return Err(OwnerlessError::Invalid("observation lacks verifier's local signature"));
    }
    Ok(())
}
