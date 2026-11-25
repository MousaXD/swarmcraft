//! Actual CLI subprocesses and isolated private journals, never live game worlds.
#![cfg(target_os = "linux")]

use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{
    ownerless_checkpoint_commitment, Hash32, OwnerlessCreationApprovalV1, OwnerlessCreationV1, OwnerlessGenesisV1,
    OwnerlessValidatorV1, OWNERLESS_FORMAT_V1, OWNERLESS_NO_OP_V1,
};

fn cmd(profile: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    c.arg("--data-dir").arg(profile).args(["ownerless", "authority"]).args(args);
    c
}
fn invoke(profile: &Path, args: &[&str]) -> Output {
    cmd(profile, args).output().unwrap()
}
fn success(out: Output) -> Vec<u8> {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}
fn json(out: Output) -> Value {
    serde_json::from_slice(&success(out)).unwrap()
}
fn rejected(out: Output, reason: &str) {
    assert!(!out.status.success(), "unexpected success: {}", String::from_utf8_lossy(&out.stdout));
    assert!(String::from_utf8_lossy(&out.stderr).contains(reason), "{}", String::from_utf8_lossy(&out.stderr));
}
fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

struct Fixture {
    temp: tempfile::TempDir,
    profiles: Vec<PathBuf>,
    identities: Vec<PeerIdentity>,
    world: String,
    genesis: PathBuf,
    initial: PathBuf,
    initial_head: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let profiles: Vec<_> = (0..5).map(|i| temp.path().join(format!("peer-{i}"))).collect();
        let identities: Vec<_> =
            profiles.iter().map(|p| PeerIdentity::load_or_create(&DataPaths::from_root(p)).unwrap()).collect();
        let mut validators: Vec<_> =
            identities[..3].iter().map(|k| OwnerlessValidatorV1::from_key(k.public_key())).collect();
        validators.sort_by_key(|v| v.peer_id);
        let initial = temp.path().join("initial.bin");
        fs::write(&initial, b"accepted genesis bytes").unwrap();
        let genesis = OwnerlessGenesisV1 {
            format_version: OWNERLESS_FORMAT_V1,
            name: "Authority test".into(),
            creation_nonce: [99; 32],
            compatibility_fingerprint: Hash32([73; 32]),
            initial_state_commitment: ownerless_checkpoint_commitment(&fs::read(&initial).unwrap()).unwrap(),
            policy: OWNERLESS_NO_OP_V1,
            validators,
        };
        let world_id = genesis.world_id().unwrap();
        let mut approvals: Vec<_> = identities[..3]
            .iter()
            .map(|k| {
                let validator = k.peer_id();
                OwnerlessCreationApprovalV1 {
                    world_id,
                    validator,
                    signature: k.sign(&genesis.approval_bytes(validator).unwrap()),
                }
            })
            .collect();
        approvals.sort_by_key(|a| a.validator);
        let certificate = temp.path().join("creation.json");
        fs::write(&certificate, serde_json::to_vec(&OwnerlessCreationV1 { genesis, approvals }).unwrap()).unwrap();
        let world = world_id.to_string();
        let organizer = temp.path().join("organizer");
        let h = temp.path().join("history0.json");
        fs::write(
            &h,
            success(invoke(
                &organizer,
                &["genesis-history", "--certificate", s(&certificate), "--expected-world", &world],
            )),
        )
        .unwrap();
        let report = json(invoke(&organizer, &["inspect", "--history", s(&h), "--expected-world", &world]));
        assert_eq!(report["sequence"], 0);
        let initial_head = report["head"].as_str().unwrap().to_owned();
        let fixture = Self { temp, profiles, identities, world, genesis: h, initial, initial_head };
        for i in 0..5 {
            let status = json(fixture.init(i));
            assert_eq!(status["local_validator"], i < 3);
            assert_eq!(status["runtime_activation"], false);
        }
        assert!(!organizer.exists());
        fixture
    }
    fn path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
    fn organizer(&self) -> PathBuf {
        self.path("organizer")
    }
    fn init(&self, i: usize) -> Output {
        invoke(
            &self.profiles[i],
            &[
                "init",
                "--history",
                s(&self.genesis),
                "--expected-world",
                &self.world,
                "--expected-head",
                &self.initial_head,
                "--checkpoint",
                s(&self.initial),
            ],
        )
    }
    fn journal(&self, i: usize) -> PathBuf {
        self.profiles[i]
            .join("ownerless-authority-lab-v1")
            .join(self.world.strip_prefix("scownerless:").unwrap())
            .join("journal.json")
    }
    fn state(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, bytes).unwrap();
        path
    }
    fn propose(&self, history: &Path, head: &str, name: &str, action: &[&str]) -> (PathBuf, String) {
        let mut args =
            vec!["propose", "--history", s(history), "--expected-world", &self.world, "--expected-head", head];
        args.extend(action);
        let bytes = success(invoke(&self.organizer(), &args));
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let hash = value["proposal_hash"].as_str().unwrap().to_owned();
        let p = self.path(name);
        fs::write(&p, bytes).unwrap();
        (p, hash)
    }
    fn vote_cmd(&self, i: usize, head: &str, p: &(PathBuf, String), state: &Path) -> Command {
        cmd(
            &self.profiles[i],
            &[
                "vote",
                "--expected-world",
                &self.world,
                "--expected-head",
                head,
                "--proposal",
                s(&p.0),
                "--expected-proposal",
                &p.1,
                "--checkpoint",
                s(state),
                "--confirm",
            ],
        )
    }
    fn vote(&self, i: usize, head: &str, p: &(PathBuf, String), state: &Path, name: &str) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, success(self.vote_cmd(i, head, p, state).output().unwrap())).unwrap();
        path
    }
    fn assemble(&self, history: &Path, head: &str, p: &(PathBuf, String), votes: &[&Path]) -> Output {
        let mut args = vec![
            "assemble",
            "--history",
            s(history),
            "--expected-world",
            &self.world,
            "--expected-head",
            head,
            "--proposal",
            s(&p.0),
            "--expected-proposal",
            &p.1,
        ];
        for vote in votes {
            args.extend(["--vote", s(vote)]);
        }
        invoke(&self.organizer(), &args)
    }
    fn certified(&self, history: &Path, head: &str, p: &(PathBuf, String), votes: &[&Path], name: &str) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, success(self.assemble(history, head, p, votes))).unwrap();
        let info =
            json(invoke(&self.organizer(), &["inspect", "--history", s(&path), "--expected-world", &self.world]));
        assert_eq!(info["head"], p.1);
        assert_eq!(info["live_quorum_verified"], false);
        path
    }
    fn accept(&self, i: usize, history: &Path, head: &str, state: &Path) -> Output {
        invoke(
            &self.profiles[i],
            &[
                "accept",
                "--history",
                s(history),
                "--expected-world",
                &self.world,
                "--expected-head",
                head,
                "--checkpoint",
                s(state),
            ],
        )
    }
    fn status(&self, i: usize) -> Output {
        invoke(&self.profiles[i], &["status", "--expected-world", &self.world])
    }
    fn sync(&self, peers: &[usize], history: &Path, head: &str, state: &Path, seq: u64, fence: u64) {
        for &i in peers {
            let r = json(self.accept(i, history, head, state));
            assert_eq!(r["head"], head);
            assert_eq!(r["sequence"], seq);
            assert_eq!(r["reference_fence"], fence);
            assert_eq!(r["pending_proposal"], Value::Null);
            assert_eq!(r["runtime_activation"], false);
            assert_eq!(r["player_admission"], false);
        }
    }
    fn assert_no_live_storage(&self) {
        for (i, p) in self.profiles.iter().enumerate() {
            assert_eq!(fs::read_dir(p.join("worlds")).unwrap().count(), 0);
            assert!(!p.join("control").exists());
            assert!(!p.join("runtime").exists());
            assert_eq!(
                PeerIdentity::load_existing(&DataPaths::from_root(p)).unwrap().public_key(),
                self.identities[i].public_key()
            );
        }
        assert!(!self.organizer().exists());
    }
}

#[test]
fn founder_offline_majority_recovery_joint_replacement_and_minority_copy() {
    let f = Fixture::new();
    let founder_before = fs::read(f.journal(0)).unwrap();
    let state1 = f.state("state1.bin", b"first accepted reference state");
    let p1 = f.propose(&f.genesis, &f.initial_head, "p1.json", &["--checkpoint", s(&state1)]);
    let v1 = f.vote(1, &f.initial_head, &p1, &state1, "p1v1.json");
    assert_eq!(json(f.status(1))["head"], f.initial_head);
    rejected(f.assemble(&f.genesis, &f.initial_head, &p1, &[&v1]), "missing current validator majority");
    rejected(f.assemble(&f.genesis, &f.initial_head, &p1, &[&v1, &v1]), "noncanonical ownerless vote set");
    rejected(f.vote_cmd(3, &f.initial_head, &p1, &state1).output().unwrap(), "not an eligible");
    let v2 = f.vote(2, &f.initial_head, &p1, &state1, "p1v2.json");
    let h1 = f.certified(&f.genesis, &f.initial_head, &p1, &[&v1, &v2], "h1.json");
    rejected(f.accept(1, &h1, &p1.1, &f.initial), "checkpoint bytes do not match");
    f.sync(&[1, 2, 3, 4], &h1, &p1.1, &state1, 1, 1);

    let p2 = f.propose(&h1, &p1.1, "p2.json", &["--recover"]);
    let v1 = f.vote(1, &p1.1, &p2, &state1, "p2v1.json");
    let v2 = f.vote(2, &p1.1, &p2, &state1, "p2v2.json");
    let h2 = f.certified(&h1, &p1.1, &p2, &[&v1, &v2], "h2.json");
    f.sync(&[1, 2, 3, 4], &h2, &p2.1, &state1, 2, 2);
    rejected(f.vote_cmd(1, &p1.1, &p2, &state1).output().unwrap(), "differs from expected pin");

    let new: Vec<_> = (1..=3).map(|i| hex::encode(f.identities[i].public_key())).collect();
    let p3 = f.propose(
        &h2,
        &p2.1,
        "p3.json",
        &["--validator-key", &new[0], "--validator-key", &new[1], "--validator-key", &new[2]],
    );
    let v1 = f.vote(1, &p2.1, &p3, &state1, "p3v1.json");
    let v2 = f.vote(2, &p2.1, &p3, &state1, "p3v2.json");
    rejected(f.assemble(&h2, &p2.1, &p3, &[&v1, &v2]), "new validator has not explicitly approved enrollment");
    let v3 = f.vote(3, &p2.1, &p3, &state1, "p3v3.json");
    rejected(f.assemble(&h2, &p2.1, &p3, &[&v1, &v3]), "missing current validator majority");
    let h3 = f.certified(&h2, &p2.1, &p3, &[&v1, &v2, &v3], "h3.json");
    f.sync(&[1, 2, 3, 4], &h3, &p3.1, &state1, 3, 3);
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder_before, "founder journal changed while offline");

    let state2 = f.state("state2.bin", b"state after validator replacement");
    let p4 = f.propose(&h3, &p3.1, "p4.json", &["--checkpoint", s(&state2)]);
    // A late returning founder can catch up as a replica but no longer vote.
    f.sync(&[0], &h3, &p3.1, &state1, 3, 3);
    assert_eq!(json(f.status(0))["local_validator"], false);
    rejected(f.vote_cmd(0, &p3.1, &p4, &state2).output().unwrap(), "not an eligible");
    rejected(f.vote_cmd(4, &p3.1, &p4, &state2).output().unwrap(), "not an eligible");
    let v1 = f.vote(1, &p3.1, &p4, &state2, "p4v1.json");
    let v3 = f.vote(3, &p3.1, &p4, &state2, "p4v3.json");
    let h4 = f.certified(&h3, &p3.1, &p4, &[&v1, &v3], "h4.json");
    f.sync(&[1, 3, 4], &h4, &p4.1, &state2, 4, 3);
    rejected(f.accept(3, &h3, &p3.1, &state1), "rollback or conflicting");
    let report = json(f.status(3));
    assert_eq!(report["local_validator"], true);
    assert_eq!(report["checkpoint"], ownerless_checkpoint_commitment(&fs::read(&state2).unwrap()).unwrap().to_string());

    // Lose two validators. The surviving key can retain a vote but cannot
    // advance the head, rotate its fence, or synthesize a new roster.
    let p5 = f.propose(&h4, &p4.1, "p5.json", &["--recover"]);
    let only = f.vote(3, &p4.1, &p5, &state2, "p5v3.json");
    rejected(f.assemble(&h4, &p4.1, &p5, &[&only]), "missing current validator majority");
    assert_eq!(json(f.status(3))["head"], p4.1);
    let copy = f.path("recovery-copy.bin");
    let args = ["recovery-copy", "--expected-world", &f.world, "--expected-head", &p4.1, "--output", s(&copy)];
    let copy_report = json(invoke(&f.profiles[3], &args));
    assert_eq!(copy_report["write_authority"], false);
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&state2).unwrap());
    rejected(invoke(&f.profiles[3], &args), "exists");
    let exported = success(invoke(&f.profiles[3], &["export-history", "--expected-world", &f.world]));
    assert_eq!(
        serde_json::from_slice::<Value>(&exported).unwrap(),
        serde_json::from_slice::<Value>(&fs::read(&h4).unwrap()).unwrap()
    );
    f.assert_no_live_storage();
}

#[test]
fn concurrent_votes_and_lost_output_cannot_unlock_or_equivocate() {
    let f = Fixture::new();
    let state = f.state("candidate.bin", b"new candidate");
    let p = f.propose(&f.genesis, &f.initial_head, "candidate.json", &["--checkpoint", s(&state)]);
    let q = f.propose(&f.genesis, &f.initial_head, "recovery.json", &["--recover"]);
    let a = f.vote_cmd(0, &f.initial_head, &p, &state).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let b =
        f.vote_cmd(0, &f.initial_head, &q, &f.initial).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let a = a.wait_with_output().unwrap();
    let b = b.wait_with_output().unwrap();
    assert_ne!(a.status.success(), b.status.success(), "both conflicting requests had same result");
    let (winner, bytes, other, other_state, loser, winner_state) = if a.status.success() {
        (&p, a.stdout, &q, &f.initial, b, &state)
    } else {
        (&q, b.stdout, &p, &state, a, &f.initial)
    };
    rejected(loser, "already locked");
    let pending = json(f.status(0));
    assert_eq!(pending["pending_proposal"], winner.1);
    assert_eq!(pending["head"], f.initial_head);
    let same = json(f.accept(0, &f.genesis, &f.initial_head, &f.initial));
    assert_eq!(same["pending_proposal"], winner.1);
    rejected(f.init(0), "already exists");
    assert_eq!(success(f.vote_cmd(0, &f.initial_head, winner, winner_state).output().unwrap()), bytes);
    rejected(f.vote_cmd(0, &f.initial_head, other, other_state).output().unwrap(), "already locked");

    // A real process cannot deliver its vote (ENOSPC on stdout). The next
    // process must still observe the durable lock, not sign a different vote.
    let full = OpenOptions::new().write(true).open("/dev/full").unwrap();
    let lost = f
        .vote_cmd(1, &f.initial_head, winner, winner_state)
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!lost.status.success());
    assert_eq!(json(f.status(1))["pending_proposal"], winner.1);
    rejected(f.vote_cmd(1, &f.initial_head, other, other_state).output().unwrap(), "already locked");
    let v1 = f.path("winner0.json");
    fs::write(&v1, bytes).unwrap();
    let v2 = f.vote(1, &f.initial_head, winner, winner_state, "winner1.json");
    let h = f.certified(&f.genesis, &f.initial_head, winner, &[&v1, &v2], "accepted.json");
    let fence = if winner.1 == q.1 { 2 } else { 1 };
    f.sync(&[0, 1], &h, &winner.1, winner_state, 1, fence);
    f.assert_no_live_storage();
}

#[test]
fn corrupted_missing_linked_journals_and_checkpoint_bytes_fail_closed() {
    let f = Fixture::new();
    let journal = f.journal(0);
    let original = fs::read(&journal).unwrap();
    let mut tampered: Value = serde_json::from_slice(&original).unwrap();
    tampered["journal"]["format"] = 2.into();
    fs::write(&journal, serde_json::to_vec(&tampered).unwrap()).unwrap();
    rejected(f.status(0), "identity mismatch");
    tampered["journal"]["format"] = 1.into();
    tampered["signature"][0] = ((tampered["signature"][0].as_u64().unwrap() + 1) % 256).into();
    fs::write(&journal, serde_json::to_vec(&tampered).unwrap()).unwrap();
    rejected(f.status(0), "journal signature invalid");
    fs::write(&journal, &original).unwrap();
    let aside = journal.with_extension("saved");
    fs::rename(&journal, &aside).unwrap();
    rejected(f.status(0), "inspect");
    rejected(f.init(0), "already exists");
    std::os::unix::fs::symlink(&aside, &journal).unwrap();
    rejected(f.status(0), "unsafe ownerless journal");
    fs::remove_file(&journal).unwrap();
    fs::hard_link(&aside, &journal).unwrap();
    rejected(f.status(0), "unsafe ownerless journal");
    fs::remove_file(&journal).unwrap();
    fs::rename(&aside, &journal).unwrap();
    let checkpoint = journal
        .parent()
        .unwrap()
        .join("checkpoints")
        .join(format!("{}.bin", ownerless_checkpoint_commitment(&fs::read(&f.initial).unwrap()).unwrap()));
    let bytes = fs::read(&checkpoint).unwrap();
    fs::write(&checkpoint, b"corrupted").unwrap();
    rejected(f.status(0), "stored ownerless checkpoint commitment mismatch");
    fs::write(&checkpoint, bytes).unwrap();
    assert_eq!(json(f.status(0))["head"], f.initial_head);
    f.assert_no_live_storage();
}

#[test]
fn same_head_witness_changes_do_not_unlock_but_certified_extension_resolves_pending_vote() {
    let f = Fixture::new();
    let p = f.propose(&f.genesis, &f.initial_head, "recover1.json", &["--recover"]);
    let v0 = f.vote(0, &f.initial_head, &p, &f.initial, "r1v0.json");
    let v1 = f.vote(1, &f.initial_head, &p, &f.initial, "r1v1.json");
    let v2 = f.vote(2, &f.initial_head, &p, &f.initial, "r1v2.json");
    let h1 = f.certified(&f.genesis, &f.initial_head, &p, &[&v0, &v1], "r1a.json");
    let alternative = f.certified(&f.genesis, &f.initial_head, &p, &[&v1, &v2], "r1b.json");
    f.sync(&[0, 1, 2], &h1, &p.1, &f.initial, 1, 2);

    let state = f.state("uncommitted.bin", b"uncommitted branch");
    let candidate = f.propose(&h1, &p.1, "candidate2.json", &["--checkpoint", s(&state)]);
    let other = f.propose(&h1, &p.1, "recover2.json", &["--recover"]);
    f.vote(0, &p.1, &candidate, &state, "c2v0.json");
    let original = fs::read(f.journal(0)).unwrap();
    let report = json(f.accept(0, &alternative, &p.1, &f.initial));
    assert_eq!(report["pending_proposal"], candidate.1);
    assert_eq!(fs::read(f.journal(0)).unwrap(), original);
    rejected(f.vote_cmd(0, &p.1, &other, &f.initial).output().unwrap(), "already locked");

    // The other two honest voters can certify another proposal from the
    // same parent. Its majority is an actual decision, unlike a timeout.
    let v1 = f.vote(1, &p.1, &other, &f.initial, "r2v1.json");
    let v2 = f.vote(2, &p.1, &other, &f.initial, "r2v2.json");
    let h2 = f.certified(&alternative, &p.1, &other, &[&v1, &v2], "r2.json");
    f.sync(&[0, 1, 2], &h2, &other.1, &f.initial, 2, 3);
    rejected(f.vote_cmd(0, &p.1, &candidate, &state).output().unwrap(), "differs from expected pin");
    rejected(f.accept(0, &h1, &p.1, &f.initial), "rollback or conflicting");
    f.assert_no_live_storage();
}
