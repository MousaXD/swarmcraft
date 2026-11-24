#![cfg(unix)]

//! Process-level gates for the Desktop authority transfer flow.
//!
//! Drives prepare → export → accept → commit → activate → observe through the
//! packaged `swarmcraft` sidecar binary as separate OS processes across two
//! independent peer data directories, without Minecraft. These are exactly the
//! CLI invocations the Tauri transfer bridge performs per signed stage; a
//! regression in argument wiring, token encoding, or stage ordering fails here
//! before it can reach players. The deterministic sleeping-source path is used,
//! matching `migration_core` semantics at process granularity.

use std::{
    fs,
    net::UdpSocket,
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use swarm_cli::migration::{load_migration_status, TransferProgress, TransferProgressPhase};
use swarm_core::{create_world_genesis_with_fingerprint, sign_world_config, DataPaths, PeerIdentity};
use swarm_network::load_or_create_transport_key;
use swarm_protocol::{
    AuthorityPolicyV1, EpochMode, EpochRecordV1, MembershipPolicyV1, MembershipRecordV1,
    RuntimeCompatibilityManifestV1, SleepRecordV1, TransferPhase, WorldConfigV1, WorldDescriptorV1, WorldId,
    WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, WorldMetadataV1};
use tempfile::TempDir;

struct PeerFixture {
    _temp: TempDir,
    paths: DataPaths,
    storage: Storage,
    identity: PeerIdentity,
}

struct SharedWorld {
    world: WorldId,
    epoch: EpochRecordV1,
}

fn hosting_capacity_for_process_test() -> bool {
    match swarm_cli::hosting_budget::memory_snapshot()
        .and_then(|sample| swarm_cli::hosting_budget::check_budget(512, sample))
    {
        Ok(()) => true,
        Err(error) => {
            // A runner too small for a real host must reject admission; never
            // weaken the production RAM safety margin to make CI appear green.
            assert!(
                error.to_string().contains("insufficient hosting memory")
                    || error.to_string().contains("cannot safely establish"),
                "unexpected memory-probe error: {error}"
            );
            eprintln!("Host process scenario requires >=3840 MiB currently available; low-memory runner correctly denies capacity: {error}");
            false
        }
    }
}

fn peer(temp: TempDir) -> PeerFixture {
    let paths = DataPaths::from_root(temp.path().join("data"));
    let storage = Storage::open(paths.root.clone()).unwrap();
    let identity = PeerIdentity::load_or_create(&paths).unwrap();
    PeerFixture { _temp: temp, paths, storage, identity }
}

fn run_cli(peer: &PeerFixture, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .args(args)
        .env("RUST_LOG", "error")
        .output()
        .expect("sidecar binary should launch")
}

fn run_cli_ok(peer: &PeerFixture, args: &[&str]) -> String {
    let output = run_cli(peer, args);
    assert!(output.status.success(), "sidecar stage {args:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn decode_token<T: serde::de::DeserializeOwned>(token: &str) -> T {
    postcard::from_bytes(&hex::decode(token.trim()).unwrap()).unwrap()
}

// Mirrors the Desktop bridge's `transfer_token` extraction: some stages print
// player guidance after the signed token, so only the first stdout line is
// machine data.
fn first_line(output: &str) -> &str {
    output.lines().next().unwrap_or("").trim()
}

fn progress(peer: &PeerFixture, world: &str) -> TransferProgress {
    serde_json::from_str(&run_cli_ok(peer, &["world", "transfer-status", world])).unwrap()
}

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn snapshot(
    storage: &Storage,
    authority: &PeerIdentity,
    world: WorldId,
    source: &Path,
) -> swarm_protocol::SnapshotManifestV1 {
    let mut manifest = storage
        .snapshot_directory(
            source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 0,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: authority.peer_id(),
                authority_public_key: authority.public_key(),
            },
        )
        .unwrap();
    authority.sign_snapshot(&mut manifest).unwrap();
    storage.commit_snapshot(&manifest).unwrap();
    manifest.manifest().clone()
}

fn initialize_peer_world(alice: &PeerFixture, others: &[&PeerFixture], source: &Path) -> SharedWorld {
    let peers = std::iter::once(alice).chain(others.iter().copied()).collect::<Vec<_>>();
    let replica_count = peers.len().try_into().unwrap();
    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: "26.1.2".into(),
        loader_id: "fabric".into(),
        loader_version: "0.19.3".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: env!("CARGO_PKG_VERSION").into(),
        required_server_mods: Vec::new(),
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let fingerprint = compatibility.fingerprint().unwrap();
    let (world, genesis) = create_world_genesis_with_fingerprint(
        &alice.identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        fingerprint,
    )
    .unwrap();
    let metadata = WorldMetadataV1 {
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        display_name: "transfer-process-gate".into(),
        world_id: world,
        genesis: genesis.clone(),
    };
    for peer in &peers {
        peer.storage.create_world(&metadata).unwrap();
    }

    let mut descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        compatibility_fingerprint: genesis.compatibility_fingerprint,
        members: peers.iter().map(|peer| member(&peer.identity)).collect(),
        preferred_replication_factor: replica_count,
    };
    descriptor.normalize();
    for peer in &peers {
        peer.storage.save_world_descriptor(&descriptor).unwrap();
    }

    let mut membership = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch: 0,
        sequence: 0,
        previous_membership_hash: None,
        members: descriptor.members.clone(),
        authority_peer_id: alice.identity.peer_id(),
        authority_public_key: alice.identity.public_key(),
        signature: Vec::new(),
    };
    alice.identity.sign_membership(&mut membership).unwrap();
    for peer in &peers {
        peer.storage.save_membership_record(&membership).unwrap();
    }

    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 {
            allow_solo_advancement: false,
            preferred_replication_factor: replica_count,
        },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: "transfer-process-gate".into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: alice.identity.peer_id(),
        authority_public_key: alice.identity.public_key(),
        signature: Vec::new(),
    };
    sign_world_config(&alice.identity, &mut config).unwrap();
    for peer in &peers {
        peer.storage.save_world_config(&config).unwrap();
    }

    let alice_snapshot = snapshot(&alice.storage, &alice.identity, world, source);
    for peer in others {
        let replica = snapshot(&peer.storage, &alice.identity, world, source);
        assert_eq!(alice_snapshot.manifest_hash().unwrap(), replica.manifest_hash().unwrap());
    }

    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 0,
        previous_epoch_hash: None,
        base_state_hash: alice_snapshot.state_root,
        authority_peer_id: alice.identity.peer_id(),
        authority_public_key: alice.identity.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: 1,
        reason: "initial authority".into(),
        signature: Vec::new(),
    };
    epoch.signature = alice.identity.sign(&epoch.signing_bytes().unwrap());
    for peer in &peers {
        peer.storage.save_epoch_record(&epoch).unwrap();
    }
    SharedWorld { world, epoch }
}

fn save_sleep(peer_fixture: &PeerFixture, world: WorldId) {
    let latest = peer_fixture.storage.latest_snapshot(world).unwrap().unwrap();
    let epoch = peer_fixture.storage.load_epoch_record(world).unwrap();
    let mut record = SleepRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        latest_snapshot_hash: latest.manifest_hash().unwrap(),
        epoch: epoch.epoch_number,
        fencing_token: epoch.fencing_token,
        authority_peer_id: peer_fixture.identity.peer_id(),
        authority_public_key: peer_fixture.identity.public_key(),
        signature: Vec::new(),
    };
    peer_fixture.identity.sign_sleep_record(&mut record).unwrap();
    peer_fixture.storage.save_sleep_record(&record).unwrap();
}

#[test]
fn manual_transfer_stages_run_as_separate_sidecar_processes_without_minecraft() {
    if !hosting_capacity_for_process_test() {
        return;
    }
    let alice = peer(tempfile::tempdir().unwrap());
    let bob = peer(tempfile::tempdir().unwrap());

    let source_temp = tempfile::tempdir().unwrap();
    let source = source_temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-before-transfer\n").unwrap();

    let shared = initialize_peer_world(&alice, &[&bob], &source);
    swarm_cli::hosting_budget::set_contribution(&alice.paths, shared.world, 512).unwrap();
    swarm_cli::hosting_budget::set_contribution(&bob.paths, shared.world, 512).unwrap();
    save_sleep(&alice, shared.world);
    let world_hex = shared.world.to_hex();
    let bob_peer = bob.identity.peer_id().to_string();
    let source_epoch = shared.epoch.epoch_number.to_string();
    assert_eq!(progress(&alice, &world_hex).phase, TransferProgressPhase::Idle);
    assert!(!run_cli(&alice, &["world", "transfer-start", &world_hex, &bob_peer, "999"]).status.success());
    assert_eq!(progress(&alice, &world_hex).phase, TransferProgressPhase::Idle);

    // Desktop's default one-click API produces a typed, durable result,
    // without exporting a token to the UI. Repeated requests do not create
    // another transfer. The manual commands remain the advanced fallback.
    let started: TransferProgress =
        serde_json::from_str(&run_cli_ok(&alice, &["world", "transfer-start", &world_hex, &bob_peer, &source_epoch]))
            .unwrap();
    assert_eq!(started.phase, TransferProgressPhase::Prepared);
    assert_eq!(started.target_peer.as_deref(), Some(bob_peer.as_str()));
    assert_eq!(progress(&alice, &world_hex), started);
    let repeated: TransferProgress =
        serde_json::from_str(&run_cli_ok(&alice, &["world", "transfer-start", &world_hex, &bob_peer, &source_epoch]))
            .unwrap();
    assert_eq!(repeated, started);

    // Stage 1/6: prepare on the sleeping source returns the prepared token.
    let prepared = first_line(&run_cli_ok(&alice, &["world", "transfer-prepare", &world_hex, &bob_peer])).to_string();
    // Stage 2/6: export re-reads the durable record from a separate process.
    let exported = first_line(&run_cli_ok(&alice, &["world", "transfer-export", &world_hex])).to_string();
    assert_eq!(prepared, exported);

    let decoded_prepared: swarm_protocol::AuthorityTransferV1 = decode_token(&prepared);
    assert_eq!(decoded_prepared.from_peer_id, alice.identity.peer_id());
    assert_eq!(decoded_prepared.to_peer_id, bob.identity.peer_id());
    assert_eq!(decoded_prepared.phase, TransferPhase::Prepared);
    assert_eq!(decoded_prepared.next_epoch, shared.epoch.epoch_number + 1);

    // Stage 3/6: accept on the target validates checkpoint and signatures.
    let accepted = first_line(&run_cli_ok(&bob, &["world", "transfer-accept", &world_hex, &prepared])).to_string();
    assert_eq!(progress(&bob, &world_hex).phase, TransferProgressPhase::Accepted);
    // Stage 4/6: commit fences the source generation.
    let committed = first_line(&run_cli_ok(&alice, &["world", "transfer-commit", &world_hex, &accepted])).to_string();
    assert_eq!(progress(&alice, &world_hex).phase, TransferProgressPhase::Committed);
    let retry_committed: TransferProgress =
        serde_json::from_str(&run_cli_ok(&alice, &["world", "transfer-start", &world_hex, &bob_peer, &source_epoch]))
            .unwrap();
    assert_eq!(retry_committed.phase, TransferProgressPhase::Committed);
    assert_ne!(decode_token::<swarm_protocol::AuthorityTransferV1>(&committed).phase, TransferPhase::Prepared);
    // Stage 5/6: activate mints the successor epoch on the target.
    let epoch_token =
        first_line(&run_cli_ok(&bob, &["world", "transfer-activate", &world_hex, &committed])).to_string();

    let decoded_epoch: EpochRecordV1 = decode_token(&epoch_token);
    assert_eq!(decoded_epoch.epoch_number, shared.epoch.epoch_number + 1);
    assert_eq!(decoded_epoch.authority_peer_id, bob.identity.peer_id());
    assert_eq!(decoded_epoch.fencing_token, shared.epoch.fencing_token + 1);
    assert_eq!(decoded_epoch.mode, EpochMode::Quorum);

    // Stage 6/6: observe adopts the successor generation durably on the source.
    run_cli_ok(&alice, &["world", "transfer-observe", &world_hex, &epoch_token]);

    // End state reads back through the same text format the Desktop wizard parses.
    for fixture in [&alice, &bob] {
        let durable = progress(fixture, &world_hex);
        assert_eq!(durable.phase, TransferProgressPhase::SuccessorObserved);
        assert_eq!(durable.current_epoch, decoded_epoch.epoch_number);
        assert_eq!(durable.current_authority, bob_peer);
        let status = run_cli_ok(fixture, &["world", "migration-status", &world_hex]);
        assert!(status.contains(&format!("Authority: {bob_peer}")), "unexpected status: {status}");
        assert!(status.contains(&format!("Epoch: {}", decoded_epoch.epoch_number)), "unexpected status: {status}");
    }
    assert!(load_migration_status(&alice.paths, shared.world)
        .is_ok_and(|status| status.authority_peer_id.as_deref() == Some(bob_peer.as_str())));

    // The completed handoff advances the generation: the old authority can no
    // longer prepare another transfer for the superseded generation.
    let refused = run_cli(&alice, &["world", "transfer-prepare", &world_hex, &bob_peer]);
    assert!(!refused.status.success(), "stale-authority prepare must fail after the transfer");
    assert!(!run_cli(&alice, &["world", "transfer-start", &world_hex, &bob_peer, &source_epoch]).status.success());
}

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn daemon_address(peer: &PeerFixture, port: u16) -> String {
    let key = load_or_create_transport_key(&peer.paths.transport_key()).unwrap();
    format!("/ip4/127.0.0.1/udp/{port}/quic-v1/p2p/{}", key.public().to_peer_id())
}

fn spawn_daemon(peer: &PeerFixture, port: u16, others: &[String]) -> Daemon {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    command
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/127.0.0.1/udp/{port}/quic-v1"))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    for address in others {
        command.arg("--peer").arg(address);
    }
    Daemon(command.spawn().unwrap())
}

fn wait_until(label: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for {label}");
}

#[test]
fn automatic_handoff_waits_for_witness_and_consent_then_converges_without_tokens() {
    if !hosting_capacity_for_process_test() {
        return;
    }
    let alice = peer(tempfile::tempdir().unwrap());
    let bob = peer(tempfile::tempdir().unwrap());
    let carol = peer(tempfile::tempdir().unwrap());
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("level.dat"), b"canonical-network-handoff\n").unwrap();
    let shared = initialize_peer_world(&alice, &[&bob, &carol], source.path());
    let world = shared.world;
    let world_hex = world.to_hex();
    let target = bob.identity.peer_id();
    swarm_cli::hosting_budget::set_contribution(&alice.paths, world, 512).unwrap();
    save_sleep(&alice, world);
    let sleep = alice.storage.load_sleep_record(world).unwrap();
    for peer in [&bob, &carol] {
        peer.storage.save_sleep_record(&sleep).unwrap();
    }

    // Reserve all ports together so this test never chooses the same one.
    let sockets = (0..3).map(|_| UdpSocket::bind("127.0.0.1:0").unwrap()).collect::<Vec<_>>();
    let ports = sockets.iter().map(|socket| socket.local_addr().unwrap().port()).collect::<Vec<_>>();
    let addresses =
        [&alice, &bob, &carol].iter().zip(&ports).map(|(peer, port)| daemon_address(peer, *port)).collect::<Vec<_>>();
    drop(sockets);

    let started: TransferProgress = serde_json::from_str(&run_cli_ok(
        &alice,
        &["world", "transfer-start", &world_hex, &target.to_string(), &shared.epoch.epoch_number.to_string()],
    ))
    .unwrap();
    assert_eq!(started.phase, TransferProgressPhase::Prepared);
    let _a = spawn_daemon(&alice, ports[0], &[]);
    thread::sleep(Duration::from_secs(1));
    let _b = spawn_daemon(&bob, ports[1], &addresses[..1]);

    // Even a connected intended successor cannot receive a Prepared record
    // until the third voter acknowledges the exact final checkpoint.
    thread::sleep(Duration::from_secs(3));
    assert_eq!(progress(&alice, &world_hex).phase, TransferProgressPhase::Prepared);
    assert_eq!(progress(&bob, &world_hex).phase, TransferProgressPhase::Idle);

    let _c = spawn_daemon(&carol, ports[2], &addresses[..2]);
    wait_until("authenticated Prepared delivery to the unconsenting target", || {
        bob.storage.load_transfer_record(world).is_ok_and(|record| record.phase == TransferPhase::Prepared)
    });
    // Canonical transfer evidence and authority eligibility are not consent.
    thread::sleep(Duration::from_secs(3));
    assert_eq!(progress(&bob, &world_hex).phase, TransferProgressPhase::Prepared);
    assert_eq!(bob.storage.load_epoch_record(world).unwrap().authority_peer_id, alice.identity.peer_id());

    swarm_cli::hosting_budget::set_contribution(&bob.paths, world, 512).unwrap();
    wait_until("signed automatic successor epoch on all three replicas", || {
        [&alice, &bob, &carol].iter().all(|peer| {
            peer.storage.load_epoch_record(world).is_ok_and(|epoch| {
                epoch.authority_peer_id == target
                    && epoch.epoch_number == shared.epoch.epoch_number + 1
                    && epoch.fencing_token == shared.epoch.fencing_token + 1
            })
        })
    });
    for peer in [&alice, &bob, &carol] {
        let observed = progress(peer, &world_hex);
        assert_eq!(observed.phase, TransferProgressPhase::SuccessorObserved);
        assert_eq!(observed.current_authority, target.to_string());
        assert_eq!(observed.checkpoint_hash, started.checkpoint_hash);
        // No real Java runtime was configured: a successful transfer of
        // authority must not pretend that Minecraft is already playable.
        assert!(!load_migration_status(&peer.paths, world).is_ok_and(|status| status.runtime_ready));
    }
}
