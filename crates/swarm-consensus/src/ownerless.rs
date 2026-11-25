//! Real Ed25519 checks for the isolated ownerless creation contract.
//! Verification is not canonical acceptance and cannot authorize execution.
use ed25519_dalek::{Signature, VerifyingKey};
use swarm_protocol::{
    Hash32, OwnerlessCreationApprovalV1, OwnerlessCreationV1, OwnerlessError, OwnerlessGenesisV1, OwnerlessValidatorV1,
    OwnerlessWorldIdV1, MAX_OWNERLESS_VALIDATORS, MIN_OWNERLESS_VALIDATORS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedOwnerlessCreationV1 {
    world_id: OwnerlessWorldIdV1,
    policy_hash: Hash32,
    maintenance_threshold: usize,
}

impl VerifiedOwnerlessCreationV1 {
    pub fn world_id(&self) -> OwnerlessWorldIdV1 {
        self.world_id
    }

    pub fn policy_hash(&self) -> Hash32 {
        self.policy_hash
    }

    pub fn maintenance_threshold(&self) -> usize {
        self.maintenance_threshold
    }
}

pub fn validate_ownerless_genesis(genesis: &OwnerlessGenesisV1) -> Result<(), OwnerlessError> {
    genesis.validate()?;
    validate_ownerless_roster(&genesis.validators)
}

pub fn validate_ownerless_roster(roster: &[OwnerlessValidatorV1]) -> Result<(), OwnerlessError> {
    if !(MIN_OWNERLESS_VALIDATORS..=MAX_OWNERLESS_VALIDATORS).contains(&roster.len())
        || roster.windows(2).any(|w| w[0].peer_id >= w[1].peer_id)
        || roster.iter().any(|v| *v != OwnerlessValidatorV1::from_key(v.public_key))
    {
        return Err(OwnerlessError::Invalid("invalid ownerless validator roster"));
    }
    for validator in roster {
        let key = VerifyingKey::from_bytes(&validator.public_key)
            .map_err(|_| OwnerlessError::Invalid("invalid validator public key"))?;
        // Reject noncanonical encodings and mixed-torsion aliases, not
        // just weak (small-order) keys. Roster uniqueness must count
        // canonical prime-order keys rather than alternate encodings.
        let point = key.to_edwards();
        if key.is_weak() || !point.is_torsion_free() || point.compress().to_bytes() != validator.public_key {
            return Err(OwnerlessError::Invalid("weak or noncanonical validator public key"));
        }
    }
    Ok(())
}

pub(crate) fn verify_signature(key: &[u8; 32], bytes: &[u8], signature: &[u8]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(key) else { return false };
    let Ok(signature) = Signature::from_slice(signature) else { return false };
    key.verify_strict(bytes, &signature).is_ok()
}

pub fn verify_ownerless_approval(
    genesis: &OwnerlessGenesisV1,
    expected: OwnerlessWorldIdV1,
    approval: &OwnerlessCreationApprovalV1,
) -> Result<(), OwnerlessError> {
    validate_ownerless_genesis(genesis)?;
    if genesis.world_id()? != expected || approval.world_id != expected {
        return Err(OwnerlessError::Invalid("wrong approval identity"));
    }
    let validator = genesis.validator(approval.validator).ok_or(OwnerlessError::Invalid("unknown validator"))?;
    if !verify_signature(&validator.public_key, &genesis.approval_bytes(approval.validator)?, &approval.signature) {
        return Err(OwnerlessError::Signature);
    }
    Ok(())
}

pub fn verify_ownerless_creation(
    expected: OwnerlessWorldIdV1,
    creation: &OwnerlessCreationV1,
) -> Result<VerifiedOwnerlessCreationV1, OwnerlessError> {
    validate_ownerless_genesis(&creation.genesis)?;
    creation.verify(expected, verify_signature)?;
    Ok(VerifiedOwnerlessCreationV1 {
        world_id: expected,
        policy_hash: creation.genesis.policy_hash()?,
        maintenance_threshold: creation.genesis.maintenance_threshold()?,
    })
}
