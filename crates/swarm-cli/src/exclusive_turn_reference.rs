//! Isolated, Linux-only durable reference workflow for Sabathon #8.
//! No normal world storage, Minecraft runtime, game route, permit, or hosting
//! consent is opened or created. The reference checkpoint is opaque test data.
use anyhow::{bail, ensure, Context, Result};
use clap::{Args, Subcommand};
use ed25519_dalek::{Signature, VerifyingKey};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use swarm_consensus::exclusive_turn::*;
use swarm_core::{random_nonce, DataPaths, PeerIdentity};
use swarm_protocol::*;

#[path = "exclusive_turn_network.rs"]
mod network;
#[path = "exclusive_turn_writer.rs"]
mod writer;

const STORE: &str = "exclusive-turn-reference";
const JOURNAL_LIMIT: usize = 512 * 1024;
const DOCUMENT_LIMIT: usize = 1024 * 1024;
const MAX_RETAINED_CHECKPOINTS: usize = 512;
const JOURNAL_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/local-reference-journal/v1\0";

#[derive(Debug, Args)]
pub struct Pin {
    #[arg(long)]
    expected_world: ExclusiveTurnWorldIdV1,
    #[arg(long)]
    expected_grant: Hash32,
}

#[derive(Debug, Args)]
pub struct Reviewed {
    #[command(flatten)]
    pin: Pin,
    #[arg(long)]
    confirm: bool,
}

impl Reviewed {
    fn confirm(&self) -> Result<()> {
        ensure!(self.confirm, "explicit --confirm required for reviewed exclusive-turn reference mutation");
        Ok(())
    }
}

#[derive(Debug, Args)]
pub struct ReferenceCheckpoint {
    #[arg(long)]
    checkpoint: PathBuf,
    #[arg(long)]
    manifest_hash: Hash32,
    #[arg(long)]
    state_root: Hash32,
}

impl ReferenceCheckpoint {
    fn read(&self) -> Result<(ExclusiveTurnCheckpointV1, Vec<u8>)> {
        let bytes = read_bytes(&self.checkpoint, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
        let checkpoint = ExclusiveTurnCheckpointV1 {
            manifest_hash: self.manifest_hash,
            state_root: self.state_root,
            reference_commitment: exclusive_turn_reference_commitment(&bytes)?,
        };
        checkpoint.validate()?;
        Ok((checkpoint, bytes))
    }
}

#[derive(Debug, Subcommand)]
pub enum ExclusiveTurnCommand {
    /// Bounded in-process opaque-state writer; never a Minecraft runtime.
    Writer {
        #[command(subcommand)]
        command: writer::WriterCommand,
    },
    /// Authenticated private reference handoff, not a Minecraft runtime transfer.
    Network {
        #[command(subcommand)]
        command: network::NetworkCommand,
    },
    /// Draft a separate fixed two-device reference identity; not a playable world.
    Draft {
        #[arg(long = "device-key", num_args = 1, required = true)]
        device_keys: Vec<String>,
        #[arg(long)]
        initial_holder: PeerId,
        #[arg(long)]
        compatibility: Hash32,
        #[command(flatten)]
        state: ReferenceCheckpoint,
    },
    /// Approve the independently reviewed draft and exact initial opaque bytes.
    Approve {
        #[arg(long)]
        draft: PathBuf,
        #[arg(long)]
        expected_world: ExclusiveTurnWorldIdV1,
        #[arg(long)]
        checkpoint: PathBuf,
        #[arg(long)]
        confirm: bool,
    },
    /// Assemble both device approvals into a separate genesis history.
    Assemble {
        #[arg(long)]
        draft: PathBuf,
        #[arg(long)]
        expected_world: ExclusiveTurnWorldIdV1,
        #[arg(long = "approval", required = true)]
        approvals: Vec<PathBuf>,
    },
    /// Inspect a signed history; never establishes local or live authority.
    Inspect {
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        expected_world: ExclusiveTurnWorldIdV1,
    },
    /// Initialize a new local reference journal from the mutually signed genesis only.
    Init {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        checkpoint: PathBuf,
    },
    /// Verify the local signed journal and retained current/staged bytes.
    Status {
        #[arg(long)]
        expected_world: ExclusiveTurnWorldIdV1,
    },
    /// Update opaque reference bytes only while this local journal holds the turn.
    Checkpoint {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[command(flatten)]
        state: ReferenceCheckpoint,
    },
    /// Durably prepare a reference handoff and stop local reference mutations.
    Prepare {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
    },
    /// Explicitly cancel the exact prepared offer before irreversible release.
    Cancel {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_offer: Hash32,
    },
    /// Stage exact received bytes and durably retain the recipient receipt.
    Stage {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        offer: PathBuf,
        #[arg(long)]
        expected_offer: Hash32,
        #[arg(long)]
        checkpoint: PathBuf,
    },
    /// Irreversibly relinquish the local reference turn BEFORE returning the release.
    Commit {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        expected_offer: Hash32,
    },
    /// Retry the exact still-current durable outgoing release after restart.
    ExportRelease {
        #[command(flatten)]
        pin: Pin,
    },
    /// Accept the exact committed release only with a retained matching receipt and bytes.
    Accept {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        release: PathBuf,
        #[arg(long)]
        expected_release: Hash32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Replica,
    Held,
    Prepared(Box<ExclusiveTurnSignedOfferV1>),
    Released,
}

impl Phase {
    fn name(&self) -> &'static str {
        match self {
            Self::Replica => "replica",
            Self::Held => "held",
            Self::Prepared(_) => "prepared",
            Self::Released => "released",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Staged {
    offer: ExclusiveTurnSignedOfferV1,
    receipt: ExclusiveTurnReceiptV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    format: u16,
    identity: PeerId,
    history: ExclusiveTurnHistoryV1,
    phase: Phase,
    // Local highest signed offer attempt, including canceled attempts.
    attempt: u64,
    checkpoint: ExclusiveTurnCheckpointV1,
    staged: Option<Staged>,
}

#[derive(Serialize, Deserialize)]
struct SignedJournal {
    journal: Journal,
    signature: Vec<u8>,
}

fn read_bounded(mut file: File, limit: usize) -> Result<Vec<u8>> {
    ensure!(file.metadata()?.is_file(), "exclusive-turn input must be a regular file");
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "exclusive-turn input exceeds byte limit");
    Ok(bytes)
}

fn open_regular(path: &Path) -> Result<File> {
    ensure!(fs::symlink_metadata(path)?.is_file(), "exclusive-turn input must be a regular non-symlink file");
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    Ok(options.open(path)?)
}

fn read_bytes(path: &Path, limit: usize) -> Result<Vec<u8>> {
    read_bounded(open_regular(path).with_context(|| format!("open {}", path.display()))?, limit)
}

fn read_document<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&read_bytes(path, DOCUMENT_LIMIT)?)?)
}

fn print_document<T: Serialize>(value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= DOCUMENT_LIMIT, "exclusive-turn output exceeds byte limit");
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn private(metadata: &fs::Metadata, directory: bool) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            if directory { metadata.is_dir() } else { metadata.is_file() && metadata.nlink() == 1 },
            "unsafe exclusive-turn path type or link count"
        );
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
            "exclusive-turn journal requires owner-only paths"
        );
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (metadata, directory);
        bail!("durable exclusive-turn reference journal is qualified on Linux only");
    }
}

fn check_dir(path: &Path) -> Result<()> {
    private(&fs::symlink_metadata(path)?, true)
}

fn private_file(path: &Path) -> Result<File> {
    private(&fs::symlink_metadata(path)?, false)?;
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
    builder.create(path)?;
    check_dir(path)?;
    sync_dir(path.parent().context("missing parent directory")?)
}

fn publish(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let parent = path.parent().context("missing journal parent")?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    if replace {
        private_file(path)?;
        tmp.persist(path).map_err(|error| error.error)?;
    } else {
        tmp.persist_noclobber(path).map_err(|error| error.error)?;
    }
    sync_dir(parent)
}

fn identity(paths: &DataPaths) -> Result<PeerIdentity> {
    check_dir(&paths.root)?;
    check_dir(&paths.identity_dir())?;
    let bytes = read_bounded(private_file(&paths.identity_key())?, 32)?;
    let secret: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("invalid existing identity length"))?;
    Ok(PeerIdentity::from_secret_bytes(secret))
}

fn dir(paths: &DataPaths, world: ExclusiveTurnWorldIdV1) -> Result<PathBuf> {
    let name = world.to_string();
    Ok(paths.root.join(STORE).join(name.strip_prefix("scpass:").context("invalid exclusive-turn identity")?))
}

fn local_member(head: &VerifiedExclusiveTurnHead, identity: &PeerIdentity) -> Result<()> {
    ensure!(
        head.devices().iter().any(|d| d.peer_id == identity.peer_id() && d.public_key == identity.public_key()),
        "local identity is not one of the two fixed devices"
    );
    Ok(())
}

fn signing_bytes(journal: &Journal) -> Result<Vec<u8>> {
    let mut bytes = JOURNAL_DOMAIN.to_vec();
    bytes.extend(postcard::to_allocvec(journal)?);
    ensure!(bytes.len() <= JOURNAL_LIMIT, "exclusive-turn journal exceeds size limit");
    Ok(bytes)
}

struct Store {
    dir: PathBuf,
    _lock: File,
    identity: PeerIdentity,
}

impl Store {
    fn create(paths: &DataPaths, world: ExclusiveTurnWorldIdV1, identity: PeerIdentity) -> Result<Self> {
        let base = paths.root.join(STORE);
        match fs::symlink_metadata(&base) {
            Ok(_) => check_dir(&base)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(error) = create_dir(&base) {
                    check_dir(&base).with_context(|| format!("cannot create exclusive-turn store: {error}"))?;
                }
                sync_dir(&paths.root)?;
            }
            Err(error) => return Err(error.into()),
        }
        let dir = dir(paths, world)?;
        // Never initialize over an existing world directory, even if its
        // signed journal is missing or partially initialized.
        create_dir(&dir).context("exclusive-turn journal already exists or cannot be initialized")?;
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
        FileExt::try_lock_exclusive(&lock)?;
        lock.sync_all()?;
        sync_dir(&dir)?;
        Ok(Self { dir, _lock: lock, identity })
    }

    fn open(paths: &DataPaths, world: ExclusiveTurnWorldIdV1) -> Result<Self> {
        let identity = identity(paths)?;
        let dir = dir(paths, world)?;
        check_dir(&paths.root.join(STORE))?;
        check_dir(&dir)?;
        check_dir(&dir.join("checkpoints"))?;
        let lock = private_file(&dir.join("journal.lock"))?;
        FileExt::try_lock_exclusive(&lock).context("exclusive-turn journal busy; operation refused")?;
        Ok(Self { dir, _lock: lock, identity })
    }

    fn journal_path(&self) -> PathBuf {
        self.dir.join("journal.postcard")
    }

    fn state_path(&self, checkpoint: ExclusiveTurnCheckpointV1) -> PathBuf {
        self.dir.join("checkpoints").join(format!("{}.bin", checkpoint.reference_commitment))
    }

    fn state(&self, checkpoint: ExclusiveTurnCheckpointV1) -> Result<Vec<u8>> {
        let file = private_file(&self.state_path(checkpoint))?;
        // Retry synchronization before reissuing any externally visible
        // message from a publication that may have returned an fsync error.
        file.sync_all()?;
        sync_dir(&self.dir.join("checkpoints"))?;
        let bytes = read_bounded(file, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
        checkpoint.verify_reference_bytes(&bytes)?;
        Ok(bytes)
    }

    fn retain(&self, checkpoint: ExclusiveTurnCheckpointV1, bytes: &[u8]) -> Result<()> {
        checkpoint.verify_reference_bytes(bytes)?;
        let path = self.state_path(checkpoint);
        match fs::symlink_metadata(&path) {
            Ok(_) => ensure!(self.state(checkpoint)? == bytes, "retained reference bytes differ"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Retain old immutable bytes for inspection. Never silently
                // delete history to make room or permit unbounded staging.
                let count = fs::read_dir(self.dir.join("checkpoints"))?
                    .take(MAX_RETAINED_CHECKPOINTS)
                    .collect::<std::io::Result<Vec<_>>>()?
                    .len();
                ensure!(count < MAX_RETAINED_CHECKPOINTS, "exclusive-turn retained checkpoint limit reached");
                publish(&path, bytes, false)?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn validate(&self, journal: &Journal, world: ExclusiveTurnWorldIdV1) -> Result<VerifiedExclusiveTurnHead> {
        ensure!(
            journal.format == EXCLUSIVE_TURN_FORMAT_V1 && journal.identity == self.identity.peer_id(),
            "exclusive-turn journal identity or version mismatch"
        );
        let head = verify_exclusive_turn_history(world, &journal.history)?;
        local_member(&head, &self.identity)?;
        let local = journal.identity;
        ensure!(journal.attempt >= head.attempt_floor(local)?, "local attempt counter regressed");
        match &journal.phase {
            Phase::Held => ensure!(head.holder() == local && journal.staged.is_none(), "invalid held phase"),
            Phase::Prepared(offer) => {
                ensure!(
                    head.holder() == local
                        && journal.staged.is_none()
                        && offer.offer.source == local
                        && offer.offer.attempt == journal.attempt
                        && offer.offer.checkpoint == journal.checkpoint,
                    "invalid prepared phase"
                );
                verify_exclusive_turn_offer(&head, offer)?;
            }
            Phase::Replica => ensure!(
                journal.history.releases.is_empty()
                    && head.holder() != local
                    && journal.attempt == 0
                    && journal.checkpoint == head.checkpoint(),
                "invalid initial replica phase"
            ),
            Phase::Released => {
                ensure!(
                    head.holder() != local
                        && journal.checkpoint == head.checkpoint()
                        && journal.history.releases.last().is_some_and(|release| release.offer.offer.source == local
                            && release.offer.offer.attempt == journal.attempt),
                    "invalid released phase"
                )
            }
        }
        self.state(journal.checkpoint)?;
        if let Some(staged) = &journal.staged {
            ensure!(
                matches!(journal.phase, Phase::Replica | Phase::Released) && staged.offer.offer.recipient == local,
                "invalid staged recipient phase"
            );
            verify_exclusive_turn_receipt(&head, &staged.offer, &staged.receipt)?;
            self.state(staged.offer.offer.checkpoint)?;
        }
        Ok(head)
    }

    fn load(&self, world: ExclusiveTurnWorldIdV1) -> Result<(Journal, VerifiedExclusiveTurnHead)> {
        let file = private_file(&self.journal_path())?;
        file.sync_all()?;
        sync_dir(&self.dir)?;
        let bytes = read_bounded(file, JOURNAL_LIMIT)?;
        let signed: SignedJournal = postcard::from_bytes(&bytes)?;
        ensure!(postcard::to_allocvec(&signed)? == bytes, "noncanonical exclusive-turn journal");
        let key = VerifyingKey::from_bytes(&self.identity.public_key())?;
        key.verify_strict(&signing_bytes(&signed.journal)?, &Signature::from_slice(&signed.signature)?)
            .context("exclusive-turn journal signature invalid")?;
        let head = self.validate(&signed.journal, world)?;
        Ok((signed.journal, head))
    }

    fn save(
        &self,
        world: ExclusiveTurnWorldIdV1,
        journal: &Journal,
        replace: bool,
    ) -> Result<VerifiedExclusiveTurnHead> {
        let head = self.validate(journal, world)?;
        let signed =
            SignedJournal { journal: journal.clone(), signature: self.identity.sign(&signing_bytes(journal)?) };
        let bytes = postcard::to_allocvec(&signed)?;
        ensure!(bytes.len() <= JOURNAL_LIMIT, "exclusive-turn journal exceeds size limit");
        publish(&self.journal_path(), &bytes, replace)?;
        Ok(head)
    }
}

fn pin_grant(head: &VerifiedExclusiveTurnHead, expected: Hash32) -> Result<()> {
    ensure!(head.grant() == expected, "exclusive-turn accepted grant differs from expected pin");
    Ok(())
}

fn report(head: &VerifiedExclusiveTurnHead, local: Option<&Journal>) -> serde_json::Value {
    serde_json::json!({
        "classification": "exclusive_turn_reference_only",
        "world_id": head.world().to_string(),
        "grant": head.grant().to_string(),
        "reference_holder": head.holder().to_string(),
        "reference_epoch": head.epoch(),
        "reference_fence": head.fence(),
        "compatibility": head.compatibility().to_string(),
        "grant_checkpoint": head.checkpoint(),
        "local_identity": local.map(|j| j.identity.to_string()),
        "local_phase": local.map(|j| j.phase.name()),
        "local_checkpoint": local.map(|j| j.checkpoint),
        "local_attempt": local.map(|j| j.attempt),
        "prepared_offer": local.and_then(|j| match &j.phase { Phase::Prepared(o) => o.offer_hash().ok().map(|h| h.to_string()), _ => None }),
        "staged_offer": local.and_then(|j| j.staged.as_ref()).and_then(|s| s.offer.offer_hash().ok()).map(|h| h.to_string()),
        "local_reference_mutations_allowed": local.is_some_and(|j| j.phase == Phase::Held),
        "canonical_authority": false,
        "write_authority": false,
        "runtime_activation": false,
        "player_admission": false,
        "v1_canonical_world_created": false,
    })
}

fn opened(paths: &DataPaths, pin: &Pin) -> Result<(Store, Journal, VerifiedExclusiveTurnHead)> {
    let store = Store::open(paths, pin.expected_world)?;
    let (journal, head) = store.load(pin.expected_world)?;
    pin_grant(&head, pin.expected_grant)?;
    Ok((store, journal, head))
}

// All checkpoint publications, including the bounded reference writer,
// must pass the same held-turn compare-and-swap under the journal lock.
fn replace_checkpoint(
    store: &Store,
    journal: &mut Journal,
    head: &VerifiedExclusiveTurnHead,
    expected_checkpoint: Hash32,
    checkpoint: ExclusiveTurnCheckpointV1,
    bytes: &[u8],
) -> Result<()> {
    ensure!(journal.phase == Phase::Held && head.holder() == journal.identity, "reference turn is not held");
    ensure!(
        journal.checkpoint.reference_commitment == expected_checkpoint,
        "local checkpoint differs from expected pin"
    );
    checkpoint.verify_reference_bytes(bytes)?;
    ensure!(
        checkpoint.reference_commitment != expected_checkpoint || checkpoint == journal.checkpoint,
        "same opaque bytes cannot substitute checkpoint digest claims"
    );
    if checkpoint != journal.checkpoint {
        store.retain(checkpoint, bytes)?;
        journal.checkpoint = checkpoint;
        store.save(head.world(), journal, true)?;
    }
    Ok(())
}

// The file and encrypted transport workflows share exactly the same durable transitions.
fn stage_reference(
    store: &Store,
    journal: &mut Journal,
    head: &VerifiedExclusiveTurnHead,
    offer: ExclusiveTurnSignedOfferV1,
    expected_offer: Hash32,
    bytes: &[u8],
) -> Result<ExclusiveTurnReceiptV1> {
    ensure!(
        matches!(journal.phase, Phase::Replica | Phase::Released),
        "local reference phase cannot stage incoming offer"
    );
    verify_exclusive_turn_offer(head, &offer)?;
    ensure!(
        offer.offer_hash()? == expected_offer && offer.offer.recipient == journal.identity,
        "incoming offer or recipient differs from expected pin"
    );
    offer.offer.checkpoint.verify_reference_bytes(bytes)?;
    if let Some(staged) = &journal.staged {
        if staged.offer == offer {
            ensure!(store.state(offer.offer.checkpoint)? == bytes, "staged reference bytes differ");
            return Ok(staged.receipt.clone());
        }
        ensure!(
            offer.offer.attempt > staged.offer.offer.attempt,
            "incoming offer is stale or conflicts with retained receipt"
        );
    }
    store.retain(offer.offer.checkpoint, bytes)?;
    let mut receipt = ExclusiveTurnReceiptV1 {
        world_id: head.world(),
        offer_hash: expected_offer,
        recipient: journal.identity,
        signature: vec![],
    };
    receipt.signature = store.identity.sign(&receipt.signing_bytes()?);
    journal.staged = Some(Staged { offer, receipt: receipt.clone() });
    store.save(head.world(), journal, true)?;
    Ok(receipt)
}

fn commit_reference(
    store: &Store,
    journal: &mut Journal,
    head: &VerifiedExclusiveTurnHead,
    receipt: ExclusiveTurnReceiptV1,
    expected_offer: Hash32,
) -> Result<ExclusiveTurnReleaseV1> {
    let Phase::Prepared(offer) = &journal.phase else { bail!("no prepared reference offer to commit") };
    ensure!(offer.offer_hash()? == expected_offer, "prepared offer differs from expected pin");
    verify_exclusive_turn_receipt(head, offer, &receipt)?;
    let mut release = ExclusiveTurnReleaseV1 { offer: offer.as_ref().clone(), receipt, signature: vec![] };
    release.signature = store.identity.sign(&release.signing_bytes()?);
    let next = apply_exclusive_turn_release(head, &release)?;
    journal.history.releases.push(release.clone());
    journal.phase = Phase::Released;
    journal.checkpoint = next.checkpoint();
    // The one atomic signed replacement is both permanent local
    // relinquishment and exact outgoing release journal. No output
    // before file and directory synchronization completes.
    let durable = store.save(head.world(), journal, true)?;
    ensure!(durable == next, "durable reference grant mismatch");
    Ok(release)
}

fn accept_reference(
    store: &Store,
    journal: &mut Journal,
    head: &VerifiedExclusiveTurnHead,
    release: ExclusiveTurnReleaseV1,
    expected_base: Hash32,
    expected_release: Hash32,
) -> Result<VerifiedExclusiveTurnHead> {
    ensure!(release.grant_hash()? == expected_release, "committed release differs from expected pin");
    if head.grant() == expected_release {
        // Exact retry must not restore a prepared/retired turn or
        // overwrite a newer local reference checkpoint.
        ensure!(
            journal.phase == Phase::Held
                && journal.history.releases.last() == Some(&release)
                && release.offer.offer.parent_grant == expected_base
                && journal.checkpoint == head.checkpoint(),
            "release retry cannot restore a changed or nonheld reference turn"
        );
        return Ok(head.clone());
    }
    pin_grant(head, expected_base)?;
    ensure!(matches!(journal.phase, Phase::Replica | Phase::Released), "local reference phase cannot accept release");
    let staged = journal.staged.as_ref().context("matching durable checkpoint receipt is absent")?;
    ensure!(
        staged.offer == release.offer
            && staged.receipt == release.receipt
            && release.offer.offer.recipient == journal.identity,
        "release does not match exact retained recipient receipt"
    );
    let next = apply_exclusive_turn_release(head, &release)?;
    journal.history.releases.push(release);
    journal.phase = Phase::Held;
    journal.checkpoint = next.checkpoint();
    journal.staged = None;
    let durable = store.save(head.world(), journal, true)?;
    ensure!(durable == next, "durable reference grant mismatch");
    Ok(next)
}

pub fn run(paths: &DataPaths, command: ExclusiveTurnCommand) -> Result<()> {
    use ExclusiveTurnCommand::*;
    match command {
        Writer { command } => writer::run(paths, command),
        Network { command } => network::run(paths, command),
        Draft { device_keys, initial_holder, compatibility, state } => {
            ensure!(device_keys.len() == 2, "exactly two device keys required");
            let mut devices = Vec::new();
            for key in device_keys {
                ensure!(key.len() == 64, "device public key must be 64 hex characters");
                let key: [u8; 32] = hex::decode(key)?.try_into().map_err(|_| anyhow::anyhow!("invalid device key"))?;
                devices.push(ExclusiveTurnDeviceV1::from_key(key));
            }
            devices.sort_by_key(|d| d.peer_id);
            let (initial_checkpoint, _) = state.read()?;
            let genesis = ExclusiveTurnGenesisV1 {
                format_version: EXCLUSIVE_TURN_FORMAT_V1,
                creation_nonce: random_nonce(),
                devices: [devices[0], devices[1]],
                initial_holder,
                compatibility,
                initial_checkpoint,
            };
            validate_exclusive_turn_genesis(&genesis)?;
            print_document(&genesis)
        }
        Approve { draft, expected_world, checkpoint, confirm } => {
            ensure!(confirm, "explicit --confirm required for reviewed genesis approval");
            let genesis: ExclusiveTurnGenesisV1 = read_document(&draft)?;
            validate_exclusive_turn_genesis(&genesis)?;
            ensure!(genesis.world_id()? == expected_world, "genesis differs from expected identity pin");
            genesis
                .initial_checkpoint
                .verify_reference_bytes(&read_bytes(&checkpoint, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?)?;
            let identity = identity(paths)?;
            ensure!(
                genesis.device(identity.peer_id()).is_some_and(|d| d.public_key == identity.public_key()),
                "local identity is not a founding device"
            );
            let approval = ExclusiveTurnApprovalV1 {
                world_id: expected_world,
                device: identity.peer_id(),
                signature: identity.sign(&genesis.approval_bytes(identity.peer_id())?),
            };
            verify_exclusive_turn_approval(expected_world, &genesis, &approval)?;
            print_document(&approval)
        }
        Assemble { draft, expected_world, approvals } => {
            ensure!(approvals.len() == 2, "exactly two approvals required");
            let genesis: ExclusiveTurnGenesisV1 = read_document(&draft)?;
            let mut approvals: Vec<ExclusiveTurnApprovalV1> =
                approvals.iter().map(|p| read_document(p)).collect::<Result<_>>()?;
            approvals.sort_by_key(|approval| approval.device);
            let history = ExclusiveTurnHistoryV1 {
                creation: ExclusiveTurnCreationV1 { genesis, approvals: [approvals[0].clone(), approvals[1].clone()] },
                releases: vec![],
            };
            verify_exclusive_turn_history(expected_world, &history)?;
            print_document(&history)
        }
        Inspect { history, expected_world } => {
            let history = read_document(&history)?;
            let head = verify_exclusive_turn_history(expected_world, &history)?;
            print_document(&report(&head, None))
        }
        Init { review, history, checkpoint } => {
            review.confirm()?;
            let world = review.pin.expected_world;
            let history: ExclusiveTurnHistoryV1 = read_document(&history)?;
            ensure!(history.releases.is_empty(), "initialization requires genesis only; no history reset");
            let head = verify_exclusive_turn_history(world, &history)?;
            pin_grant(&head, review.pin.expected_grant)?;
            let identity = identity(paths)?;
            local_member(&head, &identity)?;
            let bytes = read_bytes(&checkpoint, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
            head.checkpoint().verify_reference_bytes(&bytes)?;
            let store = Store::create(paths, world, identity)?;
            store.retain(head.checkpoint(), &bytes)?;
            let local = store.identity.peer_id();
            let journal = Journal {
                format: EXCLUSIVE_TURN_FORMAT_V1,
                identity: local,
                history,
                phase: if head.holder() == local { Phase::Held } else { Phase::Replica },
                attempt: 0,
                checkpoint: head.checkpoint(),
                staged: None,
            };
            let head = store.save(world, &journal, false)?;
            print_document(&report(&head, Some(&journal)))
        }
        Status { expected_world } => {
            let store = Store::open(paths, expected_world)?;
            let (journal, head) = store.load(expected_world)?;
            print_document(&report(&head, Some(&journal)))
        }
        Checkpoint { review, expected_checkpoint, state } => {
            review.confirm()?;
            let (store, mut journal, head) = opened(paths, &review.pin)?;
            let (checkpoint, bytes) = state.read()?;
            replace_checkpoint(&store, &mut journal, &head, expected_checkpoint, checkpoint, &bytes)?;
            print_document(&report(&head, Some(&journal)))
        }
        Prepare { review, expected_checkpoint } => {
            review.confirm()?;
            let (store, mut journal, head) = opened(paths, &review.pin)?;
            ensure!(
                journal.checkpoint.reference_commitment == expected_checkpoint,
                "local checkpoint differs from expected pin"
            );
            if let Phase::Prepared(offer) = &journal.phase {
                return print_document(offer);
            }
            ensure!(journal.phase == Phase::Held, "reference turn is not held");
            let attempt = journal.attempt.checked_add(1).context("local attempt counter exhausted")?;
            let offer = head.draft_offer(attempt, journal.checkpoint)?;
            let signature = store.identity.sign(&offer.signing_bytes()?);
            let signed = ExclusiveTurnSignedOfferV1 { offer, signature };
            journal.attempt = attempt;
            journal.phase = Phase::Prepared(Box::new(signed.clone()));
            store.save(head.world(), &journal, true)?;
            print_document(&signed)
        }
        Cancel { review, expected_offer } => {
            review.confirm()?;
            let (store, mut journal, head) = opened(paths, &review.pin)?;
            let Phase::Prepared(offer) = &journal.phase else { bail!("no cancelable prepared reference offer") };
            ensure!(offer.offer_hash()? == expected_offer, "prepared offer differs from expected pin");
            // Retain the monotonically advanced attempt; a delayed receipt
            // cannot ever match a subsequently prepared offer.
            journal.phase = Phase::Held;
            store.save(head.world(), &journal, true)?;
            print_document(&report(&head, Some(&journal)))
        }
        Stage { review, offer, expected_offer, checkpoint } => {
            review.confirm()?;
            let (store, mut journal, head) = opened(paths, &review.pin)?;
            let offer = read_document(&offer)?;
            let bytes = read_bytes(&checkpoint, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
            print_document(&stage_reference(&store, &mut journal, &head, offer, expected_offer, &bytes)?)
        }
        Commit { review, receipt, expected_offer } => {
            review.confirm()?;
            let (store, mut journal, head) = opened(paths, &review.pin)?;
            let receipt = read_document(&receipt)?;
            print_document(&commit_reference(&store, &mut journal, &head, receipt, expected_offer)?)
        }
        ExportRelease { pin } => {
            let (_store, journal, _) = opened(paths, &pin)?;
            ensure!(journal.phase == Phase::Released, "no still-current released reference outbox");
            print_document(journal.history.releases.last().context("missing release outbox")?)
        }
        Accept { review, release, expected_release } => {
            review.confirm()?;
            let world = review.pin.expected_world;
            let store = Store::open(paths, world)?;
            let (mut journal, head) = store.load(world)?;
            let release: ExclusiveTurnReleaseV1 = read_document(&release)?;
            let next =
                accept_reference(&store, &mut journal, &head, release, review.pin.expected_grant, expected_release)?;
            print_document(&report(&next, Some(&journal)))
        }
    }
}
