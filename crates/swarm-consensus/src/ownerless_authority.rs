//! CFT reference authority. No live route, epoch, permit or Minecraft authority.
//! Honest validators must durably lock one proposal per accepted parent before
//! releasing a vote. There is intentionally no timeout-based lock reset.
use swarm_protocol::{
    ownerless_genesis_head, Hash32, OwnerlessAuthorityActionV1, OwnerlessAuthorityCertificateV1,
    OwnerlessAuthorityHistoryV1, OwnerlessAuthorityProposalV1, OwnerlessAuthorityVoteV1, OwnerlessError,
    OwnerlessParticipantV1, OwnerlessParticipationMode, OwnerlessValidatorV1, OwnerlessWorkerEligibilityV1,
    OwnerlessWorldIdV1, PeerId, MAX_OWNERLESS_HISTORY_RECORDS, MAX_OWNERLESS_PLAYERS, MAX_OWNERLESS_VALIDATORS,
    MAX_OWNERLESS_WORKERS,
};

use crate::ownerless::{validate_ownerless_roster, verify_ownerless_creation, verify_signature};
use crate::ownerless_participation::validate_participant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedOwnerlessHead {
    world: OwnerlessWorldIdV1,
    policy: Hash32,
    compatibility: Hash32,
    sequence: u64,
    hash: Hash32,
    fence: u64,
    checkpoint: Hash32,
    validators: Vec<OwnerlessValidatorV1>,
    participation_mode: OwnerlessParticipationMode,
    invited_players: Vec<OwnerlessParticipantV1>,
    eligible_workers: Vec<OwnerlessWorkerEligibilityV1>,
}

impl VerifiedOwnerlessHead {
    pub fn world(&self) -> OwnerlessWorldIdV1 {
        self.world
    }
    pub fn policy(&self) -> Hash32 {
        self.policy
    }
    pub fn compatibility(&self) -> Hash32 {
        self.compatibility
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn hash(&self) -> Hash32 {
        self.hash
    }
    pub fn fence(&self) -> u64 {
        self.fence
    }
    pub fn checkpoint(&self) -> Hash32 {
        self.checkpoint
    }
    pub fn validators(&self) -> &[OwnerlessValidatorV1] {
        &self.validators
    }
    pub fn participation_mode(&self) -> OwnerlessParticipationMode {
        self.participation_mode
    }
    pub fn invited_players(&self) -> &[OwnerlessParticipantV1] {
        &self.invited_players
    }
    pub fn eligible_workers(&self) -> &[OwnerlessWorkerEligibilityV1] {
        &self.eligible_workers
    }
    pub fn threshold(&self) -> usize {
        majority(&self.validators)
    }

    pub fn draft(&self, action: OwnerlessAuthorityActionV1) -> Result<OwnerlessAuthorityProposalV1, OwnerlessError> {
        let proposal = OwnerlessAuthorityProposalV1 {
            world_id: self.world,
            sequence: self.sequence.checked_add(1).ok_or(OwnerlessError::Invalid("sequence exhausted"))?,
            parent: self.hash,
            policy_hash: self.policy,
            compatibility: self.compatibility,
            base_fence: self.fence,
            base_checkpoint: self.checkpoint,
            action,
        };
        self.validate_proposal(&proposal)?;
        Ok(proposal)
    }

    pub fn validate_proposal(&self, proposal: &OwnerlessAuthorityProposalV1) -> Result<(), OwnerlessError> {
        if self.sequence >= MAX_OWNERLESS_HISTORY_RECORDS as u64
            || Some(proposal.sequence) != self.sequence.checked_add(1)
            || proposal.world_id != self.world
            || proposal.parent != self.hash
            || proposal.policy_hash != self.policy
            || proposal.compatibility != self.compatibility
            || proposal.base_fence != self.fence
            || proposal.base_checkpoint != self.checkpoint
        {
            return Err(OwnerlessError::Invalid("proposal does not extend exact accepted ownerless head"));
        }
        match &proposal.action {
            OwnerlessAuthorityActionV1::Checkpoint { commitment } => {
                if *commitment == Hash32::default() || *commitment == self.checkpoint {
                    return Err(OwnerlessError::Invalid("checkpoint commitment must be nonzero and new"));
                }
            }
            OwnerlessAuthorityActionV1::Recover => {
                self.fence.checked_add(1).ok_or(OwnerlessError::Invalid("fence exhausted"))?;
            }
            OwnerlessAuthorityActionV1::Reconfigure { validators } => {
                validate_ownerless_roster(validators)?;
                if validators == &self.validators {
                    return Err(OwnerlessError::Invalid("validator roster unchanged"));
                }
                // An overlapping peer identity must continue to bind the exact key.
                for v in validators {
                    if let Some(old) = find(&self.validators, v.peer_id) {
                        if old != v {
                            return Err(OwnerlessError::Invalid("validator identity key changed"));
                        }
                    }
                }
                self.fence.checked_add(1).ok_or(OwnerlessError::Invalid("fence exhausted"))?;
            }
            OwnerlessAuthorityActionV1::SetPlayer { player, allowed } => {
                validate_participant(player)?;
                if self.participation_mode != OwnerlessParticipationMode::InviteOnly {
                    return Err(OwnerlessError::Invalid("public participation does not use private invitations"));
                }
                match self.invited_players.binary_search_by_key(&player.peer_id, |p| p.peer_id) {
                    Ok(i) if !*allowed && self.invited_players[i] == *player => {}
                    Err(_) if *allowed && self.invited_players.len() < MAX_OWNERLESS_PLAYERS => {}
                    _ => return Err(OwnerlessError::Invalid("unchanged or invalid player eligibility")),
                }
                self.fence.checked_add(1).ok_or(OwnerlessError::Invalid("fence exhausted"))?;
            }
            OwnerlessAuthorityActionV1::SetWorker { worker, maximum } => {
                validate_participant(worker)?;
                if let Some(budget) = maximum {
                    budget.validate()?;
                }
                match self.eligible_workers.binary_search_by_key(&worker.peer_id, |w| w.worker.peer_id) {
                    Ok(i)
                        if self.eligible_workers[i].worker == *worker
                            && maximum.as_ref() != Some(&self.eligible_workers[i].maximum) => {}
                    Err(_) if maximum.is_some() && self.eligible_workers.len() < MAX_OWNERLESS_WORKERS => {}
                    _ => return Err(OwnerlessError::Invalid("unchanged or invalid worker eligibility")),
                }
                self.fence.checked_add(1).ok_or(OwnerlessError::Invalid("fence exhausted"))?;
            }
        }
        Ok(())
    }

    pub fn candidate_checkpoint(&self, proposal: &OwnerlessAuthorityProposalV1) -> Result<Hash32, OwnerlessError> {
        self.validate_proposal(proposal)?;
        Ok(match &proposal.action {
            OwnerlessAuthorityActionV1::Checkpoint { commitment } => *commitment,
            _ => self.checkpoint,
        })
    }
}

fn majority(roster: &[OwnerlessValidatorV1]) -> usize {
    roster.len() / 2 + 1
}
fn find(roster: &[OwnerlessValidatorV1], peer: PeerId) -> Option<&OwnerlessValidatorV1> {
    roster.binary_search_by_key(&peer, |v| v.peer_id).ok().map(|i| &roster[i])
}

/// Current validators, plus explicitly proposed entrants, may sign a joint
/// transition. A new key may not sign any other operation before enrollment.
pub fn ownerless_vote_key(
    head: &VerifiedOwnerlessHead,
    proposal: &OwnerlessAuthorityProposalV1,
    peer: PeerId,
) -> Result<[u8; 32], OwnerlessError> {
    head.validate_proposal(proposal)?;
    if let Some(v) = find(&head.validators, peer) {
        return Ok(v.public_key);
    }
    if let OwnerlessAuthorityActionV1::Reconfigure { validators } = &proposal.action {
        if let Some(v) = find(validators, peer) {
            return Ok(v.public_key);
        }
    }
    Err(OwnerlessError::Invalid("not an eligible ownerless validator"))
}

pub fn verify_ownerless_vote(
    head: &VerifiedOwnerlessHead,
    proposal: &OwnerlessAuthorityProposalV1,
    vote: &OwnerlessAuthorityVoteV1,
) -> Result<(), OwnerlessError> {
    let key = ownerless_vote_key(head, proposal, vote.validator)?;
    if vote.proposal_hash != proposal.proposal_hash()?
        || !verify_signature(&key, &proposal.vote_bytes(vote.validator)?, &vote.signature)
    {
        return Err(OwnerlessError::Signature);
    }
    Ok(())
}

pub fn apply_ownerless_certificate(
    head: &VerifiedOwnerlessHead,
    certificate: &OwnerlessAuthorityCertificateV1,
) -> Result<VerifiedOwnerlessHead, OwnerlessError> {
    let proposal = &certificate.proposal;
    head.validate_proposal(proposal)?;
    let votes = &certificate.votes;
    if votes.len() > MAX_OWNERLESS_VALIDATORS * 2 || votes.windows(2).any(|w| w[0].validator >= w[1].validator) {
        return Err(OwnerlessError::Invalid("noncanonical ownerless vote set"));
    }
    for vote in votes {
        verify_ownerless_vote(head, proposal, vote)?;
    }
    let voted = |peer| votes.binary_search_by_key(&peer, |v| v.validator).is_ok();
    let count = |roster: &[OwnerlessValidatorV1]| roster.iter().filter(|v| voted(v.peer_id)).count();
    if count(&head.validators) < majority(&head.validators) {
        return Err(OwnerlessError::Invalid("missing current validator majority"));
    }
    let mut next = head.clone();
    next.sequence = proposal.sequence;
    next.hash = proposal.proposal_hash()?;
    match &proposal.action {
        OwnerlessAuthorityActionV1::Checkpoint { commitment } => next.checkpoint = *commitment,
        OwnerlessAuthorityActionV1::Recover => next.fence += 1,
        OwnerlessAuthorityActionV1::Reconfigure { validators } => {
            if count(validators) < majority(validators) {
                return Err(OwnerlessError::Invalid("missing proposed validator majority"));
            }
            if validators.iter().any(|v| find(&head.validators, v.peer_id).is_none() && !voted(v.peer_id)) {
                return Err(OwnerlessError::Invalid("new validator has not explicitly approved enrollment"));
            }
            next.validators = validators.clone();
            next.fence += 1;
        }
        OwnerlessAuthorityActionV1::SetPlayer { player, allowed } => {
            let position = next.invited_players.binary_search_by_key(&player.peer_id, |p| p.peer_id);
            match (position, allowed) {
                (Err(i), true) => next.invited_players.insert(i, *player),
                (Ok(i), false) => {
                    next.invited_players.remove(i);
                }
                _ => return Err(OwnerlessError::Invalid("invalid player transition")),
            }
            next.fence += 1;
        }
        OwnerlessAuthorityActionV1::SetWorker { worker, maximum } => {
            let position = next.eligible_workers.binary_search_by_key(&worker.peer_id, |w| w.worker.peer_id);
            match (position, maximum) {
                (Ok(i), Some(maximum)) => next.eligible_workers[i].maximum = *maximum,
                (Err(i), Some(maximum)) => {
                    next.eligible_workers.insert(i, OwnerlessWorkerEligibilityV1 { worker: *worker, maximum: *maximum })
                }
                (Ok(i), None) => {
                    next.eligible_workers.remove(i);
                }
                _ => return Err(OwnerlessError::Invalid("invalid worker transition")),
            }
            next.fence += 1;
        }
    }
    Ok(next)
}

pub fn verify_ownerless_history(
    expected: OwnerlessWorldIdV1,
    history: &OwnerlessAuthorityHistoryV1,
) -> Result<VerifiedOwnerlessHead, OwnerlessError> {
    if history.records.len() > MAX_OWNERLESS_HISTORY_RECORDS {
        return Err(OwnerlessError::Invalid("ownerless history exceeds reference limit"));
    }
    let creation = verify_ownerless_creation(expected, &history.creation)?;
    let genesis = &history.creation.genesis;
    let mut head = VerifiedOwnerlessHead {
        world: creation.world_id(),
        policy: creation.policy_hash(),
        compatibility: genesis.compatibility_fingerprint,
        sequence: 0,
        hash: ownerless_genesis_head(expected)?,
        fence: 1,
        checkpoint: genesis.initial_state_commitment,
        validators: genesis.validators.clone(),
        participation_mode: if genesis.policy.authenticated_invite_only {
            OwnerlessParticipationMode::InviteOnly
        } else {
            OwnerlessParticipationMode::PublicAuthenticated
        },
        invited_players: Vec::new(),
        eligible_workers: Vec::new(),
    };
    for certificate in &history.records {
        head = apply_ownerless_certificate(&head, certificate)?;
    }
    Ok(head)
}

/// Witness subsets are not history identities. Compare exact proposals, not
/// their interchangeable valid certificate witness sets. Callers MUST verify
/// both chains first.
pub fn ownerless_history_extends(old: &OwnerlessAuthorityHistoryV1, new: &OwnerlessAuthorityHistoryV1) -> bool {
    old.creation.genesis == new.creation.genesis
        && old.records.len() <= new.records.len()
        && old.records.iter().zip(&new.records).all(|(a, b)| a.proposal == b.proposal)
}
