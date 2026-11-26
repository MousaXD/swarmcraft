//! Bounded certified-head discovery from explicit, independently pinned
//! base-roster peers. Never a freshness claim, new vote, or game permit.
use super::{
    accept, authenticate, current_key, endpoint, identity, pin, pinned_base, print, private_ip, remaining,
    ChannelContext, DiscoveryReview, Store, ACCEPT_WINDOW, COLLECTION_NETWORK_WINDOW, SESSION_WINDOW,
};
use crate::ownerless_authority::{encoded, report, Journal};
use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeSet,
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
    time::Instant,
};
use swarm_consensus::ownerless_authority::{
    ownerless_history_extends, verify_ownerless_history, VerifiedOwnerlessHead,
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{
    ownerless_checkpoint_commitment, Hash32, OwnerlessAuthorityHistoryV1, PeerId, MAX_OWNERLESS_CHECKPOINT_BYTES,
    MAX_OWNERLESS_DOCUMENT_BYTES, MAX_OWNERLESS_HISTORY_BYTES, MAX_OWNERLESS_VALIDATORS,
};

const DOMAIN: &[u8] = b"swarmcraft/ownerless-decision/certified-head-discovery/v1\0";

fn context(review: &DiscoveryReview, initiator: PeerId, responder: PeerId) -> ChannelContext {
    ChannelContext {
        domain: DOMAIN,
        world: review.expected_world,
        head: review.expected_base,
        // Unknown target is intentional and domain-separated from all
        // vote, recovery and exact-target synchronization channels.
        proposal: Hash32::default(),
        initiator,
        responder,
    }
}

fn open(paths: &DataPaths, review: &DiscoveryReview) -> Result<(Store, Journal, VerifiedOwnerlessHead)> {
    ensure!(review.confirm, "explicit --confirm required for certified head discovery");
    let store = Store::try_open(paths, review.expected_world)?;
    let (journal, head) = store.load(review.expected_world)?;
    Ok((store, journal, head))
}

fn eligible(base: &VerifiedOwnerlessHead, head: &VerifiedOwnerlessHead, peer: PeerId, key: [u8; 32]) -> Result<()> {
    ensure!(current_key(base, peer)? == key, "discovery peer not in base roster");
    ensure!(
        current_key(head, peer).context("discovery peer retired or not in target roster")? == key,
        "discovery peer key changed"
    );
    Ok(())
}

pub(super) fn serve(paths: &DataPaths, review: &DiscoveryReview, from: PeerId, bind: SocketAddr) -> Result<()> {
    ensure!(private_ip(bind.ip()), "decision listener must bind an explicit private address");
    let (store, journal, head) = open(paths, review)?;
    let base = pinned_base(&journal.history, review.expected_world, review.expected_base)?;
    let local = store.identity.peer_id();
    let own_key = store.identity.public_key();
    ensure!(local != from, "decision peer must be remote");
    let remote_key = current_key(&base, from)?;
    eligible(&base, &head, from, remote_key)?;
    eligible(&base, &head, local, own_key)?;
    drop(store);

    let listener = TcpListener::bind(bind)?;
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_head_listener",
            "address": listener.local_addr()?.to_string(), "validator": local.to_string(),
            "world_id": review.expected_world.to_string(), "base": review.expected_base.to_string(),
            "runtime_activation": false, "live_quorum_verified": false,
        }),
        MAX_OWNERLESS_DOCUMENT_BYTES,
    )?;
    let stream = accept(listener, Instant::now() + ACCEPT_WINDOW)?;
    let deadline = Instant::now() + SESSION_WINDOW;
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local && signer.public_key() == own_key, "discovery responder identity changed");
    let mut channel = authenticate(stream, &context(review, from, local), &signer, remote_key, false, deadline)?;

    // Read only after mutual authentication; recheck the accepted ancestry
    // and both participants under the stable lock. An intervening certified
    // extension may be served, but never to/from a newly retired validator.
    let (store, journal, head) = open(paths, review)?;
    let current_base = pinned_base(&journal.history, review.expected_world, review.expected_base)?;
    ensure!(
        current_base == base && store.identity.peer_id() == local && store.identity.public_key() == own_key,
        "discovery responder base or identity changed"
    );
    eligible(&base, &head, from, remote_key)?;
    eligible(&base, &head, local, own_key)?;
    let bytes = store.state(head.checkpoint())?;
    remaining(deadline)?;
    channel.send(&encoded(&journal.history, MAX_OWNERLESS_HISTORY_BYTES)?, MAX_OWNERLESS_HISTORY_BYTES)?;
    channel.send(&bytes, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    // No acknowledgment is required or interpreted as remote acceptance.
    remaining(deadline)?;
    Ok(())
}

fn fetch(
    address: SocketAddr,
    context: &ChannelContext,
    signer: &PeerIdentity,
    key: [u8; 32],
    deadline: Instant,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    let mut channel = authenticate(stream, context, signer, key, true, deadline)?;
    let history = channel.receive(MAX_OWNERLESS_HISTORY_BYTES)?;
    let checkpoint = channel.receive(MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    remaining(deadline)?;
    Ok((history, checkpoint))
}

struct Candidate {
    history: OwnerlessAuthorityHistoryV1,
    head: VerifiedOwnerlessHead,
    bytes: Vec<u8>,
}

impl Candidate {
    fn verify(
        review: &DiscoveryReview,
        base_history: &OwnerlessAuthorityHistoryV1,
        base: &VerifiedOwnerlessHead,
        signer: &PeerIdentity,
        peer: PeerId,
        raw: &[u8],
        bytes: Vec<u8>,
    ) -> Result<Self> {
        let history = serde_json::from_slice(raw).context("invalid discovered history")?;
        let head = verify_ownerless_history(review.expected_world, &history)?;
        ensure!(
            ownerless_history_extends(base_history, &history)
                && pinned_base(&history, review.expected_world, review.expected_base)? == *base,
            "discovered history conflicts with accepted base"
        );
        eligible(base, &head, signer.peer_id(), signer.public_key())?;
        eligible(base, &head, peer, current_key(base, peer)?)?;
        ensure!(ownerless_checkpoint_commitment(&bytes)? == head.checkpoint(), "discovered checkpoint mismatch");
        Ok(Self { history, head, bytes })
    }

    fn merge(self, other: Self) -> Result<Self> {
        // Comparing sequence numbers alone can hide divergent certified
        // histories. Every successful verified response must be comparable.
        // Same proposal chain may have different valid certificate witnesses.
        if self.head.sequence() == other.head.sequence() {
            ensure!(
                self.head == other.head
                    && self.bytes == other.bytes
                    && ownerless_history_extends(&self.history, &other.history),
                "conflicting certified discovery heads"
            );
            return Ok(self);
        }
        let (short, long) = if self.head.sequence() < other.head.sequence() { (self, other) } else { (other, self) };
        ensure!(ownerless_history_extends(&short.history, &long.history), "conflicting certified discovery histories");
        Ok(long)
    }
}

pub(super) fn pull(paths: &DataPaths, review: &DiscoveryReview, peers: &[String]) -> Result<()> {
    let (store, journal, base) = open(paths, review)?;
    pin(&base, review.expected_base)?;
    let local = store.identity.peer_id();
    let own_key = store.identity.public_key();
    ensure!(current_key(&base, local)? == own_key, "discovery requester key mismatch");
    ensure!(!peers.is_empty() && peers.len() < MAX_OWNERLESS_VALIDATORS, "invalid discovery peer count");
    let mut identities = BTreeSet::from([local]);
    let mut addresses = BTreeSet::new();
    let mut targets = Vec::new();
    for raw in peers {
        let (peer, address) = endpoint(raw)?;
        let key = current_key(&base, peer)?;
        ensure!(identities.insert(peer) && addresses.insert(address), "duplicate discovery validator or endpoint");
        targets.push((peer, address, key));
    }
    let base_history = journal.history;
    drop(store);
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local && signer.public_key() == own_key, "discovery requester identity changed");
    let started = Instant::now();
    let deadline = started + SESSION_WINDOW;
    let network_deadline = started + COLLECTION_NETWORK_WINDOW;
    let attempts = thread::scope(|scope| {
        let handles: Vec<_> = targets
            .into_iter()
            .map(|(peer, address, key)| {
                let context = context(review, local, peer);
                let signer = &signer;
                (peer, scope.spawn(move || fetch(address, &context, signer, key, network_deadline)))
            })
            .collect();
        handles.into_iter().map(|(peer, handle)| (peer, handle.join())).collect::<Vec<_>>()
    });
    let mut selected: Option<Candidate> = None;
    let mut responded = Vec::new();
    let mut unavailable = Vec::new();
    for (peer, attempt) in attempts {
        match attempt {
            Ok(Ok((raw, bytes))) => {
                // Unlike network unavailability, a complete authenticated
                // but invalid/conflicting reply refuses the entire pull.
                let candidate = Candidate::verify(review, &base_history, &base, &signer, peer, &raw, bytes)?;
                selected = Some(match selected {
                    Some(existing) => existing.merge(candidate)?,
                    None => candidate,
                });
                responded.push(peer.to_string());
            }
            _ => unavailable.push(peer.to_string()),
        }
    }
    let selected = selected.context("no verified certified head response")?;
    let (store, mut journal, current) = open(paths, review)?;
    ensure!(
        current == base && store.identity.peer_id() == local && store.identity.public_key() == own_key,
        "discovery requester base or identity changed"
    );
    // This also preserves a concurrently written pending vote on a
    // same-head reply; only a verified extension can resolve that lock.
    remaining(deadline)?;
    store.accept_verified(&mut journal, &current, selected.history, &selected.head, &selected.bytes)?;
    remaining(deadline)?;
    let mut result = report(&selected.head, Some(&journal));
    result["classification"] = "ownerless_reference_head_pulled".into();
    result["base"] = review.expected_base.to_string().into();
    result["advanced"] = (current != selected.head).into();
    result["responded"] = serde_json::json!(responded);
    result["unavailable"] = serde_json::json!(unavailable);
    result["globally_latest_verified"] = false.into();
    result["canonical_authority"] = false.into();
    result["write_authority"] = false.into();
    result["worker_admission"] = false.into();
    print(&result, MAX_OWNERLESS_DOCUMENT_BYTES)
}
