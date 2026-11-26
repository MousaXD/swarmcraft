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
    os::unix::fs::PermissionsExt,
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

fn private_peer_dir() -> TempDir {
    tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap()
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
    epoch: u64,
) -> swarm_protocol::SnapshotManifestV1 {
    let mut manifest = storage
        .snapshot_directory(
            source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch,
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

fn initialize_peer_world(
    alice: &PeerFixture,
    others: &[&PeerFixture],
    source: &Path,
    initial_epoch: u64,
) -> SharedWorld {
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
    let (_, mut genesis) = create_world_genesis_with_fingerprint(
        &alice.identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        fingerprint,
    )
    .unwrap();
    // Initially configured voters must be anchored in world genesis.
    genesis.initial_membership = peers.iter().map(|peer| peer.identity.peer_id()).collect();
    genesis.initial_membership.sort();
    let world = genesis.world_id().unwrap();
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

    let alice_snapshot = snapshot(&alice.storage, &alice.identity, world, source, initial_epoch);
    for peer in others {
        let replica = snapshot(&peer.storage, &alice.identity, world, source, initial_epoch);
        assert_eq!(alice_snapshot.manifest_hash().unwrap(), replica.manifest_hash().unwrap());
    }

    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: initial_epoch,
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
    if initial_epoch != 0 {
        let mut promoted = membership.clone();
        promoted.epoch = initial_epoch;
        promoted.sequence += 1;
        promoted.previous_membership_hash = Some(membership.record_hash().unwrap());
        alice.identity.sign_membership(&mut promoted).unwrap();
        for peer in &peers {
            peer.storage.save_membership_record(&promoted).unwrap();
        }
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
    let alice = peer(private_peer_dir());
    let bob = peer(private_peer_dir());

    let source_temp = tempfile::tempdir().unwrap();
    let source = source_temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-before-transfer\n").unwrap();

    let shared = initialize_peer_world(&alice, &[&bob], &source, 0);
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
    let alice = peer(private_peer_dir());
    let bob = peer(private_peer_dir());
    let carol = peer(private_peer_dir());
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("level.dat"), b"canonical-network-handoff\n").unwrap();
    let shared = initialize_peer_world(&alice, &[&bob, &carol], source.path(), 0);
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

// This is deliberately a separate hosted gate: it uses official Minecraft,
// a real candidate Fabric JAR, real daemon-owned IPC, and the actual gateway.
// A status handshake exercises game bytes without pretending that a Microsoft
// account logged in or that these local processes are independent machines.
#[cfg(target_os = "linux")]
mod hosted_live {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpStream,
        os::unix::process::CommandExt,
    };
    use swarm_cli::game_route_api::GameRouteStatusV1;

    struct LiveDaemon(Child, std::path::PathBuf);

    impl Drop for LiveDaemon {
        fn drop(&mut self) {
            // This process group was created solely for this disposable peer.
            // Kill its JVM too on a failed assertion or a simulated crash.
            let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", self.0.id())]).status();
            let _ = self.0.wait();
            if std::thread::panicking() {
                let log = fs::read_to_string(&self.1).unwrap_or_default();
                let lines = log.lines().rev().take(100).collect::<Vec<_>>();
                eprintln!(
                    "hosted daemon tail ({}):\n{}",
                    self.1.display(),
                    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
                );
            }
        }
    }

    fn spawn(peer: &PeerFixture, port: u16, addresses: &[String]) -> LiveDaemon {
        let log_path = peer.paths.root.join("hosted-daemon.log");
        let log = fs::File::create(&log_path).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
        command
            .arg("--data-dir")
            .arg(&peer.paths.root)
            .arg("daemon")
            .arg("--listen")
            .arg(format!("/ip4/127.0.0.1/udp/{port}/quic-v1"))
            .env("RUST_LOG", "info,swarm_network::game_transport=debug,swarm_cli::live_authority=debug")
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .process_group(0);
        for address in addresses {
            command.arg("--peer").arg(address);
        }
        LiveDaemon(command.spawn().unwrap(), log_path)
    }

    fn runtime(peer: &PeerFixture, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_swarmcraft-runtime"))
            .arg("--data-dir")
            .arg(&peer.paths.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "runtime {args:?}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn eventually(label: &str, peers: &[&PeerFixture], mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(180);
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            thread::sleep(Duration::from_millis(500));
        }
        for peer in peers {
            let log = fs::read_to_string(peer.paths.root.join("hosted-daemon.log")).unwrap_or_default();
            let lines = log.lines().rev().take(50).collect::<Vec<_>>();
            eprintln!(
                "peer {} last daemon messages:\n{}",
                peer.identity.peer_id(),
                lines.into_iter().rev().collect::<Vec<_>>().join("\n")
            );
        }
        panic!("timed out waiting for {label}");
    }

    fn verified_route(
        player: &PeerFixture,
        world: WorldId,
        authority: &PeerFixture,
        epoch: u64,
        peers: &[&PeerFixture],
    ) -> String {
        let mut endpoint = None;
        eventually("live signed route to expected authority", peers, || {
            let output = run_cli(player, &["world", "game-route-status", &world.to_string(), "--json"]);
            let Ok(status) = serde_json::from_slice::<GameRouteStatusV1>(&output.stdout) else { return false };
            if !output.status.success()
                || status.world_id != world.to_string()
                || status.state != "verified"
                || !status.quorum_verified
                || !status.hosting_consent_verified
                || !status.server_ready
                || !status.route_verified
                || status.authority_peer_id.as_deref() != Some(authority.identity.peer_id().to_string().as_str())
                || status.epoch != Some(epoch)
                || status.snapshot_hash.is_none()
            {
                return false;
            }
            endpoint = status.local_gateway;
            endpoint.is_some()
        });
        endpoint.unwrap()
    }

    fn put_varint(mut value: u32, bytes: &mut Vec<u8>) {
        loop {
            let next = (value & 0x7f) as u8;
            value >>= 7;
            bytes.push(if value == 0 { next } else { next | 0x80 });
            if value == 0 {
                break;
            }
        }
    }

    fn get_varint(reader: &mut impl Read) -> u32 {
        let mut value = 0;
        for offset in (0..35).step_by(7) {
            let mut byte = [0u8];
            reader.read_exact(&mut byte).expect("Minecraft response truncated");
            value |= u32::from(byte[0] & 0x7f) << offset;
            if byte[0] & 0x80 == 0 {
                return value;
            }
        }
        panic!("invalid Minecraft VarInt");
    }

    fn minecraft_status(endpoint: &str) {
        let address: std::net::SocketAddr = endpoint.parse().unwrap();
        assert!(address.ip().is_loopback());
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(25))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut handshake = vec![0]; // packet id 0
        put_varint(u32::MAX, &mut handshake); // status protocol version negotiation
        put_varint(9, &mut handshake);
        handshake.extend_from_slice(b"localhost");
        handshake.extend_from_slice(&address.port().to_be_bytes());
        handshake.push(1); // next state: status
        let mut packet = Vec::new();
        put_varint(handshake.len().try_into().unwrap(), &mut packet);
        packet.extend(handshake);
        packet.extend([1, 0]); // framed status request
        stream.write_all(&packet).unwrap();
        let length = get_varint(&mut stream) as usize;
        assert!((1..=1_048_576).contains(&length));
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).unwrap();
        let mut body = bytes.as_slice();
        assert_eq!(get_varint(&mut body), 0);
        let json_length = get_varint(&mut body) as usize;
        assert_eq!(json_length, body.len());
        let response: serde_json::Value = serde_json::from_slice(body).unwrap();
        assert!(
            response["version"]["name"].as_str().unwrap().contains("26.1.2"),
            "unexpected real server status {response}"
        );
    }

    /// Optional test-only Java process uses the exact built reconnect
    /// artifact (not a Rust reimplementation of its decoder) and a new
    /// Minecraft status socket. No accounts or GUI are involved.
    fn reconnect_probe(player: &PeerFixture, world: WorldId, endpoint: &str, expected: Option<(&PeerFixture, u64)>) {
        let Some(classpath) = std::env::var_os("SWARMCRAFT_RECONNECT_PROBE_CLASSPATH") else {
            return;
        };
        let config = tempfile::NamedTempFile::new_in(player._temp.path()).unwrap();
        serde_json::to_writer(
            config.as_file(),
            &serde_json::json!({
                "cli": env!("CARGO_BIN_EXE_swarmcraft"),
                "data_dir": player.paths.root,
                "world_id": world.to_string(),
                "local_gateway": endpoint,
                "auto_reconnect": false,
            }),
        )
        .unwrap();
        let (mode, authority, epoch) = expected.map_or_else(
            || ("deny", String::from("-"), String::from("0")),
            |(peer, epoch)| ("allow", peer.identity.peer_id().to_string(), epoch.to_string()),
        );
        let output = Command::new("java")
            .arg("-cp")
            .arg(classpath)
            .arg("dev.swarmcraft.reconnect.LiveGatewayProbe")
            .arg(config.path())
            .args([mode, &authority, &epoch])
            .output()
            .expect("test-only reconnect probe must launch");
        assert!(
            output.status.success(),
            "Java reconnect probe {mode} failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if let Some((peer, epoch)) = expected {
            assert_eq!(result["outcome"], "minecraft_status");
            assert_eq!(result["world"], world.to_string());
            assert_eq!(result["gateway"], endpoint);
            assert_eq!(result["authority"], peer.identity.peer_id().to_string());
            assert_eq!(result["epoch"], epoch);
            assert!(result["fence"].as_u64().is_some_and(|fence| fence > 0));
            assert!(result["snapshot"].as_str().is_some_and(|hash| hash.len() == 64));
            assert!(result["version"].as_str().is_some_and(|version| version.contains("26.1.2")));
            println!("JAVA_RECONNECT_STATUS_PASS epoch={epoch}");
        } else {
            assert_eq!(result, serde_json::json!({"outcome": "refused"}));
            println!("JAVA_RECONNECT_REFUSAL_PASS");
        }
    }

    #[test]
    #[ignore = "requires official managed Minecraft and candidate Fabric artifact; run explicitly on a disposable local lab"]
    fn real_minecraft_gateway_handoff_and_recovery() {
        assert!(hosting_capacity_for_process_test(), "hosted live gate must have enough memory; never skip");
        assert!(std::env::var_os("SWARMCRAFT_FABRIC_MOD_JAR").is_some(), "candidate Fabric artifact is mandatory");
        let alice = peer(private_peer_dir());
        let bob = peer(private_peer_dir());
        let carol = peer(private_peer_dir());
        let source = tempfile::tempdir().unwrap();
        let shared = initialize_peer_world(&alice, &[&bob, &carol], source.path(), 1);
        let world = shared.world;
        let world_id = world.to_string();
        let peers = [&alice, &bob, &carol];
        for host in [&alice, &bob] {
            swarm_cli::hosting_budget::set_contribution(&host.paths, world, 512).unwrap();
            runtime(host, &["install", &world_id, "--accept-eula"]);
            let verified: serde_json::Value = serde_json::from_str(&runtime(host, &["verify", &world_id])).unwrap();
            assert_eq!(verified["ready"], true);
        }
        assert!(swarm_cli::hosting_budget::load_consent(&carol.paths, world).unwrap().is_none());
        let sockets = (0..3).map(|_| UdpSocket::bind("127.0.0.1:0").unwrap()).collect::<Vec<_>>();
        let ports = sockets.iter().map(|socket| socket.local_addr().unwrap().port()).collect::<Vec<_>>();
        let addresses = peers.iter().zip(&ports).map(|(peer, port)| daemon_address(peer, *port)).collect::<Vec<_>>();
        drop(sockets);
        let _alice = spawn(&alice, ports[0], &[]);
        let bob_daemon = spawn(&bob, ports[1], &addresses[..1]);
        let _carol = spawn(&carol, ports[2], &addresses[..2]);
        eventually("player gateway selection", &peers, || {
            run_cli(&carol, &["world", "game-route-select", &world_id]).status.success()
        });
        let endpoint = verified_route(&carol, world, &alice, 1, &peers);
        minecraft_status(&endpoint);
        reconnect_probe(&carol, world, &endpoint, Some((&alice, 1)));
        assert!(!carol.paths.root.join("runtime").join(world.to_hex()).exists());
        // libtest writes the test name without a trailing newline under
        // --nocapture. Emit a standalone exact marker for the CI gate.
        println!("\nHOSTED_LIVE_GATEWAY_INITIAL_PASS");

        let relative =
            Path::new("runtime").join(world.to_hex()).join("world").join("swarmcraft-hosted-gateway-marker.txt");
        fs::write(alice.paths.root.join(&relative), b"real Fabric save before signed handoff\n").unwrap();
        run_cli_ok(&alice, &["world", "transfer-start", &world_id, &bob.identity.peer_id().to_string(), "1"]);
        let successor = verified_route(&carol, world, &bob, 2, &peers);
        assert_eq!(successor, endpoint, "player's permanent loopback address changed");
        minecraft_status(&successor);
        reconnect_probe(&carol, world, &endpoint, Some((&bob, 2)));
        assert_eq!(fs::read(bob.paths.root.join(&relative)).unwrap(), b"real Fabric save before signed handoff\n");
        println!("\nHOSTED_LIVE_GATEWAY_HANDOFF_PASS");

        // Re-verify Alice's installed runtime after releasing her old host.
        // This is the actual managed artifact verifier, not a cached Ready.
        let verified: serde_json::Value = serde_json::from_str(&runtime(&alice, &["verify", &world_id])).unwrap();
        assert_eq!(verified["ready"], true);
        let accepted = bob.storage.latest_snapshot(world).unwrap().unwrap().manifest_hash().unwrap();
        eventually("all replicas hold the exact successor checkpoint", &peers, || {
            peers.iter().all(|peer| {
                peer.storage
                    .latest_snapshot(world)
                    .is_ok_and(|m| m.is_some_and(|m| m.manifest_hash().ok() == Some(accepted)))
                    && peer.storage.load_membership_record(world).is_ok_and(|m| m.epoch == 2)
                    && peer
                        .storage
                        .load_world_config(world)
                        .is_ok_and(|c| c.authority_peer_id == bob.identity.peer_id())
            })
        });
        drop(bob_daemon); // Simulated host crash, including its real JVM.
        let recovered = verified_route(&carol, world, &alice, 3, &peers);
        assert_eq!(recovered, endpoint);
        minecraft_status(&recovered);
        reconnect_probe(&carol, world, &endpoint, Some((&alice, 3)));
        assert_eq!(fs::read(alice.paths.root.join(&relative)).unwrap(), b"real Fabric save before signed handoff\n");
        let epoch = alice.storage.load_epoch_record(world).unwrap();
        assert_eq!(epoch.mode, EpochMode::Recovery);
        swarm_cli::live_authority::verify_playable_epoch(&alice.storage, &epoch).unwrap();
        run_cli_ok(&alice, &["world", "stop", &world_id]);
        eventually("real recovered JVM safe checkpoint", &peers, || alice.storage.load_sleep_record(world).is_ok());
        let saved = alice.storage.load_sleep_record(world).unwrap();
        eventually("surviving voter receives exact signed sleep and checkpoint", &peers, || {
            carol.storage.load_sleep_record(world).is_ok_and(|record| record == saved)
                && carol.storage.latest_snapshot(world).is_ok_and(|head| {
                    head.is_some_and(|head| head.manifest_hash().ok() == Some(saved.latest_snapshot_hash))
                })
        });
        let report = swarm_cli::host_readiness::load_host_readiness_report(&alice.paths, world).unwrap();
        assert!(!report.safe_to_shutdown, "offline voter has not acknowledged the sleep boundary");
        reconnect_probe(&carol, world, &endpoint, None);
        println!("\nHOSTED_LIVE_GATEWAY_RECOVERY_PASS");
    }
}
