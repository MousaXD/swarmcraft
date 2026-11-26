//! Isolated #66 reference authority, not live game admission or V1 storage.
//! The Linux lab uses one signed, durable journal per existing local identity
//! and ownerless world. Never clear a vote on timeout or missing state.
use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use ed25519_dalek::{Signature, VerifyingKey};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use swarm_consensus::ownerless_authority::{
    apply_ownerless_certificate, ownerless_history_extends, ownerless_vote_key, verify_ownerless_history,
    verify_ownerless_vote, VerifiedOwnerlessHead,
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{
    ownerless_checkpoint_commitment, Hash32, OwnerlessAuthorityActionV1, OwnerlessAuthorityCertificateV1,
    OwnerlessAuthorityHistoryV1, OwnerlessAuthorityProposalV1, OwnerlessAuthorityVoteV1, OwnerlessCreationV1,
    OwnerlessValidatorV1, OwnerlessWorldIdV1, PeerId, MAX_OWNERLESS_CHECKPOINT_BYTES, MAX_OWNERLESS_DOCUMENT_BYTES,
    MAX_OWNERLESS_HISTORY_BYTES, MAX_OWNERLESS_VALIDATORS,
};

#[path = "ownerless_decision.rs"]
mod decision;
#[path = "ownerless_observation.rs"]
mod observation;
#[path = "ownerless_participation.rs"]
mod participation;

const JOURNAL_LIMIT: usize = MAX_OWNERLESS_HISTORY_BYTES + MAX_OWNERLESS_DOCUMENT_BYTES * 2;
const JOURNAL_DOMAIN: &[u8] = b"swarmcraft/ownerless-authority/local-journal/v1\0";
const STORE: &str = "ownerless-authority-lab-v1";

#[derive(Debug, Subcommand)]
pub enum AuthorityCommand {
    /// Explicitly reviewed, encrypted reference decision exchange; no runtime authority.
    Decision {
        #[command(subcommand)]
        command: decision::DecisionCommand,
    },
    /// Bounded signed TCP reference head observations; never a lease or permit.
    Observe {
        #[command(subcommand)]
        command: observation::ObservationCommand,
    },
    /// Separate reference player/worker eligibility and single-use identity proofs.
    Participation {
        #[command(subcommand)]
        command: participation::ParticipationCommand,
    },
    /// Compute the bounded reference checkpoint commitment for a genesis or proposal.
    StateHash {
        #[arg(long)]
        checkpoint: PathBuf,
    },
    /// Build an initial history from the unanimous creation certificate.
    GenesisHistory {
        #[arg(long)]
        certificate: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
    },
    /// Verify the whole reference history; no freshness or execution authorization.
    Inspect {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
    },
    /// Explicitly draft exactly one checkpoint, recovery or joint roster transition.
    Propose {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        checkpoint: Option<PathBuf>,
        #[arg(long)]
        recover: bool,
        #[arg(long = "validator-key")]
        validator_keys: Vec<String>,
    },
    /// Create a new isolated journal under an EXISTING identity, once only.
    Init {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        checkpoint: PathBuf,
    },
    /// Vote once for the reviewed proposal, persisting the lock before output.
    Vote {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        proposal: PathBuf,
        #[arg(long)]
        expected_proposal: Hash32,
        #[arg(long)]
        checkpoint: PathBuf,
        #[arg(long)]
        confirm: bool,
    },
    /// Assemble a verified candidate history from explicit validator votes.
    Assemble {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        proposal: PathBuf,
        #[arg(long)]
        expected_proposal: Hash32,
        #[arg(long = "vote", required = true)]
        votes: Vec<PathBuf>,
    },
    /// Accept a fully verified extension and its exact checkpoint bytes.
    Accept {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        checkpoint: PathBuf,
    },
    /// Inspect the local verified head and pending vote; no live quorum claim.
    Status {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
    },
    /// Export the local accepted proof, not a fresh authority lease.
    ExportHistory {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
    },
    /// Export exact locally verified checkpoint bytes without write authority.
    RecoveryCopy {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingVote {
    proposal: OwnerlessAuthorityProposalV1,
    vote: OwnerlessAuthorityVoteV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: u16,
    identity: PeerId,
    history: OwnerlessAuthorityHistoryV1,
    pending: Option<PendingVote>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedJournal {
    journal: Journal,
    signature: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalDocument {
    proposal_hash: String,
    proposal: OwnerlessAuthorityProposalV1,
}

fn encoded<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= max, "ownerless document exceeds size limit");
    Ok(bytes)
}

fn print<T: Serialize>(value: &T, max: usize) -> Result<()> {
    let bytes = encoded(value, max - 1)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn bounded_file(mut file: File, max: usize) -> Result<Vec<u8>> {
    let metadata = file.metadata()?;
    ensure!(metadata.is_file() && metadata.len() <= max as u64, "invalid or oversized ownerless file");
    let mut data = Vec::new();
    (&mut file).take((max + 1) as u64).read_to_end(&mut data)?;
    ensure!(data.len() <= max, "ownerless file exceeds size limit");
    Ok(data)
}

fn open_regular(path: &Path) -> Result<File> {
    ensure!(
        fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?.is_file(),
        "ownerless input must be a regular non-symlink file"
    );
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path).with_context(|| format!("open {}", path.display()))
}

fn read<T: DeserializeOwned>(path: &Path, max: usize) -> Result<T> {
    serde_json::from_slice(&bounded_file(open_regular(path)?, max)?).context("invalid ownerless JSON document")
}

fn checkpoint(path: &Path, expected: Hash32) -> Result<Vec<u8>> {
    let bytes = bounded_file(open_regular(path)?, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    ensure!(ownerless_checkpoint_commitment(&bytes)? == expected, "reference checkpoint bytes do not match commitment");
    Ok(bytes)
}

fn history(path: &Path, world: OwnerlessWorldIdV1) -> Result<(OwnerlessAuthorityHistoryV1, VerifiedOwnerlessHead)> {
    let value = read(path, MAX_OWNERLESS_HISTORY_BYTES)?;
    let head = verify_ownerless_history(world, &value)?;
    Ok((value, head))
}

fn pin(head: &VerifiedOwnerlessHead, expected: Hash32) -> Result<()> {
    ensure!(head.hash() == expected, "ownerless accepted head differs from expected pin");
    Ok(())
}

fn proposal(path: &Path, expected: Hash32, head: &VerifiedOwnerlessHead) -> Result<OwnerlessAuthorityProposalV1> {
    let document: ProposalDocument = read(path, MAX_OWNERLESS_DOCUMENT_BYTES)?;
    ensure!(
        document.proposal_hash == expected.to_string() && document.proposal.proposal_hash()? == expected,
        "ownerless proposal differs from expected pin"
    );
    head.validate_proposal(&document.proposal)?;
    Ok(document.proposal)
}

fn report(head: &VerifiedOwnerlessHead, local: Option<&Journal>) -> serde_json::Value {
    serde_json::json!({
        "classification": "verified_ownerless_reference_head",
        "world_id": head.world().to_string(),
        "head": head.hash().to_string(),
        "sequence": head.sequence(),
        "reference_fence": head.fence(),
        "checkpoint": head.checkpoint().to_string(),
        "policy_hash": head.policy().to_string(),
        "compatibility": head.compatibility().to_string(),
        "validators": head.validators().iter().map(|v| serde_json::json!({
            "peer_id": v.peer_id.to_string(), "public_key": hex::encode(v.public_key)
        })).collect::<Vec<_>>(),
        "majority_threshold": head.threshold(),
        "participation_mode": head.participation_mode(),
        "invited_player_count": head.invited_players().len(),
        "eligible_worker_count": head.eligible_workers().len(),
        "local_identity": local.map(|j| j.identity.to_string()),
        "local_validator": local.is_some_and(|j| head.validators().iter().any(|v| v.peer_id == j.identity)),
        "pending_proposal": local.and_then(|j| j.pending.as_ref()).map(|p| p.vote.proposal_hash.to_string()),
        "live_quorum_verified": false,
        "runtime_activation": false,
        "player_admission": false,
        "v1_canonical_world_created": false,
    })
}

fn signing_bytes(journal: &Journal) -> Result<Vec<u8>> {
    let mut bytes = JOURNAL_DOMAIN.to_vec();
    bytes.extend(encoded(journal, JOURNAL_LIMIT)?);
    Ok(bytes)
}

fn seal(journal: &Journal, identity: &PeerIdentity) -> Result<Vec<u8>> {
    let signature = identity.sign(&signing_bytes(journal)?);
    let signed = SignedJournal { journal: journal.clone(), signature };
    encoded(&signed, JOURNAL_LIMIT)
}

fn check_journal(
    signed: SignedJournal,
    world: OwnerlessWorldIdV1,
    identity: &PeerIdentity,
) -> Result<(Journal, VerifiedOwnerlessHead)> {
    let j = signed.journal;
    ensure!(j.format == 1 && j.identity == identity.peer_id(), "ownerless journal identity mismatch");
    let signature = Signature::from_slice(&signed.signature)?;
    let key = VerifyingKey::from_bytes(&identity.public_key())?;
    key.verify_strict(&signing_bytes(&j)?, &signature).context("ownerless journal signature invalid")?;
    let head = verify_ownerless_history(world, &j.history)?;
    if let Some(pending) = &j.pending {
        ensure!(pending.vote.validator == j.identity, "foreign pending ownerless vote");
        verify_ownerless_vote(&head, &pending.proposal, &pending.vote)?;
    }
    Ok((j, head))
}

// This reference journal is intentionally supported on Linux only until the
// locking, directory fsync and no-follow semantics are qualified elsewhere.
fn private(metadata: &fs::Metadata, directory: bool) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            if directory { metadata.is_dir() } else { metadata.is_file() && metadata.nlink() == 1 },
            "unsafe ownerless journal file type or link count"
        );
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
            "ownerless journal requires owner-only local paths"
        );
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (metadata, directory);
        anyhow::bail!("durable ownerless reference journal is qualified on Linux only");
    }
}

fn check_dir(path: &Path) -> Result<()> {
    private(&fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?, true)
}

fn private_file(path: &Path) -> Result<File> {
    private(&fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?, false)?;
    let file = open_regular(path)?;
    private(&file.metadata()?, false)?;
    Ok(file)
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn create_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).with_context(|| format!("create isolated directory {}", path.display()))?;
    check_dir(path)?;
    sync_dir(path.parent().context("missing parent directory")?)?;
    Ok(())
}

fn publish(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    if replace {
        // Existing state must not have vanished or become a link.
        private_file(path)?;
        temp.persist(path).map_err(|e| e.error)?;
    } else {
        temp.persist_noclobber(path).map_err(|e| e.error)?;
    }
    sync_dir(parent)?;
    Ok(())
}

fn identity(paths: &DataPaths) -> Result<PeerIdentity> {
    check_dir(&paths.root)?;
    check_dir(&paths.identity_dir())?;
    // The lab never creates or repairs a voter identity.
    private_file(&paths.identity_key())?;
    Ok(PeerIdentity::load_existing(paths)?)
}

fn world_dir(paths: &DataPaths, world: OwnerlessWorldIdV1) -> Result<PathBuf> {
    let display = world.to_string();
    Ok(paths.root.join(STORE).join(display.strip_prefix("scownerless:").context("invalid ownerless identity")?))
}

struct Store {
    dir: PathBuf,
    _lock: File,
    identity: PeerIdentity,
}

impl Store {
    fn create(paths: &DataPaths, world: OwnerlessWorldIdV1) -> Result<Self> {
        let identity = identity(paths)?;
        let base = paths.root.join(STORE);
        match fs::symlink_metadata(&base) {
            Ok(_) => check_dir(&base)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Serialize different-world first initialization by accepting
                // an already-created directory only after validating it.
                if let Err(error) = create_dir(&base) {
                    check_dir(&base).with_context(|| format!("cannot create ownerless store: {error}"))?;
                }
                sync_dir(&paths.root)?;
            }
            Err(e) => return Err(e.into()),
        }
        let dir = world_dir(paths, world)?;
        // A missing journal in an existing world directory is not an empty
        // history. A partial/crashed initialization stays fail-closed.
        create_dir(&dir).context("ownerless journal already exists or cannot be initialized")?;
        create_dir(&dir.join("checkpoints"))?;
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let lock = opts.open(dir.join("journal.lock"))?;
        private(&lock.metadata()?, false)?;
        FileExt::lock_exclusive(&lock)?;
        lock.sync_all()?;
        sync_dir(&dir)?;
        Ok(Self { dir, _lock: lock, identity })
    }

    fn open(paths: &DataPaths, world: OwnerlessWorldIdV1) -> Result<Self> {
        Self::open_with_lock(paths, world, false)
    }

    fn try_open(paths: &DataPaths, world: OwnerlessWorldIdV1) -> Result<Self> {
        Self::open_with_lock(paths, world, true)
    }

    fn open_with_lock(paths: &DataPaths, world: OwnerlessWorldIdV1, nonblocking: bool) -> Result<Self> {
        let identity = identity(paths)?;
        let dir = world_dir(paths, world)?;
        check_dir(&paths.root.join(STORE))?;
        check_dir(&dir)?;
        check_dir(&dir.join("checkpoints"))?;
        let lock = private_file(&dir.join("journal.lock"))?;
        if nonblocking {
            FileExt::try_lock_exclusive(&lock).context("ownerless journal busy; observation refused")?;
        } else {
            FileExt::lock_exclusive(&lock)?;
        }
        Ok(Self { dir, _lock: lock, identity })
    }

    fn journal_path(&self) -> PathBuf {
        self.dir.join("journal.json")
    }
    fn state_path(&self, hash: Hash32) -> PathBuf {
        self.dir.join("checkpoints").join(format!("{hash}.bin"))
    }

    fn save(&self, journal: &Journal, replace: bool) -> Result<()> {
        publish(&self.journal_path(), &seal(journal, &self.identity)?, replace)
    }

    fn state(&self, hash: Hash32) -> Result<Vec<u8>> {
        let file = private_file(&self.state_path(hash))?;
        // An earlier publication may have been visible but returned a sync
        // error. Retry durability before relying on the bytes or reissuing votes.
        file.sync_all()?;
        sync_dir(&self.dir.join("checkpoints"))?;
        let bytes = bounded_file(file, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
        ensure!(ownerless_checkpoint_commitment(&bytes)? == hash, "stored ownerless checkpoint commitment mismatch");
        Ok(bytes)
    }

    fn retain(&self, hash: Hash32, bytes: &[u8]) -> Result<()> {
        ensure!(ownerless_checkpoint_commitment(bytes)? == hash, "reference checkpoint commitment mismatch");
        let path = self.state_path(hash);
        match fs::symlink_metadata(&path) {
            Ok(_) => ensure!(self.state(hash)? == bytes, "stored ownerless checkpoint differs"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => publish(&path, bytes, false)?,
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    fn persist_vote(
        &self,
        journal: &mut Journal,
        head: &VerifiedOwnerlessHead,
        proposal: OwnerlessAuthorityProposalV1,
        bytes: &[u8],
    ) -> Result<OwnerlessAuthorityVoteV1> {
        let key = ownerless_vote_key(head, &proposal, self.identity.peer_id())?;
        ensure!(key == self.identity.public_key(), "ownerless voter key mismatch");
        let commitment = head.candidate_checkpoint(&proposal)?;
        ensure!(
            ownerless_checkpoint_commitment(bytes)? == commitment,
            "reference checkpoint bytes do not match commitment"
        );
        if let Some(pending) = &journal.pending {
            ensure!(pending.proposal == proposal, "durable ownerless vote already locked to another proposal");
            ensure!(self.state(commitment)? == bytes, "stored pending checkpoint differs");
            return Ok(pending.vote.clone());
        }
        // Retain exact bytes, then durably sign and replace the journal.
        // A vote is returned only after the durable state is synchronized.
        self.retain(commitment, bytes)?;
        let voter = self.identity.peer_id();
        let vote = OwnerlessAuthorityVoteV1 {
            proposal_hash: proposal.proposal_hash()?,
            validator: voter,
            signature: self.identity.sign(&proposal.vote_bytes(voter)?),
        };
        verify_ownerless_vote(head, &proposal, &vote)?;
        journal.pending = Some(PendingVote { proposal, vote: vote.clone() });
        self.save(journal, true)?;
        Ok(vote)
    }

    fn load(&self, world: OwnerlessWorldIdV1) -> Result<(Journal, VerifiedOwnerlessHead)> {
        let file = private_file(&self.journal_path())?;
        file.sync_all()?;
        sync_dir(&self.dir)?;
        let signed =
            serde_json::from_slice(&bounded_file(file, JOURNAL_LIMIT)?).context("invalid durable ownerless journal")?;
        let (journal, head) = check_journal(signed, world, &self.identity)?;
        self.state(head.checkpoint())?;
        Ok((journal, head))
    }
}

pub fn run(paths: &DataPaths, command: AuthorityCommand) -> Result<()> {
    use AuthorityCommand::*;
    match command {
        Decision { command } => decision::run(paths, command),
        Observe { command } => observation::run(paths, command),
        Participation { command } => participation::run(paths, command),
        StateHash { checkpoint: file } => {
            let bytes = bounded_file(open_regular(&file)?, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
            print(
                &serde_json::json!({"checkpoint": ownerless_checkpoint_commitment(&bytes)?.to_string()}),
                MAX_OWNERLESS_DOCUMENT_BYTES,
            )
        }
        GenesisHistory { certificate, expected_world } => {
            let creation: OwnerlessCreationV1 = read(&certificate, MAX_OWNERLESS_DOCUMENT_BYTES)?;
            let history = OwnerlessAuthorityHistoryV1 { creation, records: Vec::new() };
            verify_ownerless_history(expected_world, &history)?;
            print(&history, MAX_OWNERLESS_HISTORY_BYTES)
        }
        Inspect { history: file, expected_world } => {
            let (_, head) = history(&file, expected_world)?;
            print(&report(&head, None), MAX_OWNERLESS_DOCUMENT_BYTES)
        }
        Propose { history: file, expected_world, expected_head, checkpoint: file_state, recover, validator_keys } => {
            let (_, head) = history(&file, expected_world)?;
            pin(&head, expected_head)?;
            ensure!(
                usize::from(file_state.is_some()) + usize::from(recover) + usize::from(!validator_keys.is_empty()) == 1,
                "choose exactly one checkpoint, recovery, or validator roster"
            );
            let action = if let Some(file) = file_state {
                let bytes = bounded_file(open_regular(&file)?, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
                OwnerlessAuthorityActionV1::Checkpoint { commitment: ownerless_checkpoint_commitment(&bytes)? }
            } else if recover {
                OwnerlessAuthorityActionV1::Recover
            } else {
                ensure!(validator_keys.len() <= MAX_OWNERLESS_VALIDATORS, "too many ownerless validators");
                let mut validators = Vec::new();
                for key in validator_keys {
                    ensure!(key.len() == 64, "validator key must be exactly 64 hex characters");
                    let bytes: [u8; 32] = hex::decode(key)?.try_into().map_err(|_| anyhow::anyhow!("invalid key"))?;
                    validators.push(OwnerlessValidatorV1::from_key(bytes));
                }
                validators.sort_by_key(|v| v.peer_id);
                OwnerlessAuthorityActionV1::Reconfigure { validators }
            };
            let proposal = head.draft(action)?;
            print(
                &ProposalDocument { proposal_hash: proposal.proposal_hash()?.to_string(), proposal },
                MAX_OWNERLESS_DOCUMENT_BYTES,
            )
        }
        Init { history: file, expected_world, expected_head, checkpoint: file_state } => {
            let (history, head) = history(&file, expected_world)?;
            pin(&head, expected_head)?;
            let bytes = checkpoint(&file_state, head.checkpoint())?;
            let store = Store::create(paths, expected_world)?;
            store.retain(head.checkpoint(), &bytes)?;
            let journal = Journal { format: 1, identity: store.identity.peer_id(), history, pending: None };
            store.save(&journal, false)?;
            print(&report(&head, Some(&journal)), MAX_OWNERLESS_DOCUMENT_BYTES)
        }
        Vote { expected_world, expected_head, proposal: file, expected_proposal, checkpoint: file_state, confirm } => {
            ensure!(confirm, "explicit --confirm required for reviewed ownerless vote");
            let store = Store::open(paths, expected_world)?;
            let (mut journal, head) = store.load(expected_world)?;
            pin(&head, expected_head)?;
            let proposal = proposal(&file, expected_proposal, &head)?;
            let commitment = head.candidate_checkpoint(&proposal)?;
            let bytes = checkpoint(&file_state, commitment)?;
            let vote = store.persist_vote(&mut journal, &head, proposal, &bytes)?;
            print(&vote, MAX_OWNERLESS_DOCUMENT_BYTES)
        }
        Assemble {
            history: file,
            expected_world,
            expected_head,
            proposal: file_proposal,
            expected_proposal,
            votes,
        } => {
            let (mut history, head) = history(&file, expected_world)?;
            pin(&head, expected_head)?;
            let proposal = proposal(&file_proposal, expected_proposal, &head)?;
            ensure!(votes.len() <= MAX_OWNERLESS_VALIDATORS * 2, "too many ownerless votes");
            let mut signed: Vec<OwnerlessAuthorityVoteV1> =
                votes.iter().map(|p| read(p, MAX_OWNERLESS_DOCUMENT_BYTES)).collect::<Result<_>>()?;
            signed.sort_by_key(|v| v.validator);
            let certificate = OwnerlessAuthorityCertificateV1 { proposal, votes: signed };
            apply_ownerless_certificate(&head, &certificate)?;
            history.records.push(certificate);
            print(&history, MAX_OWNERLESS_HISTORY_BYTES)
        }
        Accept { history: file, expected_world, expected_head, checkpoint: file_state } => {
            let (new, head) = history(&file, expected_world)?;
            pin(&head, expected_head)?;
            let bytes = checkpoint(&file_state, head.checkpoint())?;
            let store = Store::open(paths, expected_world)?;
            let (mut journal, prior) = store.load(expected_world)?;
            ensure!(
                ownerless_history_extends(&journal.history, &new),
                "ownerless history rollback or conflicting accepted ancestry"
            );
            if head.sequence() != prior.sequence() {
                store.retain(head.checkpoint(), &bytes)?;
                journal.history = new;
                journal.pending = None;
                store.save(&journal, true)?;
            } else {
                // A different valid witness subset for the same head is not
                // a new decision and MUST NOT clear a pending vote.
                ensure!(head.hash() == prior.hash(), "ownerless head identity differs");
            }
            print(&report(&head, Some(&journal)), MAX_OWNERLESS_DOCUMENT_BYTES)
        }
        Status { expected_world } => {
            let store = Store::open(paths, expected_world)?;
            let (journal, head) = store.load(expected_world)?;
            print(&report(&head, Some(&journal)), MAX_OWNERLESS_DOCUMENT_BYTES)
        }
        ExportHistory { expected_world } => {
            let store = Store::open(paths, expected_world)?;
            let (journal, _) = store.load(expected_world)?;
            print(&journal.history, MAX_OWNERLESS_HISTORY_BYTES)
        }
        RecoveryCopy { expected_world, expected_head, output } => {
            let store = Store::open(paths, expected_world)?;
            let (_, head) = store.load(expected_world)?;
            pin(&head, expected_head)?;
            let bytes = store.state(head.checkpoint())?;
            publish(&output, &bytes, false)?;
            print(
                &serde_json::json!({
                    "classification": "ownerless_reference_recovery_copy_only",
                    "world_id": expected_world.to_string(), "head": head.hash().to_string(),
                    "checkpoint": head.checkpoint().to_string(), "bytes": bytes.len(),
                    "runtime_activation": false, "write_authority": false
                }),
                MAX_OWNERLESS_DOCUMENT_BYTES,
            )
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{os::unix::process::ExitStatusExt, process::Command};
    use swarm_protocol::{OwnerlessCreationApprovalV1, OwnerlessGenesisV1, OWNERLESS_FORMAT_V1, OWNERLESS_NO_OP_V1};

    // Test-only subprocess deliberately dies after the real durable write,
    // while still holding the kernel lock and before any vote is emitted.
    // No production CLI flag or environment-controlled failpoint exists.
    #[test]
    fn hard_kill_after_durable_vote() {
        const INITIAL: &[u8] = b"accepted reference checkpoint";
        const CANDIDATE: &[u8] = b"durably locked candidate";
        if let Some(root) = std::env::var_os("SC66_TEST_CHILD_ROOT") {
            let world: OwnerlessWorldIdV1 = std::env::var("SC66_TEST_CHILD_WORLD").unwrap().parse().unwrap();
            let paths = DataPaths::from_root(root);
            let store = Store::open(&paths, world).unwrap();
            let (mut journal, head) = store.load(world).unwrap();
            let proposal = head
                .draft(OwnerlessAuthorityActionV1::Checkpoint {
                    commitment: ownerless_checkpoint_commitment(CANDIDATE).unwrap(),
                })
                .unwrap();
            let vote = store.persist_vote(&mut journal, &head, proposal, CANDIDATE).unwrap();
            assert_eq!(journal.pending.as_ref().unwrap().vote, vote);
            // Kill only this test helper. No signature has been emitted.
            unsafe {
                libc::raise(libc::SIGKILL);
            }
            unreachable!("SIGKILL returned");
        }

        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("peer"));
        let identities =
            [PeerIdentity::load_or_create(&paths).unwrap(), PeerIdentity::generate(), PeerIdentity::generate()];
        let mut validators: Vec<_> =
            identities.iter().map(|id| OwnerlessValidatorV1::from_key(id.public_key())).collect();
        validators.sort_by_key(|v| v.peer_id);
        let genesis = OwnerlessGenesisV1 {
            format_version: OWNERLESS_FORMAT_V1,
            name: "crash regression".into(),
            creation_nonce: [32; 32],
            compatibility_fingerprint: Hash32([37; 32]),
            initial_state_commitment: ownerless_checkpoint_commitment(INITIAL).unwrap(),
            policy: OWNERLESS_NO_OP_V1,
            validators,
        };
        let world = genesis.world_id().unwrap();
        let mut approvals: Vec<_> = identities
            .iter()
            .map(|id| {
                let validator = id.peer_id();
                OwnerlessCreationApprovalV1 {
                    world_id: world,
                    validator,
                    signature: id.sign(&genesis.approval_bytes(validator).unwrap()),
                }
            })
            .collect();
        approvals.sort_by_key(|a| a.validator);
        let history =
            OwnerlessAuthorityHistoryV1 { creation: OwnerlessCreationV1 { genesis, approvals }, records: vec![] };
        let initial = verify_ownerless_history(world, &history).unwrap();
        {
            let store = Store::create(&paths, world).unwrap();
            store.retain(initial.checkpoint(), INITIAL).unwrap();
            let journal = Journal { format: 1, identity: store.identity.peer_id(), history, pending: None };
            store.save(&journal, false).unwrap();
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ownerless_authority::tests::hard_kill_after_durable_vote", "--nocapture"])
            .env("SC66_TEST_CHILD_ROOT", &paths.root)
            .env("SC66_TEST_CHILD_WORLD", world.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.signal(),
            Some(libc::SIGKILL),
            "test helper failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let store = Store::open(&paths, world).unwrap();
        let (mut journal, head) = store.load(world).unwrap();
        assert_eq!(head, initial);
        let pending = journal.pending.clone().unwrap();
        assert_eq!(store.state(ownerless_checkpoint_commitment(CANDIDATE).unwrap()).unwrap(), CANDIDATE);
        let conflicting = head.draft(OwnerlessAuthorityActionV1::Recover).unwrap();
        assert!(store
            .persist_vote(&mut journal, &head, conflicting, INITIAL)
            .unwrap_err()
            .to_string()
            .contains("already locked"));
        let retry = store.persist_vote(&mut journal, &head, pending.proposal, CANDIDATE).unwrap();
        assert_eq!(retry, pending.vote);
    }
}
