//! Real separate CLI processes, real signatures, private durable journals.
//! Reference bytes only: no Minecraft authority or runtime is created.
#![cfg(target_os = "linux")]

use fs2::FileExt;
use serde_json::Value;
use std::{
    fs::{self, File},
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
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
