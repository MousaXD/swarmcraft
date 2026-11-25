#![cfg(unix)]

use std::{
    fs,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_network::{
    load_or_create_transport_key, ConnectivityDiagnosticsV1, DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE,
};
use swarm_protocol::{EpochMode, WorldId};
use swarm_storage::{SnapshotContext, Storage};
use tempfile::TempDir;

const WAIT_STEP: Duration = Duration::from_millis(200);

struct PeerFixture {
    _temp: TempDir,
    paths: DataPaths,
    storage: Storage,
    identity: PeerIdentity,
    port: u16,
    transport_peer: String,
}

struct ManagedChild(Child);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("0.0.0.0:0").unwrap().local_addr().unwrap().port()
}

fn peer_fixture() -> PeerFixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path().join("data"));
    let storage = Storage::open(paths.root.clone()).unwrap();
    let identity = PeerIdentity::load_or_create(&paths).unwrap();
    let transport_key = load_or_create_transport_key(&paths.transport_key()).unwrap();
    let transport_peer = transport_key.public().to_peer_id().to_string();
    PeerFixture { _temp: temp, paths, storage, identity, port: free_udp_port(), transport_peer }
}

fn local_non_loopback_ipv4() -> Ipv4Addr {
    let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    socket.connect("1.1.1.1:53").unwrap();
    match socket.local_addr().unwrap().ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() && !ip.is_link_local() => ip,
        other => panic!("test runner has no usable non-loopback IPv4 route: {other}"),
    }
}

fn automatic_transport_address(peer: &PeerFixture, ip: Ipv4Addr) -> String {
    format!("/ip4/{ip}/udp/{}/quic-v1/p2p/{}", peer.port, peer.transport_peer)
}

fn spawn_daemon(peer: &PeerFixture, listen_ip: Ipv4Addr) -> ManagedChild {
    spawn_daemon_with_peers(peer, listen_ip, &[])
}

fn spawn_daemon_with_peers(peer: &PeerFixture, listen_ip: Ipv4Addr, addresses: &[String]) -> ManagedChild {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    command
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/{listen_ip}/udp/{}/quic-v1", peer.port));
    for address in addresses {
        command.arg("--peer").arg(address);
    }
    let child = command.env("RUST_LOG", "info").stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn().unwrap();
    ManagedChild(child)
}

fn run_cli(peer: &PeerFixture, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .args(args)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "CLI failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn wait_until(label: &str, timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(WAIT_STEP);
    }
    panic!("timed out waiting for {label}");
}

#[test]
fn revoked_invite_is_denied_by_restarted_issuer_while_another_grant_works() {
    let issuer = peer_fixture();
    let denied = peer_fixture();
    let allowed = peer_fixture();
    let local_ip = local_non_loopback_ipv4();
    let address = automatic_transport_address(&issuer, local_ip);
    let created = run_cli(
        &issuer,
        &["world", "create", "--name", "revocation", "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"],
    );
    let world: WorldId = created.lines().find_map(|line| line.strip_prefix("World ID: ")).unwrap().parse().unwrap();
    let initial = issuer.storage.load_membership_record(world).unwrap();
    let world_arg = world.to_string();
    let old_token = run_cli(&issuer, &["invite", "create", &world_arg, "--bootstrap", &address]);
    let new_token = run_cli(&issuer, &["invite", "create", &world_arg, "--bootstrap", &address]);
    assert_ne!(old_token, new_token);
    let wrong_issuer = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(&denied.paths.root)
        .args(["invite", "revoke", &old_token])
        .output()
        .unwrap();
    assert!(!wrong_issuer.status.success());
    assert!(run_cli(&issuer, &["invite", "revoke", &old_token]).contains(" revoked"));
    assert!(run_cli(&issuer, &["invite", "revoke", &old_token]).contains("already revoked"));
    assert!(run_cli(&issuer, &["invite", "inspect", &old_token]).contains("Status: revoked"));
    // New process reads the persisted issuer marker; capture actual denial
    // rather than relying on a timeout or an unreachable peer.
    let logs = issuer._temp.path().join("daemon-output.log");
    let log = fs::File::create(&logs).unwrap();
    let _issuer_daemon = ManagedChild(
        Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
            .arg("--data-dir")
            .arg(&issuer.paths.root)
            .args(["daemon", "--listen", &format!("/ip4/{local_ip}/udp/{}/quic-v1", issuer.port)])
            .env("RUST_LOG", "info")
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let denied_daemon = spawn_daemon(&denied, local_ip);
    let allowed_daemon = spawn_daemon(&allowed, local_ip);
    assert!(run_cli(&denied, &["world", "join", &old_token]).contains("Join request staged"));
    wait_until("authenticated revocation denial", Duration::from_secs(30), || {
        fs::read_to_string(&logs).is_ok_and(|log| log.contains("invite revoked by its issuer"))
            && denied.storage.load_pending_join(world).is_ok_and(|request| {
                denied.storage.pending_join_denial(&request).ok().flatten()
                    == Some(swarm_storage::world::JoinDenialReason::Revoked)
            })
    });
    assert_eq!(issuer.storage.load_membership_record(world).unwrap(), initial);
    assert!(issuer.storage.load_membership_promise(world).is_err());
    assert!(denied.storage.load_membership_record(world).is_err());
    assert!(denied.storage.latest_snapshot(world).unwrap().is_none());
    drop(denied_daemon);
    assert!(run_cli(&denied, &["world", "status", &world_arg])
        .contains("Membership: pending; issuer reported invitation revoked"));
    assert!(run_cli(&allowed, &["world", "join", &new_token]).contains("Join request staged"));
    wait_until("fresh grant commits and replicates", Duration::from_secs(30), || {
        let Ok(current) = issuer.storage.load_membership_record(world) else {
            return false;
        };
        current.sequence == initial.sequence + 1
            && current.members.iter().any(|member| member.peer_id == allowed.identity.peer_id())
            && !current.members.iter().any(|member| member.peer_id == denied.identity.peer_id())
            && allowed.storage.load_membership_record(world).ok().as_ref() == Some(&current)
            && allowed.storage.latest_snapshot(world).ok().flatten().is_some()
    });
    assert!(denied.storage.load_membership_record(world).is_err());
    assert!(denied.storage.latest_snapshot(world).unwrap().is_none());

    let survivor = peer_fixture();
    let _survivor_daemon = spawn_daemon(&survivor, local_ip);
    let head = issuer.storage.latest_snapshot(world).unwrap().unwrap();
    let epoch = issuer.storage.load_epoch_record(world).unwrap();
    // A reusable, unrevoked token can admit another distinct peer.
    assert!(run_cli(&survivor, &["world", "join", &new_token]).contains("Join request staged"));
    wait_until("second admission and complete replicas", Duration::from_secs(45), || {
        let Ok(current) = issuer.storage.load_membership_record(world) else { return false };
        current.sequence == initial.sequence + 2
            && [&allowed, &survivor].iter().all(|peer| {
                peer.storage.load_membership_record(world).ok().as_ref() == Some(&current)
                    && peer.storage.load_epoch_record(world).ok().as_ref() == Some(&epoch)
                    && peer.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&head)
            })
            && issuer.storage.load_membership_promise(world).is_err()
    });
    let before_leave = issuer.storage.load_membership_record(world).unwrap();
    assert!(run_cli(&issuer, &["invite", "revoke", &new_token]).contains(" revoked"));
    assert_eq!(issuer.storage.load_membership_record(world).unwrap(), before_leave);
    assert!(run_cli(&allowed, &["world", "leave", &world_arg]).contains("Leave request"));
    wait_until("signed joint leave committed at all three members", Duration::from_secs(45), || {
        let Ok(current) = issuer.storage.load_membership_record(world) else { return false };
        current.sequence == before_leave.sequence + 1
            && !current.members.iter().any(|member| member.peer_id == allowed.identity.peer_id())
            && [&allowed, &survivor]
                .iter()
                .all(|peer| peer.storage.load_membership_record(world).ok().as_ref() == Some(&current))
            && issuer.storage.load_membership_promise(world).is_err()
    });
    drop(allowed_daemon);
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("level.dat"), b"private checkpoint created after removal").unwrap();
    let mut next = issuer
        .storage
        .snapshot_directory(
            source.path(),
            SnapshotContext {
                world,
                snapshot_number: head.snapshot_number + 1,
                sequence: head.sequence + 1,
                epoch: epoch.epoch_number,
                previous_snapshot_hash: Some(head.manifest_hash().unwrap()),
                authority_peer_id: issuer.identity.peer_id(),
                authority_public_key: issuer.identity.public_key(),
            },
        )
        .unwrap();
    issuer.identity.sign_snapshot(&mut next).unwrap();
    issuer.storage.commit_snapshot(&next).unwrap();
    let next_manifest = next.manifest().clone();

    // Connect with the removed device's actual application identity. Probe
    // both private metadata and historical replay; observe the wire
    // refusals, not just an absence of new bytes on disk. Continue polling
    // until an authorized current replica has verified the new checkpoint.
    use swarm_network::{NetworkEvent, SwarmNode, WireRequest, WireResponse};
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let key = load_or_create_transport_key(&allowed.paths.transport_key()).unwrap();
        let hello = allowed.identity.signed_peer_hello(vec!["snapshot-replication-v1".into()]).unwrap();
        let mut probe = SwarmNode::new(key, hello, allowed.identity.network_signing_key()).unwrap();
        probe.listen(format!("/ip4/{local_ip}/udp/{}/quic-v1", allowed.port).parse().unwrap()).unwrap();
        probe.dial(address.parse().unwrap()).unwrap();
        let mut status_request = None;
        let mut replay_request = None;
        let mut status_denied = false;
        let mut replay_denied = false;
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                tokio::select! {
                    event = probe.next_event() => {
                        match event.unwrap() {
                            NetworkEvent::Authenticated { transport_peer, application_peer } if application_peer == issuer.identity.peer_id() => {
                                status_request = Some(probe.send_request(&transport_peer, WireRequest::WorldStatus { world_id: world }).unwrap());
                                replay_request = Some(probe.send_request(&transport_peer, WireRequest::HistoricalReplayProof { world_id: world }).unwrap());
                            }
                            NetworkEvent::Response { transport_peer, request_id, response } if probe.application_peer(&transport_peer) == Some(issuer.identity.peer_id()) => {
                                if Some(request_id) == status_request {
                                    assert!(matches!(response, WireResponse::Error { code, .. } if code == "WORLD_ACCESS_DENIED"));
                                    status_denied = true;
                                } else if Some(request_id) == replay_request {
                                    assert!(matches!(response, WireResponse::HistoricalReplayProof(None)));
                                    replay_denied = true;
                                }
                            }
                            NetworkEvent::InboundRequest { request, channel, .. } => {
                                assert!(!matches!(&request, WireRequest::SnapshotManifest(manifest) if manifest.world_id == world && manifest.snapshot_number > head.snapshot_number));
                                assert!(!matches!(&request, WireRequest::HistoricalReplayOffer { world_id, .. } if *world_id == world));
                                assert!(!matches!(&request, WireRequest::BlobChunk { world_id, .. } if *world_id == world));
                                // The removed peer may still receive its signed
                                // membership removal certificate for convergence.
                                let _ = probe.respond(channel, WireResponse::Error { code: "TEST_PROBE".into(), message: "probe has no active replica".into() });
                            }
                            _ => {}
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
                if status_denied && replay_denied && survivor.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&next_manifest)
                    && survivor.storage.verify_snapshot(&next_manifest).is_ok()
                {
                    break;
                }
            }
        }).await.expect("removed identity denied while current replica obtains new save");
    });
    assert_eq!(allowed.storage.latest_snapshot(world).unwrap().unwrap(), head);
    assert!(allowed.storage.verify_snapshot(&head).is_ok());
}

#[test]
fn automatic_invite_bootstrap_joins_and_replicates_without_manual_multiaddr() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();
    let local_ip = local_non_loopback_ipv4();
    assert_ne!(a.port, b.port);
    assert_ne!(a.port, c.port);
    assert_ne!(b.port, c.port);
    let a_address = automatic_transport_address(&a, local_ip);

    // Exercise the real create path rather than seeding authority metadata.
    let output = run_cli(
        &a,
        &["world", "create", "--name", "automatic-invite-join", "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"],
    );
    let world: WorldId = output
        .lines()
        .find_map(|line| line.strip_prefix("World ID: "))
        .expect("world create must return its signed world ID")
        .parse()
        .unwrap();
    let epoch = a.storage.load_epoch_record(world).unwrap();
    assert_eq!(epoch.mode, EpochMode::Quorum);
    let genesis_membership = a.storage.load_membership_record(world).unwrap();
    assert_eq!(genesis_membership.sequence, 0);
    assert_eq!(genesis_membership.epoch, epoch.epoch_number);
    let manifest = a.storage.latest_snapshot(world).unwrap().unwrap();
    let config = a.storage.load_world_config(world).unwrap();

    // Ordinary invite creation consumes diagnostics, not a manually supplied
    // --bootstrap argument. Seed the exact real LAN listener for this fixture.
    let diagnostics = ConnectivityDiagnosticsV1 { local_addresses: vec![a_address.clone()], ..Default::default() };
    let diagnostics_path = a.paths.root.join(DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE);
    fs::write(&diagnostics_path, serde_json::to_vec(&diagnostics).unwrap()).unwrap();
    let token = run_cli(&a, &["invite", "create", &world.to_string()]);
    assert!(token.starts_with("scinvite:"));
    assert!(!token.contains(&a_address));

    // Desktop daemons are normally already running before a user joins.
    let _daemon_a = spawn_daemon(&a, local_ip);
    let _daemon_b = spawn_daemon(&b, local_ip);
    let _daemon_c = spawn_daemon(&c, local_ip);
    thread::sleep(Duration::from_secs(1));

    assert!(run_cli(&b, &["world", "join", &token, "--host-candidate"]).contains("Join request staged"));
    let pending = b.storage.load_pending_join(world).unwrap();
    assert_eq!(pending.invite.bootstrap_addrs, vec![a_address]);
    assert!(pending.joining_member.authority_eligible);

    wait_until("first join, accepted epoch and verified replica", Duration::from_secs(30), || {
        let Ok(current) = a.storage.load_membership_record(world) else { return false };
        current.sequence == 1
            && b.storage.load_membership_record(world).ok().as_ref() == Some(&current)
            && b.storage.load_epoch_record(world).ok().as_ref() == Some(&epoch)
            && b.storage.load_world_config(world).ok().as_ref() == Some(&config)
            && b.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&manifest)
            && b.storage.verify_snapshot(&manifest).is_ok()
    });

    // The third peer joins an already played world. Publish two distinct
    // signed direct extensions while the authority generation remains the
    // original one. A blind latest-manifest push cannot bootstrap this peer:
    // it must verify checkpoint #1, accept epoch #1 at its base, then catch
    // up through #2 and #3. No Java or hosting consent is needed to replicate.
    let source = tempfile::tempdir().unwrap();
    let mut head = manifest.clone();
    for number in 2..=3 {
        fs::write(source.path().join("level.dat"), format!("checkpoint-{number}")).unwrap();
        let mut next = a
            .storage
            .snapshot_directory(
                source.path(),
                SnapshotContext {
                    world,
                    snapshot_number: number,
                    epoch: epoch.epoch_number,
                    sequence: head.sequence + 1,
                    previous_snapshot_hash: Some(head.manifest_hash().unwrap()),
                    authority_peer_id: a.identity.peer_id(),
                    authority_public_key: a.identity.public_key(),
                },
            )
            .unwrap();
        a.identity.sign_snapshot(&mut next).unwrap();
        a.storage.commit_snapshot(&next).unwrap();
        head = next.manifest().clone();
    }
    assert_ne!(head.state_root, epoch.base_state_hash);
    assert!(c.storage.load_world(world).is_err());

    assert!(run_cli(&c, &["world", "join", &token]).contains("Join request staged"));
    assert!(!c.storage.load_pending_join(world).unwrap().joining_member.authority_eligible);
    wait_until(
        "third peer receives full genesis history, epoch and three checkpoints",
        Duration::from_secs(90),
        || {
            let Ok(current) = a.storage.load_membership_record(world) else { return false };
            if current.sequence != 2 {
                return false;
            }
            [&b, &c].iter().all(|peer| {
                peer.storage.load_membership_record(world).ok().as_ref() == Some(&current)
                    && peer.storage.load_epoch_record(world).ok().as_ref() == Some(&epoch)
                    && peer.storage.load_world_config(world).ok().as_ref() == Some(&config)
                    && peer.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&head)
                    && peer.storage.verify_snapshot(&head).is_ok()
                    && peer.storage.load_membership_certificate_chain(world).is_ok_and(|chain| chain.len() == 2)
            })
        },
    );

    let committed = a.storage.load_membership_record(world).unwrap();
    assert_eq!(committed.members.len(), 3);
    assert!(committed.members.iter().any(|member| member.peer_id == b.identity.peer_id() && member.authority_eligible));
    assert!(committed
        .members
        .iter()
        .any(|member| member.peer_id == c.identity.peer_id() && !member.authority_eligible));
    for peer in [&a, &b, &c] {
        let history = peer.storage.list_snapshots(world).unwrap();
        assert_eq!(history.len(), 3);
        for (index, checkpoint) in history.iter().enumerate() {
            assert_eq!(checkpoint.snapshot_number, index as u64 + 1);
            assert_eq!(checkpoint, &a.storage.load_snapshot(world, checkpoint.snapshot_number).unwrap());
            peer.storage.verify_snapshot(checkpoint).unwrap();
            swarm_core::verify_snapshot_signature(checkpoint).unwrap();
            if index > 0 {
                assert_eq!(checkpoint.previous_snapshot_hash, Some(history[index - 1].manifest_hash().unwrap()));
            }
        }
        assert!(peer.storage.load_pending_join(world).is_err());
        assert!(swarm_cli::hosting_budget::load_consent(&peer.paths, world).unwrap().is_none());
        let proof = swarm_protocol::DiscoveryMembershipProofV1 {
            protocol_version: swarm_protocol::PROTOCOL_VERSION,
            world_id: world,
            genesis: peer.storage.load_world(world).unwrap().genesis,
            initial_membership: peer.storage.load_genesis_membership(world).unwrap(),
            current_membership: peer.storage.load_membership_record(world).unwrap(),
            membership_certificates: peer.storage.load_membership_certificate_chain(world).unwrap(),
            pending_membership: None,
        };
        swarm_cli::live_authority::verify_genesis_anchored_membership(&proof).unwrap();
    }
    assert!(run_cli(&c, &["world", "join", &token, "--host-candidate"]).contains("Already a signed member"));
    assert_eq!(c.storage.load_membership_record(world).unwrap(), committed);
    assert_eq!(c.storage.load_world_descriptor(world).unwrap().members, committed.members);

    // The player disconnects before the old authority's final save and
    // misses the signed transfer. Exercise real authenticated RPC between
    // the newer authority and the already admitted stale player. The
    // historical envelope may be staged but must not advance live state.
    drop((_daemon_a, _daemon_b, _daemon_c));
    let anchor = c.storage.stale_replay_anchor(world).unwrap();
    fs::write(source.path().join("level.dat"), b"last checkpoint before transfer").unwrap();
    // Use incompressible data to force actual blob replication instead of
    // completing the promoted checkpoint solely from the old blob store.
    let mut seed = 0x5EED_u64;
    let data: Vec<u8> = (0..1024 * 1024)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect();
    fs::write(source.path().join("offline-region.dat"), data).unwrap();
    let mut boundary = b
        .storage
        .snapshot_directory(
            source.path(),
            SnapshotContext {
                world,
                snapshot_number: 4,
                sequence: head.sequence + 1,
                epoch: epoch.epoch_number,
                previous_snapshot_hash: Some(head.manifest_hash().unwrap()),
                authority_peer_id: a.identity.peer_id(),
                authority_public_key: a.identity.public_key(),
            },
        )
        .unwrap();
    a.identity.sign_snapshot(&mut boundary).unwrap();
    b.storage.commit_snapshot(&boundary).unwrap();
    let mut next = swarm_protocol::EpochRecordV1 {
        epoch_number: epoch.epoch_number + 1,
        fencing_token: epoch.fencing_token + 1,
        previous_epoch_hash: Some(epoch.record_hash().unwrap()),
        base_state_hash: boundary.state_root,
        authority_peer_id: b.identity.peer_id(),
        authority_public_key: b.identity.public_key(),
        reason: "certified transfer after disconnected player".into(),
        signature: Vec::new(),
        ..epoch.clone()
    };
    next.signature = b.identity.sign(&next.signing_bytes().unwrap());
    let mut transfer = swarm_protocol::AuthorityTransferV1 {
        protocol_version: swarm_protocol::PROTOCOL_VERSION,
        world_id: world,
        from_peer_id: a.identity.peer_id(),
        to_peer_id: b.identity.peer_id(),
        base_snapshot_hash: boundary.manifest_hash().unwrap(),
        next_epoch: next.epoch_number,
        next_fencing_token: next.fencing_token,
        phase: swarm_protocol::TransferPhase::Prepared,
        signer_peer_id: a.identity.peer_id(),
        signer_public_key: a.identity.public_key(),
        signature: Vec::new(),
    };
    for (phase, signer) in [
        (swarm_protocol::TransferPhase::Prepared, &a.identity),
        (swarm_protocol::TransferPhase::Accepted, &b.identity),
        (swarm_protocol::TransferPhase::Committed, &a.identity),
    ] {
        transfer.phase = phase;
        transfer.signer_peer_id = signer.peer_id();
        transfer.signer_public_key = signer.public_key();
        transfer.signature = signer.sign(&transfer.signing_bytes().unwrap());
        b.storage.save_transfer_record(&transfer).unwrap();
        // The retired source durably signed its own committed decision,
        // but its local save history is behind the successor's boundary.
        a.storage.save_transfer_record(&transfer).unwrap();
    }
    b.storage.save_epoch_record(&next).unwrap();
    swarm_cli::migration::ensure_authority_artifacts(&b.storage, &b.identity, &next).unwrap();
    let expected = swarm_cli::historical_authority::collect_replay_proof(&b.storage, &b.identity, world).unwrap();
    assert_eq!(expected.checkpoints.len(), 5);
    let _new_host = spawn_daemon(&b, local_ip);
    let _stale_player = spawn_daemon_with_peers(&c, local_ip, &[automatic_transport_address(&b, local_ip)]);
    wait_until("authenticated newer authority proof pinned to older accepted replica", Duration::from_secs(60), || {
        c.storage.load_staged_stale_replay_proof(world).ok().flatten().as_ref()
            == Some(&(anchor.clone(), expected.clone()))
    });
    wait_until("complete stale replica publication", Duration::from_secs(90), || {
        c.storage.load_epoch_record(world).ok().as_ref() == Some(&next)
            && c.storage.latest_snapshot(world).ok().flatten().as_ref()
                == b.storage.latest_snapshot(world).ok().flatten().as_ref()
            && c.storage.stale_publication_pending(world).ok() == Some(false)
    });
    let history = c.storage.list_snapshots(world).unwrap();
    assert_eq!(history, b.storage.list_snapshots(world).unwrap());
    let verified = swarm_cli::historical_authority::verify_replay_proof(
        expected.clone(),
        &c.storage.load_world(world).unwrap().genesis,
        expected.membership.last().unwrap(),
    )
    .unwrap();
    for manifest in &history {
        verified.verify_manifest(manifest).unwrap();
        c.storage.verify_snapshot(manifest).unwrap();
    }
    let new_blob = &boundary.entries.iter().max_by_key(|entry| entry.blob.encoded_size).unwrap().blob;
    assert!(new_blob.encoded_size > 256 * 1024);
    assert!(c.storage.has_complete_blob(world, new_blob));
    assert_eq!(c.storage.load_epoch_record(world).unwrap(), next);
    assert_eq!(c.storage.load_membership_record(world).unwrap(), *expected.membership.last().unwrap());
    assert_eq!(c.storage.load_epoch_by_hash(world, epoch.record_hash().unwrap()).unwrap(), epoch);
    assert_eq!(c.storage.load_transfer_phase(&transfer, swarm_protocol::TransferPhase::Committed).unwrap(), transfer);
    assert!(c.storage.load_transfer_record(world).is_err());
    swarm_cli::live_authority::verify_playable_epoch(&c.storage, &next).unwrap();
    assert!(swarm_cli::hosting_budget::load_consent(&c.paths, world).unwrap().is_none());
    drop(_stale_player);
    let reopened = Storage::open(c.paths.root.clone()).unwrap();
    assert_eq!(reopened.load_epoch_record(world).unwrap(), next);
    assert_eq!(reopened.list_snapshots(world).unwrap(), history);
    assert!(reopened.load_staged_stale_replay_proof(world).unwrap().is_none());

    // The original source has the exact local committed transfer, but not
    // the successor's checkpoint suffix. A surviving JVM ownership record
    // must block canonical publication even after all data is verified.
    let retired_anchor = a.storage.stale_replay_anchor(world).unwrap();
    let process_dir = a.paths.root.join("control").join(world.to_hex());
    fs::create_dir_all(&process_dir).unwrap();
    let process_record = process_dir.join("runtime-process.json");
    fs::write(
        &process_record,
        serde_json::to_vec(&serde_json::json!({"version": 1, "controller_pid": 1, "java_pid": std::process::id()}))
            .unwrap(),
    )
    .unwrap();
    let retired = spawn_daemon_with_peers(&a, local_ip, &[automatic_transport_address(&b, local_ip)]);
    wait_until(
        "retired source downloads exact isolated suffix while runtime remains occupied",
        Duration::from_secs(90),
        || {
            a.storage.stale_replay_progress(world).ok().flatten().is_some_and(|progress| {
                progress.anchor == retired_anchor
                    && progress.proof == expected
                    && progress.completed == expected.checkpoints.len()
            })
        },
    );
    assert_eq!(a.storage.load_epoch_record(world).unwrap(), epoch);
    assert_eq!(a.storage.load_transfer_record(world).unwrap(), transfer);
    assert!(!a.storage.stale_publication_pending(world).unwrap());
    assert_eq!(a.storage.latest_snapshot(world).unwrap().unwrap(), head);
    // Exercise the publisher directly as well, so the test proves a denial
    // rather than merely observing the interval before the next daemon tick.
    let error = swarm_cli::historical_authority::publish_stale_replay(&a.storage, &a.identity, world).unwrap_err();
    assert!(error.to_string().contains("unaccounted Minecraft process"), "{error:#}");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(process_dir.join("authority-runtime.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    // The test PID stands in for an unaccounted surviving process. An
    // independently held runtime lock must deny publication as well.
    fs::remove_file(&process_record).unwrap();
    let error = swarm_cli::historical_authority::publish_stale_replay(&a.storage, &a.identity, world).unwrap_err();
    assert!(error.to_string().contains("active authority runtime"), "{error:#}");
    assert_eq!(a.storage.load_epoch_record(world).unwrap(), epoch);
    assert!(!a.storage.stale_publication_pending(world).unwrap());
    fs2::FileExt::unlock(&lock).unwrap();
    drop(lock);
    wait_until("retired source re-enters as a replica", Duration::from_secs(90), || {
        a.storage.load_epoch_record(world).ok().as_ref() == Some(&next)
            && a.storage.latest_snapshot(world).ok().flatten().as_ref()
                == b.storage.latest_snapshot(world).ok().flatten().as_ref()
            && a.storage.stale_publication_pending(world).ok() == Some(false)
            && a.storage.load_transfer_record(world).is_err()
    });
    assert_eq!(a.storage.list_snapshots(world).unwrap(), history);
    assert_eq!(a.storage.load_membership_record(world).unwrap(), *expected.membership.last().unwrap());
    assert_eq!(a.storage.load_transfer_phase(&transfer, swarm_protocol::TransferPhase::Committed).unwrap(), transfer);
    assert!(swarm_cli::hosting_budget::load_consent(&a.paths, world).unwrap().is_none());
    swarm_cli::live_authority::verify_playable_epoch(&a.storage, &next).unwrap();
    drop(retired);
    let reopened = Storage::open(a.paths.root.clone()).unwrap();
    assert_eq!(reopened.load_epoch_record(world).unwrap(), next);
    assert_eq!(reopened.list_snapshots(world).unwrap(), history);

    // Now the newer authority crashes without a locally committed outbound
    // transfer. A different peer has an additional old-generation checkpoint
    // and a certified recovery. The crashed source must freeze immediately
    // upon verifying that history and resume the isolated replay on restart.
    drop(_new_host);
    let crash_anchor = b.storage.stale_replay_anchor(world).unwrap();
    let recovery_membership = a.storage.load_membership_record(world).unwrap();
    let crash_head = a.storage.latest_snapshot(world).unwrap().unwrap();
    fs::write(source.path().join("level.dat"), b"last replicated checkpoint before authority crash").unwrap();
    let mut boundary = a
        .storage
        .snapshot_directory(
            source.path(),
            SnapshotContext {
                world,
                snapshot_number: crash_head.snapshot_number + 1,
                sequence: crash_head.sequence + 1,
                epoch: next.epoch_number,
                previous_snapshot_hash: Some(crash_head.manifest_hash().unwrap()),
                authority_peer_id: b.identity.peer_id(),
                authority_public_key: b.identity.public_key(),
            },
        )
        .unwrap();
    b.identity.sign_snapshot(&mut boundary).unwrap();
    a.storage.commit_snapshot(&boundary).unwrap();
    let mut recovered = swarm_protocol::EpochRecordV1 {
        epoch_number: next.epoch_number + 1,
        fencing_token: next.fencing_token + 1,
        previous_epoch_hash: Some(next.record_hash().unwrap()),
        base_state_hash: boundary.state_root,
        authority_peer_id: a.identity.peer_id(),
        authority_public_key: a.identity.public_key(),
        mode: EpochMode::Recovery,
        reason: "certified crash recovery with retained replica suffix".into(),
        signature: Vec::new(),
        ..next.clone()
    };
    recovered.signature = a.identity.sign(&recovered.signing_bytes().unwrap());
    let mut ballot = swarm_protocol::RecoveryBallotV1 {
        protocol_version: swarm_protocol::PROTOCOL_VERSION,
        world_id: world,
        base_epoch: next.epoch_number,
        base_fencing_token: next.fencing_token,
        target_epoch: recovered.epoch_number,
        target_fencing_token: recovered.fencing_token,
        round: 1,
        candidate_peer_id: a.identity.peer_id(),
        candidate_public_key: a.identity.public_key(),
        base_snapshot_hash: boundary.manifest_hash().unwrap(),
        base_state_hash: boundary.state_root,
        membership_hash: recovery_membership.record_hash().unwrap(),
        signature: Vec::new(),
    };
    ballot.signature = a.identity.sign(&ballot.signing_bytes().unwrap());
    let votes = [&a.identity, &c.identity]
        .iter()
        .map(|voter| {
            let mut vote = swarm_protocol::RecoveryVoteV1 {
                protocol_version: swarm_protocol::PROTOCOL_VERSION,
                world_id: world,
                ballot_hash: ballot.ballot_hash().unwrap(),
                base_epoch: ballot.base_epoch,
                target_epoch: ballot.target_epoch,
                round: ballot.round,
                candidate_peer_id: ballot.candidate_peer_id,
                voter_peer_id: voter.peer_id(),
                voter_public_key: voter.public_key(),
                signature: Vec::new(),
            };
            vote.signature = voter.sign(&vote.signing_bytes().unwrap());
            vote
        })
        .collect();
    let certificate = swarm_protocol::RecoveryCertificateV1 { ballot, votes };
    a.storage.save_recovery_certificate(&certificate).unwrap();
    a.storage.save_epoch_record(&recovered).unwrap();
    swarm_cli::migration::ensure_authority_artifacts(&a.storage, &a.identity, &recovered).unwrap();
    let crash_proof = swarm_cli::historical_authority::collect_replay_proof(&a.storage, &a.identity, world).unwrap();
    let recovered_host = spawn_daemon(&a, local_ip);
    let crashed_host = spawn_daemon_with_peers(&b, local_ip, &[automatic_transport_address(&a, local_ip)]);
    wait_until("crashed authority durably freezes exact recovery envelope", Duration::from_secs(60), || {
        b.storage.crash_retirement_pending(world).ok() == Some(true)
            && b.storage.load_staged_stale_replay_proof(world).ok().flatten().as_ref()
                == Some(&(crash_anchor.clone(), crash_proof.clone()))
    });
    assert!(b.storage.load_epoch_record(world).is_err());
    assert!(b.storage.latest_snapshot(world).is_err());
    assert!(b.storage.save_epoch_record(&next).is_err());
    assert!(swarm_cli::live_authority::verify_playable_epoch(&b.storage, &next).is_err());
    drop(crashed_host);
    let reopened = Storage::open(b.paths.root.clone()).unwrap();
    assert!(reopened.crash_retirement_pending(world).unwrap());
    assert!(reopened.load_epoch_record(world).is_err());
    let resumed = spawn_daemon_with_peers(&b, local_ip, &[automatic_transport_address(&a, local_ip)]);
    wait_until("crashed authority resumes isolated suffix and re-enters as replica", Duration::from_secs(90), || {
        b.storage.load_epoch_record(world).ok().as_ref() == Some(&recovered)
            && b.storage.latest_snapshot(world).ok().flatten().as_ref()
                == a.storage.latest_snapshot(world).ok().flatten().as_ref()
            && b.storage.stale_publication_pending(world).ok() == Some(false)
            && b.storage.crash_retirement_pending(world).ok() == Some(false)
    });
    assert_eq!(b.storage.list_snapshots(world).unwrap(), a.storage.list_snapshots(world).unwrap());
    assert_eq!(b.storage.load_recovery_certificate(world).unwrap(), certificate);
    assert_eq!(b.storage.load_membership_record(world).unwrap(), *crash_proof.membership.last().unwrap());
    assert!(swarm_cli::hosting_budget::load_consent(&b.paths, world).unwrap().is_none());
    swarm_cli::live_authority::verify_playable_epoch(&b.storage, &recovered).unwrap();
    drop((resumed, recovered_host));

    // With every authority process stopped, the former host's exact verified
    // replica can produce an explicitly NEW private world. It cannot sign an
    // old-world replay envelope or acquire old-world authority.
    let original_epoch = b.storage.load_epoch_record(world).unwrap();
    let original_membership = b.storage.load_membership_record(world).unwrap();
    let original_checkpoint = b.storage.latest_snapshot(world).unwrap().unwrap();
    let original_hash = original_checkpoint.manifest_hash().unwrap().to_string();
    assert_ne!(original_epoch.authority_peer_id, b.identity.peer_id());
    assert!(swarm_cli::historical_authority::collect_replay_proof(&b.storage, &b.identity, world).is_err());
    let recovered_output = run_cli(
        &b,
        &[
            "world",
            "recover-copy",
            &world.to_string(),
            "--checkpoint",
            &original_hash,
            "--confirm-new-identity",
            "--json",
        ],
    );
    let copied: swarm_cli::world_import::RecoverCopyResult = serde_json::from_str(&recovered_output).unwrap();
    let new_world: WorldId = copied.world_id.parse().unwrap();
    assert_ne!(new_world, world);
    assert_eq!(copied.source_world_id, world.to_string());
    assert_eq!(copied.source_snapshot_hash, original_hash);
    let copied_manifest = b.storage.latest_snapshot(new_world).unwrap().unwrap();
    assert_eq!(copied_manifest.entries, original_checkpoint.entries);
    assert_eq!(copied_manifest.state_root, original_checkpoint.state_root);
    b.storage.verify_snapshot(&copied_manifest).unwrap();
    assert_eq!(b.storage.load_membership_record(new_world).unwrap().members.len(), 1);
    assert_eq!(b.storage.load_epoch_record(new_world).unwrap().authority_peer_id, b.identity.peer_id());
    assert!(swarm_cli::hosting_budget::load_consent(&b.paths, new_world).unwrap().is_none());
    assert_eq!(b.storage.load_epoch_record(world).unwrap(), original_epoch);
    assert_eq!(b.storage.load_membership_record(world).unwrap(), original_membership);
    assert_eq!(b.storage.latest_snapshot(world).unwrap().unwrap(), original_checkpoint);
}

#[test]
fn late_join_after_signed_wake_retargets_during_download_and_publishes_only_final_epoch() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();
    let d = peer_fixture();
    let local_ip = local_non_loopback_ipv4();
    let output = run_cli(
        &a,
        &["world", "create", "--name", "historical-replay-join", "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"],
    );
    let world: WorldId = output.lines().find_map(|line| line.strip_prefix("World ID: ")).unwrap().parse().unwrap();
    let first = a.storage.load_epoch_record(world).unwrap();
    let base = a.storage.latest_snapshot(world).unwrap().unwrap();
    // Exercise a genuine signed sleep boundary and promoted checkpoint,
    // without starting a Minecraft JVM or weakening the RAM admission gate.
    let mut sleep = swarm_protocol::SleepRecordV1 {
        protocol_version: swarm_protocol::PROTOCOL_VERSION,
        world_id: world,
        latest_snapshot_hash: base.manifest_hash().unwrap(),
        epoch: first.epoch_number,
        fencing_token: first.fencing_token,
        authority_peer_id: a.identity.peer_id(),
        authority_public_key: a.identity.public_key(),
        signature: Vec::new(),
    };
    sleep.signature = a.identity.sign(&sleep.signing_bytes().unwrap());
    a.storage.save_sleep_record(&sleep).unwrap();
    let mut next = swarm_protocol::EpochRecordV1 {
        epoch_number: first.epoch_number + 1,
        fencing_token: first.fencing_token + 1,
        previous_epoch_hash: Some(first.record_hash().unwrap()),
        reason: "wake from durable sleep".into(),
        signature: Vec::new(),
        ..first.clone()
    };
    next.signature = a.identity.sign(&next.signing_bytes().unwrap());
    a.storage.save_epoch_record(&next).unwrap();
    a.storage.clear_sleep_record(world).unwrap();
    swarm_cli::migration::ensure_authority_artifacts(&a.storage, &a.identity, &next).unwrap();
    assert_eq!(a.storage.latest_snapshot(world).unwrap().unwrap().epoch, next.epoch_number);
    // Include fresh encoded blobs after the epoch promotion, so the
    // restarted receiver must download real bytes rather than just metadata.
    let source = tempfile::tempdir().unwrap();
    let mut randomish = Vec::with_capacity(1024 * 1024);
    let mut seed = 0xC0FFEE_u64;
    for _ in 0..1024 * 1024 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        randomish.push(seed as u8);
    }
    fs::write(source.path().join("region.dat"), randomish).unwrap();
    fs::write(source.path().join("level.dat"), b"post-wake-checkpoint").unwrap();
    let promoted = a.storage.latest_snapshot(world).unwrap().unwrap();
    let mut published = a
        .storage
        .snapshot_directory(
            source.path(),
            SnapshotContext {
                world,
                snapshot_number: 3,
                sequence: promoted.sequence + 1,
                epoch: next.epoch_number,
                previous_snapshot_hash: Some(promoted.manifest_hash().unwrap()),
                authority_peer_id: a.identity.peer_id(),
                authority_public_key: a.identity.public_key(),
            },
        )
        .unwrap();
    a.identity.sign_snapshot(&mut published).unwrap();
    a.storage.commit_snapshot(&published).unwrap();
    let target = published.manifest().clone();
    let diagnostics = ConnectivityDiagnosticsV1 {
        local_addresses: vec![automatic_transport_address(&a, local_ip)],
        ..Default::default()
    };
    fs::write(a.paths.root.join(DEFAULT_CONNECTIVITY_DIAGNOSTICS_JSON_FILE), serde_json::to_vec(&diagnostics).unwrap())
        .unwrap();
    let token = run_cli(&a, &["invite", "create", &world.to_string()]);
    let daemon_a = spawn_daemon(&a, local_ip);
    let daemon_c = spawn_daemon(&c, local_ip);
    thread::sleep(Duration::from_secs(1));
    assert!(run_cli(&c, &["world", "join", &token, "--host-candidate"]).contains("Join request staged"));
    wait_until("eligible successor receives signed epoch and full history", Duration::from_secs(90), || {
        c.storage.load_epoch_record(world).ok().as_ref() == Some(&next)
            && c.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&target)
            && c.storage.load_membership_record(world).ok() == a.storage.load_membership_record(world).ok()
    });
    let daemon_b = spawn_daemon(&b, local_ip);
    thread::sleep(Duration::from_secs(1));
    assert!(run_cli(&b, &["world", "join", &token]).contains("Join request staged"));
    wait_until("authenticated historical replay proof staged", Duration::from_secs(60), || {
        let Ok(membership) = b.storage.load_membership_record(world) else { return false };
        let Ok(hash) = membership.record_hash() else { return false };
        b.storage.load_staged_historical_replay_proof(world, hash).is_ok_and(|proof| proof.is_some())
    });
    let membership = b.storage.load_membership_record(world).unwrap();
    assert_eq!(membership.epoch, 2);
    let membership_hash = membership.record_hash().unwrap();
    let bytes = b.storage.load_staged_historical_replay_proof(world, membership_hash).unwrap().unwrap();
    let proof: swarm_protocol::HistoricalReplayProofV1 = postcard::from_bytes(&bytes).unwrap();
    let verified = swarm_cli::historical_authority::verify_replay_proof(
        proof,
        &b.storage.load_world(world).unwrap().genesis,
        &membership,
    )
    .unwrap();
    assert_eq!(verified.proof().epochs, vec![first.clone(), next.clone()]);
    assert_eq!(verified.proof().checkpoints.len(), 3);
    // Metadata proof receipt must never become a live epoch, checkpoint
    // receipt, config, or implicit hosting consent.
    assert!(b.storage.load_epoch_record(world).is_err());
    assert!(b.storage.latest_snapshot(world).unwrap().is_none());
    assert!(b.storage.load_world_config(world).is_err());
    assert!(swarm_cli::hosting_budget::load_consent(&b.paths, world).unwrap().is_none());
    let completed = b.storage.load_completed_join(world).unwrap();
    assert_eq!(completed.invite.bootstrap_addrs, vec![automatic_transport_address(&a, local_ip)]);
    assert!(b.storage.load_pending_join(world).is_err(), "a committed invite must never be resent as a join");
    drop(daemon_b);
    wait_until("successor receives the late player's signed admission", Duration::from_secs(60), || {
        c.storage.load_membership_record(world).ok().as_ref() == Some(&membership)
            && c.storage.load_world_descriptor(world).is_ok_and(|descriptor| descriptor.members == membership.members)
    });
    // B is offline, so it cannot know the next host candidate. A and C
    // must jointly admit D and D must catch up before the transfer.
    let daemon_d = spawn_daemon(&d, local_ip);
    thread::sleep(Duration::from_secs(1));
    assert!(run_cli(&d, &["world", "join", &token, "--host-candidate"]).contains("Join request staged"));
    wait_until("new candidate admitted while late player offline", Duration::from_secs(90), || {
        let Ok(admitted) = a.storage.load_membership_record(world) else { return false };
        admitted.members.iter().any(|member| member.peer_id == d.identity.peer_id() && member.authority_eligible)
            && c.storage.load_membership_record(world).ok().as_ref() == Some(&admitted)
            && d.storage.load_membership_record(world).ok().as_ref() == Some(&admitted)
            && d.storage.load_epoch_record(world).ok().as_ref() == Some(&next)
            && d.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&target)
    });
    assert!(!membership.members.iter().any(|member| member.peer_id == d.identity.peer_id()));
    assert_eq!(b.storage.load_membership_record(world).unwrap(), membership);
    drop((daemon_a, daemon_c, daemon_d));
    let reopened = Storage::open(b.paths.root.clone()).unwrap();
    assert_eq!(reopened.load_staged_historical_replay_proof(world, membership_hash).unwrap(), Some(bytes.clone()));
    assert_eq!(reopened.load_completed_join(world).unwrap(), completed);
    assert!(reopened.load_epoch_record(world).is_err());
    assert!(verified.publish(&reopened).is_err(), "an incomplete proof must not publish an epoch");
    assert!(reopened.save_epoch_record(&next).is_err(), "ordinary epoch writes cannot bypass the staged replay");
    // Persist a verified first checkpoint plus an interrupted download of
    // a later blob. These are exactly the on-disk artifacts a killed daemon
    // leaves; no mock authority or application lease is manufactured.
    for entry in &base.entries {
        let blob = &entry.blob;
        if reopened.has_complete_blob(world, blob) {
            continue;
        }
        let (data, finished) =
            a.storage.read_encoded_blob_chunk(world, blob, 0, usize::try_from(blob.encoded_size).unwrap()).unwrap();
        assert!(finished);
        reopened.receive_blob_chunk(world, blob, 0, &data, true).unwrap();
    }
    verified.verify_manifest(&base).unwrap();
    reopened.finalize_replica(&base).unwrap();
    let large = &target.entries.iter().max_by_key(|entry| entry.blob.encoded_size).unwrap().blob;
    assert!(large.encoded_size > 256 * 1024);
    let (chunk, finished) = a.storage.read_encoded_blob_chunk(world, large, 0, 128 * 1024).unwrap();
    assert!(!finished);
    reopened.receive_blob_chunk(world, large, 0, &chunk, false).unwrap();
    let small = &target.entries.iter().min_by_key(|entry| entry.blob.encoded_size).unwrap().blob;
    let (chunk, finished) =
        a.storage.read_encoded_blob_chunk(world, small, 0, usize::try_from(small.encoded_size).unwrap()).unwrap();
    assert!(finished);
    reopened.receive_blob_chunk(world, small, 0, &chunk, false).unwrap();
    assert_eq!(reopened.partial_blob_offset(world, large).unwrap(), 128 * 1024);
    assert_eq!(reopened.partial_blob_offset(world, small).unwrap(), small.encoded_size);
    assert!(reopened.load_epoch_record(world).is_err());
    // The inviter disappears while this player still has only a data
    // prefix. A certified transfer to an newly admitted eligible member
    // must retarget the pinned replay without accepting the intermediate epoch.
    let mut successor = swarm_protocol::EpochRecordV1 {
        epoch_number: next.epoch_number + 1,
        fencing_token: next.fencing_token + 1,
        previous_epoch_hash: Some(next.record_hash().unwrap()),
        base_state_hash: target.state_root,
        authority_peer_id: d.identity.peer_id(),
        authority_public_key: d.identity.public_key(),
        reason: "signed transfer while late join is offline".into(),
        signature: Vec::new(),
        ..next.clone()
    };
    successor.signature = d.identity.sign(&successor.signing_bytes().unwrap());
    let mut transfer = swarm_protocol::AuthorityTransferV1 {
        protocol_version: swarm_protocol::PROTOCOL_VERSION,
        world_id: world,
        from_peer_id: a.identity.peer_id(),
        to_peer_id: d.identity.peer_id(),
        base_snapshot_hash: target.manifest_hash().unwrap(),
        next_epoch: successor.epoch_number,
        next_fencing_token: successor.fencing_token,
        phase: swarm_protocol::TransferPhase::Prepared,
        signer_peer_id: a.identity.peer_id(),
        signer_public_key: a.identity.public_key(),
        signature: Vec::new(),
    };
    for (phase, signer) in [
        (swarm_protocol::TransferPhase::Prepared, &a.identity),
        (swarm_protocol::TransferPhase::Accepted, &d.identity),
        (swarm_protocol::TransferPhase::Committed, &a.identity),
    ] {
        transfer.phase = phase;
        transfer.signer_peer_id = signer.peer_id();
        transfer.signer_public_key = signer.public_key();
        transfer.signature = signer.sign(&transfer.signing_bytes().unwrap());
        d.storage.save_transfer_record(&transfer).unwrap();
    }
    d.storage.save_epoch_record(&successor).unwrap();
    swarm_cli::migration::ensure_authority_artifacts(&d.storage, &d.identity, &successor).unwrap();
    let extended = swarm_cli::historical_authority::collect_replay_proof(&d.storage, &d.identity, world).unwrap();
    assert!(extended.extends_replay_history(verified.proof()));
    let final_target = d.storage.latest_snapshot(world).unwrap().unwrap();
    let _daemon_d = spawn_daemon(&d, local_ip);
    let _daemon_b = spawn_daemon_with_peers(&b, local_ip, &[automatic_transport_address(&d, local_ip)]);
    wait_until("new authority extends the durable pinned first-contact proof", Duration::from_secs(60), || {
        assert!(b.storage.load_epoch_record(world).is_err(), "intermediate epoch must not be installed");
        b.storage
            .load_first_contact_replay(world)
            .ok()
            .flatten()
            .is_some_and(|staged| staged.admission == membership && staged.proof == extended)
    });
    assert_eq!(b.storage.load_membership_record(world).unwrap(), membership);
    wait_until("retargeted replay reaches exact final epoch and checkpoint", Duration::from_secs(90), || {
        if let Ok(accepted) = b.storage.load_epoch_record(world) {
            assert_eq!(accepted, successor);
        }
        b.storage.load_epoch_record(world).ok().as_ref() == Some(&successor)
            && b.storage.latest_snapshot(world).ok().flatten().as_ref() == Some(&final_target)
            && b.storage.load_world_config(world).ok() == d.storage.load_world_config(world).ok()
    });
    let history = b.storage.list_snapshots(world).unwrap();
    assert_eq!(history, d.storage.list_snapshots(world).unwrap());
    assert_eq!(b.storage.load_membership_record(world).unwrap(), *extended.membership.last().unwrap());
    let verified = swarm_cli::historical_authority::verify_replay_proof(
        extended.clone(),
        &b.storage.load_world(world).unwrap().genesis,
        extended.membership.last().unwrap(),
    )
    .unwrap();
    for manifest in &history {
        verified.verify_manifest(manifest).unwrap();
        b.storage.verify_snapshot(manifest).unwrap();
    }
    assert_eq!(b.storage.load_epoch_by_hash(world, first.record_hash().unwrap()).unwrap(), first);
    assert_eq!(b.storage.load_epoch_by_hash(world, next.record_hash().unwrap()).unwrap(), next);
    assert_eq!(b.storage.load_transfer_phase(&transfer, swarm_protocol::TransferPhase::Committed).unwrap(), transfer);
    assert!(b.storage.load_transfer_record(world).is_err());
    assert_eq!(b.storage.load_sleep_boundary(&first, base.manifest_hash().unwrap()).unwrap(), sleep);
    assert!(b.storage.load_sleep_record(world).is_err());
    assert!(swarm_cli::hosting_budget::load_consent(&b.paths, world).unwrap().is_none());
    assert!(swarm_cli::live_authority::verify_playable_epoch(&b.storage, &next).is_err());
    swarm_cli::live_authority::verify_playable_epoch(&b.storage, &successor).unwrap();
}
