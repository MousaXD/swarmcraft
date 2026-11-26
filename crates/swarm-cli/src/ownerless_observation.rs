//! An explicit one-shot private TCP reference observation, not a runtime lease.
//! Mutual application-key authentication protects the signed content; there
//! is no transport encryption, remote attestation, or production routing.
use super::{encoded, pin, print, Journal, Store};
use anyhow::{bail, ensure, Context, Result};
use clap::Subcommand;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};
use swarm_consensus::{
    ownerless_authority::VerifiedOwnerlessHead,
    ownerless_observation::{
        observation_context, verify_observation_majority, verify_observation_request, verify_observation_response,
    },
};
use swarm_core::{random_nonce, DataPaths};
use swarm_protocol::{
    Hash32, OwnerlessObservationRequestV1, OwnerlessObservationResponseV1, OwnerlessWorldIdV1, PeerId,
    MAX_OWNERLESS_VALIDATORS,
};

const FRAME_LIMIT: usize = 8192;
const COLLECTION_WINDOW: Duration = Duration::from_secs(5);
const NETWORK_WINDOW: Duration = Duration::from_secs(3);
const ACCEPT_WINDOW: Duration = Duration::from_secs(15);

#[derive(Debug, Subcommand)]
pub enum ObservationCommand {
    /// Serve exactly one explicitly requested reference-head observation.
    Serve {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        /// Explicit private or loopback address, never a wildcard or public listener.
        #[arg(long)]
        bind: SocketAddr,
        #[arg(long)]
        confirm: bool,
    },
    /// Observe a recent majority at this exact locally accepted reference head.
    Collect {
        #[arg(long)]
        expected_world: OwnerlessWorldIdV1,
        #[arg(long)]
        expected_head: Hash32,
        /// Exact current validator identity and explicit private endpoint: scpeer:...@IP:port.
        #[arg(long = "peer", required = true)]
        peers: Vec<String>,
        #[arg(long)]
        confirm: bool,
    },
}

pub(super) fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
    }
}

pub(super) fn remaining(deadline: Instant) -> Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    ensure!(!left.is_zero(), "ownerless observation deadline expired");
    Ok(left)
}

pub(super) fn read_all(stream: &mut TcpStream, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(bytes) {
            Ok(0) => bail!("truncated observation frame"),
            Ok(n) => bytes = &mut bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    remaining(deadline)?;
    Ok(())
}

pub(super) fn write_all(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(bytes) {
            Ok(0) => bail!("observation frame write failed"),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    remaining(deadline)?;
    Ok(())
}

fn receive<T: DeserializeOwned>(stream: &mut TcpStream, deadline: Instant) -> Result<T> {
    let mut size = [0; 4];
    read_all(stream, &mut size, deadline)?;
    let size = u32::from_be_bytes(size) as usize;
    ensure!((1..=FRAME_LIMIT).contains(&size), "invalid observation frame size");
    let mut bytes = vec![0; size];
    read_all(stream, &mut bytes, deadline)?;
    let value = serde_json::from_slice(&bytes).context("invalid observation frame")?;
    remaining(deadline)?;
    Ok(value)
}

fn send<T: Serialize>(stream: &mut TcpStream, value: &T, deadline: Instant) -> Result<()> {
    let bytes = encoded(value, FRAME_LIMIT)?;
    write_all(stream, &(bytes.len() as u32).to_be_bytes(), deadline)?;
    write_all(stream, &bytes, deadline)
}

fn checked_store(
    paths: &DataPaths,
    world: OwnerlessWorldIdV1,
    expected: Hash32,
) -> Result<(Store, Journal, VerifiedOwnerlessHead)> {
    let store = Store::try_open(paths, world)?;
    let (journal, head) = store.load(world)?;
    pin(&head, expected)?;
    ensure!(journal.pending.is_none(), "pending governance vote prevents observation");
    ensure!(
        head.validators()
            .iter()
            .any(|v| v.peer_id == store.identity.peer_id() && v.public_key == store.identity.public_key()),
        "local identity is not a current ownerless validator"
    );
    Ok((store, journal, head))
}

fn sign_response(
    store: &Store,
    head: &VerifiedOwnerlessHead,
    request: &OwnerlessObservationRequestV1,
) -> Result<OwnerlessObservationResponseV1> {
    verify_observation_request(head, request)?;
    let validator = store.identity.peer_id();
    let response = OwnerlessObservationResponseV1 {
        context_hash: request.context.context_hash()?,
        validator,
        signature: store.identity.sign(&request.context.response_bytes(validator)?),
    };
    verify_observation_response(head, request, validator, &response)?;
    Ok(response)
}

fn parse_peers(peers: &[String], head: &VerifiedOwnerlessHead, local: PeerId) -> Result<Vec<(PeerId, SocketAddr)>> {
    ensure!(!peers.is_empty() && peers.len() < MAX_OWNERLESS_VALIDATORS, "invalid observation peer count");
    let mut identities = BTreeSet::new();
    let mut endpoints = BTreeSet::new();
    let mut result = Vec::new();
    for raw in peers {
        let (id, endpoint) = raw.split_once('@').context("observation peer requires identity@address")?;
        let id: PeerId = id.parse()?;
        let endpoint: SocketAddr = endpoint.parse()?;
        ensure!(private_ip(endpoint.ip()) && endpoint.port() != 0, "observation endpoint must be explicit and private");
        ensure!(
            id != local && head.validators().iter().any(|v| v.peer_id == id),
            "peer is not a current remote validator"
        );
        ensure!(identities.insert(id) && endpoints.insert(endpoint), "duplicate observation validator or endpoint");
        result.push((id, endpoint));
    }
    ensure!(result.len() + 1 >= head.threshold(), "insufficient configured observation validators");
    Ok(result)
}

fn serve(paths: &DataPaths, world: OwnerlessWorldIdV1, expected: Hash32, bind: SocketAddr) -> Result<()> {
    ensure!(private_ip(bind.ip()), "observation listener must bind an explicit private or loopback address");
    // Do not hold the journal lock while waiting for a connection. Reopen
    // and revalidate the exact accepted head after receiving the request.
    let (store, _, _) = checked_store(paths, world, expected)?;
    let validator = store.identity.peer_id();
    drop(store);
    let listener = TcpListener::bind(bind)?;
    listener.set_nonblocking(true)?;
    let accept_deadline = Instant::now() + ACCEPT_WINDOW;
    print(
        &serde_json::json!({
            "classification": "ownerless_reference_observation_listener",
            "address": listener.local_addr()?.to_string(),
            "validator": validator.to_string(),
            "world_id": world.to_string(),
            "head": expected.to_string(),
            "runtime_activation": false,
        }),
        FRAME_LIMIT,
    )?;
    let (mut stream, remote) = loop {
        remaining(accept_deadline)?;
        match listener.accept() {
            Ok(pair) => break pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10).min(remaining(accept_deadline)?));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    };
    drop(listener);
    ensure!(private_ip(remote.ip()), "observation connection is not from a private address");
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    let deadline = Instant::now() + COLLECTION_WINDOW;
    let request: OwnerlessObservationRequestV1 = receive(&mut stream, deadline)?;
    let (store, _, head) = checked_store(paths, world, expected)?;
    ensure!(store.identity.peer_id() == validator, "observation listener identity changed");
    ensure!(request.context.verifier != validator, "remote observation cannot request its own signature");
    let response = sign_response(&store, &head, &request)?;
    remaining(deadline)?;
    // Keep the local journal locked until the signed bytes are sent. This
    // does not stop another validator or guarantee a later fresh global head.
    send(&mut stream, &response, deadline)
}

fn fetch(
    endpoint: SocketAddr,
    request: &OwnerlessObservationRequestV1,
    deadline: Instant,
) -> Result<OwnerlessObservationResponseV1> {
    let mut stream = TcpStream::connect_timeout(&endpoint, remaining(deadline)?)?;
    stream.set_nodelay(true)?;
    send(&mut stream, request, deadline)?;
    receive(&mut stream, deadline)
}

fn collect(paths: &DataPaths, world: OwnerlessWorldIdV1, expected: Hash32, peers: &[String]) -> Result<()> {
    let (store, _, initial) = checked_store(paths, world, expected)?;
    let local = store.identity.peer_id();
    let peers = parse_peers(peers, &initial, local)?;
    // Start the verifier's monotonic window before creating its private nonce.
    let started = Instant::now();
    let deadline = started + COLLECTION_WINDOW;
    let network_deadline = started + NETWORK_WINDOW;
    let context = observation_context(&initial, local, random_nonce())?;
    let request = OwnerlessObservationRequestV1 { signature: store.identity.sign(&context.request_bytes()?), context };
    drop(store);
    // Bounded parallel connections share ONE deadline, so a missing or
    // stalled endpoint cannot multiply the collection window per validator.
    let attempts = thread::scope(|scope| {
        let handles: Vec<_> = peers
            .iter()
            .map(|&(peer, endpoint)| {
                let request = &request;
                (peer, scope.spawn(move || fetch(endpoint, request, network_deadline)))
            })
            .collect();
        handles.into_iter().map(|(peer, handle)| (peer, handle.join())).collect::<Vec<_>>()
    });
    let mut responses = Vec::new();
    let mut failed = Vec::new();
    for (peer, result) in attempts {
        match result {
            Ok(Ok(response)) if verify_observation_response(&initial, &request, peer, &response).is_ok() => {
                responses.push(response);
            }
            _ => failed.push(peer.to_string()),
        }
    }
    remaining(deadline)?;
    let (store, _, current) = checked_store(paths, world, expected)?;
    ensure!(current == initial && store.identity.peer_id() == local, "local observation head or identity changed");
    responses.push(sign_response(&store, &current, &request)?);
    responses.sort_by_key(|r| r.validator);
    verify_observation_majority(&current, &request, &responses)?;
    remaining(deadline)?;
    print(
        &serde_json::json!({
            "classification": "observed_recent_ownerless_reference_majority",
            "world_id": world.to_string(), "head": expected.to_string(),
            "sequence": current.sequence(), "reference_fence": current.fence(),
            "checkpoint": current.checkpoint().to_string(),
            "majority_threshold": current.threshold(), "observation_count": responses.len(),
            "collection_elapsed_ms": started.elapsed().as_millis(),
            "collection_window_ms": COLLECTION_WINDOW.as_millis(),
            "unavailable_or_invalid_peers": failed, "request": request, "responses": responses,
            "freshness_lease": false, "latest_global_head_proven": false, "live_quorum_verified": false,
            "write_authority": false, "canonical_authority": false, "runtime_activation": false,
            "player_admission": false, "worker_admission": false,
        }),
        FRAME_LIMIT * MAX_OWNERLESS_VALIDATORS,
    )
}

pub(super) fn run(paths: &DataPaths, command: ObservationCommand) -> Result<()> {
    match command {
        ObservationCommand::Serve { expected_world, expected_head, bind, confirm } => {
            ensure!(confirm, "explicit --confirm required for reference observation listener");
            serve(paths, expected_world, expected_head, bind)
        }
        ObservationCommand::Collect { expected_world, expected_head, peers, confirm } => {
            ensure!(confirm, "explicit --confirm required for reference observation collection");
            collect(paths, expected_world, expected_head, &peers)
        }
    }
}
