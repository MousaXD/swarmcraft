//! Real single-process reference writer and route. No child process, JVM,
//! Minecraft save, runtime permit or ordinary game gateway is involved.
//! The writer owns the same stable lock as every handoff/mutation for its
//! entire lifetime, including the last socket and durable publication.
use super::{opened, print_document, read_bytes, replace_checkpoint, Journal, Phase, Reviewed, Store};
use crate::reference_channel::{authenticate, private_ip, remaining, verify_auth, Channel};
use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use swarm_consensus::exclusive_turn::VerifiedExclusiveTurnHead;
use swarm_core::{random_nonce, DataPaths};
use swarm_protocol::{
    exclusive_turn_reference_commitment, ExclusiveTurnCheckpointV1, ExclusiveTurnWorldIdV1, Hash32, PeerId,
    MAX_EXCLUSIVE_TURN_REFERENCE_BYTES,
};

const DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/opaque-writer/v1\0";
const AUTH_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/opaque-writer-auth/v1\0";
const RECEIPT_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/opaque-writer-receipt/v1\0";
const MANIFEST_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/opaque-writer-manifest/v1\0";
const STATE_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/opaque-writer-state/v1\0";
const RECORD_LIMIT: usize = 4096;
const SESSION_WINDOW: Duration = Duration::from_secs(10);

#[derive(Debug, Subcommand)]
pub enum WriterCommand {
    /// Hold an explicit reference turn and serve only the reviewed fixed device.
    Serve {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        from: PeerId,
        #[arg(long)]
        bind: SocketAddr,
        #[arg(long, default_value_t = 1)]
        max_sessions: u32,
        #[arg(long, default_value_t = 30)]
        run_seconds: u64,
    },
    /// Observe an exact checkpoint on a fresh mutually authenticated route.
    Observe {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        peer: String,
    },
    /// Compare-and-swap opaque bytes, receiving a signed durable receipt.
    Replace {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        checkpoint: PathBuf,
        #[arg(long)]
        peer: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    world: ExclusiveTurnWorldIdV1,
    grant: Hash32,
    epoch: u64,
    fence: u64,
    compatibility: Hash32,
    holder: PeerId,
    client: PeerId,
}

impl Scope {
    fn new(head: &VerifiedExclusiveTurnHead, client: PeerId) -> Result<Self> {
        ensure!(
            head.holder() != client && head.devices().iter().any(|device| device.peer_id == client),
            "reference writer client is not the other fixed device"
        );
        Ok(Self {
            world: head.world(),
            grant: head.grant(),
            epoch: head.epoch(),
            fence: head.fence(),
            compatibility: head.compatibility(),
            holder: head.holder(),
            client,
        })
    }

    fn prologue(&self) -> Result<Vec<u8>> {
        let mut result = DOMAIN.to_vec();
        result.extend(postcard::to_allocvec(self)?);
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    scope: Scope,
    nonce: [u8; 32],
    expected_checkpoint: Hash32,
    replacement: Option<ExclusiveTurnCheckpointV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptBody {
    request: Request,
    checkpoint: ExclusiveTurnCheckpointV1,
}

impl ReceiptBody {
    fn signing_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = RECEIPT_DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(self)?);
        Ok(bytes)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    body: ReceiptBody,
    signature: Vec<u8>,
}

fn derived_checkpoint(bytes: &[u8]) -> Result<ExclusiveTurnCheckpointV1> {
    Ok(ExclusiveTurnCheckpointV1 {
        // These are deliberately reference-only digests, not claims of
        // Minecraft manifest/state-root validation.
        manifest_hash: Hash32::from_domain_bytes(MANIFEST_DOMAIN, bytes),
        state_root: Hash32::from_domain_bytes(STATE_DOMAIN, bytes),
        reference_commitment: exclusive_turn_reference_commitment(bytes)?,
    })
}

fn key(head: &VerifiedExclusiveTurnHead, peer: PeerId) -> Result<[u8; 32]> {
    head.devices()
        .iter()
        .find(|device| device.peer_id == peer)
        .map(|device| device.public_key)
        .context("reference writer peer not in fixed roster")
}

fn send<T: Serialize>(channel: &mut Channel, value: &T) -> Result<()> {
    channel.send(&postcard::to_allocvec(value)?, RECORD_LIMIT)
}

fn receive<T: DeserializeOwned + Serialize>(channel: &mut Channel) -> Result<T> {
    let bytes = channel.receive(RECORD_LIMIT)?;
    let value = postcard::from_bytes(&bytes)?;
    ensure!(postcard::to_allocvec(&value)? == bytes, "noncanonical reference writer record");
    Ok(value)
}

// Keep all runtime ownership in this process, not an inherited lock or
// a detached helper. The listener and stream are dropped before Store.
fn accept(listener: &TcpListener, deadline: Instant) -> Result<Option<TcpStream>> {
    loop {
        if Instant::now() >= deadline {
            return Ok(None);
        }
        match listener.accept() {
            Ok((stream, remote)) => {
                ensure!(private_ip(remote.ip()), "reference writer connection is not private");
                return Ok(Some(stream));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

fn session(
    store: &Store,
    journal: &mut Journal,
    head: &VerifiedExclusiveTurnHead,
    scope: &Scope,
    stream: TcpStream,
    deadline: Instant,
) -> Result<()> {
    // No other supported mutation or handoff can change this journal
    // while the process owns the stable lock. Re-read the signed durable
    // journal and current immutable bytes before every route observation.
    let (durable, current) = store.load(scope.world)?;
    ensure!(
        current == *head
            && durable.phase == Phase::Held
            && durable.identity == scope.holder
            && durable.checkpoint == journal.checkpoint
            && durable.attempt == journal.attempt,
        "reference writer durable state changed"
    );
    *journal = durable;
    let mut channel = authenticate(
        stream,
        &scope.prologue()?,
        AUTH_DOMAIN,
        &store.identity,
        key(head, scope.client)?,
        false,
        deadline,
    )?;
    let request: Request = receive(&mut channel)?;
    ensure!(request.scope == *scope && request.nonce != [0; 32], "reference writer request scope or nonce differs");
    ensure!(
        request.expected_checkpoint == journal.checkpoint.reference_commitment,
        "reference writer checkpoint differs from expected pin"
    );
    if let Some(replacement) = request.replacement {
        let bytes = channel.receive(MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
        ensure!(derived_checkpoint(&bytes)? == replacement, "opaque writer replacement digests differ");
        remaining(deadline)?;
        replace_checkpoint(store, journal, head, request.expected_checkpoint, replacement, &bytes)?;
    }
    // A reply is never sent ahead of a successful durable publication
    // and verified bytes. Loss of a reply is not permission to roll back.
    let body = ReceiptBody { request, checkpoint: journal.checkpoint };
    store.state(body.checkpoint)?;
    let signature = store.identity.sign(&body.signing_bytes()?);
    remaining(deadline)?;
    send(&mut channel, &Receipt { body, signature })
}

#[allow(clippy::too_many_arguments)]
fn serve(
    paths: &DataPaths,
    review: &Reviewed,
    expected_checkpoint: Hash32,
    from: PeerId,
    bind: SocketAddr,
    max_sessions: u32,
    run_seconds: u64,
) -> Result<()> {
    review.confirm()?;
    ensure!(private_ip(bind.ip()), "reference writer must bind an explicit private address");
    ensure!(
        (1..=32).contains(&max_sessions) && (1..=120).contains(&run_seconds),
        "reference writer limits out of bounds"
    );
    let (store, mut journal, head) = opened(paths, &review.pin)?;
    ensure!(journal.phase == Phase::Held && head.holder() == store.identity.peer_id(), "reference turn is not held");
    ensure!(
        journal.checkpoint.reference_commitment == expected_checkpoint,
        "local checkpoint differs from expected pin"
    );
    let scope = Scope::new(&head, from)?;
    let listener = TcpListener::bind(bind)?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(run_seconds);
    print_document(&serde_json::json!({
        "classification": "exclusive_turn_opaque_writer_listener",
        "address": listener.local_addr()?.to_string(),
        "world_id": scope.world.to_string(), "grant": scope.grant.to_string(),
        "reference_epoch": scope.epoch, "reference_fence": scope.fence,
        "holder": scope.holder.to_string(), "client": scope.client.to_string(),
        "checkpoint": journal.checkpoint, "writer_pid": std::process::id(),
        "canonical_authority": false, "write_authority": false,
        "runtime_activation": false, "player_admission": false,
    }))?;
    for _ in 0..max_sessions {
        let Some(stream) = accept(&listener, deadline)? else { break };
        // A failed session exits rather than reusing a potentially
        // ambiguous in-memory journal after a failed fsync or connection.
        session(&store, &mut journal, &head, &scope, stream, deadline.min(Instant::now() + SESSION_WINDOW))?;
    }
    drop(listener);
    // No route can remain alive after release of the local admission lock.
    drop(store);
    Ok(())
}

fn endpoint(peer: &str) -> Result<(PeerId, SocketAddr)> {
    let (peer, address) = peer.split_once('@').context("reference writer peer requires identity@address")?;
    let address: SocketAddr = address.parse()?;
    ensure!(private_ip(address.ip()) && address.port() != 0, "reference writer endpoint must be explicit and private");
    Ok((peer.parse()?, address))
}

fn request(
    paths: &DataPaths,
    review: &Reviewed,
    expected_checkpoint: Hash32,
    peer: &str,
    checkpoint: Option<&Path>,
) -> Result<()> {
    review.confirm()?;
    let (remote, address) = endpoint(peer)?;
    let (store, journal, head) = opened(paths, &review.pin)?;
    let local = store.identity.peer_id();
    ensure!(
        matches!(journal.phase, Phase::Replica | Phase::Released) && head.holder() == remote,
        "reference writer endpoint is not the current remote holder"
    );
    let scope = Scope::new(&head, local)?;
    let bytes = checkpoint.map(|path| read_bytes(path, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)).transpose()?;
    let replacement = bytes.as_deref().map(derived_checkpoint).transpose()?;
    let request = Request { scope, nonce: random_nonce(), expected_checkpoint, replacement };
    let deadline = Instant::now() + SESSION_WINDOW;
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    let remote_key = key(&head, remote)?;
    let mut channel =
        authenticate(stream, &request.scope.prologue()?, AUTH_DOMAIN, &store.identity, remote_key, true, deadline)?;
    send(&mut channel, &request)?;
    if let Some(bytes) = bytes {
        channel.send(&bytes, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
    }
    let receipt: Receipt = receive(&mut channel)?;
    ensure!(receipt.body.request == request, "reference writer receipt differs from exact request");
    receipt.body.checkpoint.validate()?;
    if let Some(replacement) = request.replacement {
        ensure!(receipt.body.checkpoint == replacement, "reference writer receipt differs from exact replacement");
    } else {
        ensure!(
            receipt.body.checkpoint.reference_commitment == expected_checkpoint,
            "reference writer observed checkpoint differs"
        );
    }
    verify_auth(&remote_key, &receipt.body.signing_bytes()?, &receipt.signature)?;
    remaining(deadline)?;
    // Fresh per-connection proof of a verified reference operation only.
    print_document(&serde_json::json!({
        "classification": "exclusive_turn_opaque_writer_receipt",
        "world_id": request.scope.world.to_string(), "grant": request.scope.grant.to_string(),
        "reference_epoch": request.scope.epoch, "reference_fence": request.scope.fence,
        "receipt": receipt, "canonical_authority": false, "write_authority": false,
        "runtime_activation": false, "player_admission": false, "freshness_lease": false,
    }))
}

pub(super) fn run(paths: &DataPaths, command: WriterCommand) -> Result<()> {
    match command {
        WriterCommand::Serve { review, expected_checkpoint, from, bind, max_sessions, run_seconds } => {
            serve(paths, &review, expected_checkpoint, from, bind, max_sessions, run_seconds)
        }
        WriterCommand::Observe { review, expected_checkpoint, peer } => {
            request(paths, &review, expected_checkpoint, &peer, None)
        }
        WriterCommand::Replace { review, expected_checkpoint, checkpoint, peer } => {
            request(paths, &review, expected_checkpoint, &peer, Some(&checkpoint))
        }
    }
}
