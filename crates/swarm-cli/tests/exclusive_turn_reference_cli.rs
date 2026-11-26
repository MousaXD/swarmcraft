//! Real separate CLI processes, real signatures, private durable journals.
//! Reference bytes only: no Minecraft authority or runtime is created.
#![cfg(target_os = "linux")]

use fs2::FileExt;
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::*;

fn cmd(profile: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    command.arg("--data-dir").arg(profile).arg("exclusive-turn").args(args);
    command
}

fn invoke(profile: &Path, args: &[&str]) -> Output {
    cmd(profile, args).output().unwrap()
}

fn success(output: Output) -> Vec<u8> {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}

fn json(output: Output) -> Value {
    serde_json::from_slice(&success(output)).unwrap()
}

fn rejected(output: Output, reason: &str) {
    assert!(!output.status.success(), "unexpected success: {}", String::from_utf8_lossy(&output.stdout));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(reason),
        "expected {reason}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "refusal emitted an authority record");
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn report(status: &Value, phase: &str, epoch: u64) {
    assert_eq!(status["classification"], "exclusive_turn_reference_only");
    assert_eq!(status["local_phase"], phase);
    assert_eq!(status["reference_epoch"], epoch);
    assert_eq!(status["reference_fence"], epoch);
    assert_eq!(status["local_reference_mutations_allowed"], phase == "held");
    for flag in [
        "canonical_authority",
        "write_authority",
        "runtime_activation",
        "player_admission",
        "v1_canonical_world_created",
    ] {
        assert_eq!(status[flag], false, "{flag}");
    }
}

struct Fixture {
    temp: tempfile::TempDir,
    profiles: [PathBuf; 3],
    identities: [PeerIdentity; 3],
    world: String,
    initial_grant: String,
    initial: PathBuf,
    history: PathBuf,
}

impl Fixture {
    fn new_save_backed() -> (Self, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let profiles = std::array::from_fn(|i| temp.path().join(format!("device-{i}")));
        let identities =
            std::array::from_fn(|i| PeerIdentity::load_or_create(&DataPaths::from_root(&profiles[i])).unwrap());
        let source = offline_save(temp.path());
        let bundle = temp.path().join("initial-bundle");
        let organizer = temp.path().join("organizer");
        let captured = json(capture_save(&organizer, &source, &bundle).output().unwrap());
        let initial = bundle.join("manifest.postcard");
        let draft = temp.path().join("save-draft.json");
        let keys = [hex::encode(identities[0].public_key()), hex::encode(identities[1].public_key())];
        let draft_bytes = success(
            save_cmd(&organizer, "draft")
                .args([
                    "--device-key",
                    &keys[0],
                    "--device-key",
                    &keys[1],
                    "--initial-holder",
                    &identities[0].peer_id().to_string(),
                    "--bundle",
                    path(&bundle),
                ])
                .output()
                .unwrap(),
        );
        fs::write(&draft, &draft_bytes).unwrap();
        let genesis: ExclusiveTurnGenesisV1 = serde_json::from_slice(&draft_bytes).unwrap();
        assert!(genesis.save_backed());
        let world = genesis.world_id().unwrap().to_string();
        let initial_grant = genesis.initial_grant_hash().unwrap().to_string();
        assert_eq!(genesis.initial_checkpoint.reference_commitment.to_string(), captured["reference_commitment"]);
        rejected(
            invoke(
                &profiles[0],
                &[
                    "approve",
                    "--draft",
                    path(&draft),
                    "--expected-world",
                    &world,
                    "--checkpoint",
                    path(&initial),
                    "--confirm",
                ],
            ),
            "requires save approve",
        );
        let approve = |i: usize| {
            save_cmd(&profiles[i], "approve")
                .args(["--draft", path(&draft), "--expected-world", &world, "--bundle", path(&bundle), "--confirm"])
                .output()
                .unwrap()
        };
        let blob = bundle.join("blobs").join(
            Hash32::from_domain_bytes(b"swarmcraft/exclusive-turn/save-blob/v1\0", b"synthetic level.dat fixture")
                .to_string(),
        );
        let hidden = temp.path().join("temporarily-missing-blob");
        fs::rename(&blob, &hidden).unwrap();
        rejected(approve(0), "missing blob");
        fs::rename(&hidden, &blob).unwrap();
        rejected(approve(2), "not a founding device");
        let approvals: [PathBuf; 2] = std::array::from_fn(|i| {
            let file = temp.path().join(format!("save-approval-{i}.json"));
            fs::write(&file, success(approve(i))).unwrap();
            file
        });
        let history = temp.path().join("save-history.json");
        fs::write(
            &history,
            success(invoke(
                &organizer,
                &[
                    "assemble",
                    "--draft",
                    path(&draft),
                    "--expected-world",
                    &world,
                    "--approval",
                    path(&approvals[0]),
                    "--approval",
                    path(&approvals[1]),
                ],
            )),
        )
        .unwrap();
        let inspected = json(invoke(&organizer, &["inspect", "--history", path(&history), "--expected-world", &world]));
        assert_eq!(inspected["save_backed_reference"], true);
        assert_eq!(inspected["local_save_data_verified"], false);
        assert!(!organizer.exists());
        let f = Self { temp, profiles, identities, world, initial_grant, initial, history };
        rejected(
            f.mutate(0, "init", &f.initial_grant, &["--history", path(&f.history), "--checkpoint", path(&f.initial)]),
            "requires save init",
        );
        assert!(!f.store(0).exists());
        for i in 0..2 {
            let result = json(
                save_cmd(&f.profiles[i], "init")
                    .args([
                        "--expected-world",
                        &f.world,
                        "--expected-grant",
                        &f.initial_grant,
                        "--confirm",
                        "--history",
                        path(&f.history),
                        "--bundle",
                        path(&bundle),
                    ])
                    .output()
                    .unwrap(),
            );
            report(&result, if i == 0 { "held" } else { "replica" }, 1);
            assert_eq!(result["save_backed_reference"], true);
            assert_eq!(result["local_save_data_verified"], true);
        }
        (f, source, bundle)
    }

    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let profiles = std::array::from_fn(|i| temp.path().join(format!("device-{i}")));
        let identities =
            std::array::from_fn(|i| PeerIdentity::load_or_create(&DataPaths::from_root(&profiles[i])).unwrap());
        let initial = temp.path().join("genesis.bin");
        fs::write(&initial, b"initial opaque reference bytes").unwrap();
        let draft = temp.path().join("draft.json");
        let organizer = temp.path().join("organizer");
        let initial_holder = identities[0].peer_id().to_string();
        let keys = [hex::encode(identities[0].public_key()), hex::encode(identities[1].public_key())];
        let draft_bytes = success(invoke(
            &organizer,
            &[
                "draft",
                "--device-key",
                &keys[0],
                "--device-key",
                &keys[1],
                "--initial-holder",
                &initial_holder,
                "--compatibility",
                &Hash32([3; 32]).to_string(),
                "--checkpoint",
                path(&initial),
                "--manifest-hash",
                &Hash32([4; 32]).to_string(),
                "--state-root",
                &Hash32([5; 32]).to_string(),
            ],
        ));
        assert!(!organizer.exists());
        fs::write(&draft, &draft_bytes).unwrap();
        let genesis: ExclusiveTurnGenesisV1 = serde_json::from_slice(&draft_bytes).unwrap();
        let world = genesis.world_id().unwrap().to_string();
        let initial_grant = genesis.initial_grant_hash().unwrap().to_string();
        let approve_args = [
            "approve",
            "--draft",
            path(&draft),
            "--expected-world",
            &world,
            "--checkpoint",
            path(&initial),
            "--confirm",
        ];
        rejected(invoke(&profiles[2], &approve_args), "not a founding device");
        rejected(invoke(&profiles[0], &approve_args[..approve_args.len() - 1]), "--confirm required");
        let approvals: [PathBuf; 2] = std::array::from_fn(|i| {
            let file = temp.path().join(format!("approval-{i}.json"));
            fs::write(&file, success(invoke(&profiles[i], &approve_args))).unwrap();
            file
        });
        rejected(
            invoke(
                &organizer,
                &[
                    "assemble",
                    "--draft",
                    path(&draft),
                    "--expected-world",
                    &world,
                    "--approval",
                    path(&approvals[0]),
                    "--approval",
                    path(&approvals[0]),
                ],
            ),
            "both fixed devices",
        );
        let history = temp.path().join("history.json");
        fs::write(
            &history,
            success(invoke(
                &organizer,
                &[
                    "assemble",
                    "--draft",
                    path(&draft),
                    "--expected-world",
                    &world,
                    "--approval",
                    path(&approvals[1]),
                    "--approval",
                    path(&approvals[0]),
                ],
            )),
        )
        .unwrap();
        assert!(!organizer.exists());
        let f = Self { temp, profiles, identities, world, initial_grant, initial, history };
        rejected(
            f.mutate(2, "init", &f.initial_grant, &["--history", path(&f.history), "--checkpoint", path(&f.initial)]),
            "not one of the two fixed devices",
        );
        assert!(!f.store(2).exists());
        for i in 0..2 {
            report(
                &json(f.mutate(
                    i,
                    "init",
                    &f.initial_grant,
                    &["--history", path(&f.history), "--checkpoint", path(&f.initial)],
                )),
                if i == 0 { "held" } else { "replica" },
                1,
            );
        }
        f
    }

    fn store(&self, i: usize) -> PathBuf {
        self.profiles[i].join("exclusive-turn-reference").join(self.world.strip_prefix("scpass:").unwrap())
    }

    fn journal(&self, i: usize) -> PathBuf {
        self.store(i).join("journal.postcard")
    }

    fn state(&self, i: usize, commitment: Hash32) -> PathBuf {
        self.store(i).join("checkpoints").join(format!("{commitment}.bin"))
    }

    fn managed_bundle(&self, i: usize, commitment: &str) -> PathBuf {
        self.store(i).join("save-bundles").join(commitment)
    }

    fn snapshot(&self, i: usize) -> Vec<u8> {
        fs::read(self.journal(i)).unwrap()
    }

    fn mutation(&self, i: usize, action: &str, grant: &str, args: &[&str]) -> Command {
        let mut command =
            cmd(&self.profiles[i], &[action, "--expected-world", &self.world, "--expected-grant", grant, "--confirm"]);
        command.args(args);
        command
    }

    fn mutate(&self, i: usize, action: &str, grant: &str, args: &[&str]) -> Output {
        self.mutation(i, action, grant, args).output().unwrap()
    }

    fn status(&self, i: usize) -> Value {
        json(invoke(&self.profiles[i], &["status", "--expected-world", &self.world]))
    }

    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let file = self.temp.path().join(name);
        fs::write(&file, bytes).unwrap();
        file
    }

    fn checkpoint(&self, i: usize, grant: &str, old: &str, file: &Path) -> Value {
        json(self.mutate(
            i,
            "checkpoint",
            grant,
            &[
                "--expected-checkpoint",
                old,
                "--checkpoint",
                path(file),
                "--manifest-hash",
                &Hash32([21; 32]).to_string(),
                "--state-root",
                &Hash32([22; 32]).to_string(),
            ],
        ))
    }

    fn prepare(&self, i: usize, grant: &str, commitment: &str) -> (ExclusiveTurnSignedOfferV1, Vec<u8>) {
        let bytes = success(self.mutate(i, "prepare", grant, &["--expected-checkpoint", commitment]));
        (serde_json::from_slice(&bytes).unwrap(), bytes)
    }

    fn stage(&self, i: usize, grant: &str, offer: &Path, hash: &str, checkpoint: &Path) -> Vec<u8> {
        success(self.mutate(
            i,
            "stage",
            grant,
            &["--offer", path(offer), "--expected-offer", hash, "--checkpoint", path(checkpoint)],
        ))
    }

    fn export(&self, i: usize, grant: &str) -> Vec<u8> {
        success(invoke(
            &self.profiles[i],
            &["export-release", "--expected-world", &self.world, "--expected-grant", grant],
        ))
    }

    fn assert_no_live_storage(&self) {
        for profile in &self.profiles {
            assert_eq!(fs::read_dir(profile.join("worlds")).unwrap().count(), 0);
            for absent in ["runtime", "control", "ownerless-authority", "host-readiness"] {
                assert!(!profile.join(absent).exists(), "unexpected live path {absent}");
            }
        }
    }
}

#[test]
fn real_process_round_trip_cancellation_stale_receipts_and_lost_commit_output() {
    let f = Fixture::new();
    let initial = exclusive_turn_reference_commitment(&fs::read(&f.initial).unwrap()).unwrap().to_string();
    let next_file = f.put("next.bin", b"new reference checkpoint, not Minecraft");
    let next = exclusive_turn_reference_commitment(&fs::read(&next_file).unwrap()).unwrap().to_string();
    report(&f.checkpoint(0, &f.initial_grant, &initial, &next_file), "held", 1);
    rejected(f.mutate(1, "prepare", &f.initial_grant, &["--expected-checkpoint", &initial]), "not held");
    let (first, first_bytes) = f.prepare(0, &f.initial_grant, &next);
    let first_hash = first.offer_hash().unwrap().to_string();
    assert_eq!(first.offer.attempt, 1);
    let first_file = f.put("first-offer.json", &first_bytes);
    assert_eq!(f.prepare(0, &f.initial_grant, &next).1, first_bytes);
    report(&f.status(0), "prepared", 1);
    rejected(
        f.mutate(
            0,
            "checkpoint",
            &f.initial_grant,
            &[
                "--expected-checkpoint",
                &next,
                "--checkpoint",
                path(&next_file),
                "--manifest-hash",
                &Hash32([21; 32]).to_string(),
                "--state-root",
                &Hash32([22; 32]).to_string(),
            ],
        ),
        "not held",
    );
    let before = f.snapshot(1);
    rejected(
        f.mutate(
            1,
            "stage",
            &f.initial_grant,
            &["--offer", path(&first_file), "--expected-offer", &first_hash, "--checkpoint", path(&f.initial)],
        ),
        "checkpoint bytes differ",
    );
    assert_eq!(f.snapshot(1), before);
    let first_receipt = f.stage(1, &f.initial_grant, &first_file, &first_hash, &next_file);
    let first_ack = f.put("first-receipt.json", &first_receipt);
    assert_eq!(f.stage(1, &f.initial_grant, &first_file, &first_hash, &next_file), first_receipt);
    report(&f.status(1), "replica", 1);
    report(&json(f.mutate(0, "cancel", &f.initial_grant, &["--expected-offer", &first_hash])), "held", 1);
    rejected(
        f.mutate(0, "commit", &f.initial_grant, &["--receipt", path(&first_ack), "--expected-offer", &first_hash]),
        "no prepared",
    );
    let (second, second_bytes) = f.prepare(0, &f.initial_grant, &next);
    assert_eq!(second.offer.attempt, 2);
    let second_hash = second.offer_hash().unwrap().to_string();
    let second_file = f.put("second-offer.json", &second_bytes);
    let second_receipt = f.stage(1, &f.initial_grant, &second_file, &second_hash, &next_file);
    let second_ack = f.put("second-receipt.json", &second_receipt);
    let before = f.snapshot(1);
    rejected(
        f.mutate(
            1,
            "stage",
            &f.initial_grant,
            &["--offer", path(&first_file), "--expected-offer", &first_hash, "--checkpoint", path(&next_file)],
        ),
        "stale or conflicts",
    );
    assert_eq!(f.snapshot(1), before);
    rejected(
        f.mutate(0, "commit", &f.initial_grant, &["--receipt", path(&first_ack), "--expected-offer", &second_hash]),
        "receipt does not acknowledge",
    );
    // A receipt without a committed release cannot activate the recipient.
    let not_committed = ExclusiveTurnReleaseV1 {
        offer: second.clone(),
        receipt: serde_json::from_slice(&second_receipt).unwrap(),
        signature: vec![0; 64],
    };
    let bogus = f.put("unsigned-release.json", &serde_json::to_vec(&not_committed).unwrap());
    rejected(
        f.mutate(
            1,
            "accept",
            &f.initial_grant,
            &["--release", path(&bogus), "--expected-release", &not_committed.grant_hash().unwrap().to_string()],
        ),
        "signature invalid",
    );
    // Lose the commit output itself: close the receiving pipe before the
    // source writes. The release must already be durable and retryable.
    let mut child = f
        .mutation(0, "commit", &f.initial_grant, &["--receipt", path(&second_ack), "--expected-offer", &second_hash])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let lost = child.wait_with_output().unwrap();
    assert!(!lost.status.success(), "broken output pipe must be observable");
    let source = f.status(0);
    report(&source, "released", 2);
    report(&f.status(1), "replica", 1);
    let grant = source["grant"].as_str().unwrap().to_owned();
    let release = f.export(0, &grant);
    assert_eq!(f.export(0, &grant), release);
    let released: ExclusiveTurnReleaseV1 = serde_json::from_slice(&release).unwrap();
    assert_eq!(released.offer, second);
    let release_file = f.put("committed-release.json", &release);
    rejected(f.mutate(0, "cancel", &grant, &["--expected-offer", &second_hash]), "no cancelable");
    rejected(f.mutate(0, "prepare", &grant, &["--expected-checkpoint", &next]), "not held");
    let before = f.snapshot(0);
    rejected(
        f.mutate(0, "accept", &f.initial_grant, &["--release", path(&release_file), "--expected-release", &grant]),
        "cannot restore",
    );
    assert_eq!(f.snapshot(0), before);
    let accepted = json(f.mutate(
        1,
        "accept",
        &f.initial_grant,
        &["--release", path(&release_file), "--expected-release", &grant],
    ));
    report(&accepted, "held", 2);
    assert_eq!(accepted["grant"], grant);
    let before = f.snapshot(1);
    assert_eq!(
        json(f.mutate(
            1,
            "accept",
            &f.initial_grant,
            &["--release", path(&release_file), "--expected-release", &grant]
        )),
        accepted
    );
    assert_eq!(f.snapshot(1), before);

    let return_file = f.put("return.bin", b"second holder's saved reference bytes");
    let returned = exclusive_turn_reference_commitment(&fs::read(&return_file).unwrap()).unwrap().to_string();
    report(&f.checkpoint(1, &grant, &next, &return_file), "held", 2);
    rejected(
        f.mutate(1, "accept", &f.initial_grant, &["--release", path(&release_file), "--expected-release", &grant]),
        "cannot restore",
    );
    let (reverse, reverse_bytes) = f.prepare(1, &grant, &returned);
    assert_eq!(reverse.offer.attempt, 1);
    let reverse_hash = reverse.offer_hash().unwrap().to_string();
    let reverse_file = f.put("reverse-offer.json", &reverse_bytes);
    let reverse_receipt = f.stage(0, &grant, &reverse_file, &reverse_hash, &return_file);
    let reverse_ack = f.put("reverse-receipt.json", &reverse_receipt);
    let reverse_release =
        success(f.mutate(1, "commit", &grant, &["--receipt", path(&reverse_ack), "--expected-offer", &reverse_hash]));
    let reverse_record: ExclusiveTurnReleaseV1 = serde_json::from_slice(&reverse_release).unwrap();
    let reverse_grant = reverse_record.grant_hash().unwrap().to_string();
    let reverse_release_file = f.put("reverse-release.json", &reverse_release);
    report(&f.status(1), "released", 3);
    report(
        &json(f.mutate(
            0,
            "accept",
            &grant,
            &["--release", path(&reverse_release_file), "--expected-release", &reverse_grant],
        )),
        "held",
        3,
    );
    let (third, _) = f.prepare(0, &reverse_grant, &returned);
    assert_eq!(third.offer.attempt, 3, "canceled attempts survive reverse transfer");
    rejected(
        invoke(&f.profiles[0], &["export-release", "--expected-world", &f.world, "--expected-grant", &reverse_grant]),
        "no still-current",
    );
    rejected(
        f.mutate(1, "accept", &f.initial_grant, &["--release", path(&release_file), "--expected-release", &grant]),
        "grant differs",
    );
    f.assert_no_live_storage();
}

#[test]
fn private_lock_corruption_and_missing_staged_bytes_refuse_without_reinitializing() {
    let f = Fixture::new();
    let commitment = exclusive_turn_reference_commitment(&fs::read(&f.initial).unwrap()).unwrap();
    let lock = File::open(f.store(0).join("journal.lock")).unwrap();
    FileExt::lock_exclusive(&lock).unwrap();
    let before = f.snapshot(0);
    rejected(
        f.mutate(0, "prepare", &f.initial_grant, &["--expected-checkpoint", &commitment.to_string()]),
        "journal busy",
    );
    assert_eq!(f.snapshot(0), before);
    drop(lock);
    let (offer, offer_bytes) = f.prepare(0, &f.initial_grant, &commitment.to_string());
    let hash = offer.offer_hash().unwrap().to_string();
    let offer_file = f.put("offer.json", &offer_bytes);
    let receipt = f.stage(1, &f.initial_grant, &offer_file, &hash, &f.initial);
    let receipt_file = f.put("receipt.json", &receipt);
    let release = success(f.mutate(
        0,
        "commit",
        &f.initial_grant,
        &["--receipt", path(&receipt_file), "--expected-offer", &hash],
    ));
    let record: ExclusiveTurnReleaseV1 = serde_json::from_slice(&release).unwrap();
    let grant = record.grant_hash().unwrap().to_string();
    let release_file = f.put("release.json", &release);
    let bytes = fs::read(f.state(1, commitment)).unwrap();
    let before = f.snapshot(1);
    let state = f.state(1, commitment);
    fs::write(&state, b"changed bytes").unwrap();
    rejected(
        f.mutate(1, "accept", &f.initial_grant, &["--release", path(&release_file), "--expected-release", &grant]),
        "checkpoint bytes differ",
    );
    assert_eq!(f.snapshot(1), before);
    fs::remove_file(&state).unwrap();
    symlink(&f.initial, &state).unwrap();
    rejected(invoke(&f.profiles[1], &["status", "--expected-world", &f.world]), "unsafe exclusive-turn path");
    fs::remove_file(&state).unwrap();
    fs::write(&state, &bytes).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = f.temp.path().join("hardlink.bin");
    fs::hard_link(&state, &alias).unwrap();
    rejected(invoke(&f.profiles[1], &["status", "--expected-world", &f.world]), "link count");
    fs::remove_file(alias).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o644)).unwrap();
    rejected(invoke(&f.profiles[1], &["status", "--expected-world", &f.world]), "owner-only");
    fs::set_permissions(&state, fs::Permissions::from_mode(0o600)).unwrap();
    let journal = f.journal(1);
    let mut trailing = before.clone();
    trailing.push(0);
    fs::write(&journal, trailing).unwrap();
    rejected(invoke(&f.profiles[1], &["status", "--expected-world", &f.world]), "noncanonical");
    let mut corrupt = before.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    fs::write(&journal, corrupt).unwrap();
    rejected(invoke(&f.profiles[1], &["status", "--expected-world", &f.world]), "signature invalid");
    fs::write(&journal, &before).unwrap();
    report(
        &json(f.mutate(
            1,
            "accept",
            &f.initial_grant,
            &["--release", path(&release_file), "--expected-release", &grant],
        )),
        "held",
        2,
    );
    fs::remove_file(f.journal(0)).unwrap();
    rejected(
        f.mutate(0, "init", &f.initial_grant, &["--history", path(&f.history), "--checkpoint", path(&f.initial)]),
        "already exists",
    );
    assert!(!f.journal(0).exists());
    f.assert_no_live_storage();
}

#[test]
fn concurrent_cancel_and_commit_cannot_both_succeed() {
    let f = Fixture::new();
    let initial = exclusive_turn_reference_commitment(&fs::read(&f.initial).unwrap()).unwrap().to_string();
    let (offer, bytes) = f.prepare(0, &f.initial_grant, &initial);
    let hash = offer.offer_hash().unwrap().to_string();
    let offer_file = f.put("offer.json", &bytes);
    let receipt = f.stage(1, &f.initial_grant, &offer_file, &hash, &f.initial);
    let receipt_file = f.put("receipt.json", &receipt);
    let launch = |mut command: Command| command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let commit = launch(f.mutation(
        0,
        "commit",
        &f.initial_grant,
        &["--receipt", path(&receipt_file), "--expected-offer", &hash],
    ));
    let cancel = launch(f.mutation(0, "cancel", &f.initial_grant, &["--expected-offer", &hash]));
    let commit = commit.wait_with_output().unwrap();
    let cancel = cancel.wait_with_output().unwrap();
    assert_ne!(commit.status.success(), cancel.status.success(), "exactly one durable transition may succeed");
    let status = f.status(0);
    if commit.status.success() {
        report(&status, "released", 2);
        assert_eq!(f.export(0, status["grant"].as_str().unwrap()), commit.stdout);
    } else {
        report(&status, "held", 1);
        let (next, _) = f.prepare(0, &f.initial_grant, &initial);
        assert_eq!(next.offer.attempt, 2);
    }
    report(&f.status(1), "replica", 1);
    assert_eq!(f.status(1)["reference_holder"], f.identities[0].peer_id().to_string());
    f.assert_no_live_storage();
}

// Own only these disposable child processes; kill/reap on an assertion panic.
struct Server(Option<Child>);

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Server {
    fn finish(mut self) -> Output {
        self.0.take().unwrap().wait_with_output().unwrap()
    }

    fn kill(mut self) -> Output {
        let mut child = self.0.take().unwrap();
        child.kill().unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Fixture {
    fn network(&self, i: usize, action: &str, base: &str, offer: &str) -> Command {
        cmd(
            &self.profiles[i],
            &[
                "network",
                action,
                "--expected-world",
                &self.world,
                "--expected-grant",
                base,
                "--expected-offer",
                offer,
                "--confirm",
            ],
        )
    }

    fn serve(&self, target: usize, base: &str, offer: &str, approved: &Path) -> (Server, SocketAddr) {
        let source = 1 - target;
        let child = self
            .network(target, "serve-handoff", base, offer)
            .args([
                "--offer",
                path(approved),
                "--from",
                &self.identities[source].peer_id().to_string(),
                "--bind",
                "127.0.0.1:0",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut server = Server(Some(child));
        let mut line = String::new();
        BufReader::new(server.0.as_mut().unwrap().stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "listener did not announce readiness: {:?}", server.0.as_mut().unwrap().try_wait());
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["classification"], "exclusive_turn_reference_handoff_listener");
        assert_eq!(ready["world_id"], self.world);
        assert_eq!(ready["base"], base);
        assert_eq!(ready["offer"], offer);
        assert_eq!(ready["recipient"], self.identities[target].peer_id().to_string());
        assert_eq!(ready["runtime_activation"], false);
        let address: SocketAddr = ready["address"].as_str().unwrap().parse().unwrap();
        assert!(address.ip().is_loopback());
        (server, address)
    }

    fn send_handoff(&self, source: usize, base: &str, offer: &str, address: SocketAddr) -> Output {
        self.network(source, "handoff", base, offer)
            .args(["--peer", &format!("{}@{address}", self.identities[1 - source].peer_id())])
            .output()
            .unwrap()
    }
}

#[derive(Clone, Copy)]
enum WireFault {
    /// Flip the encrypted checkpoint frame; no valid stage receipt exists.
    CorruptCheckpoint,
    /// The target has staged the checkpoint, but the receipt never reaches source.
    LoseReceipt,
    /// Source has committed its release, but the target receives none of it.
    LoseRelease,
    /// Target has durably accepted, but source receives no final acknowledgment.
    LoseAcceptance,
}

fn frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut len = [0; 4];
    stream.read_exact(&mut len).unwrap();
    let size = u32::from_be_bytes(len) as usize;
    assert!((1..=32784).contains(&size), "bounded encrypted frame");
    let mut result = vec![0; size + 4];
    result[..4].copy_from_slice(&len);
    stream.read_exact(&mut result[4..]).unwrap();
    result
}

fn forward(from: &mut TcpStream, to: &mut TcpStream) {
    to.write_all(&frame(from)).unwrap();
}

// This proxy knows ONLY the bounded encrypted frame schedule. It never has
// either private key, plaintext or Noise session state, and does not inject
// any test hook into production code. Both sides are real TCP CLI processes.
fn fault_proxy(target: SocketAddr, fault: WireFault) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut source = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "proxy client did not connect");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        let mut dest = TcpStream::connect_timeout(&target, Duration::from_secs(5)).unwrap();
        for stream in [&source, &dest] {
            stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(15))).unwrap();
            stream.set_nodelay(true).unwrap();
        }
        forward(&mut source, &mut dest); // initiator Noise handshake
        forward(&mut dest, &mut source); // responder Noise handshake
        forward(&mut dest, &mut source); // encrypted responder authentication
        forward(&mut source, &mut dest); // encrypted initiator authentication
        forward(&mut source, &mut dest); // offer length
        forward(&mut source, &mut dest); // signed offer
        forward(&mut source, &mut dest); // checkpoint length
        let mut checkpoint = frame(&mut source); // one small checkpoint chunk
        assert!(!checkpoint.windows(b"opaque reference bytes".len()).any(|w| w == b"opaque reference bytes"));
        if matches!(fault, WireFault::CorruptCheckpoint) {
            *checkpoint.last_mut().unwrap() ^= 1;
            dest.write_all(&checkpoint).unwrap();
        } else {
            dest.write_all(&checkpoint).unwrap();
            let receipt_length = frame(&mut dest);
            let receipt = frame(&mut dest); // target has persisted receipt
            if !matches!(fault, WireFault::LoseReceipt) {
                source.write_all(&receipt_length).unwrap();
                source.write_all(&receipt).unwrap();
                let release_length = frame(&mut source); // source has relinquished
                let release = frame(&mut source);
                if !matches!(fault, WireFault::LoseRelease) {
                    dest.write_all(&release_length).unwrap();
                    dest.write_all(&release).unwrap();
                    let _ack_length = frame(&mut dest); // recipient already accepted
                    let _ack = frame(&mut dest);
                    assert!(matches!(fault, WireFault::LoseAcceptance));
                }
            }
        }
        let _ = source.shutdown(Shutdown::Both);
        let _ = dest.shutdown(Shutdown::Both);
    });
    (address, handle)
}

fn confirmed(
    output: Output,
    f: &Fixture,
    offer: &ExclusiveTurnSignedOfferV1,
    grant: &str,
    epoch: u64,
    recipient: usize,
) -> Value {
    use ed25519_dalek::{Signature, VerifyingKey};
    let result = json(output);
    assert_eq!(result["classification"], "exclusive_turn_reference_handoff_confirmed");
    assert_eq!(result["world_id"], f.world);
    assert_eq!(result["base"], offer.offer.parent_grant.to_string());
    assert_eq!(result["offer"], offer.offer_hash().unwrap().to_string());
    assert_eq!(result["grant"], grant);
    assert_eq!(result["recipient"], f.identities[recipient].peer_id().to_string());
    assert_eq!(result["reference_epoch"], epoch);
    assert_eq!(result["reference_fence"], epoch);
    for flag in ["canonical_authority", "write_authority", "runtime_activation", "player_admission"] {
        assert_eq!(result[flag], false);
    }
    // Independently verify the actual recipient receipt's exact canonical
    // postcard input, not merely its presence or the source process exit.
    #[derive(serde::Serialize)]
    struct AckBody {
        world: ExclusiveTurnWorldIdV1,
        base: Hash32,
        offer: Hash32,
        grant: Hash32,
        recipient: PeerId,
        epoch: u64,
        fence: u64,
        checkpoint: ExclusiveTurnCheckpointV1,
    }
    let body = AckBody {
        world: offer.offer.world_id,
        base: offer.offer.parent_grant,
        offer: offer.offer_hash().unwrap(),
        grant: grant.parse().unwrap(),
        recipient: offer.offer.recipient,
        epoch,
        fence: epoch,
        checkpoint: offer.offer.checkpoint,
    };
    let expected = serde_json::to_value(&body).unwrap();
    assert_eq!(result["receipt"]["body"], expected);
    let mut bytes = b"swarmcraft/exclusive-turn/reference-durable-acceptance/v1\0".to_vec();
    bytes.extend(postcard::to_allocvec(&body).unwrap());
    let signature: Vec<u8> = serde_json::from_value(result["receipt"]["signature"].clone()).unwrap();
    VerifyingKey::from_bytes(&f.identities[recipient].public_key())
        .unwrap()
        .verify_strict(&bytes, &Signature::from_slice(&signature).unwrap())
        .unwrap();
    result
}

impl Fixture {
    fn writer(&self, device: usize, action: &str, grant: &str, checkpoint: &str) -> Command {
        cmd(
            &self.profiles[device],
            &[
                "writer",
                action,
                "--expected-world",
                &self.world,
                "--expected-grant",
                grant,
                "--expected-checkpoint",
                checkpoint,
                "--confirm",
            ],
        )
    }

    fn writer_serve(&self, holder: usize, grant: &str, checkpoint: &str, sessions: u32) -> Command {
        let mut command = self.writer(holder, "serve", grant, checkpoint);
        command.args([
            "--from",
            &self.identities[1 - holder].peer_id().to_string(),
            "--bind",
            "127.0.0.1:0",
            "--max-sessions",
            &sessions.to_string(),
            "--run-seconds",
            "60",
        ]);
        command
    }

    fn writer_listener(
        &self,
        holder: usize,
        grant: &str,
        checkpoint: &str,
        epoch: u64,
        sessions: u32,
    ) -> (Server, SocketAddr) {
        let child = self
            .writer_serve(holder, grant, checkpoint, sessions)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut server = Server(Some(child));
        let mut line = String::new();
        BufReader::new(server.0.as_mut().unwrap().stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "writer listener did not become ready: {:?}", server.0.as_mut().unwrap().try_wait());
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["classification"], "exclusive_turn_opaque_writer_listener");
        assert_eq!(ready["grant"], grant);
        assert_eq!(ready["world_id"], self.world);
        assert_eq!(ready["holder"], self.identities[holder].peer_id().to_string());
        assert_eq!(ready["client"], self.identities[1 - holder].peer_id().to_string());
        assert_eq!(ready["reference_epoch"], epoch);
        assert_eq!(ready["reference_fence"], epoch);
        assert_eq!(ready["writer_pid"], server.0.as_ref().unwrap().id());
        assert_eq!(ready["runtime_activation"], false);
        (server, ready["address"].as_str().unwrap().parse().unwrap())
    }

    fn writer_request(
        &self,
        client: usize,
        grant: &str,
        checkpoint: &str,
        address: SocketAddr,
        replacement: Option<&Path>,
    ) -> Output {
        let mut command =
            self.writer(client, if replacement.is_some() { "replace" } else { "observe" }, grant, checkpoint);
        command.args(["--peer", &format!("{}@{address}", self.identities[1 - client].peer_id())]);
        if let Some(file) = replacement {
            command.args(["--checkpoint", path(file)]);
        }
        command.output().unwrap()
    }
}

// Independently deserialize the exact signed receipt with a separate
// test-side schema and verify the real holder's Ed25519 signature.
fn writer_receipt(
    output: Output,
    f: &Fixture,
    holder: usize,
    grant: &str,
    epoch: u64,
    checkpoints: (Hash32, Hash32),
    changed: bool,
) -> [u8; 32] {
    let (previous, expected) = checkpoints;
    use ed25519_dalek::{Signature, VerifyingKey};
    #[derive(serde::Deserialize, serde::Serialize)]
    struct Scope {
        world: ExclusiveTurnWorldIdV1,
        grant: Hash32,
        epoch: u64,
        fence: u64,
        compatibility: Hash32,
        holder: PeerId,
        client: PeerId,
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    struct Request {
        scope: Scope,
        nonce: [u8; 32],
        expected_checkpoint: Hash32,
        replacement: Option<ExclusiveTurnCheckpointV1>,
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    struct Body {
        request: Request,
        checkpoint: ExclusiveTurnCheckpointV1,
    }
    let result = json(output);
    assert_eq!(result["classification"], "exclusive_turn_opaque_writer_receipt");
    assert_eq!(result["grant"], grant);
    assert_eq!(result["world_id"], f.world);
    assert_eq!(result["reference_epoch"], epoch);
    assert_eq!(result["reference_fence"], epoch);
    for flag in ["canonical_authority", "write_authority", "runtime_activation", "player_admission", "freshness_lease"]
    {
        assert_eq!(result[flag], false);
    }
    let body: Body = serde_json::from_value(result["receipt"]["body"].clone()).unwrap();
    assert_eq!(body.request.scope.world.to_string(), f.world);
    assert_eq!(body.request.scope.grant.to_string(), grant);
    assert_eq!(body.request.scope.epoch, epoch);
    assert_eq!(body.request.scope.fence, epoch);
    assert_eq!(body.request.scope.compatibility, Hash32([3; 32]));
    assert_eq!(body.request.scope.holder, f.identities[holder].peer_id());
    assert_eq!(body.request.scope.client, f.identities[1 - holder].peer_id());
    assert_eq!(body.request.expected_checkpoint, previous);
    assert_eq!(body.checkpoint.reference_commitment, expected);
    assert_eq!(body.request.replacement, changed.then_some(body.checkpoint));
    assert_ne!(body.request.nonce, [0; 32]);
    let mut signed = b"swarmcraft/exclusive-turn/opaque-writer-receipt/v1\0".to_vec();
    signed.extend(postcard::to_allocvec(&body).unwrap());
    let signature: Vec<u8> = serde_json::from_value(result["receipt"]["signature"].clone()).unwrap();
    VerifyingKey::from_bytes(&f.identities[holder].public_key())
        .unwrap()
        .verify_strict(&signed, &Signature::from_slice(&signature).unwrap())
        .unwrap();
    body.request.nonce
}

// Real encrypted exchange with a lost final reply. The proxy forwards
// the exact request and opaque replacement but withholds the receipt
// after the real writer has completed its durable publication.
fn lose_writer_reply(target: SocketAddr) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut source = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        let mut dest = TcpStream::connect_timeout(&target, Duration::from_secs(5)).unwrap();
        for stream in [&source, &dest] {
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        }
        forward(&mut source, &mut dest);
        forward(&mut dest, &mut source);
        forward(&mut dest, &mut source);
        forward(&mut source, &mut dest);
        for _ in 0..4 {
            forward(&mut source, &mut dest); // request and small checkpoint frames
        }
        let _length = frame(&mut dest);
        let _receipt = frame(&mut dest);
        let _ = source.shutdown(Shutdown::Both);
        let _ = dest.shutdown(Shutdown::Both);
    });
    (address, handle)
}

#[test]
fn real_writer_lock_route_and_crash_fence_survive_network_handoff() {
    use std::os::unix::process::ExitStatusExt;
    let f = Fixture::new();
    let initial = exclusive_turn_reference_commitment(&fs::read(&f.initial).unwrap()).unwrap();
    let next_bytes = b"durably replaced opaque state before handoff";
    let next_file = f.put("writer-next.bin", next_bytes);
    let next = exclusive_turn_reference_commitment(next_bytes).unwrap();
    let old = initial.to_string();
    let next_pin = next.to_string();

    // The replica cannot launch even though it has the identical
    // checkpoint. Exactly one real process may own the local writer.
    rejected(f.writer_serve(1, &f.initial_grant, &old, 1).output().unwrap(), "not held");
    let (server, address) = f.writer_listener(0, &f.initial_grant, &old, 1, 8);
    rejected(f.writer_serve(0, &f.initial_grant, &old, 1).output().unwrap(), "journal busy");
    rejected(f.mutate(0, "prepare", &f.initial_grant, &["--expected-checkpoint", &old]), "journal busy");
    rejected(
        f.mutate(
            0,
            "checkpoint",
            &f.initial_grant,
            &[
                "--expected-checkpoint",
                &old,
                "--checkpoint",
                path(&next_file),
                "--manifest-hash",
                &Hash32([4; 32]).to_string(),
                "--state-root",
                &Hash32([5; 32]).to_string(),
            ],
        ),
        "journal busy",
    );
    rejected(
        f.network(0, "handoff", &f.initial_grant, &Hash32([9; 32]).to_string())
            .args(["--peer", &format!("{}@127.0.0.1:1", f.identities[1].peer_id())])
            .output()
            .unwrap(),
        "journal busy",
    );
    let nonce1 = writer_receipt(
        f.writer_request(1, &f.initial_grant, &old, address, None),
        &f,
        0,
        &f.initial_grant,
        1,
        (initial, initial),
        false,
    );
    let nonce2 = writer_receipt(
        f.writer_request(1, &f.initial_grant, &old, address, Some(&next_file)),
        &f,
        0,
        &f.initial_grant,
        1,
        (initial, next),
        true,
    );
    assert_ne!(nonce1, nonce2);
    assert_eq!(fs::read(f.state(0, next)).unwrap(), next_bytes);
    // Keep an actual TCP connection open to the old route, and complete
    // the first Noise handshake frame so the server has accepted it.
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut handshake =
        snow::Builder::new("Noise_NN_25519_ChaChaPoly_BLAKE2s".parse().unwrap()).build_initiator().unwrap();
    let mut output = [0; 128];
    let len = handshake.write_message(&[], &mut output).unwrap();
    stream.write_all(&(len as u32).to_be_bytes()).unwrap();
    stream.write_all(&output[..len]).unwrap();
    let _response = frame(&mut stream);
    // Hard-kill the process while its old socket is open. There is no
    // detached process or inherited lock that could keep writing.
    let killed = server.kill();
    assert_eq!(killed.status.signal(), Some(9));
    let mut drained = [0; 4096];
    loop {
        match stream.read(&mut drained) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(error)
                if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe) =>
            {
                break
            }
            Err(error) => panic!("old route did not close: {error}"),
        }
    }
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_err());
    let held = f.status(0);
    report(&held, "held", 1);
    let cp: ExclusiveTurnCheckpointV1 = serde_json::from_value(held["local_checkpoint"].clone()).unwrap();
    assert_eq!(cp.reference_commitment, next);
    assert_eq!(
        cp.manifest_hash,
        Hash32::from_domain_bytes(b"swarmcraft/exclusive-turn/opaque-writer-manifest/v1\0", next_bytes)
    );
    assert_eq!(
        cp.state_root,
        Hash32::from_domain_bytes(b"swarmcraft/exclusive-turn/opaque-writer-state/v1\0", next_bytes)
    );
    let (offer, bytes) = f.prepare(0, &f.initial_grant, &next_pin);
    assert_eq!(offer.offer.checkpoint, cp);
    let hash = offer.offer_hash().unwrap().to_string();
    let file = f.put("writer-offer.json", &bytes);
    rejected(f.writer_serve(0, &f.initial_grant, &next_pin, 1).output().unwrap(), "not held");
    rejected(f.writer_serve(1, &f.initial_grant, &old, 1).output().unwrap(), "not held");
    // Partition after release, before acceptance: there is no eligible
    // reference writer on either device until the exact outbox arrives.
    let (server, target) = f.serve(1, &f.initial_grant, &hash, &file);
    let (proxy_address, proxy) = fault_proxy(target, WireFault::LoseRelease);
    rejected(f.send_handoff(0, &f.initial_grant, &hash, proxy_address), "source relinquished");
    proxy.join().unwrap();
    assert!(!server.finish().status.success());
    let released = f.status(0);
    report(&released, "released", 2);
    report(&f.status(1), "replica", 1);
    let grant = released["grant"].as_str().unwrap();
    rejected(f.writer_serve(0, grant, &next_pin, 1).output().unwrap(), "not held");
    rejected(f.writer_serve(0, &f.initial_grant, &next_pin, 1).output().unwrap(), "grant differs");
    rejected(f.writer_serve(1, &f.initial_grant, &old, 1).output().unwrap(), "not held");
    let (server, target) = f.serve(1, &f.initial_grant, &hash, &file);
    confirmed(f.send_handoff(0, &f.initial_grant, &hash, target), &f, &offer, grant, 2, 1);
    success(server.finish());
    assert_eq!(fs::read(f.state(1, next)).unwrap(), next_bytes);
    let (server, new_address) = f.writer_listener(1, grant, &next_pin, 2, 1);
    rejected(f.writer_serve(0, grant, &next_pin, 1).output().unwrap(), "not held");
    writer_receipt(f.writer_request(0, grant, &next_pin, new_address, None), &f, 1, grant, 2, (next, next), false);
    success(server.finish());
    assert!(TcpStream::connect_timeout(&new_address, Duration::from_millis(500)).is_err());
    f.assert_no_live_storage();
}

#[test]
fn writer_lost_ack_keeps_exact_durable_checkpoint_and_rejects_stale_cas() {
    let f = Fixture::new();
    let initial = exclusive_turn_reference_commitment(&fs::read(&f.initial).unwrap()).unwrap();
    let bytes = b"opaque state retained after lost writer acknowledgment";
    let replacement = f.put("writer-replace.bin", bytes);
    let next = exclusive_turn_reference_commitment(bytes).unwrap();
    let old = initial.to_string();
    let new = next.to_string();
    let (server, address) = f.writer_listener(0, &f.initial_grant, &old, 1, 1);
    let (proxy_address, proxy) = lose_writer_reply(address);
    let output = f.writer_request(1, &f.initial_grant, &old, proxy_address, Some(&replacement));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    proxy.join().unwrap();
    success(server.finish());
    let before = f.snapshot(0);
    assert_eq!(fs::read(f.state(0, next)).unwrap(), bytes);
    rejected(f.writer_serve(0, &f.initial_grant, &old, 1).output().unwrap(), "checkpoint differs");
    let (server, address) = f.writer_listener(0, &f.initial_grant, &new, 1, 1);
    let stale = f.writer_request(1, &f.initial_grant, &old, address, Some(&f.initial));
    assert!(!stale.status.success());
    assert!(stale.stdout.is_empty());
    rejected(server.finish(), "reference writer checkpoint differs");
    assert_eq!(f.snapshot(0), before);
    let (server, address) = f.writer_listener(0, &f.initial_grant, &new, 1, 2);
    let nonce1 = writer_receipt(
        f.writer_request(1, &f.initial_grant, &new, address, None),
        &f,
        0,
        &f.initial_grant,
        1,
        (next, next),
        false,
    );
    let nonce2 = writer_receipt(
        f.writer_request(1, &f.initial_grant, &new, address, None),
        &f,
        0,
        &f.initial_grant,
        1,
        (next, next),
        false,
    );
    assert_ne!(nonce1, nonce2);
    success(server.finish());
    assert_eq!(f.snapshot(0), before);
    f.assert_no_live_storage();
}

// Synthetic stopped save tree with actual bytes exceeding the old 1 MiB
// opaque-checkpoint cap. This is a data-integrity fixture, not a playable save.
fn offline_save(root: &Path) -> PathBuf {
    let source = root.join("offline-save");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("region")).unwrap();
    fs::create_dir(source.join("playerdata")).unwrap();
    fs::create_dir(source.join("DIM-1")).unwrap();
    fs::write(source.join("level.dat"), b"synthetic level.dat fixture").unwrap();
    fs::write(source.join("session.lock"), b"original session marker").unwrap();
    let region: Vec<u8> = (0..1_310_721).map(|i| ((i * 17 + i / 1009) % 251) as u8).collect();
    fs::write(source.join("region/r.0.0.mca"), &region).unwrap();
    fs::write(source.join("region/r.1.0.mca"), &region).unwrap();
    fs::write(source.join("playerdata/empty.dat"), []).unwrap();
    source
}

fn save_cmd(profile: &Path, action: &str) -> Command {
    cmd(profile, &["save", action])
}

fn capture_save(profile: &Path, source: &Path, destination: &Path) -> Command {
    let mut command = save_cmd(profile, "capture");
    command.args([
        "--source",
        path(source),
        "--destination",
        path(destination),
        "--compatibility",
        &Hash32([3; 32]).to_string(),
        "--confirm",
    ]);
    command
}

fn pinned_save(profile: &Path, action: &str, bundle: &Path, report: &Value) -> Command {
    let mut command = save_cmd(profile, action);
    command.args([
        "--bundle",
        path(bundle),
        "--expected-manifest",
        report["manifest_hash"].as_str().unwrap(),
        "--expected-state-root",
        report["state_root"].as_str().unwrap(),
        "--expected-compatibility",
        report["compatibility"].as_str().unwrap(),
    ]);
    command
}

fn assert_save_flags(report: &Value, classification: &str) {
    assert_eq!(report["classification"], classification);
    for flag in [
        "canonical_authority",
        "write_authority",
        "runtime_activation",
        "player_admission",
        "handoff_receipt_issued",
        "minecraft_gameplay_verified",
    ] {
        assert_eq!(report[flag], false, "{flag}");
    }
}

fn assert_same_save(source: &Path, restored: &Path) {
    for name in ["level.dat", "region/r.0.0.mca", "region/r.1.0.mca", "playerdata/empty.dat"] {
        assert_eq!(fs::read(source.join(name)).unwrap(), fs::read(restored.join(name)).unwrap());
    }
    assert!(restored.join("DIM-1").is_dir());
    assert_eq!(fs::read(restored.join("session.lock")).unwrap(), b"");
    assert_eq!(fs::read(source.join("session.lock")).unwrap(), b"original session marker");
}

#[test]
fn offline_save_exact_capture_restore_and_corruption_refusals() {
    use std::os::fd::AsRawFd;
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().join("never-created-profile");
    let source = offline_save(temp.path());
    let bundle = temp.path().join("bundle");
    let lock_file = std::fs::OpenOptions::new().read(true).write(true).open(source.join("session.lock")).unwrap();
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    // Real process-associated record lock held by the test process;
    // the capture CLI is a different process.
    assert_eq!(unsafe { libc::fcntl(lock_file.as_raw_fd(), libc::F_SETLK, &lock) }, 0);
    rejected(capture_save(&profile, &source, &bundle).output().unwrap(), "save is open");
    assert!(!bundle.exists());
    drop(lock_file);

    // A source alias is denied, not dereferenced into the archive.
    let alias = source.join("region/alias.mca");
    symlink(source.join("region/r.0.0.mca"), &alias).unwrap();
    rejected(capture_save(&profile, &source, &bundle).output().unwrap(), "unsafe save entry");
    fs::remove_file(&alias).unwrap();
    fs::hard_link(source.join("region/r.0.0.mca"), &alias).unwrap();
    rejected(capture_save(&profile, &source, &bundle).output().unwrap(), "unsafe save entry");
    fs::remove_file(&alias).unwrap();
    fs::write(source.join("region/R.0.0.mca"), b"case collision").unwrap();
    rejected(capture_save(&profile, &source, &bundle).output().unwrap(), "duplicate or noncanonical");
    fs::remove_file(source.join("region/R.0.0.mca")).unwrap();
    fs::write(source.join("CON.dat"), b"reserved").unwrap();
    rejected(capture_save(&profile, &source, &bundle).output().unwrap(), "reserved portable");
    fs::remove_file(source.join("CON.dat")).unwrap();
    let nested = source.join("inside");
    rejected(capture_save(&profile, &source, &nested).output().unwrap(), "inside source");
    assert!(!nested.exists());

    let captured = json(capture_save(&profile, &source, &bundle).output().unwrap());
    assert_save_flags(&captured, "exclusive_turn_save_data_captured");
    assert_eq!(captured["entries"], 7);
    assert_eq!(captured["unique_blobs"], 3);
    assert_eq!(captured["total_bytes"], 2 * 1_310_721 + b"synthetic level.dat fixture".len() as u64);
    let manifest = fs::read(bundle.join("manifest.postcard")).unwrap();
    assert_eq!(
        captured["manifest_hash"],
        Hash32::from_domain_bytes(b"swarmcraft/exclusive-turn/save-manifest/v1\0", &manifest).to_string()
    );
    assert_eq!(captured["reference_commitment"], exclusive_turn_reference_commitment(&manifest).unwrap().to_string());
    let verified = json(pinned_save(&profile, "verify", &bundle, &captured).output().unwrap());
    assert_save_flags(&verified, "exclusive_turn_save_data_verified");
    let second = temp.path().join("bundle-again");
    let again = json(capture_save(&profile, &source, &second).output().unwrap());
    assert_eq!(again["manifest_hash"], captured["manifest_hash"]);
    assert_eq!(fs::read(second.join("manifest.postcard")).unwrap(), manifest);
    rejected(capture_save(&profile, &source, &bundle).output().unwrap(), "already exists");

    let mut wrong = captured.clone();
    wrong["state_root"] = serde_json::json!(Hash32([7; 32]).to_string());
    rejected(pinned_save(&profile, "verify", &bundle, &wrong).output().unwrap(), "independent pin");
    let restored = temp.path().join("restored");
    let result = json(
        pinned_save(&profile, "restore", &bundle, &captured)
            .args(["--destination", path(&restored), "--confirm"])
            .output()
            .unwrap(),
    );
    assert_save_flags(&result, "exclusive_turn_save_data_restored");
    assert_same_save(&source, &restored);
    rejected(
        pinned_save(&profile, "restore", &bundle, &captured)
            .args(["--destination", path(&restored), "--confirm"])
            .output()
            .unwrap(),
        "already exists",
    );
    assert_same_save(&source, &restored);

    let large_blob = bundle.join("blobs").join(
        Hash32::from_domain_bytes(
            b"swarmcraft/exclusive-turn/save-blob/v1\0",
            &fs::read(source.join("region/r.0.0.mca")).unwrap(),
        )
        .to_string(),
    );
    let original = fs::read(&large_blob).unwrap();
    let mut changed = original.clone();
    changed[123_456] ^= 1;
    fs::write(&large_blob, &changed).unwrap();
    let failed = temp.path().join("corrupt-restore");
    rejected(
        pinned_save(&profile, "restore", &bundle, &captured)
            .args(["--destination", path(&failed), "--confirm"])
            .output()
            .unwrap(),
        "digest or length differs",
    );
    assert!(!failed.exists());
    fs::write(&large_blob, &original).unwrap();
    let extra = bundle.join("blobs").join(Hash32([99; 32]).to_string());
    fs::write(&extra, b"unexpected").unwrap();
    fs::set_permissions(&extra, fs::Permissions::from_mode(0o600)).unwrap();
    rejected(pinned_save(&profile, "verify", &bundle, &captured).output().unwrap(), "unexpected save blob");
    fs::remove_file(&extra).unwrap();
    let mut malformed = manifest.clone();
    malformed.push(0);
    fs::write(bundle.join("manifest.postcard"), malformed).unwrap();
    rejected(pinned_save(&profile, "verify", &bundle, &captured).output().unwrap(), "noncanonical");
    fs::write(bundle.join("manifest.postcard"), &manifest).unwrap();
    success(pinned_save(&profile, "verify", &bundle, &captured).output().unwrap());
    assert!(!profile.exists());
    assert!(!fs::read_dir(temp.path()).unwrap().any(|item| item
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".exclusive-save-")));
}

impl Fixture {
    fn save_transfer(&self, device: usize, action: &str, base: &str, offer: &str) -> Command {
        let mut command = save_cmd(&self.profiles[device], action);
        command.args([
            "--expected-world",
            &self.world,
            "--expected-grant",
            base,
            "--expected-offer",
            offer,
            "--confirm",
        ]);
        command
    }

    fn save_serve(&self, base: &str, offer: &str, bundle: &Path) -> (Server, SocketAddr) {
        self.save_listener(0, base, offer, Some(bundle))
    }

    fn save_listener(&self, source: usize, base: &str, offer: &str, bundle: Option<&Path>) -> (Server, SocketAddr) {
        let mut command = self.save_transfer(source, "serve", base, offer);
        if let Some(bundle) = bundle {
            command.args(["--bundle", path(bundle)]);
        }
        let child =
            command.args(["--bind", "127.0.0.1:0"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut server = Server(Some(child));
        let mut line = String::new();
        BufReader::new(server.0.as_mut().unwrap().stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "save listener not ready: {:?}", server.0.as_mut().unwrap().try_wait());
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["classification"], "exclusive_turn_save_listener");
        assert_eq!(ready["world_id"], self.world);
        assert_eq!(ready["base"], base);
        assert_eq!(ready["offer"], offer);
        assert_eq!(ready["handoff_receipt_issued"], false);
        (server, ready["address"].as_str().unwrap().parse().unwrap())
    }

    fn save_fetch(&self, base: &str, offer: &str, file: &Path, destination: &Path, address: SocketAddr) -> Output {
        self.fetch_save(1, base, offer, file, Some(destination), address)
    }

    fn fetch_save(
        &self,
        recipient: usize,
        base: &str,
        offer: &str,
        file: &Path,
        destination: Option<&Path>,
        address: SocketAddr,
    ) -> Output {
        let mut command = self.save_transfer(recipient, "fetch", base, offer);
        if let Some(destination) = destination {
            command.args(["--destination", path(destination)]);
        }
        command
            .args(["--offer", path(file), "--peer", &format!("{}@{address}", self.identities[1 - recipient].peer_id())])
            .output()
            .unwrap()
    }

    fn restore_held(&self, device: usize, grant: &str, commitment: &str, destination: &Path) -> Output {
        save_cmd(&self.profiles[device], "restore-held")
            .args([
                "--expected-world",
                &self.world,
                "--expected-grant",
                grant,
                "--expected-checkpoint",
                commitment,
                "--destination",
                path(destination),
                "--confirm",
            ])
            .output()
            .unwrap()
    }

    fn serve_checked_offer(&self, target: usize, grant: &str, hash: &str, offer: &Path) -> Output {
        self.network(target, "serve-handoff", grant, hash)
            .args([
                "--offer",
                path(offer),
                "--from",
                &self.identities[1 - target].peer_id().to_string(),
                "--bind",
                "127.0.0.1:0",
            ])
            .output()
            .unwrap()
    }
}

// Encrypted manifest arrives, then an authenticated blob frame is corrupted.
// Proxy does not possess either key or know any plaintext.
fn corrupt_save_blob(target: SocketAddr) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut client = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        let mut source = TcpStream::connect_timeout(&target, Duration::from_secs(5)).unwrap();
        for stream in [&client, &source] {
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        }
        forward(&mut client, &mut source);
        forward(&mut source, &mut client);
        forward(&mut source, &mut client);
        forward(&mut client, &mut source);
        forward(&mut source, &mut client); // manifest length
        forward(&mut source, &mut client); // small canonical manifest
        let mut blob = frame(&mut source);
        *blob.last_mut().unwrap() ^= 1;
        client.write_all(&blob).unwrap();
        let _ = client.shutdown(Shutdown::Both);
        let _ = source.shutdown(Shutdown::Both);
    });
    (address, handle)
}

#[test]
fn authenticated_save_blob_transfer_is_exact_and_never_releases_a_turn() {
    let f = Fixture::new();
    let source = offline_save(f.temp.path());
    let bundle = f.temp.path().join("save-bundle");
    let captured = json(capture_save(&f.profiles[0], &source, &bundle).output().unwrap());
    let initial = exclusive_turn_reference_commitment(&fs::read(&f.initial).unwrap()).unwrap().to_string();
    let manifest = bundle.join("manifest.postcard");
    let commitment = captured["reference_commitment"].as_str().unwrap();
    report(
        &json(f.mutate(
            0,
            "checkpoint",
            &f.initial_grant,
            &[
                "--expected-checkpoint",
                &initial,
                "--checkpoint",
                path(&manifest),
                "--manifest-hash",
                captured["manifest_hash"].as_str().unwrap(),
                "--state-root",
                captured["state_root"].as_str().unwrap(),
            ],
        )),
        "held",
        1,
    );
    let (offer, offer_bytes) = f.prepare(0, &f.initial_grant, commitment);
    let offer_hash = offer.offer_hash().unwrap().to_string();
    let file = f.put("save-offer.json", &offer_bytes);
    let before = [f.snapshot(0), f.snapshot(1)];
    let destination = f.temp.path().join("received");
    rejected(
        f.save_fetch(
            &f.initial_grant,
            &Hash32([88; 32]).to_string(),
            &file,
            &destination,
            "127.0.0.1:1".parse().unwrap(),
        ),
        "offer differs",
    );
    assert!(!destination.exists());
    let (server, target) = f.save_serve(&f.initial_grant, &offer_hash, &bundle);
    rejected(f.mutate(0, "cancel", &f.initial_grant, &["--expected-offer", &offer_hash]), "journal busy");
    let (proxy_address, proxy) = corrupt_save_blob(target);
    let failure = f.save_fetch(&f.initial_grant, &offer_hash, &file, &destination, proxy_address);
    assert!(!failure.status.success());
    assert!(failure.stdout.is_empty());
    proxy.join().unwrap();
    assert!(!server.finish().status.success());
    assert!(!destination.exists());
    assert_eq!([f.snapshot(0), f.snapshot(1)], before);
    assert!(!fs::read_dir(f.temp.path()).unwrap().any(|item| item
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".exclusive-save-")));
    let (server, target) = f.save_serve(&f.initial_grant, &offer_hash, &bundle);
    let received = json(f.save_fetch(&f.initial_grant, &offer_hash, &file, &destination, target));
    success(server.finish());
    assert_save_flags(&received, "exclusive_turn_save_data_received");
    for field in ["manifest_hash", "state_root", "reference_commitment", "compatibility", "total_bytes", "unique_blobs"]
    {
        assert_eq!(received[field], captured[field]);
    }
    assert_eq!(fs::read(destination.join("manifest.postcard")).unwrap(), fs::read(manifest).unwrap());
    assert_eq!([f.snapshot(0), f.snapshot(1)], before);
    report(&f.status(0), "prepared", 1);
    report(&f.status(1), "replica", 1);
    assert_eq!(f.status(1)["staged_offer"], Value::Null);
    let restored = f.temp.path().join("network-restored");
    let result = json(
        pinned_save(&f.profiles[1], "restore", &destination, &captured)
            .args(["--destination", path(&restored), "--confirm"])
            .output()
            .unwrap(),
    );
    assert_save_flags(&result, "exclusive_turn_save_data_restored");
    assert_same_save(&source, &restored);
    f.assert_no_live_storage();
}

#[test]
fn save_backed_release_requires_full_retention_before_and_after_commit() {
    let (f, source, original_bundle) = Fixture::new_save_backed();
    let initial = exclusive_turn_reference_commitment(&fs::read(&f.initial).unwrap()).unwrap().to_string();
    let output = f.temp.path().join("initial-restore");
    assert_save_flags(
        &json(f.restore_held(0, &f.initial_grant, &initial, &output)),
        "exclusive_turn_held_save_data_restored",
    );
    assert_same_save(&source, &output);
    rejected(f.restore_held(1, &f.initial_grant, &initial, &f.temp.path().join("not-holder")), "requires exact held");
    rejected(f.writer_serve(0, &f.initial_grant, &initial, 1).output().unwrap(), "opaque writer is unavailable");
    rejected(
        f.mutate(
            0,
            "checkpoint",
            &f.initial_grant,
            &[
                "--expected-checkpoint",
                &initial,
                "--checkpoint",
                path(&f.initial),
                "--manifest-hash",
                &Hash32([4; 32]).to_string(),
                "--state-root",
                &Hash32([5; 32]).to_string(),
            ],
        ),
        "requires save checkpoint",
    );

    // New checkpoint contains actual changed bytes, not just a manifest update.
    let changed_level = b"second synthetic level.dat checkpoint";
    fs::write(source.join("level.dat"), changed_level).unwrap();
    let next_bundle = f.temp.path().join("next-save-bundle");
    let captured = json(capture_save(&f.profiles[0], &source, &next_bundle).output().unwrap());
    let next = captured["reference_commitment"].as_str().unwrap().to_owned();
    assert_ne!(next, initial);
    let updated = json(
        save_cmd(&f.profiles[0], "checkpoint")
            .args([
                "--expected-world",
                &f.world,
                "--expected-grant",
                &f.initial_grant,
                "--expected-checkpoint",
                &initial,
                "--bundle",
                path(&next_bundle),
                "--confirm",
            ])
            .output()
            .unwrap(),
    );
    report(&updated, "held", 1);
    assert_eq!(updated["local_save_data_verified"], true);
    let (offer, bytes) = f.prepare(0, &f.initial_grant, &next);
    let hash = offer.offer_hash().unwrap().to_string();
    let file = f.put("save-backed-offer.json", &bytes);
    let before = [f.snapshot(0), f.snapshot(1)];
    let managed = f.managed_bundle(1, &next);
    assert!(!managed.exists());
    rejected(f.serve_checked_offer(1, &f.initial_grant, &hash, &file), "complete retained save bundle");
    rejected(
        f.mutate(
            1,
            "stage",
            &f.initial_grant,
            &[
                "--offer",
                path(&file),
                "--expected-offer",
                &hash,
                "--checkpoint",
                path(&next_bundle.join("manifest.postcard")),
            ],
        ),
        "requires authenticated network handoff",
    );
    rejected(
        f.mutate(0, "commit", &f.initial_grant, &["--receipt", path(&file), "--expected-offer", &hash]),
        "requires authenticated network handoff",
    );
    rejected(
        f.restore_held(0, &f.initial_grant, &next, &f.temp.path().join("prepared-restore")),
        "requires exact held",
    );
    assert_eq!([f.snapshot(0), f.snapshot(1)], before);

    // Managed fetch accepts no alternate location or source override.
    rejected(
        f.fetch_save(
            1,
            &f.initial_grant,
            &hash,
            &file,
            Some(&f.temp.path().join("unmanaged")),
            "127.0.0.1:1".parse().unwrap(),
        ),
        "uses only its managed retained bundle",
    );
    rejected(
        f.save_transfer(0, "serve", &f.initial_grant, &hash)
            .args(["--bundle", path(&next_bundle), "--bind", "127.0.0.1:0"])
            .output()
            .unwrap(),
        "uses only its managed retained bundle",
    );
    let (server, address) = f.save_listener(0, &f.initial_grant, &hash, None);
    let (proxy_address, proxy) = corrupt_save_blob(address);
    let failure = f.fetch_save(1, &f.initial_grant, &hash, &file, None, proxy_address);
    assert!(!failure.status.success() && failure.stdout.is_empty());
    proxy.join().unwrap();
    assert!(!server.finish().status.success());
    assert!(!managed.exists());
    assert_eq!([f.snapshot(0), f.snapshot(1)], before);
    // Fetch, and then exact retained-data retry; neither operation changes the journal.
    for _ in 0..2 {
        let (server, address) = f.save_listener(0, &f.initial_grant, &hash, None);
        let received = json(f.fetch_save(1, &f.initial_grant, &hash, &file, None, address));
        success(server.finish());
        assert_save_flags(&received, "exclusive_turn_save_data_received");
        assert_eq!(received["path"], serde_json::json!(managed));
        assert_eq!(received["reference_commitment"], next);
        assert_eq!([f.snapshot(0), f.snapshot(1)], before);
    }
    let blob_name = Hash32::from_domain_bytes(b"swarmcraft/exclusive-turn/save-blob/v1\0", changed_level).to_string();
    let target_blob = managed.join("blobs").join(&blob_name);
    let saved = f.temp.path().join("removed-target-blob");
    fs::rename(&target_blob, &saved).unwrap();
    rejected(f.serve_checked_offer(1, &f.initial_grant, &hash, &file), "missing blob");
    assert_eq!([f.snapshot(0), f.snapshot(1)], before);
    fs::rename(&saved, &target_blob).unwrap();

    // Drop the actual committed release on the encrypted TCP connection.
    // Source is permanently retired; recipient is staged but not held.
    let (server, address) = f.serve(1, &f.initial_grant, &hash, &file);
    let (proxy_address, proxy) = fault_proxy(address, WireFault::LoseRelease);
    rejected(f.send_handoff(0, &f.initial_grant, &hash, proxy_address), "source relinquished");
    proxy.join().unwrap();
    assert!(!server.finish().status.success());
    let released = f.status(0);
    report(&released, "released", 2);
    let grant = released["grant"].as_str().unwrap().to_owned();
    report(&f.status(1), "replica", 1);
    assert_eq!(f.status(1)["staged_offer"], hash);
    let staged = [f.snapshot(0), f.snapshot(1)];
    rejected(f.restore_held(0, &grant, &next, &f.temp.path().join("released-restore")), "requires exact held");
    rejected(
        f.restore_held(1, &f.initial_grant, &initial, &f.temp.path().join("staged-restore")),
        "requires exact held",
    );
    rejected(f.mutate(0, "cancel", &grant, &["--expected-offer", &hash]), "no cancelable");
    rejected(
        invoke(&f.profiles[0], &["export-release", "--expected-world", &f.world, "--expected-grant", &grant]),
        "requires exact authenticated network retry",
    );
    // Cold reload with missing staged bytes must fail, even after a valid
    // receipt was signed. Reinstating exact original bytes is test-only.
    fs::rename(&target_blob, &saved).unwrap();
    rejected(invoke(&f.profiles[1], &["status", "--expected-world", &f.world]), "missing blob");
    rejected(f.serve_checked_offer(1, &f.initial_grant, &hash, &file), "missing blob");
    assert_eq!([f.snapshot(0), f.snapshot(1)], staged);
    fs::rename(&saved, &target_blob).unwrap();
    let (server, address) = f.serve(1, &f.initial_grant, &hash, &file);
    confirmed(f.send_handoff(0, &f.initial_grant, &hash, address), &f, &offer, &grant, 2, 1);
    success(server.finish());
    let held = f.status(1);
    report(&held, "held", 2);
    assert_eq!(held["local_save_data_verified"], true);
    let committed = [f.snapshot(0), f.snapshot(1)];
    // Recheck the full bytes before an already-accepted retry or restore.
    fs::write(&target_blob, b"corrupted save blob").unwrap();
    rejected(f.serve_checked_offer(1, &f.initial_grant, &hash, &file), "digest or length differs");
    rejected(f.restore_held(1, &grant, &next, &f.temp.path().join("corrupt-held-restore")), "digest or length differs");
    assert_eq!([f.snapshot(0), f.snapshot(1)], committed);
    fs::write(&target_blob, changed_level).unwrap();
    let (server, address) = f.serve(1, &f.initial_grant, &hash, &file);
    confirmed(f.send_handoff(0, &f.initial_grant, &hash, address), &f, &offer, &grant, 2, 1);
    success(server.finish());
    assert_eq!([f.snapshot(0), f.snapshot(1)], committed);
    let restored = f.temp.path().join("recipient-restored");
    assert_save_flags(&json(f.restore_held(1, &grant, &next, &restored)), "exclusive_turn_held_save_data_restored");
    assert_same_save(&source, &restored);

    // Return the same verified content to the original device. The local
    // blob store already contains it, so fetch must verify and not replace.
    let (reverse, bytes) = f.prepare(1, &grant, &next);
    let reverse_hash = reverse.offer_hash().unwrap().to_string();
    let reverse_file = f.put("save-backed-reverse.json", &bytes);
    let (server, address) = f.save_listener(1, &grant, &reverse_hash, None);
    assert_save_flags(
        &json(f.fetch_save(0, &grant, &reverse_hash, &reverse_file, None, address)),
        "exclusive_turn_save_data_received",
    );
    success(server.finish());
    let (server, address) = f.serve(0, &grant, &reverse_hash, &reverse_file);
    let output = f.send_handoff(1, &grant, &reverse_hash, address);
    success(server.finish());
    let returned = f.status(0);
    let final_grant = returned["grant"].as_str().unwrap();
    confirmed(output, &f, &reverse, final_grant, 3, 0);
    report(&returned, "held", 3);
    report(&f.status(1), "released", 3);
    assert_eq!(returned["local_save_data_verified"], true);
    assert_eq!(
        fs::read(f.managed_bundle(0, &initial).join("manifest.postcard")).unwrap(),
        fs::read(original_bundle.join("manifest.postcard")).unwrap()
    );
    f.assert_no_live_storage();
}

#[test]
fn encrypted_handoff_partition_boundaries_and_exact_outbox_retry() {
    let f = Fixture::new();
    let initial = exclusive_turn_reference_commitment(&fs::read(&f.initial).unwrap()).unwrap().to_string();
    let (first, first_bytes) = f.prepare(0, &f.initial_grant, &initial);
    let first_hash = first.offer_hash().unwrap().to_string();
    let first_file = f.put("network-first.json", &first_bytes);
    // Public and wildcard sockets and outsiders are refused before listening.
    for bind in ["0.0.0.0:0", "8.8.8.8:0"] {
        rejected(
            f.network(1, "serve-handoff", &f.initial_grant, &first_hash)
                .args(["--offer", path(&first_file), "--from", &f.identities[0].peer_id().to_string(), "--bind", bind])
                .output()
                .unwrap(),
            "explicit private",
        );
    }
    rejected(
        f.network(0, "handoff", &f.initial_grant, &first_hash)
            .args(["--peer", &format!("{}@127.0.0.1:1", f.identities[2].peer_id())])
            .output()
            .unwrap(),
        "not the other fixed device",
    );
    let source_before = f.snapshot(0);
    let target_before = f.snapshot(1);
    let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = unavailable.local_addr().unwrap();
    // Keep an actual socket reserved but not listening after dropping it;
    // a different local process cannot authenticate as the expected peer.
    drop(unavailable);
    let out = f.send_handoff(0, &f.initial_grant, &first_hash, address);
    assert!(!out.status.success());
    assert_eq!(f.snapshot(0), source_before);

    let (server, target) = f.serve(1, &f.initial_grant, &first_hash, &first_file);
    let (address, proxy) = fault_proxy(target, WireFault::CorruptCheckpoint);
    assert!(!f.send_handoff(0, &f.initial_grant, &first_hash, address).status.success());
    proxy.join().unwrap();
    assert!(!server.finish().status.success());
    assert_eq!(f.snapshot(0), source_before);
    assert_eq!(f.snapshot(1), target_before);
    let (server, target) = f.serve(1, &f.initial_grant, &first_hash, &first_file);
    let (address, proxy) = fault_proxy(target, WireFault::LoseReceipt);
    assert!(!f.send_handoff(0, &f.initial_grant, &first_hash, address).status.success());
    proxy.join().unwrap();
    assert!(!server.finish().status.success());
    assert_eq!(f.snapshot(0), source_before);
    assert_eq!(f.status(1)["staged_offer"], first_hash);
    report(&f.status(1), "replica", 1);
    report(&json(f.mutate(0, "cancel", &f.initial_grant, &["--expected-offer", &first_hash])), "held", 1);
    let (second, second_bytes) = f.prepare(0, &f.initial_grant, &initial);
    let second_hash = second.offer_hash().unwrap().to_string();
    let second_file = f.put("network-second.json", &second_bytes);
    assert_eq!(second.offer.attempt, 2);
    // An actual live listener pinned to the canceled offer cannot talk to
    // the new offer: the Noise prologue differs and no journal mutates.
    let before = [f.snapshot(0), f.snapshot(1)];
    let (server, address) = f.serve(1, &f.initial_grant, &first_hash, &first_file);
    assert!(!f.send_handoff(0, &f.initial_grant, &second_hash, address).status.success());
    assert!(!server.finish().status.success());
    assert_eq!([f.snapshot(0), f.snapshot(1)], before);

    let (server, target) = f.serve(1, &f.initial_grant, &second_hash, &second_file);
    let (address, proxy) = fault_proxy(target, WireFault::LoseRelease);
    let failed = f.send_handoff(0, &f.initial_grant, &second_hash, address);
    rejected(failed, "source relinquished");
    proxy.join().unwrap();
    assert!(!server.finish().status.success());
    let released = f.status(0);
    report(&released, "released", 2);
    report(&f.status(1), "replica", 1);
    let grant = released["grant"].as_str().unwrap().to_owned();
    let outbox = f.export(0, &grant);
    let source_released = f.snapshot(0);
    rejected(f.mutate(0, "cancel", &grant, &["--expected-offer", &second_hash]), "no cancelable");
    let (server, target) = f.serve(1, &f.initial_grant, &second_hash, &second_file);
    let (address, proxy) = fault_proxy(target, WireFault::LoseAcceptance);
    rejected(f.send_handoff(0, &f.initial_grant, &second_hash, address), "source relinquished");
    proxy.join().unwrap();
    assert!(server.finish().status.success());
    assert_eq!(f.snapshot(0), source_released);
    report(&f.status(1), "held", 2);
    assert_eq!(f.status(1)["grant"], grant);
    let target_accepted = f.snapshot(1);
    let (server, target) = f.serve(1, &f.initial_grant, &second_hash, &second_file);
    confirmed(f.send_handoff(0, &f.initial_grant, &second_hash, target), &f, &second, &grant, 2, 1);
    success(server.finish());
    assert_eq!(f.snapshot(0), source_released);
    assert_eq!(f.snapshot(1), target_accepted);
    assert_eq!(f.export(0, &grant), outbox);
    // Reverse transfer crosses multiple encrypted 32 KiB frames. No
    // recipient-side file copy is used for the actual checkpoint bytes.
    let large = (0..70_123).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    let checkpoint = f.put("network-large.bin", &large);
    let commitment = exclusive_turn_reference_commitment(&large).unwrap();
    f.checkpoint(1, &grant, &initial, &checkpoint);
    rejected(
        f.network(1, "serve-handoff", &f.initial_grant, &second_hash)
            .args([
                "--offer",
                path(&second_file),
                "--from",
                &f.identities[0].peer_id().to_string(),
                "--bind",
                "127.0.0.1:0",
            ])
            .output()
            .unwrap(),
        "cannot restore",
    );
    let (reverse, reverse_bytes) = f.prepare(1, &grant, &commitment.to_string());
    let reverse_hash = reverse.offer_hash().unwrap().to_string();
    let reverse_file = f.put("network-reverse.json", &reverse_bytes);
    let (server, target) = f.serve(0, &grant, &reverse_hash, &reverse_file);
    let output = f.send_handoff(1, &grant, &reverse_hash, target);
    success(server.finish());
    let current = f.status(0);
    let new_grant = current["grant"].as_str().unwrap();
    confirmed(output, &f, &reverse, new_grant, 3, 0);
    report(&current, "held", 3);
    report(&f.status(1), "released", 3);
    assert_eq!(fs::read(f.state(0, commitment)).unwrap(), large);
    let (third, _) = f.prepare(0, new_grant, &commitment.to_string());
    assert_eq!(third.offer.attempt, 3);
    f.assert_no_live_storage();
}
