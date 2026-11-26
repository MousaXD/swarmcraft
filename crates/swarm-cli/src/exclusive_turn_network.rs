//! Explicit, bounded, encrypted handoff of opaque reference state.
//! Both peers independently pin the exact signed offer. No game authority.
use super::{
    accept_reference, commit_reference, print_document, read_document, stage_reference, Journal, Phase, Pin, Store,
};
use crate::reference_channel::{authenticate, private_ip, remaining, verify_auth, Channel};
use anyhow::{ensure, Context, Result};
use clap::{Args, Subcommand};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use swarm_consensus::exclusive_turn::{
    verify_exclusive_turn_history, verify_exclusive_turn_offer, verify_exclusive_turn_receipt,
    VerifiedExclusiveTurnHead,
};
use swarm_core::DataPaths;
use swarm_protocol::{
    ExclusiveTurnCheckpointV1, ExclusiveTurnReceiptV1, ExclusiveTurnReleaseV1, ExclusiveTurnSignedOfferV1,
    ExclusiveTurnWorldIdV1, Hash32, PeerId, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES,
};

const DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/reference-handoff/v1\0";
const AUTH_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/reference-channel-auth/v1\0";
const ACK_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/reference-durable-acceptance/v1\0";
const RECORD_LIMIT: usize = 16 * 1024;
const ACCEPT_WINDOW: Duration = Duration::from_secs(15);
const SESSION_WINDOW: Duration = Duration::from_secs(15);

#[derive(Debug, Args)]
pub struct Review {
    #[command(flatten)]
    pin: Pin,
    #[arg(long)]
    expected_offer: Hash32,
    /// Independently review the exact offer and device identities.
    #[arg(long)]
    confirm: bool,
}

#[derive(Debug, Subcommand)]
pub enum NetworkCommand {
    /// Accept one reviewed handoff or retry from the exact other fixed device.
    ServeHandoff {
        #[command(flatten)]
        review: Review,
        /// Locally reviewed signed offer; must match the independently pinned hash.
        #[arg(long)]
        offer: PathBuf,
        #[arg(long)]
        from: PeerId,
        #[arg(long)]
        bind: SocketAddr,
    },
    /// Send the durable prepared offer, receive a receipt, relinquish, and deliver the release.
    /// After relinquishment, retry only the same exact still-current outbox.
    Handoff {
        #[command(flatten)]
        review: Review,
        /// Exact other fixed device and explicit private socket: identity@address.
        #[arg(long)]
        peer: String,
    },
}

#[derive(Serialize)]
struct ChannelContext {
    world: ExclusiveTurnWorldIdV1,
    base: Hash32,
    offer: Hash32,
    source: PeerId,
    recipient: PeerId,
}

impl ChannelContext {
    fn new(review: &Review, source: PeerId, recipient: PeerId) -> Self {
        Self {
            world: review.pin.expected_world,
            base: review.pin.expected_grant,
            offer: review.expected_offer,
            source,
            recipient,
        }
    }

    fn prologue(&self) -> Result<Vec<u8>> {
        let mut bytes = DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(self)?);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceBody {
    world: ExclusiveTurnWorldIdV1,
    base: Hash32,
    offer: Hash32,
    grant: Hash32,
    recipient: PeerId,
    epoch: u64,
    fence: u64,
    checkpoint: ExclusiveTurnCheckpointV1,
}

impl AcceptanceBody {
    fn for_release(release: &ExclusiveTurnReleaseV1) -> Result<Self> {
        let offer = &release.offer.offer;
        Ok(Self {
            world: offer.world_id,
            base: offer.parent_grant,
            offer: release.offer.offer_hash()?,
            grant: release.grant_hash()?,
            recipient: offer.recipient,
            epoch: offer.next_epoch,
            fence: offer.next_fence,
            checkpoint: offer.checkpoint,
        })
    }

    fn signing_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = ACK_DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(self)?);
        Ok(bytes)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Acceptance {
    body: AcceptanceBody,
    signature: Vec<u8>,
}

fn send<T: Serialize>(channel: &mut Channel, value: &T) -> Result<()> {
    channel.send(&postcard::to_allocvec(value)?, RECORD_LIMIT)
}

fn receive<T: DeserializeOwned + Serialize>(channel: &mut Channel) -> Result<T> {
    let bytes = channel.receive(RECORD_LIMIT)?;
    let value = postcard::from_bytes(&bytes)?;
    ensure!(postcard::to_allocvec(&value)? == bytes, "noncanonical exclusive-turn network record");
    Ok(value)
}

fn remote_key(head: &VerifiedExclusiveTurnHead, peer: PeerId, local: PeerId) -> Result<[u8; 32]> {
    ensure!(peer != local, "exclusive-turn peer must be remote");
    head.devices()
        .iter()
        .find(|d| d.peer_id == peer)
        .map(|d| d.public_key)
        .context("exclusive-turn peer is not the other fixed device")
}

fn endpoint(raw: &str) -> Result<(PeerId, SocketAddr)> {
    let (peer, address) = raw.split_once('@').context("exclusive-turn peer requires identity@address")?;
    let address: SocketAddr = address.parse()?;
    ensure!(private_ip(address.ip()) && address.port() != 0, "exclusive-turn endpoint must be explicit and private");
    Ok((peer.parse()?, address))
}

fn accept(listener: TcpListener, deadline: Instant) -> Result<TcpStream> {
    listener.set_nonblocking(true)?;
    loop {
        remaining(deadline)?;
        match listener.accept() {
            Ok((stream, remote)) => {
                ensure!(private_ip(remote.ip()), "exclusive-turn connection is not from a private address");
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10).min(remaining(deadline)?));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

struct View {
    store: Store,
    journal: Journal,
    head: VerifiedExclusiveTurnHead,
    base: VerifiedExclusiveTurnHead,
}

impl View {
    fn open(paths: &DataPaths, review: &Review) -> Result<Self> {
        ensure!(review.confirm, "explicit --confirm required for reviewed network handoff");
        let store = Store::open(paths, review.pin.expected_world)?;
        let (journal, head) = store.load(review.pin.expected_world)?;
        // Only the exact base or its immediate still-current release may
        // take part. Never reconcile arbitrary history or revive old turns.
        let base = if head.grant() == review.pin.expected_grant {
            head.clone()
        } else {
            let last = journal.history.releases.last().context("missing exact prior release")?;
            ensure!(
                last.offer.offer.parent_grant == review.pin.expected_grant
                    && last.offer.offer_hash()? == review.expected_offer
                    && last.grant_hash()? == head.grant(),
                "exclusive-turn local grant is not exact reviewed base or release"
            );
            let mut prefix = journal.history.clone();
            prefix.releases.pop();
            let base = verify_exclusive_turn_history(review.pin.expected_world, &prefix)?;
            ensure!(base.grant() == review.pin.expected_grant, "exclusive-turn prior grant differs");
            base
        };
        Ok(Self { store, journal, head, base })
    }

    fn local(&self) -> PeerId {
        self.store.identity.peer_id()
    }

    fn already_released(&self) -> bool {
        self.head.grant() != self.base.grant()
    }

    fn source_offer(&self, review: &Review, recipient: PeerId) -> Result<ExclusiveTurnSignedOfferV1> {
        let offer = if self.already_released() {
            ensure!(self.journal.phase == Phase::Released, "source is not in still-current released phase");
            self.journal.history.releases.last().context("release outbox absent")?.offer.clone()
        } else {
            let Phase::Prepared(offer) = &self.journal.phase else {
                anyhow::bail!("source requires exact durable prepared offer");
            };
            offer.as_ref().clone()
        };
        verify_exclusive_turn_offer(&self.base, &offer)?;
        ensure!(
            offer.offer_hash()? == review.expected_offer
                && offer.offer.source == self.local()
                && offer.offer.recipient == recipient,
            "source offer or recipient differs from reviewed pin"
        );
        Ok(offer)
    }

    fn receiver_offer(&self, review: &Review, from: PeerId, offer: &ExclusiveTurnSignedOfferV1) -> Result<()> {
        verify_exclusive_turn_offer(&self.base, offer)?;
        ensure!(
            offer.offer_hash()? == review.expected_offer
                && offer.offer.source == from
                && offer.offer.recipient == self.local(),
            "incoming offer or fixed-device roles differ from reviewed pin"
        );
        if self.already_released() {
            ensure!(
                self.journal.phase == Phase::Held
                    && self.journal.history.releases.last().is_some_and(|r| r.offer == *offer)
                    && self.journal.checkpoint == self.head.checkpoint(),
                "release retry cannot restore a changed or nonheld reference turn"
            );
        } else {
            ensure!(
                matches!(self.journal.phase, Phase::Replica | Phase::Released),
                "recipient cannot stage in current reference phase"
            );
            if let Some(staged) = &self.journal.staged {
                ensure!(
                    staged.offer == *offer || offer.offer.attempt > staged.offer.offer.attempt,
                    "incoming offer is stale or conflicts with retained receipt"
                );
            }
        }
        if self.base.save_backed() {
            // Refuse before advertising readiness, and again after
            // reacquiring the lock. Stage/accept reverify independently.
            super::save::verify_offer_bundle(&self.store, offer)?;
        }
        Ok(())
    }
}

fn serve_handoff(
    paths: &DataPaths,
    review: &Review,
    file: &std::path::Path,
    from: PeerId,
    bind: SocketAddr,
) -> Result<()> {
    ensure!(private_ip(bind.ip()), "exclusive-turn listener must bind an explicit private address");
    let approved: ExclusiveTurnSignedOfferV1 = read_document(file)?;
    let view = View::open(paths, review)?;
    view.receiver_offer(review, from, &approved)?;
    let local = view.local();
    let key = remote_key(&view.base, from, local)?;
    let context = ChannelContext::new(review, from, local);
    let initial_head = view.head.clone();
    // Do not reserve the local journal during the bounded accept window.
    drop(view);
    let listener = TcpListener::bind(bind)?;
    print_document(&serde_json::json!({
        "classification": "exclusive_turn_reference_handoff_listener",
        "address": listener.local_addr()?.to_string(),
        "world_id": context.world.to_string(), "base": context.base.to_string(),
        "offer": context.offer.to_string(), "recipient": local.to_string(),
        "runtime_activation": false, "player_admission": false,
    }))?;
    let stream = accept(listener, Instant::now() + ACCEPT_WINDOW)?;
    let deadline = Instant::now() + SESSION_WINDOW;
    let mut view = View::open(paths, review)?;
    ensure!(view.head == initial_head && view.local() == local, "recipient grant or identity changed");
    view.receiver_offer(review, from, &approved)?;
    // Both devices hold their own stable journal lock during the bounded
    // session. No cancel, competing offer, checkpoint or next turn can race it.
    let mut channel =
        authenticate(stream, &context.prologue()?, AUTH_DOMAIN, &view.store.identity, key, false, deadline)?;
    let offer: ExclusiveTurnSignedOfferV1 = receive(&mut channel)?;
    ensure!(offer == approved, "remote offer differs from locally reviewed offer");
    let bytes = channel.receive(MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
    offer.offer.checkpoint.verify_reference_bytes(&bytes)?;
    remaining(deadline)?;
    let receipt = if view.already_released() {
        ensure!(view.store.state(offer.offer.checkpoint)? == bytes, "accepted reference bytes differ");
        view.journal.history.releases.last().context("accepted release absent")?.receipt.clone()
    } else {
        stage_reference(&view.store, &mut view.journal, &view.head, offer.clone(), review.expected_offer, &bytes)?
    };
    send(&mut channel, &receipt)?;
    let release: ExclusiveTurnReleaseV1 = receive(&mut channel)?;
    ensure!(release.offer == offer && release.receipt == receipt, "release differs from exact durable receipt");
    let body = AcceptanceBody::for_release(&release)?;
    remaining(deadline)?;
    let accepted = accept_reference(&view.store, &mut view.journal, &view.head, release, context.base, body.grant)?;
    ensure!(
        accepted.grant() == body.grant
            && accepted.holder() == local
            && accepted.epoch() == body.epoch
            && accepted.fence() == body.fence
            && accepted.checkpoint() == body.checkpoint,
        "durable acceptance does not match exact release"
    );
    let signature = view.store.identity.sign(&body.signing_bytes()?);
    // Lost ACK does not revoke durable acceptance. Exact retry is allowed
    // only while the accepted reference turn remains unchanged and held.
    send(&mut channel, &Acceptance { body, signature })
}

fn handoff(paths: &DataPaths, review: &Review, peer: &str) -> Result<()> {
    let (recipient, address) = endpoint(peer)?;
    let mut view = View::open(paths, review)?;
    let local = view.local();
    let key = remote_key(&view.base, recipient, local)?;
    let offer = view.source_offer(review, recipient)?;
    let bytes = view.store.verify_checkpoint(&view.head, offer.offer.checkpoint)?;
    let context = ChannelContext::new(review, local, recipient);
    let deadline = Instant::now() + SESSION_WINDOW;
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    let mut channel =
        authenticate(stream, &context.prologue()?, AUTH_DOMAIN, &view.store.identity, key, true, deadline)?;
    send(&mut channel, &offer)?;
    channel.send(&bytes, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
    let receipt: ExclusiveTurnReceiptV1 = receive(&mut channel)?;
    verify_exclusive_turn_receipt(&view.base, &offer, &receipt)?;
    remaining(deadline)?;
    let release = if view.already_released() {
        let release = view.journal.history.releases.last().context("release outbox absent")?;
        ensure!(release.offer == offer && release.receipt == receipt, "retry receipt differs from committed release");
        release.clone()
    } else {
        // Commit locally before putting the release on the wire. An error
        // after this point is NOT permission to cancel or regain the turn.
        commit_reference(&view.store, &mut view.journal, &view.head, receipt, review.expected_offer)?
    };
    let body = AcceptanceBody::for_release(&release)?;
    send(&mut channel, &release)
        .context("reference source relinquished; exact outbox retry required if delivery failed")?;
    let ack: Acceptance = receive(&mut channel)
        .context("reference source relinquished; recipient acceptance unconfirmed, retry exact outbox")?;
    ensure!(ack.body == body, "recipient durable acceptance differs from exact release");
    verify_auth(&key, &ack.body.signing_bytes()?, &ack.signature)?;
    remaining(deadline)?;
    print_document(&serde_json::json!({
        "classification": "exclusive_turn_reference_handoff_confirmed",
        "world_id": body.world.to_string(), "base": body.base.to_string(), "offer": body.offer.to_string(),
        "grant": body.grant.to_string(), "recipient": body.recipient.to_string(),
        "reference_epoch": body.epoch, "reference_fence": body.fence,
        "receipt": ack, "canonical_authority": false, "write_authority": false,
        "runtime_activation": false, "player_admission": false,
    }))
}

pub(super) fn run(paths: &DataPaths, command: NetworkCommand) -> Result<()> {
    match command {
        NetworkCommand::ServeHandoff { review, offer, from, bind } => serve_handoff(paths, &review, &offer, from, bind),
        NetworkCommand::Handoff { review, peer } => handoff(paths, &review, &peer),
    }
}
