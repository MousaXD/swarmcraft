//! Recover exactly one reviewed decision after collector loss. Read existing
//! evidence; never create a vote, lease, or canonical Minecraft permit.
use super::{
    accept, authenticate, current_key, endpoint, identity, ownerless_vote_key, pin, pinned_base, print, private_ip,
    proposal, remaining, verify_ownerless_vote, ChannelContext, Review, Store, ACCEPT_WINDOW,
    COLLECTION_NETWORK_WINDOW, SESSION_WINDOW,
};
use crate::ownerless_authority::{encoded, Journal};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
    time::Instant,
};
use swarm_consensus::ownerless_authority::{apply_ownerless_certificate, VerifiedOwnerlessHead};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{
    ownerless_checkpoint_commitment, OwnerlessAuthorityActionV1, OwnerlessAuthorityCertificateV1,
    OwnerlessAuthorityProposalV1, OwnerlessAuthorityVoteV1, PeerId, MAX_OWNERLESS_CHECKPOINT_BYTES,
    MAX_OWNERLESS_DOCUMENT_BYTES, MAX_OWNERLESS_HISTORY_BYTES, MAX_OWNERLESS_VALIDATORS,
};

const DOMAIN: &[u8] = b"swarmcraft/ownerless-decision/recovery/v1\0";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Evidence {
    Pending(OwnerlessAuthorityVoteV1),
    Certified(OwnerlessAuthorityCertificateV1),
}

struct View {
    store: Store,
    journal: Journal,
    head: VerifiedOwnerlessHead,
    base: VerifiedOwnerlessHead,
    approved: OwnerlessAuthorityProposalV1,
}

impl View {
    fn open(paths: &DataPaths, review: &Review) -> Result<Self> {
        ensure!(review.confirm, "explicit --confirm required for certified decision recovery");
        let store = Store::try_open(paths, review.expected_world)?;
        let (journal, head) = store.load(review.expected_world)?;
        ensure!(
            head.hash() == review.expected_head || head.hash() == review.expected_proposal,
            "decision recovery local head is not pinned base or target"
        );
        let base = pinned_base(&journal.history, review.expected_world, review.expected_head)?;
        let approved = proposal(&review.proposal, review.expected_proposal, &base)?;
        if head.hash() != base.hash() {
            ensure!(
                journal.history.records.last().is_some_and(|r| r.proposal == approved),
                "accepted decision differs from exact reviewed proposal"
            );
        }
        Ok(Self { store, journal, head, base, approved })
    }

    fn is_accepted(&self) -> bool {
        self.head.hash() != self.base.hash()
    }

    fn evidence(&self) -> Result<Option<(Evidence, Vec<u8>)>> {
        let local = self.store.identity.peer_id();
        if self.is_accepted() {
            // A removed validator cannot serve the certified target.
            ensure!(
                current_key(&self.head, local)? == self.store.identity.public_key(),
                "recovery responder retired from target"
            );
            let certificate = self.journal.history.records.last().context("missing accepted recovery certificate")?;
            let bytes = self.store.state(self.head.checkpoint())?;
            return Ok(Some((Evidence::Certified(certificate.clone()), bytes)));
        }
        if let Some(pending) = self.journal.pending.as_ref().filter(|p| p.proposal == self.approved) {
            ensure!(pending.vote.validator == local, "foreign pending recovery vote");
            verify_ownerless_vote(&self.base, &self.approved, &pending.vote)?;
            let bytes = self.store.state(self.base.candidate_checkpoint(&self.approved)?)?;
            return Ok(Some((Evidence::Pending(pending.vote.clone()), bytes)));
        }
        Ok(None)
    }
}

fn requester_key(
    base: &VerifiedOwnerlessHead,
    approved: &OwnerlessAuthorityProposalV1,
    peer: PeerId,
) -> Result<[u8; 32]> {
    let key = current_key(base, peer)?;
    if let OwnerlessAuthorityActionV1::Reconfigure { validators } = &approved.action {
        ensure!(
            validators.iter().any(|v| v.peer_id == peer && v.public_key == key),
            "recovery requester must remain in target roster"
        );
    }
    Ok(key)
}

fn context(review: &Review, initiator: PeerId, responder: PeerId) -> ChannelContext {
    ChannelContext {
        domain: DOMAIN,
        world: review.expected_world,
        head: review.expected_head,
        proposal: review.expected_proposal,
        initiator,
        responder,
    }
}

pub(super) fn serve(paths: &DataPaths, review: &Review, from: PeerId, bind: SocketAddr) -> Result<()> {
    ensure!(private_ip(bind.ip()), "decision listener must bind an explicit private address");
    let view = View::open(paths, review)?;
    let local = view.store.identity.peer_id();
    ensure!(local != from, "decision peer must be remote");
    let remote_key = requester_key(&view.base, &view.approved, from)?;
    ensure!(
        ownerless_vote_key(&view.base, &view.approved, local)? == view.store.identity.public_key(),
        "recovery responder key mismatch"
    );
    ensure!(view.evidence()?.is_some(), "no matching durable recovery evidence");
    let base = view.base.clone();
    let approved = view.approved.clone();
    drop(view);
    let listener = TcpListener::bind(bind)?;
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_recovery_listener",
            "address": listener.local_addr()?.to_string(), "validator": local.to_string(),
            "world_id": review.expected_world.to_string(), "proposal": review.expected_proposal.to_string(),
            "runtime_activation": false,
        }),
        MAX_OWNERLESS_DOCUMENT_BYTES,
    )?;
    let stream = accept(listener, Instant::now() + ACCEPT_WINDOW)?;
    let deadline = Instant::now() + SESSION_WINDOW;
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local, "decision recovery identity changed");
    let mut channel = authenticate(stream, &context(review, from, local), &signer, remote_key, false, deadline)?;
    // Recheck before disclosure and hold the stable journal lock during
    // the bounded response. A certified transition may have completed
    // during authentication; never serve a retired or advanced profile.
    let view = View::open(paths, review)?;
    ensure!(
        view.base == base && view.approved == approved && view.store.identity.peer_id() == local,
        "decision recovery responder context changed"
    );
    let (evidence, bytes) = view.evidence()?.context("no matching durable recovery evidence")?;
    channel.send(&encoded(&evidence, MAX_OWNERLESS_DOCUMENT_BYTES)?, MAX_OWNERLESS_DOCUMENT_BYTES)?;
    channel.send(&bytes, MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    remaining(deadline)?;
    Ok(())
}

fn fetch(
    address: SocketAddr,
    context: &ChannelContext,
    signer: &PeerIdentity,
    key: [u8; 32],
    deadline: Instant,
) -> Result<(Evidence, Vec<u8>)> {
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    let mut channel = authenticate(stream, context, signer, key, true, deadline)?;
    let evidence = serde_json::from_slice(&channel.receive(MAX_OWNERLESS_DOCUMENT_BYTES)?)?;
    let bytes = channel.receive(MAX_OWNERLESS_CHECKPOINT_BYTES)?;
    remaining(deadline)?;
    Ok((evidence, bytes))
}

// Only accept already signed votes or a fully verified certificate. The
// responding peer's authenticated identity is not itself a vote. Never
// manufacture a local vote to reach quorum in a recovery attempt.
struct Recovered {
    votes: BTreeMap<PeerId, OwnerlessAuthorityVoteV1>,
    certificate: Option<OwnerlessAuthorityCertificateV1>,
    bytes: Option<Vec<u8>>,
}

impl Recovered {
    fn add(
        &mut self,
        base: &VerifiedOwnerlessHead,
        approved: &OwnerlessAuthorityProposalV1,
        peer: PeerId,
        evidence: Evidence,
        bytes: Vec<u8>,
    ) -> Result<()> {
        ensure!(
            ownerless_checkpoint_commitment(&bytes)? == base.candidate_checkpoint(approved)?,
            "recovered checkpoint commitment mismatch"
        );
        ensure!(self.bytes.as_ref().is_none_or(|existing| *existing == bytes), "recovered checkpoint bytes differ");
        match evidence {
            Evidence::Pending(vote) => {
                ensure!(vote.validator == peer, "recovered vote has wrong validator");
                verify_ownerless_vote(base, approved, &vote)?;
                self.votes.insert(peer, vote);
            }
            Evidence::Certified(certificate) => {
                ensure!(certificate.proposal == *approved, "recovered certificate does not match reviewed proposal");
                let head = apply_ownerless_certificate(base, &certificate)?;
                current_key(&head, peer).context("recovery certificate sender retired from target")?;
                self.certificate.get_or_insert(certificate);
            }
        }
        self.bytes.get_or_insert(bytes);
        Ok(())
    }
}

pub(super) fn recover(paths: &DataPaths, review: &Review, peers: &[String]) -> Result<()> {
    let view = View::open(paths, review)?;
    let local = view.store.identity.peer_id();
    ensure!(
        requester_key(&view.base, &view.approved, local)? == view.store.identity.public_key(),
        "recovery requester key mismatch"
    );
    if view.is_accepted() {
        // Exact local accepted history is already durable and verified.
        return print(&view.journal.history, MAX_OWNERLESS_HISTORY_BYTES);
    }
    ensure!(!peers.is_empty() && peers.len() < MAX_OWNERLESS_VALIDATORS * 2, "invalid recovery peer count");
    let mut identities = BTreeSet::from([local]);
    let mut addresses = BTreeSet::new();
    let mut targets = Vec::new();
    for raw in peers {
        let (peer, address) = endpoint(raw)?;
        let key = ownerless_vote_key(&view.base, &view.approved, peer)?;
        ensure!(identities.insert(peer) && addresses.insert(address), "duplicate recovery validator or endpoint");
        targets.push((peer, address, key));
    }
    let mut recovered = Recovered { votes: BTreeMap::new(), certificate: None, bytes: None };
    if let Some((evidence, bytes)) = view.evidence()? {
        recovered.add(&view.base, &view.approved, local, evidence, bytes)?;
    }
    let base = view.base.clone();
    let approved = view.approved.clone();
    drop(view);
    let signer = identity(paths)?;
    ensure!(signer.peer_id() == local, "decision recovery identity changed");
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
    for (peer, attempt) in attempts {
        match attempt {
            Ok(Ok((evidence, bytes))) => {
                if let Err(err) = recovered.add(&base, &approved, peer, evidence, bytes) {
                    eprintln!("invalid recovery evidence from {peer}: {err}");
                }
            }
            _ => eprintln!("unavailable recovery voter: {peer}"),
        }
    }
    let certificate = recovered.certificate.unwrap_or_else(|| OwnerlessAuthorityCertificateV1 {
        proposal: approved.clone(),
        votes: recovered.votes.into_values().collect(),
    });
    let next = apply_ownerless_certificate(&base, &certificate)?;
    pin(&next, review.expected_proposal)?;
    let bytes = recovered.bytes.context("no verified recovery checkpoint bytes")?;
    let mut view = View::open(paths, review)?;
    ensure!(
        view.base == base && view.approved == approved && view.store.identity.peer_id() == local,
        "decision recovery requester context changed"
    );
    remaining(deadline)?;
    if !view.is_accepted() {
        let mut history = view.journal.history.clone();
        history.records.push(certificate);
        view.store.accept_verified(&mut view.journal, &view.head, history, &next, &bytes)?;
    } else {
        ensure!(
            view.head == next && view.store.state(next.checkpoint())? == bytes,
            "concurrent recovery target differs"
        );
    }
    remaining(deadline)?;
    print(&view.journal.history, MAX_OWNERLESS_HISTORY_BYTES)
}
