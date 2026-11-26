//! Explicit, bounded reference decision exchange. Never a game route or permit.
//! Noise NN supplies ephemeral transport keys; both application identities
//! authenticate the exact handshake transcript before any decision data flows.
#[path = "ownerless_head_discovery.rs"]
mod discovery;
#[path = "ownerless_decision_recovery.rs"]
mod recovery;

use super::{
    checkpoint, encoded, identity,
    observation::{private_ip, read_all, remaining, write_all},
    pin, print, proposal, Store,
};
use anyhow::{ensure, Context, Result};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use swarm_consensus::ownerless_authority::{
    apply_ownerless_certificate, ownerless_vote_key, verify_ownerless_history, verify_ownerless_vote,
    VerifiedOwnerlessHead,
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{
    ownerless_genesis_head, Hash32, OwnerlessAuthorityActionV1, OwnerlessAuthorityCertificateV1,
    OwnerlessAuthorityHistoryV1, OwnerlessAuthorityProposalV1, OwnerlessAuthorityVoteV1, OwnerlessWorldIdV1, PeerId,
    MAX_OWNERLESS_CHECKPOINT_BYTES, MAX_OWNERLESS_DOCUMENT_BYTES, MAX_OWNERLESS_HISTORY_BYTES,
    MAX_OWNERLESS_VALIDATORS,
};

const DOMAIN: &[u8] = b"swarmcraft/ownerless-decision/noise/v1\0";
const SYNC_DOMAIN: &[u8] = b"swarmcraft/ownerless-decision/certificate-sync/v1\0";
const AUTH_DOMAIN: &[u8] = b"swarmcraft/ownerless-decision/channel-auth/v1\0";
const CHUNK: usize = 32 * 1024;
const TAG: usize = 16;
const ACCEPT_WINDOW: Duration = Duration::from_secs(15);
const SESSION_WINDOW: Duration = Duration::from_secs(10);
// Reserve time for final local verification and durable acceptance even
// when an optional remote voter stalls for its entire network window.
const COLLECTION_NETWORK_WINDOW: Duration = Duration::from_secs(7);

#[derive(Debug, Args)]
pub struct Review {
    #[arg(long)]
    expected_world: OwnerlessWorldIdV1,
    #[arg(long)]
    expected_head: Hash32,
    /// Locally reviewed proposal document (including any entrant keys).
    #[arg(long)]
    proposal: PathBuf,
    #[arg(long)]
    expected_proposal: Hash32,
    #[arg(long)]
    confirm: bool,
}

#[derive(Debug, Args)]
pub struct SyncReview {
    #[arg(long)]
    expected_world: OwnerlessWorldIdV1,
    /// An independently accepted ancestor; the receiver may be at base or target.
    #[arg(long)]
    expected_base: Hash32,
    /// Exact certified target, independently selected by both operators.
    #[arg(long)]
    expected_head: Hash32,
    #[arg(long)]
    confirm: bool,
}

#[derive(Debug, Args)]
pub struct DiscoveryReview {
    #[arg(long)]
    expected_world: OwnerlessWorldIdV1,
    /// Exact locally accepted ancestor; no unreviewed peer or world discovery.
    #[arg(long)]
    expected_base: Hash32,
    /// Allow reconciliation of a verified certified descendant, not new voting.
    #[arg(long)]
    confirm: bool,
}

#[derive(Debug, Subcommand)]
pub enum DecisionCommand {
    /// Serve exactly one explicitly approved remote vote request.
    ServeVote {
        #[command(flatten)]
        review: Review,
        /// Exact current validator who may submit the request.
        #[arg(long)]
        from: PeerId,
        #[arg(long)]
        bind: SocketAddr,
    },
    /// Send the approved proposal and exact candidate bytes; receive a durable vote.
    RequestVote {
        #[command(flatten)]
        review: Review,
        #[arg(long)]
        checkpoint: PathBuf,
        /// Exact eligible validator and explicit private endpoint (identity@address).
        #[arg(long)]
        peer: String,
    },
    /// Collect remote durable votes, certify and durably accept one reviewed decision locally.
    Collect {
        #[command(flatten)]
        review: Review,
        #[arg(long)]
        checkpoint: PathBuf,
        #[arg(long = "peer", required = true)]
        peers: Vec<String>,
    },
    /// Receive one exact certified history and checkpoint from an authorized validator.
    ServeSync {
        #[command(flatten)]
        review: SyncReview,
        #[arg(long)]
        from: PeerId,
        #[arg(long)]
        bind: SocketAddr,
    },
    /// Send a locally accepted certified checkpoint and verify the receiver's durable receipt.
    PushSync {
        #[command(flatten)]
        review: SyncReview,
        #[arg(long)]
        peer: String,
    },
    /// Serve existing durable evidence for a reviewed decision, without casting a vote.
    ServeRecovery {
        #[command(flatten)]
        review: Review,
        #[arg(long)]
        from: PeerId,
        #[arg(long)]
        bind: SocketAddr,
    },
    /// Recover a certified reviewed decision from existing durable votes or a surviving certificate.
    Recover {
        #[command(flatten)]
        review: Review,
        #[arg(long = "peer", required = true)]
        peers: Vec<String>,
    },
    /// Serve an accepted certified head to one pinned base-roster validator.
    ServeHead {
        #[command(flatten)]
        review: DiscoveryReview,
        #[arg(long)]
        from: PeerId,
        #[arg(long)]
        bind: SocketAddr,
    },
    /// Discover and accept a verified certified descendant without knowing its target hash.
    PullHead {
        #[command(flatten)]
        review: DiscoveryReview,
        #[arg(long = "peer", required = true)]
        peers: Vec<String>,
    },
}

#[derive(Serialize)]
struct ChannelContext {
    #[serde(skip)]
    domain: &'static [u8],
    world: OwnerlessWorldIdV1,
    head: Hash32,
    proposal: Hash32,
    initiator: PeerId,
    responder: PeerId,
}

impl ChannelContext {
    fn prologue(&self) -> Result<Vec<u8>> {
        let mut bytes = self.domain.to_vec();
        bytes.extend(postcard::to_allocvec(self)?);
        Ok(bytes)
    }
}

fn current_key(head: &VerifiedOwnerlessHead, peer: PeerId) -> Result<[u8; 32]> {
    head.validators()
        .iter()
        .find(|v| v.peer_id == peer)
        .map(|v| v.public_key)
        .context("decision initiator must be a current validator")
}

fn wire_send(stream: &mut TcpStream, bytes: &[u8], deadline: Instant) -> Result<()> {
    ensure!(!bytes.is_empty() && bytes.len() <= CHUNK + TAG, "invalid decision frame");
    write_all(stream, &(bytes.len() as u32).to_be_bytes(), deadline)?;
    write_all(stream, bytes, deadline)
}

fn wire_recv(stream: &mut TcpStream, max: usize, deadline: Instant) -> Result<Vec<u8>> {
    let mut size = [0; 4];
    read_all(stream, &mut size, deadline)?;
    let size = u32::from_be_bytes(size) as usize;
    ensure!((1..=max).contains(&size) && max <= CHUNK + TAG, "invalid decision frame size");
    let mut bytes = vec![0; size];
    read_all(stream, &mut bytes, deadline)?;
    Ok(bytes)
}

struct Channel {
    stream: TcpStream,
    noise: snow::TransportState,
    deadline: Instant,
}

impl Channel {
    fn send_chunk(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(!bytes.is_empty() && bytes.len() <= CHUNK, "invalid decision plaintext chunk");
        let mut cipher = vec![0; bytes.len() + TAG];
        let n = self.noise.write_message(bytes, &mut cipher)?;
        ensure!(n == cipher.len(), "unexpected decision ciphertext size");
        wire_send(&mut self.stream, &cipher, self.deadline)
    }

    fn recv_chunk(&mut self, len: usize) -> Result<Vec<u8>> {
        ensure!((1..=CHUNK).contains(&len), "invalid decision plaintext length");
        let cipher = wire_recv(&mut self.stream, len + TAG, self.deadline)?;
        ensure!(cipher.len() == len + TAG, "invalid decision ciphertext length");
        let mut plain = vec![0; len];
        let n = self.noise.read_message(&cipher, &mut plain)?;
        ensure!(n == len, "unexpected decision plaintext size");
        Ok(plain)
    }

    fn send(&mut self, bytes: &[u8], max: usize) -> Result<()> {
        ensure!(!bytes.is_empty() && bytes.len() <= max, "invalid decision message length");
        self.send_chunk(&(bytes.len() as u32).to_be_bytes())?;
        for chunk in bytes.chunks(CHUNK) {
            self.send_chunk(chunk)?;
        }
        Ok(())
    }

    fn receive(&mut self, max: usize) -> Result<Vec<u8>> {
        let size = u32::from_be_bytes(self.recv_chunk(4)?.try_into().expect("exact chunk length")) as usize;
        ensure!((1..=max).contains(&size), "invalid decision message length");
        let mut bytes = Vec::with_capacity(size);
        while bytes.len() < size {
            bytes.extend(self.recv_chunk((size - bytes.len()).min(CHUNK))?);
        }
        Ok(bytes)
    }
}

fn auth_bytes(prologue: &[u8], transcript: &[u8], initiator: bool) -> Vec<u8> {
    let mut bytes = AUTH_DOMAIN.to_vec();
    bytes.push(u8::from(initiator));
    bytes.extend(prologue);
    bytes.extend(transcript);
    bytes
}

fn verify_auth(key: &[u8; 32], bytes: &[u8], signature: &[u8]) -> Result<()> {
    ed25519_dalek::VerifyingKey::from_bytes(key)?
        .verify_strict(bytes, &ed25519_dalek::Signature::from_slice(signature)?)
        .context("decision channel peer authentication failed")
}

fn authenticate(
    mut stream: TcpStream,
    context: &ChannelContext,
    local: &PeerIdentity,
    remote_key: [u8; 32],
    initiator: bool,
    deadline: Instant,
) -> Result<Channel> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    let prologue = context.prologue()?;
    let builder = snow::Builder::new("Noise_NN_25519_ChaChaPoly_BLAKE2s".parse()?).prologue(&prologue)?;
    let mut handshake = if initiator { builder.build_initiator()? } else { builder.build_responder()? };
    let mut output = [0; 128];
    if initiator {
        let len = handshake.write_message(&[], &mut output)?;
        wire_send(&mut stream, &output[..len], deadline)?;
        let input = wire_recv(&mut stream, 128, deadline)?;
        ensure!(handshake.read_message(&input, &mut output)? == 0, "unexpected Noise handshake payload");
    } else {
        let input = wire_recv(&mut stream, 128, deadline)?;
        ensure!(handshake.read_message(&input, &mut output)? == 0, "unexpected Noise handshake payload");
        let len = handshake.write_message(&[], &mut output)?;
        wire_send(&mut stream, &output[..len], deadline)?;
    }
    let own_auth = auth_bytes(&prologue, handshake.get_handshake_hash(), initiator);
    let peer_auth = auth_bytes(&prologue, handshake.get_handshake_hash(), !initiator);
    let mut channel = Channel { stream, noise: handshake.into_transport_mode()?, deadline };
    // The initiator receives the pinned responder's proof before signing.
    // The responder accepts no application data before the initiator proves
    // its own identity on this same channel and exact reviewed context.
    if initiator {
        verify_auth(&remote_key, &peer_auth, &channel.recv_chunk(64)?)?;
        channel.send_chunk(&local.sign(&own_auth))?;
    } else {
        channel.send_chunk(&local.sign(&own_auth))?;
        verify_auth(&remote_key, &peer_auth, &channel.recv_chunk(64)?)?;
    }
    remaining(deadline)?;
    Ok(channel)
}

fn reviewed(
    paths: &DataPaths,
    review: &Review,
) -> Result<(Store, super::Journal, VerifiedOwnerlessHead, OwnerlessAuthorityProposalV1)> {
    ensure!(review.confirm, "explicit --confirm required for reviewed network decision");
    let store = Store::try_open(paths, review.expected_world)?;
    let (journal, head) = store.load(review.expected_world)?;
    pin(&head, review.expected_head)?;
    let proposal = proposal(&review.proposal, review.expected_proposal, &head)?;
    Ok((store, journal, head, proposal))
}

fn endpoint(raw: &str) -> Result<(PeerId, SocketAddr)> {
    let (peer, address) = raw.split_once('@').context("decision peer requires identity@address")?;
    let address: SocketAddr = address.parse()?;
    ensure!(private_ip(address.ip()) && address.port() != 0, "decision endpoint must be explicit and private");
    Ok((peer.parse()?, address))
}

fn accept(listener: TcpListener, deadline: Instant) -> Result<TcpStream> {
    listener.set_nonblocking(true)?;
    loop {
        remaining(deadline)?;
        match listener.accept() {
            Ok((stream, remote)) => {
                ensure!(private_ip(remote.ip()), "decision connection is not from a private address");
                return Ok(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10).min(remaining(deadline)?));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

fn serve_vote(paths: &DataPaths, review: &Review, from: PeerId, bind: SocketAddr) -> Result<()> {
    ensure!(private_ip(bind.ip()), "decision listener must bind an explicit private address");
    let (store, journal, head, approved) = reviewed(paths, review)?;
    let local = store.identity.peer_id();
    ensure!(local != from, "decision peer must be remote");
    let remote_key = current_key(&head, from)?;
    ensure!(ownerless_vote_key(&head, &approved, local)? == store.identity.public_key(), "local voter key mismatch");
    ensure!(
        journal.pending.as_ref().is_none_or(|p| p.proposal == approved),
        "durable ownerless vote already locked to another proposal"
    );
    drop(store);
    let context = ChannelContext {
        domain: DOMAIN,
        world: review.expected_world,
        head: review.expected_head,
        proposal: review.expected_proposal,
        initiator: from,
        responder: local,
    };
    let listener = TcpListener::bind(bind)?;
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_decision_listener",
            "address": listener.local_addr()?.to_string(), "validator": local.to_string(),
            "world_id": review.expected_world.to_string(), "proposal": review.expected_proposal.to_string(),
            "runtime_activation": false,
        }),
        MAX_OWNERLESS_DOCUMENT_BYTES,
    )?;
    let stream = accept(listener, Instant::now() + ACCEPT_WINDOW)?;
    let deadline = Instant::now() + SESSION_WINDOW;
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local, "decision listener identity changed");
    let mut channel = authenticate(stream, &context, &signer, remote_key, false, deadline)?;
    let received: OwnerlessAuthorityProposalV1 =
        serde_json::from_slice(&channel.receive(MAX_OWNERLESS_DOCUMENT_BYTES)?)?;
    ensure!(received == approved, "remote decision does not match reviewed proposal");
    let bytes = channel.receive(MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    let (store, mut journal, current, current_approved) = reviewed(paths, review)?;
    ensure!(
        current == head && current_approved == approved && store.identity.peer_id() == local,
        "decision listener head, proposal or identity changed"
    );
    remaining(deadline)?;
    let vote = store.persist_vote(&mut journal, &current, received, &bytes)?;
    // A failed send or expired deadline MUST NOT roll back the durable vote.
    channel.send(&encoded(&vote, MAX_OWNERLESS_DOCUMENT_BYTES)?, MAX_OWNERLESS_DOCUMENT_BYTES)
}

fn request_vote(paths: &DataPaths, review: &Review, file: &std::path::Path, peer: &str) -> Result<()> {
    let (remote, address) = endpoint(peer)?;
    let (store, _, head, approved) = reviewed(paths, review)?;
    let local = store.identity.peer_id();
    ensure!(local != remote, "decision peer must be remote");
    ensure!(current_key(&head, local)? == store.identity.public_key(), "local proposer key mismatch");
    let remote_key = ownerless_vote_key(&head, &approved, remote)?;
    let bytes = checkpoint(file, head.candidate_checkpoint(&approved)?)?;
    drop(store);
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local, "decision proposer identity changed");
    let context = ChannelContext {
        domain: DOMAIN,
        world: review.expected_world,
        head: review.expected_head,
        proposal: review.expected_proposal,
        initiator: local,
        responder: remote,
    };
    let deadline = Instant::now() + SESSION_WINDOW;
    let vote = fetch_vote(address, &context, &signer, remote_key, &approved, &bytes, deadline)?;
    let (store, _, current, current_approved) = reviewed(paths, review)?;
    ensure!(
        current == head && current_approved == approved && store.identity.peer_id() == local,
        "decision proposer head, proposal or identity changed"
    );
    ensure!(vote.validator == remote, "remote decision vote has wrong validator");
    verify_ownerless_vote(&current, &approved, &vote)?;
    remaining(deadline)?;
    print(&vote, MAX_OWNERLESS_DOCUMENT_BYTES)
}

fn fetch_vote(
    address: SocketAddr,
    context: &ChannelContext,
    signer: &PeerIdentity,
    remote_key: [u8; 32],
    approved: &OwnerlessAuthorityProposalV1,
    bytes: &[u8],
    deadline: Instant,
) -> Result<OwnerlessAuthorityVoteV1> {
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    let mut channel = authenticate(stream, context, signer, remote_key, true, deadline)?;
    channel.send(&encoded(approved, MAX_OWNERLESS_DOCUMENT_BYTES)?, MAX_OWNERLESS_DOCUMENT_BYTES)?;
    channel.send(bytes, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    Ok(serde_json::from_slice(&channel.receive(MAX_OWNERLESS_DOCUMENT_BYTES)?)?)
}

fn pinned_base(
    history: &OwnerlessAuthorityHistoryV1,
    world: OwnerlessWorldIdV1,
    base: Hash32,
) -> Result<VerifiedOwnerlessHead> {
    let mut prefix = history.clone();
    let length = if ownerless_genesis_head(world)? == base {
        0
    } else {
        let mut found = None;
        for (i, certificate) in history.records.iter().enumerate() {
            if certificate.proposal.proposal_hash()? == base {
                found = Some(i + 1);
                break;
            }
        }
        found.context("decision base is not in the accepted ancestry")?
    };
    prefix.records.truncate(length);
    let head = verify_ownerless_history(world, &prefix)?;
    pin(&head, base)?;
    Ok(head)
}

fn collect(paths: &DataPaths, review: &Review, file: &std::path::Path, peers: &[String]) -> Result<()> {
    ensure!(review.confirm, "explicit --confirm required for reviewed network decision");
    let store = Store::try_open(paths, review.expected_world)?;
    let (mut journal, head) = store.load(review.expected_world)?;
    // Recover output after the local commit succeeded but the process or
    // stdout failed. Never vote again or roll the journal back.
    if head.hash() == review.expected_proposal {
        let base = pinned_base(&journal.history, review.expected_world, review.expected_head)?;
        let approved = proposal(&review.proposal, review.expected_proposal, &base)?;
        ensure!(
            journal.history.records.last().is_some_and(|r| r.proposal == approved),
            "accepted decision differs from exact reviewed proposal"
        );
        let bytes = checkpoint(file, head.checkpoint())?;
        ensure!(store.state(head.checkpoint())? == bytes, "stored ownerless checkpoint differs");
        return print(&journal.history, MAX_OWNERLESS_HISTORY_BYTES);
    }
    pin(&head, review.expected_head)?;
    let approved = proposal(&review.proposal, review.expected_proposal, &head)?;
    let local = store.identity.peer_id();
    ensure!(current_key(&head, local)? == store.identity.public_key(), "local proposer key mismatch");
    let bytes = checkpoint(file, head.candidate_checkpoint(&approved)?)?;
    ensure!(!peers.is_empty() && peers.len() < MAX_OWNERLESS_VALIDATORS * 2, "invalid decision peer count");
    let mut identities = BTreeSet::from([local]);
    let mut addresses = BTreeSet::new();
    let mut targets = Vec::new();
    for raw in peers {
        let (peer, address) = endpoint(raw)?;
        let key = ownerless_vote_key(&head, &approved, peer)?;
        ensure!(identities.insert(peer) && addresses.insert(address), "duplicate decision validator or endpoint");
        targets.push((peer, address, key));
    }
    // Refuse a structurally impossible collection before locking even one
    // vote. The actual certificate verifier remains the authority.
    ensure!(
        head.validators().iter().filter(|v| identities.contains(&v.peer_id)).count() >= head.threshold(),
        "insufficient configured current validators"
    );
    if let OwnerlessAuthorityActionV1::Reconfigure { validators } = &approved.action {
        ensure!(validators.iter().any(|v| v.peer_id == local), "collector must remain in the proposed roster");
        ensure!(
            validators.iter().filter(|v| identities.contains(&v.peer_id)).count() > validators.len() / 2,
            "insufficient configured proposed validators"
        );
        ensure!(
            validators.iter().all(
                |v| head.validators().iter().any(|old| old.peer_id == v.peer_id) || identities.contains(&v.peer_id)
            ),
            "missing configured entrant approval"
        );
    }
    let mut votes = vec![store.persist_vote(&mut journal, &head, approved.clone(), &bytes)?];
    drop(store);
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local, "decision collector identity changed");
    let started = Instant::now();
    let deadline = started + SESSION_WINDOW;
    let network_deadline = started + COLLECTION_NETWORK_WINDOW;
    let attempts = thread::scope(|scope| {
        let handles: Vec<_> = targets
            .into_iter()
            .map(|(peer, address, key)| {
                let context = ChannelContext {
                    domain: DOMAIN,
                    world: review.expected_world,
                    head: review.expected_head,
                    proposal: review.expected_proposal,
                    initiator: local,
                    responder: peer,
                };
                let (signer, approved, bytes) = (&signer, &approved, &bytes);
                (
                    peer,
                    scope.spawn(move || fetch_vote(address, &context, signer, key, approved, bytes, network_deadline)),
                )
            })
            .collect();
        handles.into_iter().map(|(peer, handle)| (peer, handle.join())).collect::<Vec<_>>()
    });
    for (peer, attempt) in attempts {
        match attempt {
            Ok(Ok(vote)) if vote.validator == peer && verify_ownerless_vote(&head, &approved, &vote).is_ok() => {
                votes.push(vote)
            }
            _ => eprintln!("unavailable or invalid decision voter: {peer}"),
        }
    }
    votes.sort_by_key(|vote| vote.validator);
    let certificate = OwnerlessAuthorityCertificateV1 { proposal: approved.clone(), votes };
    let next = apply_ownerless_certificate(&head, &certificate)?;
    let (store, mut journal, current, current_approved) = reviewed(paths, review)?;
    ensure!(
        current == head && current_approved == approved && store.identity.peer_id() == local,
        "decision collector head, proposal or identity changed"
    );
    let mut history = journal.history.clone();
    history.records.push(certificate);
    remaining(deadline)?;
    store.accept_verified(&mut journal, &current, history, &next, &bytes)?;
    // A failed output or expired deadline after the commit is recoverable
    // by the exact same pinned collection command, never by vote reset.
    remaining(deadline)?;
    print(&journal.history, MAX_OWNERLESS_HISTORY_BYTES)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncReceiptBody {
    world: OwnerlessWorldIdV1,
    base: Hash32,
    head: Hash32,
    sequence: u64,
    fence: u64,
    checkpoint: Hash32,
    recipient: PeerId,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncReceipt {
    body: SyncReceiptBody,
    signature: Vec<u8>,
}

impl SyncReceiptBody {
    fn new(review: &SyncReview, head: &VerifiedOwnerlessHead, recipient: PeerId) -> Self {
        Self {
            world: review.expected_world,
            base: review.expected_base,
            head: head.hash(),
            sequence: head.sequence(),
            fence: head.fence(),
            checkpoint: head.checkpoint(),
            recipient,
        }
    }

    fn signing_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = b"swarmcraft/ownerless-decision/durable-sync-receipt/v1\0".to_vec();
        bytes.extend(postcard::to_allocvec(self)?);
        Ok(bytes)
    }
}

fn sync_context(review: &SyncReview, sender: PeerId, recipient: PeerId) -> ChannelContext {
    ChannelContext {
        domain: SYNC_DOMAIN,
        world: review.expected_world,
        head: review.expected_base,
        proposal: review.expected_head,
        initiator: sender,
        responder: recipient,
    }
}

fn sync_store(
    paths: &DataPaths,
    review: &SyncReview,
) -> Result<(Store, super::Journal, VerifiedOwnerlessHead, VerifiedOwnerlessHead)> {
    ensure!(review.confirm, "explicit --confirm required for certified decision sync");
    ensure!(review.expected_base != review.expected_head, "decision sync requires distinct base and target");
    let store = Store::try_open(paths, review.expected_world)?;
    let (journal, head) = store.load(review.expected_world)?;
    ensure!(
        head.hash() == review.expected_base || head.hash() == review.expected_head,
        "decision sync local head is not pinned base or target"
    );
    let base = pinned_base(&journal.history, review.expected_world, review.expected_base)?;
    Ok((store, journal, head, base))
}

fn serve_sync(paths: &DataPaths, review: &SyncReview, from: PeerId, bind: SocketAddr) -> Result<()> {
    ensure!(private_ip(bind.ip()), "decision listener must bind an explicit private address");
    let (store, _, head, base) = sync_store(paths, review)?;
    let local = store.identity.peer_id();
    ensure!(local != from, "decision peer must be remote");
    let key = current_key(&base, from)?;
    if head.hash() == review.expected_head {
        ensure!(current_key(&head, from)? == key, "sync sender retired from target");
        ensure!(current_key(&head, local)? == store.identity.public_key(), "sync recipient not in target roster");
    }
    drop(store);
    let listener = TcpListener::bind(bind)?;
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_sync_listener",
            "address": listener.local_addr()?.to_string(), "validator": local.to_string(),
            "world_id": review.expected_world.to_string(), "head": review.expected_head.to_string(),
            "runtime_activation": false,
        }),
        MAX_OWNERLESS_DOCUMENT_BYTES,
    )?;
    let stream = accept(listener, Instant::now() + ACCEPT_WINDOW)?;
    let deadline = Instant::now() + SESSION_WINDOW;
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local, "decision sync identity changed");
    let context = sync_context(review, from, local);
    let mut channel = authenticate(stream, &context, &signer, key, false, deadline)?;
    let history: OwnerlessAuthorityHistoryV1 = serde_json::from_slice(&channel.receive(MAX_OWNERLESS_HISTORY_BYTES)?)?;
    let next = verify_ownerless_history(review.expected_world, &history)?;
    pin(&next, review.expected_head)?;
    ensure!(
        pinned_base(&history, review.expected_world, review.expected_base)? == base,
        "sync ancestry differs from pinned base"
    );
    ensure!(current_key(&next, from)? == key, "sync sender retired from target");
    ensure!(current_key(&next, local)? == signer.public_key(), "sync recipient not in target roster");
    let bytes = channel.receive(MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    let (store, mut journal, current, current_base) = sync_store(paths, review)?;
    ensure!(
        current_base == base && store.identity.peer_id() == local && store.identity.public_key() == signer.public_key(),
        "decision sync base or identity changed"
    );
    remaining(deadline)?;
    store.accept_verified(&mut journal, &current, history, &next, &bytes)?;
    let body = SyncReceiptBody::new(review, &next, local);
    let receipt = SyncReceipt { signature: store.identity.sign(&body.signing_bytes()?), body };
    channel.send(&encoded(&receipt, MAX_OWNERLESS_DOCUMENT_BYTES)?, MAX_OWNERLESS_DOCUMENT_BYTES)
}

fn push_sync(paths: &DataPaths, review: &SyncReview, peer: &str) -> Result<()> {
    let (remote, address) = endpoint(peer)?;
    let (store, _, head, base) = sync_store(paths, review)?;
    pin(&head, review.expected_head)?;
    let local = store.identity.peer_id();
    ensure!(local != remote, "decision peer must be remote");
    ensure!(current_key(&base, local)? == store.identity.public_key(), "sync sender not in base roster");
    ensure!(current_key(&head, local)? == store.identity.public_key(), "sync sender retired from target");
    let remote_key = current_key(&head, remote).context("sync recipient not in target roster")?;
    drop(store);
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local, "decision sync identity changed");
    let context = sync_context(review, local, remote);
    let deadline = Instant::now() + SESSION_WINDOW;
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    let mut channel = authenticate(stream, &context, &signer, remote_key, true, deadline)?;
    // Do not disclose certified checkpoint bytes if local acceptance or
    // recipient eligibility changed during handshake. Hold the stable lock
    // through the bounded send and receipt.
    let (store, journal, current, current_base) = sync_store(paths, review)?;
    ensure!(
        current == head && current_base == base && store.identity.peer_id() == local,
        "decision sync sender head or identity changed"
    );
    let bytes = store.state(current.checkpoint())?;
    channel.send(&encoded(&journal.history, MAX_OWNERLESS_HISTORY_BYTES)?, MAX_OWNERLESS_HISTORY_BYTES)?;
    channel.send(&bytes, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    let receipt: SyncReceipt = serde_json::from_slice(&channel.receive(MAX_OWNERLESS_DOCUMENT_BYTES)?)?;
    ensure!(receipt.body == SyncReceiptBody::new(review, &current, remote), "decision sync receipt context mismatch");
    verify_auth(&remote_key, &receipt.body.signing_bytes()?, &receipt.signature)?;
    remaining(deadline)?;
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_checkpoint_synced",
            "receipt": receipt, "live_quorum_verified": false, "runtime_activation": false,
            "write_authority": false, "canonical_authority": false,
            "player_admission": false, "worker_admission": false,
        }),
        MAX_OWNERLESS_DOCUMENT_BYTES,
    )
}

pub(super) fn run(paths: &DataPaths, command: DecisionCommand) -> Result<()> {
    match command {
        DecisionCommand::ServeVote { review, from, bind } => serve_vote(paths, &review, from, bind),
        DecisionCommand::RequestVote { review, checkpoint, peer } => request_vote(paths, &review, &checkpoint, &peer),
        DecisionCommand::Collect { review, checkpoint, peers } => collect(paths, &review, &checkpoint, &peers),
        DecisionCommand::ServeSync { review, from, bind } => serve_sync(paths, &review, from, bind),
        DecisionCommand::PushSync { review, peer } => push_sync(paths, &review, &peer),
        DecisionCommand::ServeRecovery { review, from, bind } => recovery::serve(paths, &review, from, bind),
        DecisionCommand::Recover { review, peers } => recovery::recover(paths, &review, &peers),
        DecisionCommand::ServeHead { review, from, bind } => discovery::serve(paths, &review, from, bind),
        DecisionCommand::PullHead { review, peers } => discovery::pull(paths, &review, &peers),
    }
}
