//! Cryptographic verifier for the isolated two-device exclusive-turn policy.
//! Verified history is not a live writer, runtime permit or proof of quiescence.
//! Crash-fault tolerance requires a separate durable single-writer journal.
use ed25519_dalek::VerifyingKey;
use swarm_protocol::{
    ExclusiveTurnApprovalV1, ExclusiveTurnCheckpointV1, ExclusiveTurnCreationV1, ExclusiveTurnDeviceV1,
    ExclusiveTurnError, ExclusiveTurnGenesisV1, ExclusiveTurnHistoryV1, ExclusiveTurnOfferV1, ExclusiveTurnReceiptV1,
    ExclusiveTurnReleaseV1, ExclusiveTurnSignedOfferV1, ExclusiveTurnWorldIdV1, Hash32, PeerId,
    MAX_EXCLUSIVE_TURN_RELEASES,
};

use crate::ownerless::verify_signature;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExclusiveTurnHead {
    world: ExclusiveTurnWorldIdV1,
    grant: Hash32,
    devices: [ExclusiveTurnDeviceV1; 2],
    holder: PeerId,
    epoch: u64,
    fence: u64,
    compatibility: Hash32,
    checkpoint: ExclusiveTurnCheckpointV1,
    // Retain both floors across alternating handoffs.
    attempts: [u64; 2],
}

impl VerifiedExclusiveTurnHead {
    pub fn world(&self) -> ExclusiveTurnWorldIdV1 {
        self.world
    }

    pub fn grant(&self) -> Hash32 {
        self.grant
    }

    pub fn devices(&self) -> &[ExclusiveTurnDeviceV1; 2] {
        &self.devices
    }

    pub fn holder(&self) -> PeerId {
        self.holder
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn fence(&self) -> u64 {
        self.fence
    }

    pub fn compatibility(&self) -> Hash32 {
        self.compatibility
    }

    pub fn checkpoint(&self) -> ExclusiveTurnCheckpointV1 {
        self.checkpoint
    }

    fn device_index(&self, peer: PeerId) -> Result<usize, ExclusiveTurnError> {
        self.devices
            .iter()
            .position(|device| device.peer_id == peer)
            .ok_or(ExclusiveTurnError::Invalid("peer is not a fixed device"))
    }

    pub fn attempt_floor(&self, peer: PeerId) -> Result<u64, ExclusiveTurnError> {
        Ok(self.attempts[self.device_index(peer)?])
    }

    /// The caller must independently obtain quiescence and the exact current
    /// checkpoint, and durably advance the local attempt before signing.
    pub fn draft_offer(
        &self,
        attempt: u64,
        checkpoint: ExclusiveTurnCheckpointV1,
    ) -> Result<ExclusiveTurnOfferV1, ExclusiveTurnError> {
        let recipient = self.devices[1 - self.device_index(self.holder)?].peer_id;
        let offer = ExclusiveTurnOfferV1 {
            world_id: self.world,
            parent_grant: self.grant,
            base_epoch: self.epoch,
            base_fence: self.fence,
            next_epoch: self.epoch.checked_add(1).ok_or(ExclusiveTurnError::Invalid("epoch exhausted"))?,
            next_fence: self.fence.checked_add(1).ok_or(ExclusiveTurnError::Invalid("fence exhausted"))?,
            source: self.holder,
            recipient,
            attempt,
            compatibility: self.compatibility,
            checkpoint,
        };
        self.validate_offer(&offer)?;
        Ok(offer)
    }

    fn validate_offer(&self, offer: &ExclusiveTurnOfferV1) -> Result<(), ExclusiveTurnError> {
        if self.epoch > MAX_EXCLUSIVE_TURN_RELEASES as u64
            || offer.world_id != self.world
            || offer.parent_grant != self.grant
            || offer.base_epoch != self.epoch
            || offer.base_fence != self.fence
            || Some(offer.next_epoch) != self.epoch.checked_add(1)
            || Some(offer.next_fence) != self.fence.checked_add(1)
            || offer.source != self.holder
            || offer.recipient != self.devices[1 - self.device_index(self.holder)?].peer_id
            || offer.compatibility != self.compatibility
            || offer.attempt <= self.attempt_floor(self.holder)?
        {
            return Err(ExclusiveTurnError::Invalid("handoff does not extend exact accepted grant and attempt"));
        }
        offer.checkpoint.validate()
    }
}

pub fn validate_exclusive_turn_genesis(genesis: &ExclusiveTurnGenesisV1) -> Result<(), ExclusiveTurnError> {
    genesis.validate()?;
    for device in &genesis.devices {
        let key = VerifyingKey::from_bytes(&device.public_key)
            .map_err(|_| ExclusiveTurnError::Invalid("invalid device public key"))?;
        let point = key.to_edwards();
        if key.is_weak() || !point.is_torsion_free() || point.compress().to_bytes() != device.public_key {
            return Err(ExclusiveTurnError::Invalid("weak or noncanonical device public key"));
        }
    }
    Ok(())
}

pub fn verify_exclusive_turn_approval(
    expected: ExclusiveTurnWorldIdV1,
    genesis: &ExclusiveTurnGenesisV1,
    approval: &ExclusiveTurnApprovalV1,
) -> Result<(), ExclusiveTurnError> {
    validate_exclusive_turn_genesis(genesis)?;
    if genesis.world_id()? != expected || approval.world_id != expected {
        return Err(ExclusiveTurnError::Invalid("approval does not match independently pinned identity"));
    }
    let device = genesis.device(approval.device).ok_or(ExclusiveTurnError::Invalid("approval from outsider"))?;
    if !verify_signature(&device.public_key, &genesis.approval_bytes(device.peer_id)?, &approval.signature) {
        return Err(ExclusiveTurnError::Signature);
    }
    Ok(())
}

pub fn verify_exclusive_turn_creation(
    expected: ExclusiveTurnWorldIdV1,
    creation: &ExclusiveTurnCreationV1,
) -> Result<VerifiedExclusiveTurnHead, ExclusiveTurnError> {
    let genesis = &creation.genesis;
    validate_exclusive_turn_genesis(genesis)?;
    if genesis.world_id()? != expected {
        return Err(ExclusiveTurnError::Invalid("creation does not match independently pinned identity"));
    }
    for (device, approval) in genesis.devices.iter().zip(&creation.approvals) {
        if approval.world_id != expected || approval.device != device.peer_id {
            return Err(ExclusiveTurnError::Invalid("both fixed devices must approve in roster order"));
        }
        if !verify_signature(&device.public_key, &genesis.approval_bytes(device.peer_id)?, &approval.signature) {
            return Err(ExclusiveTurnError::Signature);
        }
    }
    Ok(VerifiedExclusiveTurnHead {
        world: expected,
        grant: genesis.initial_grant_hash()?,
        devices: genesis.devices,
        holder: genesis.initial_holder,
        epoch: 1,
        fence: 1,
        compatibility: genesis.compatibility,
        checkpoint: genesis.initial_checkpoint,
        attempts: [0; 2],
    })
}

pub fn verify_exclusive_turn_offer(
    head: &VerifiedExclusiveTurnHead,
    signed: &ExclusiveTurnSignedOfferV1,
) -> Result<(), ExclusiveTurnError> {
    let offer = &signed.offer;
    head.validate_offer(offer)?;
    let source = &head.devices[head.device_index(offer.source)?];
    if !verify_signature(&source.public_key, &offer.signing_bytes()?, &signed.signature) {
        return Err(ExclusiveTurnError::Signature);
    }
    Ok(())
}

pub fn verify_exclusive_turn_receipt(
    head: &VerifiedExclusiveTurnHead,
    offer: &ExclusiveTurnSignedOfferV1,
    receipt: &ExclusiveTurnReceiptV1,
) -> Result<(), ExclusiveTurnError> {
    verify_exclusive_turn_offer(head, offer)?;
    if receipt.world_id != head.world
        || receipt.offer_hash != offer.offer_hash()?
        || receipt.recipient != offer.offer.recipient
    {
        return Err(ExclusiveTurnError::Invalid("receipt does not acknowledge exact signed offer"));
    }
    let recipient = &head.devices[head.device_index(receipt.recipient)?];
    if !verify_signature(&recipient.public_key, &receipt.signing_bytes()?, &receipt.signature) {
        return Err(ExclusiveTurnError::Signature);
    }
    Ok(())
}

/// A valid release extends the immutable record history; only the durable
/// source-side relinquishment and target-side activation journal can make
/// the record operational. A historical record is never a lease.
pub fn apply_exclusive_turn_release(
    head: &VerifiedExclusiveTurnHead,
    release: &ExclusiveTurnReleaseV1,
) -> Result<VerifiedExclusiveTurnHead, ExclusiveTurnError> {
    verify_exclusive_turn_receipt(head, &release.offer, &release.receipt)?;
    let offer = &release.offer.offer;
    let source_index = head.device_index(offer.source)?;
    if !verify_signature(&head.devices[source_index].public_key, &release.signing_bytes()?, &release.signature) {
        return Err(ExclusiveTurnError::Signature);
    }
    let mut attempts = head.attempts;
    attempts[source_index] = offer.attempt;
    Ok(VerifiedExclusiveTurnHead {
        world: head.world,
        grant: release.grant_hash()?,
        devices: head.devices,
        holder: offer.recipient,
        epoch: offer.next_epoch,
        fence: offer.next_fence,
        compatibility: head.compatibility,
        checkpoint: offer.checkpoint,
        attempts,
    })
}

pub fn verify_exclusive_turn_history(
    expected: ExclusiveTurnWorldIdV1,
    history: &ExclusiveTurnHistoryV1,
) -> Result<VerifiedExclusiveTurnHead, ExclusiveTurnError> {
    history.validate_envelope()?;
    let mut head = verify_exclusive_turn_creation(expected, &history.creation)?;
    for release in &history.releases {
        head = apply_exclusive_turn_release(&head, release)?;
    }
    Ok(head)
}

/// Reverify both histories and require exact immutable ancestry. The same
/// signed genesis is necessary but insufficient: never select a competing
/// release chain or accept an old prefix as a new head.
pub fn exclusive_turn_history_extends(
    expected: ExclusiveTurnWorldIdV1,
    accepted: &ExclusiveTurnHistoryV1,
    candidate: &ExclusiveTurnHistoryV1,
) -> Result<VerifiedExclusiveTurnHead, ExclusiveTurnError> {
    verify_exclusive_turn_history(expected, accepted)?;
    let head = verify_exclusive_turn_history(expected, candidate)?;
    if accepted.creation != candidate.creation || !candidate.releases.starts_with(&accepted.releases) {
        return Err(ExclusiveTurnError::Invalid("history is regressed or divergent"));
    }
    Ok(head)
}
