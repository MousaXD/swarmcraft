#![cfg(unix)]

//! Permanent process-level gate for the Desktop manual authority transfer flow.
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
    path::Path,
    process::{Command, Output},
};

use swarm_cli::migration::load_migration_status;
use swarm_core::{create_world_genesis_with_fingerprint, sign_world_config, DataPaths, PeerIdentity};
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

fn initialize_two_peer_world(alice: &PeerFixture, bob: &PeerFixture, source: &Path) -> SharedWorld {
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
    alice.storage.create_world(&metadata).unwrap();
    bob.storage.create_world(&metadata).unwrap();

    let mut descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        compatibility_fingerprint: genesis.compatibility_fingerprint,
        members: vec![member(&alice.identity), member(&bob.identity)],
        preferred_replication_factor: 2,
    };
    descriptor.normalize();
    alice.storage.save_world_descriptor(&descriptor).unwrap();
    bob.storage.save_world_descriptor(&descriptor).unwrap();

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
    alice.storage.save_membership_record(&membership).unwrap();
    bob.storage.save_membership_record(&membership).unwrap();

    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 2 },
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
    alice.storage.save_world_config(&config).unwrap();
    bob.storage.save_world_config(&config).unwrap();

    let alice_snapshot = snapshot(&alice.storage, &alice.identity, world, source);
    let bob_snapshot = snapshot(&bob.storage, &alice.identity, world, source);
    assert_eq!(alice_snapshot.manifest_hash().unwrap(), bob_snapshot.manifest_hash().unwrap());

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
    alice.storage.save_epoch_record(&epoch).unwrap();
    bob.storage.save_epoch_record(&epoch).unwrap();
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
    let alice = peer(tempfile::tempdir().unwrap());
    let bob = peer(tempfile::tempdir().unwrap());

    let source_temp = tempfile::tempdir().unwrap();
    let source = source_temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), b"canonical-before-transfer\n").unwrap();

    let shared = initialize_two_peer_world(&alice, &bob, &source);
    save_sleep(&alice, shared.world);
    let world_hex = shared.world.to_hex();
    let bob_peer = bob.identity.peer_id().to_string();

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
    // Stage 4/6: commit fences the source generation.
    let committed = first_line(&run_cli_ok(&alice, &["world", "transfer-commit", &world_hex, &accepted])).to_string();
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
}
