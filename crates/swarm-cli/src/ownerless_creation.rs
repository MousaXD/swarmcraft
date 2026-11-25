//! Offline creation-certificate workflow; no canonical storage or runtime access.
use anyhow::{bail, ensure, Context, Result};
use clap::Subcommand;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use swarm_consensus::ownerless::{validate_ownerless_genesis, verify_ownerless_approval, verify_ownerless_creation};
use swarm_core::{random_nonce, DataPaths, PeerIdentity};
use swarm_protocol::{
    Hash32, OwnerlessCreationApprovalV1, OwnerlessCreationV1, OwnerlessGenesisV1, OwnerlessValidatorV1,
    OwnerlessWorldIdV1, MAX_OWNERLESS_DOCUMENT_BYTES, MAX_OWNERLESS_VALIDATORS, OWNERLESS_FORMAT_V1,
    OWNERLESS_NO_OP_V1,
};

#[derive(Debug, Subcommand)]
pub enum OwnerlessCommand {
    /// Draft an isolated no-OP creation proposal, not a playable world.
    Draft {
        #[arg(long)]
        name: String,
        #[arg(long)]
        compatibility_fingerprint: Hash32,
        #[arg(long)]
        initial_state_commitment: Hash32,
        /// Explicit validator public key (64 hex characters); repeat for every validator.
        #[arg(long = "validator-key", required = true)]
        validator_keys: Vec<String>,
    },
    /// Inspect the exact draft and its identity before independently approving.
    Inspect {
        #[arg(long)]
        draft: PathBuf,
    },
    /// Approve the exact reviewed draft with this device's EXISTING identity.
    Approve {
        #[arg(long)]
        draft: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        /// Explicit confirmation of the immutable policy and founding validator roster.
        #[arg(long)]
        confirm: bool,
    },
    /// Assemble unanimous approvals into a verified, non-activatable creation certificate.
    Assemble {
        #[arg(long)]
        draft: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long = "approval", required = true)]
        approvals: Vec<PathBuf>,
    },
    /// Verify a creation certificate against an independently reviewed identity pin.
    Verify {
        #[arg(long)]
        certificate: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
    },
}

fn read_document<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    ensure!(file.metadata()?.is_file(), "ownerless document must be a regular file");
    let mut raw = Vec::new();
    file.take((MAX_OWNERLESS_DOCUMENT_BYTES + 1) as u64).read_to_end(&mut raw)?;
    ensure!(raw.len() <= MAX_OWNERLESS_DOCUMENT_BYTES, "ownerless document exceeds size limit");
    serde_json::from_slice(&raw).context("invalid ownerless JSON document")
}

fn print_document(value: &impl Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    ensure!(json.len() < MAX_OWNERLESS_DOCUMENT_BYTES, "ownerless document exceeds size limit");
    println!("{json}");
    Ok(())
}

pub fn run(paths: &DataPaths, command: OwnerlessCommand) -> Result<()> {
    match command {
        OwnerlessCommand::Draft { name, compatibility_fingerprint, initial_state_commitment, validator_keys } => {
            ensure!(validator_keys.len() <= MAX_OWNERLESS_VALIDATORS, "too many validator keys");
            let mut validators = Vec::new();
            for key in validator_keys {
                ensure!(key.len() == 64, "validator key must be exactly 64 hex characters");
                let key: [u8; 32] = hex::decode(&key)?.try_into().map_err(|_| anyhow::anyhow!("invalid key length"))?;
                validators.push(OwnerlessValidatorV1::from_key(key));
            }
            validators.sort_by_key(|v| v.peer_id);
            let genesis = OwnerlessGenesisV1 {
                format_version: OWNERLESS_FORMAT_V1,
                name,
                creation_nonce: random_nonce(),
                compatibility_fingerprint,
                initial_state_commitment,
                policy: OWNERLESS_NO_OP_V1,
                validators,
            };
            validate_ownerless_genesis(&genesis)?;
            print_document(&genesis)
        }
        OwnerlessCommand::Inspect { draft } => {
            let genesis: OwnerlessGenesisV1 = read_document(&draft)?;
            validate_ownerless_genesis(&genesis)?;
            print_document(&serde_json::json!({
                "world_id": genesis.world_id()?.to_string(),
                "policy_hash": genesis.policy_hash()?.to_string(),
                "required_creation_approvals": genesis.validators.len(),
                "proposed_maintenance_threshold": genesis.maintenance_threshold()?,
                "genesis": genesis,
                "classification": "unapproved_creation_draft",
                "runtime_activation": false,
            }))
        }
        OwnerlessCommand::Approve { draft, expected_world, confirm } => {
            ensure!(confirm, "explicit --confirm required after reviewing the exact draft");
            let genesis: OwnerlessGenesisV1 = read_document(&draft)?;
            validate_ownerless_genesis(&genesis)?;
            ensure!(genesis.world_id()? == expected_world, "draft differs from expected ownerless identity");
            let identity = PeerIdentity::load_existing(paths)?;
            let voter = identity.peer_id();
            let Some(validator) = genesis.validator(voter) else {
                bail!("local identity is not an enrolled validator")
            };
            ensure!(validator.public_key == identity.public_key(), "validator key differs");
            let approval = OwnerlessCreationApprovalV1 {
                world_id: expected_world,
                validator: voter,
                signature: identity.sign(&genesis.approval_bytes(voter)?),
            };
            verify_ownerless_approval(&genesis, expected_world, &approval)?;
            print_document(&approval)
        }
        OwnerlessCommand::Assemble { draft, expected_world, approvals } => {
            let genesis: OwnerlessGenesisV1 = read_document(&draft)?;
            validate_ownerless_genesis(&genesis)?;
            ensure!(genesis.world_id()? == expected_world, "draft differs from expected ownerless identity");
            ensure!(approvals.len() == genesis.validators.len(), "one explicit approval required from every validator");
            let mut signed: Vec<OwnerlessCreationApprovalV1> =
                approvals.iter().map(|path| read_document(path)).collect::<Result<_>>()?;
            signed.sort_by_key(|approval| approval.validator);
            let certificate = OwnerlessCreationV1 { genesis, approvals: signed };
            verify_ownerless_creation(expected_world, &certificate)?;
            print_document(&certificate)
        }
        OwnerlessCommand::Verify { certificate, expected_world } => {
            let certificate: OwnerlessCreationV1 = read_document(&certificate)?;
            let verified = verify_ownerless_creation(expected_world, &certificate)?;
            print_document(&serde_json::json!({
                "classification": "verified_ownerless_creation_candidate",
                "world_id": verified.world_id().to_string(),
                "policy_hash": verified.policy_hash().to_string(),
                "verified_creation_approvals": certificate.approvals.len(),
                "proposed_maintenance_threshold": verified.maintenance_threshold(),
                "policy": certificate.genesis.policy,
                "canonical_world_created": false,
                "runtime_activation": false,
                "recovery_authorized": false,
            }))
        }
    }
}
