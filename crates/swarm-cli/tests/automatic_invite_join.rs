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
}

#[test]
fn late_join_after_signed_wake_retargets_during_download_and_publishes_only_final_epoch() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();
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
    drop((daemon_a, daemon_c));
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
    // prefix. A certified transfer to an already known eligible member
    // must retarget the pinned replay without accepting the intermediate epoch.
    let mut successor = swarm_protocol::EpochRecordV1 {
        epoch_number: next.epoch_number + 1,
        fencing_token: next.fencing_token + 1,
        previous_epoch_hash: Some(next.record_hash().unwrap()),
        base_state_hash: target.state_root,
        authority_peer_id: c.identity.peer_id(),
        authority_public_key: c.identity.public_key(),
        reason: "signed transfer while late join is offline".into(),
        signature: Vec::new(),
        ..next.clone()
    };
    successor.signature = c.identity.sign(&successor.signing_bytes().unwrap());
    let mut transfer = swarm_protocol::AuthorityTransferV1 {
        protocol_version: swarm_protocol::PROTOCOL_VERSION,
        world_id: world,
        from_peer_id: a.identity.peer_id(),
        to_peer_id: c.identity.peer_id(),
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
        (swarm_protocol::TransferPhase::Accepted, &c.identity),
        (swarm_protocol::TransferPhase::Committed, &a.identity),
    ] {
        transfer.phase = phase;
        transfer.signer_peer_id = signer.peer_id();
        transfer.signer_public_key = signer.public_key();
        transfer.signature = signer.sign(&transfer.signing_bytes().unwrap());
        c.storage.save_transfer_record(&transfer).unwrap();
    }
    c.storage.save_epoch_record(&successor).unwrap();
    swarm_cli::migration::ensure_authority_artifacts(&c.storage, &c.identity, &successor).unwrap();
    let extended = swarm_cli::historical_authority::collect_replay_proof(&c.storage, &c.identity, world).unwrap();
    assert!(extended.extends_replay_history(verified.proof()));
    let final_target = c.storage.latest_snapshot(world).unwrap().unwrap();
    let _daemon_c = spawn_daemon(&c, local_ip);
    let _daemon_b = spawn_daemon_with_peers(&b, local_ip, &[automatic_transport_address(&c, local_ip)]);
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
            && b.storage.load_world_config(world).ok() == c.storage.load_world_config(world).ok()
    });
    let history = b.storage.list_snapshots(world).unwrap();
    assert_eq!(history, c.storage.list_snapshots(world).unwrap());
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
