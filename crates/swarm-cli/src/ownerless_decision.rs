//! Explicit, bounded reference decision exchange. Never a game route or permit.
//! Noise NN supplies ephemeral transport keys; both application identities
//! authenticate the exact handshake transcript before any decision data flows.
use super::{
    checkpoint, encoded, identity,
    observation::{private_ip, read_all, remaining, write_all},
    pin, print, proposal, Store,
};
use anyhow::{ensure, Context, Result};
use clap::{Args, Subcommand};
use serde::Serialize;
use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use swarm_consensus::ownerless_authority::{ownerless_vote_key, verify_ownerless_vote, VerifiedOwnerlessHead};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{
    Hash32, OwnerlessAuthorityProposalV1, OwnerlessAuthorityVoteV1, OwnerlessWorldIdV1, PeerId,
    MAX_OWNERLESS_CHECKPOINT_BYTES, MAX_OWNERLESS_DOCUMENT_BYTES,
};

const DOMAIN: &[u8] = b"swarmcraft/ownerless-decision/noise/v1\0";
const AUTH_DOMAIN: &[u8] = b"swarmcraft/ownerless-decision/channel-auth/v1\0";
const CHUNK: usize = 32 * 1024;
const TAG: usize = 16;
const ACCEPT_WINDOW: Duration = Duration::from_secs(15);
const SESSION_WINDOW: Duration = Duration::from_secs(10);

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
}

#[derive(Serialize)]
struct ChannelContext {
    world: OwnerlessWorldIdV1,
    head: Hash32,
    proposal: Hash32,
    initiator: PeerId,
    responder: PeerId,
}

impl ChannelContext {
    fn prologue(&self) -> Result<Vec<u8>> {
        let mut bytes = DOMAIN.to_vec();
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
        world: review.expected_world,
        head: review.expected_head,
        proposal: review.expected_proposal,
        initiator: local,
        responder: remote,
    };
    let deadline = Instant::now() + SESSION_WINDOW;
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    let mut channel = authenticate(stream, &context, &signer, remote_key, true, deadline)?;
    channel.send(&encoded(&approved, MAX_OWNERLESS_DOCUMENT_BYTES)?, MAX_OWNERLESS_DOCUMENT_BYTES)?;
    channel.send(&bytes, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    let vote: OwnerlessAuthorityVoteV1 = serde_json::from_slice(&channel.receive(MAX_OWNERLESS_DOCUMENT_BYTES)?)?;
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

pub(super) fn run(paths: &DataPaths, command: DecisionCommand) -> Result<()> {
    match command {
        DecisionCommand::ServeVote { review, from, bind } => serve_vote(paths, &review, from, bind),
        DecisionCommand::RequestVote { review, checkpoint, peer } => request_vote(paths, &review, &checkpoint, &peer),
    }
}
