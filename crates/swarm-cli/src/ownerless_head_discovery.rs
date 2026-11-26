//! Bounded certified-head discovery from explicit, independently pinned
//! base-roster peers. Never a freshness claim, new vote, or game permit.
use super::{
    accept, authenticate, current_key, endpoint, identity, pin, pinned_base, print, private_ip, remaining,
    ChannelContext, DiscoveryReview, ReconciliationReview, Store, ACCEPT_WINDOW, COLLECTION_NETWORK_WINDOW,
    SESSION_WINDOW,
};
use crate::ownerless_authority::{encoded, report, Journal};
use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeSet,
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
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
const SERVICE_DOMAIN: &[u8] = b"swarmcraft/ownerless-decision/certified-head-reconciliation/v1\0";

fn context(review: &DiscoveryReview, anchor: Option<Hash32>, initiator: PeerId, responder: PeerId) -> ChannelContext {
    ChannelContext {
        domain: if anchor.is_some() { SERVICE_DOMAIN } else { DOMAIN },
        world: review.expected_world,
        head: anchor.unwrap_or(review.expected_base),
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
    let mut channel = authenticate(stream, &context(review, None, from, local), &signer, remote_key, false, deadline)?;

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
    requested_base: Option<Hash32>,
    deadline: Instant,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    let mut channel = authenticate(stream, context, signer, key, true, deadline)?;
    if let Some(base) = requested_base {
        // Dynamic base is disclosed only inside the distinct mutually
        // authenticated service channel, never in a public selector.
        channel.send_chunk(&base.0)?;
    }
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
    let result = pull_once(paths, review, peers, None, None, Instant::now() + SESSION_WINDOW)?;
    print(&result, MAX_OWNERLESS_DOCUMENT_BYTES)
}

fn pull_once(
    paths: &DataPaths,
    review: &DiscoveryReview,
    peers: &[String],
    anchor: Option<Hash32>,
    expected_identity: Option<(PeerId, [u8; 32])>,
    run_deadline: Instant,
) -> Result<serde_json::Value> {
    let (store, journal, base) = open(paths, review)?;
    pin(&base, review.expected_base)?;
    let local = store.identity.peer_id();
    let own_key = store.identity.public_key();
    ensure!(
        expected_identity.is_none_or(|expected| expected == (local, own_key)),
        "reconciliation requester identity changed"
    );
    ensure!(current_key(&base, local)? == own_key, "discovery requester key mismatch");
    let anchor_head = match anchor {
        Some(hash) => pinned_base(&journal.history, review.expected_world, hash)?,
        None => base.clone(),
    };
    eligible(&anchor_head, &base, local, own_key)?;
    ensure!(!peers.is_empty() && peers.len() < MAX_OWNERLESS_VALIDATORS, "invalid discovery peer count");
    let mut identities = BTreeSet::from([local]);
    let mut addresses = BTreeSet::new();
    let mut targets = Vec::new();
    let mut excluded = Vec::new();
    for raw in peers {
        let (peer, address) = endpoint(raw)?;
        let key = current_key(&anchor_head, peer)?;
        ensure!(identities.insert(peer) && addresses.insert(address), "duplicate discovery validator or endpoint");
        if anchor.is_some() && current_key(&base, peer).is_err() {
            excluded.push(peer.to_string());
            continue;
        }
        ensure!(current_key(&base, peer)? == key, "discovery peer key changed");
        targets.push((peer, address, key));
    }
    let base_history = journal.history;
    drop(store);
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local && signer.public_key() == own_key, "discovery requester identity changed");
    let started = Instant::now();
    let deadline = (started + SESSION_WINDOW).min(run_deadline);
    let network_deadline = (started + COLLECTION_NETWORK_WINDOW).min(deadline);
    let attempts = thread::scope(|scope| {
        let handles: Vec<_> = targets
            .into_iter()
            .map(|(peer, address, key)| {
                let context = context(review, anchor, local, peer);
                let signer = &signer;
                let requested_base = anchor.map(|_| review.expected_base);
                (peer, scope.spawn(move || fetch(address, &context, signer, key, requested_base, network_deadline)))
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
    ensure!(selected.is_some() || anchor.is_some(), "no verified certified head response");
    let (store, mut journal, current) = open(paths, review)?;
    ensure!(
        current == base && store.identity.peer_id() == local && store.identity.public_key() == own_key,
        "discovery requester base or identity changed"
    );
    // This also preserves a concurrently written pending vote on a
    // same-head reply; only a verified extension can resolve that lock.
    remaining(deadline)?;
    let observed = selected.is_some();
    let accepted = match selected {
        Some(selected) => {
            store.accept_verified(&mut journal, &current, selected.history, &selected.head, &selected.bytes)?;
            selected.head
        }
        // A retryable unavailable round is read-only. It cannot clear a
        // pending vote, choose a new base or imply a fresh quorum.
        None => current.clone(),
    };
    remaining(deadline)?;
    let mut result = report(&accepted, Some(&journal));
    result["classification"] = "ownerless_reference_head_pulled".into();
    result["base"] = review.expected_base.to_string().into();
    result["advanced"] = (current != accepted).into();
    result["responded"] = serde_json::json!(responded);
    result["unavailable"] = serde_json::json!(unavailable);
    result["globally_latest_verified"] = false.into();
    result["canonical_authority"] = false.into();
    result["write_authority"] = false.into();
    result["worker_admission"] = false.into();
    if let Some(anchor) = anchor {
        result["classification"] = "ownerless_reference_reconciliation_round".into();
        result["anchor"] = anchor.to_string().into();
        result["observed_certified_head"] = observed.into();
        result["excluded"] = serde_json::json!(excluded);
    }
    Ok(result)
}

// A persistent listener is still a single expressly authorized peer pair,
// not a public multiplexing/discovery service. Every request has fresh keys.
pub(super) fn serve_heads(
    paths: &DataPaths,
    review: &ReconciliationReview,
    from: PeerId,
    bind: SocketAddr,
    max_sessions: u32,
    run_seconds: u64,
) -> Result<()> {
    ensure!(review.confirm, "explicit --confirm required for certified head reconciliation");
    ensure!((1..=128).contains(&max_sessions) && (1..=900).contains(&run_seconds), "invalid reconciliation bounds");
    ensure!(private_ip(bind.ip()), "decision listener must bind an explicit private address");
    let scope =
        DiscoveryReview { expected_world: review.expected_world, expected_base: review.expected_anchor, confirm: true };
    let (store, journal, head) = open(paths, &scope)?;
    let anchor = pinned_base(&journal.history, review.expected_world, review.expected_anchor)?;
    let local = store.identity.peer_id();
    let own_key = store.identity.public_key();
    ensure!(local != from, "decision peer must be remote");
    let remote_key = current_key(&anchor, from)?;
    eligible(&anchor, &head, local, own_key)?;
    eligible(&anchor, &head, from, remote_key)?;
    drop(store);
    let listener = TcpListener::bind(bind)?;
    // Nonblocking accept with a single absolute service window. The
    // existing accept helper owns a listener, so keep our original fd.
    let deadline = Instant::now() + Duration::from_secs(run_seconds);
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_reconciliation_listener",
            "address": listener.local_addr()?.to_string(), "validator": local.to_string(),
            "world_id": review.expected_world.to_string(), "anchor": review.expected_anchor.to_string(),
            "runtime_activation": false, "live_quorum_verified": false,
        }),
        MAX_OWNERLESS_DOCUMENT_BYTES,
    )?;
    let mut attempted = 0;
    let mut served = 0;
    let mut rejected = 0;
    let mut previous = head;
    while attempted < max_sessions && Instant::now() < deadline {
        let stream = match accept(listener.try_clone()?, deadline) {
            Ok(stream) => stream,
            Err(_) if Instant::now() >= deadline => break,
            Err(err) => return Err(err),
        };
        attempted += 1;
        let session_deadline = (Instant::now() + SESSION_WINDOW).min(deadline);
        // Recheck durable authorization BEFORE giving any identity proof,
        // and again after authenticating the requested base.
        let (store, journal, current) = open(paths, &scope)?;
        ensure!(
            store.identity.peer_id() == local
                && store.identity.public_key() == own_key
                && pinned_base(&journal.history, review.expected_world, review.expected_anchor)? == anchor
                && pinned_base(&journal.history, review.expected_world, previous.hash())? == previous,
            "reconciliation listener identity or ancestry changed"
        );
        eligible(&anchor, &current, local, own_key)?;
        eligible(&anchor, &current, from, remote_key)?;
        previous = current;
        drop(store);
        let signer = identity(paths)?;
        ensure!(
            signer.peer_id() == local && signer.public_key() == own_key,
            "reconciliation listener identity changed"
        );
        let request = (|| -> Result<_> {
            let mut channel = authenticate(
                stream,
                &context(&scope, Some(review.expected_anchor), from, local),
                &signer,
                remote_key,
                false,
                session_deadline,
            )?;
            let base = Hash32(channel.recv_chunk(32)?.try_into().expect("exact base length"));
            Ok((channel, base))
        })();
        let (mut channel, requested) = match request {
            Ok(pair) => pair,
            Err(_) => {
                rejected += 1;
                continue;
            }
        };
        let (store, journal, current) = open(paths, &scope)?;
        ensure!(
            store.identity.peer_id() == local
                && store.identity.public_key() == own_key
                && pinned_base(&journal.history, review.expected_world, review.expected_anchor)? == anchor
                && pinned_base(&journal.history, review.expected_world, previous.hash())? == previous,
            "reconciliation listener identity or ancestry changed"
        );
        eligible(&anchor, &current, local, own_key)?;
        eligible(&anchor, &current, from, remote_key)?;
        previous = current.clone();
        // A requester ahead of this listener or on an unrelated branch
        // gets no history. Require the requested base to descend from
        // the common anchor and both participants to be eligible there.
        let base = match pinned_base(&journal.history, review.expected_world, requested) {
            Ok(base)
                if base.sequence() >= anchor.sequence()
                    && eligible(&anchor, &base, from, remote_key).is_ok()
                    && eligible(&anchor, &base, local, own_key).is_ok() =>
            {
                base
            }
            _ => {
                rejected += 1;
                continue;
            }
        };
        ensure!(base.sequence() <= current.sequence(), "reconciliation base after accepted head");
        let bytes = store.state(current.checkpoint())?;
        let history = encoded(&journal.history, MAX_OWNERLESS_HISTORY_BYTES)?;
        match channel
            .send(&history, MAX_OWNERLESS_HISTORY_BYTES)
            .and_then(|_| channel.send(&bytes, MAX_OWNERLESS_CHECKPOINT_BYTES))
            .and_then(|_| remaining(session_deadline).map(|_| ()))
        {
            Ok(()) => served += 1,
            Err(_) => rejected += 1,
        }
    }
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_reconciliation_listener_finished",
            "attempted": attempted, "served": served, "rejected": rejected,
            "durable_remote_acceptance_verified": false, "runtime_activation": false,
        }),
        MAX_OWNERLESS_DOCUMENT_BYTES,
    )
}

pub(super) fn reconcile_heads(
    paths: &DataPaths,
    review: &ReconciliationReview,
    peers: &[String],
    rounds: u32,
    interval_seconds: u64,
    run_seconds: u64,
) -> Result<()> {
    ensure!(review.confirm, "explicit --confirm required for certified head reconciliation");
    ensure!(
        (1..=64).contains(&rounds) && (1..=30).contains(&interval_seconds) && (10..=900).contains(&run_seconds),
        "invalid reconciliation bounds"
    );
    let mut scope =
        DiscoveryReview { expected_world: review.expected_world, expected_base: review.expected_anchor, confirm: true };
    let (store, journal, head) = open(paths, &scope)?;
    let anchor = pinned_base(&journal.history, review.expected_world, review.expected_anchor)?;
    let local = store.identity.peer_id();
    let own_key = store.identity.public_key();
    eligible(&anchor, &head, local, own_key)?;
    drop(store);
    let deadline = Instant::now() + Duration::from_secs(run_seconds);
    let mut previous = head;
    let mut completed = 0;
    for index in 1..=rounds {
        // Reopen signed state every round; there is no saved transport
        // cursor or implicit reset after a restart. Stop on local errors
        // or certified conflict, not just retry every possible failure.
        if Instant::now() >= deadline {
            break;
        }
        let (store, journal, current) = open(paths, &scope)?;
        ensure!(
            store.identity.peer_id() == local
                && store.identity.public_key() == own_key
                && pinned_base(&journal.history, review.expected_world, review.expected_anchor)? == anchor
                && pinned_base(&journal.history, review.expected_world, previous.hash())? == previous,
            "reconciliation local identity or ancestry changed"
        );
        eligible(&anchor, &current, local, own_key)?;
        scope.expected_base = current.hash();
        drop(store);
        let mut result =
            pull_once(paths, &scope, peers, Some(review.expected_anchor), Some((local, own_key)), deadline)?;
        let accepted: Hash32 = result["head"].as_str().context("missing accepted reconciliation head")?.parse()?;
        // Check that the returned accepted head is still in the durable
        // ancestry, allowing a certified local advance after publication.
        let (store, journal, _) = open(paths, &scope)?;
        ensure!(
            store.identity.peer_id() == local && store.identity.public_key() == own_key,
            "reconciliation identity changed after round"
        );
        previous = pinned_base(&journal.history, review.expected_world, accepted)?;
        drop(store);
        result["round"] = index.into();
        print(&result, MAX_OWNERLESS_DOCUMENT_BYTES)?;
        completed += 1;
        if index < rounds {
            let interval = Duration::from_secs(interval_seconds);
            if Instant::now() + interval + SESSION_WINDOW > deadline {
                break;
            }
            thread::sleep(interval);
        }
    }
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_reconciliation_finished",
            "completed_rounds": completed, "requested_rounds": rounds,
            "all_rounds_completed": completed == rounds,
            "globally_latest_verified": false, "live_quorum_verified": false,
            "runtime_activation": false, "write_authority": false,
        }),
        MAX_OWNERLESS_DOCUMENT_BYTES,
    )
}
