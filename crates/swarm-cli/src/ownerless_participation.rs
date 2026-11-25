//! Child of the reference authority CLI. All local challenge state uses the
//! existing private store and its per-world lock, never the V1 world store.
use super::{
    bounded_file, encoded, history, identity, pin, print, private_file, publish, read, sync_dir, ProposalDocument,
    Store,
};
use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use swarm_consensus::ownerless_participation::{
    ownerless_participation_challenge, verify_ownerless_participation, verify_ownerless_participation_request,
    ReferenceParticipationEligibility,
};
use swarm_core::{random_nonce, DataPaths, PeerIdentity};
use swarm_protocol::{
    Hash32, OwnerlessAuthorityActionV1, OwnerlessParticipantV1, OwnerlessParticipationIntentV1,
    OwnerlessParticipationRequestV1, OwnerlessParticipationResponseV1, OwnerlessWorkerBudgetV1, OwnerlessWorldIdV1,
    MAX_OWNERLESS_DOCUMENT_BYTES,
};

const LIMIT: usize = MAX_OWNERLESS_DOCUMENT_BYTES;
const STATE_DOMAIN: &[u8] = b"swarmcraft/ownerless-participation/local-challenge/v1\0";

#[derive(Debug, Subcommand)]
pub enum ParticipationCommand {
    /// Draft a majority-certified private invitation or revocation.
    ProposePlayer {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        player_key: String,
        #[arg(long)]
        allow: bool,
        #[arg(long)]
        revoke: bool,
    },
    /// Draft a separate majority-certified worker eligibility cap or revocation.
    ProposeWorker {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        worker_key: String,
        #[arg(long)]
        memory_mib: Option<u32>,
        #[arg(long)]
        cpu_millicores: Option<u32>,
        #[arg(long)]
        revoke: bool,
    },
    /// Issue a fresh signed reference challenge under this verifier's local head.
    /// Replaces the previous challenge, which cannot subsequently be consumed.
    Challenge {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        subject_key: String,
        #[arg(long)]
        play: bool,
        #[arg(long)]
        memory_mib: Option<u32>,
        #[arg(long)]
        cpu_millicores: Option<u32>,
    },
    /// Explicitly respond using this subject's existing identity.
    Respond {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        expected_challenge: Hash32,
        #[arg(long)]
        confirm: bool,
        /// Separate affirmative compute consent; must never be implied by play.
        #[arg(long)]
        confirm_volunteer: bool,
    },
    /// Consume the exact locally issued proof once, against the current local head.
    Verify {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        expected_challenge: Hash32,
        #[arg(long)]
        response: PathBuf,
    },
    /// Cancel the exact outstanding local challenge, without granting any role.
    Cancel {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        expected_challenge: Hash32,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestDocument {
    challenge_hash: String,
    request: OwnerlessParticipationRequestV1,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeState {
    request: OwnerlessParticipationRequestV1,
    consumed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedChallengeState {
    state: ChallengeState,
    signature: Vec<u8>,
}

fn participant(key: &str) -> Result<OwnerlessParticipantV1> {
    ensure!(key.len() == 64, "participant key must be exactly 64 hex characters");
    let key: [u8; 32] = hex::decode(key)?.try_into().map_err(|_| anyhow::anyhow!("invalid participant key length"))?;
    Ok(OwnerlessParticipantV1::from_key(key))
}

fn budget(memory_mib: Option<u32>, cpu_millicores: Option<u32>) -> Result<OwnerlessWorkerBudgetV1> {
    let result = OwnerlessWorkerBudgetV1 {
        memory_mib: memory_mib.context("explicit memory budget required")?,
        cpu_millicores: cpu_millicores.context("explicit CPU budget required")?,
    };
    result.validate()?;
    Ok(result)
}

fn state_bytes(state: &ChallengeState) -> Result<Vec<u8>> {
    let mut bytes = STATE_DOMAIN.to_vec();
    bytes.extend(encoded(state, LIMIT)?);
    Ok(bytes)
}

fn challenge_path(store: &Store) -> PathBuf {
    store.dir.join("participation-challenge.json")
}

fn load_state(store: &Store) -> Result<ChallengeState> {
    let file = private_file(&challenge_path(store))?;
    file.sync_all()?;
    sync_dir(&store.dir)?;
    let signed: SignedChallengeState =
        serde_json::from_slice(&bounded_file(file, LIMIT)?).context("invalid local participation challenge state")?;
    let signature = Signature::from_slice(&signed.signature)?;
    let key = VerifyingKey::from_bytes(&store.identity.public_key())?;
    key.verify_strict(&state_bytes(&signed.state)?, &signature)
        .context("local participation challenge signature invalid")?;
    ensure!(
        signed.state.request.challenge.verifier == OwnerlessParticipantV1::from_key(store.identity.public_key()),
        "participation challenge verifier mismatch"
    );
    Ok(signed.state)
}

fn save_state(store: &Store, state: ChallengeState, replace: bool) -> Result<()> {
    let signature = store.identity.sign(&state_bytes(&state)?);
    publish(&challenge_path(store), &encoded(&SignedChallengeState { state, signature }, LIMIT)?, replace)
}

fn issue_state(store: &Store, request: OwnerlessParticipationRequestV1) -> Result<()> {
    let replace = match fs::symlink_metadata(challenge_path(store)) {
        Ok(_) => {
            load_state(store)?;
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    save_state(store, ChallengeState { request, consumed: false }, replace)
}

fn request_pin(request: &OwnerlessParticipationRequestV1, expected: Hash32) -> Result<()> {
    ensure!(request.challenge.challenge_hash()? == expected, "participation challenge differs from expected pin");
    Ok(())
}

fn subject_response(
    identity: &PeerIdentity,
    request: OwnerlessParticipationRequestV1,
) -> Result<OwnerlessParticipationResponseV1> {
    ensure!(
        request.challenge.subject == OwnerlessParticipantV1::from_key(identity.public_key()),
        "local identity is not the reviewed participation subject"
    );
    let signature = identity.sign(&request.challenge.response_bytes()?);
    Ok(OwnerlessParticipationResponseV1 { challenge: request.challenge, signature })
}

fn draft(
    file: PathBuf,
    world: OwnerlessWorldIdV1,
    expected_head: Hash32,
    action: OwnerlessAuthorityActionV1,
) -> Result<()> {
    let (_, head) = history(&file, world)?;
    pin(&head, expected_head)?;
    let proposal = head.draft(action)?;
    print(&ProposalDocument { proposal_hash: proposal.proposal_hash()?.to_string(), proposal }, LIMIT)
}

pub(super) fn run(paths: &DataPaths, command: ParticipationCommand) -> Result<()> {
    use ParticipationCommand::*;
    match command {
        ProposePlayer { history, expected_world, expected_head, player_key, allow, revoke } => {
            ensure!(allow != revoke, "choose exactly one of --allow or --revoke");
            draft(
                history,
                expected_world,
                expected_head,
                OwnerlessAuthorityActionV1::SetPlayer { player: participant(&player_key)?, allowed: allow },
            )
        }
        ProposeWorker { history, expected_world, expected_head, worker_key, memory_mib, cpu_millicores, revoke } => {
            let maximum = if revoke {
                ensure!(
                    memory_mib.is_none() && cpu_millicores.is_none(),
                    "revocation cannot include a resource budget"
                );
                None
            } else {
                Some(budget(memory_mib, cpu_millicores)?)
            };
            draft(
                history,
                expected_world,
                expected_head,
                OwnerlessAuthorityActionV1::SetWorker { worker: participant(&worker_key)?, maximum },
            )
        }
        Challenge { expected_world, expected_head, subject_key, play, memory_mib, cpu_millicores } => {
            let intent = if play {
                ensure!(memory_mib.is_none() && cpu_millicores.is_none(), "play cannot include a compute offer");
                OwnerlessParticipationIntentV1::Play
            } else {
                OwnerlessParticipationIntentV1::Volunteer { budget: budget(memory_mib, cpu_millicores)? }
            };
            let store = Store::open(paths, expected_world)?;
            let (_, head) = store.load(expected_world)?;
            pin(&head, expected_head)?;
            let challenge = ownerless_participation_challenge(
                &head,
                OwnerlessParticipantV1::from_key(store.identity.public_key()),
                participant(&subject_key)?,
                random_nonce(),
                intent,
            )?;
            let signature = store.identity.sign(&challenge.request_bytes()?);
            let request = OwnerlessParticipationRequestV1 { challenge, signature };
            verify_ownerless_participation_request(&head, &request)?;
            // Publish before exposing the issued challenge.
            issue_state(&store, request.clone())?;
            print(&RequestDocument { challenge_hash: request.challenge.challenge_hash()?.to_string(), request }, LIMIT)
        }
        Respond {
            history: file,
            expected_world,
            expected_head,
            request,
            expected_challenge,
            confirm,
            confirm_volunteer,
        } => {
            ensure!(confirm, "explicit --confirm required after reviewing the challenge");
            let (_, head) = history(&file, expected_world)?;
            pin(&head, expected_head)?;
            let document: RequestDocument = read(&request, LIMIT)?;
            ensure!(
                document.challenge_hash == expected_challenge.to_string(),
                "participation challenge differs from expected pin"
            );
            request_pin(&document.request, expected_challenge)?;
            verify_ownerless_participation_request(&head, &document.request)?;
            let volunteer =
                matches!(document.request.challenge.intent, OwnerlessParticipationIntentV1::Volunteer { .. });
            ensure!(
                volunteer == confirm_volunteer,
                "compute offer requires separate --confirm-volunteer; play must not include it"
            );
            let response = subject_response(&identity(paths)?, document.request)?;
            print(&response, LIMIT)
        }
        Verify { expected_world, expected_head, expected_challenge, response } => {
            let store = Store::open(paths, expected_world)?;
            let (_, head) = store.load(expected_world)?;
            pin(&head, expected_head)?;
            let mut state = load_state(&store)?;
            request_pin(&state.request, expected_challenge)?;
            ensure!(!state.consumed, "participation challenge already consumed or canceled");
            verify_ownerless_participation_request(&head, &state.request)?;
            let response: OwnerlessParticipationResponseV1 = read(&response, LIMIT)?;
            let eligibility = verify_ownerless_participation(&head, &state.request.challenge, &response)?;
            let (role, offered_budget) = match eligibility {
                ReferenceParticipationEligibility::Player => ("reference_player", None),
                ReferenceParticipationEligibility::Volunteer { budget } => ("reference_volunteer", Some(budget)),
            };
            let subject = state.request.challenge.subject;
            // Keep consumed state durable before any successful output. A
            // lost response must not permit consuming the same proof again.
            state.consumed = true;
            save_state(&store, state, true)?;
            print(
                &serde_json::json!({
                    "classification": "verified_ownerless_reference_participation",
                    "role": role,
                    "world_id": head.world().to_string(),
                    "head": head.hash().to_string(),
                    "challenge": expected_challenge.to_string(),
                    "subject": subject.peer_id.to_string(),
                    "offered_budget": offered_budget,
                    "validator_already_enrolled": head.validators().iter().any(|v| v.peer_id == subject.peer_id && v.public_key == subject.public_key),
                    "validator_granted": false, "write_authority": false, "live_quorum_verified": false,
                    "resource_enforcement_verified": false, "runtime_activation": false,
                    "worker_admission": false, "player_admission": false,
                }),
                LIMIT,
            )
        }
        Cancel { expected_world, expected_head, expected_challenge } => {
            let store = Store::open(paths, expected_world)?;
            let (_, head) = store.load(expected_world)?;
            pin(&head, expected_head)?;
            let mut state = load_state(&store)?;
            request_pin(&state.request, expected_challenge)?;
            ensure!(!state.consumed, "participation challenge already consumed or canceled");
            // Even a challenge from an older head can be canceled. It
            // cannot be verified under the current accepted head.
            state.consumed = true;
            save_state(&store, state, true)?;
            print(
                &serde_json::json!({
                    "classification": "ownerless_reference_challenge_canceled",
                    "challenge": expected_challenge.to_string(), "runtime_activation": false,
                }),
                LIMIT,
            )
        }
    }
}
