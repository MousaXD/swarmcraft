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
        Self::with_mode(false)
    }
    fn with_mode(public: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let profiles: Vec<_> = (0..5).map(|i| temp.path().join(format!("peer-{i}"))).collect();
        let identities: Vec<_> =
            profiles.iter().map(|p| PeerIdentity::load_or_create(&DataPaths::from_root(p)).unwrap()).collect();
        let mut validators: Vec<_> =
            identities[..3].iter().map(|k| OwnerlessValidatorV1::from_key(k.public_key())).collect();
        validators.sort_by_key(|v| v.peer_id);
        let initial = temp.path().join("initial.bin");
        fs::write(&initial, b"accepted genesis bytes").unwrap();
        let organizer = temp.path().join("organizer");
        let certificate = temp.path().join("creation.json");
        let genesis = if public {
            let keys: Vec<_> = identities[..3].iter().map(|id| hex::encode(id.public_key())).collect();
            let compatibility = Hash32([73; 32]).to_string();
            let commitment = ownerless_checkpoint_commitment(&fs::read(&initial).unwrap()).unwrap().to_string();
            let result = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
                .arg("--data-dir")
                .arg(&organizer)
                .args([
                    "ownerless",
                    "draft",
                    "--name",
                    "Public reference",
                    "--public-play",
                    "--compatibility-fingerprint",
                    &compatibility,
                    "--initial-state-commitment",
                    &commitment,
                    "--validator-key",
                    &keys[0],
                    "--validator-key",
                    &keys[1],
                    "--validator-key",
                    &keys[2],
                ])
                .output()
                .unwrap();
            let draft = success(result);
            let genesis: OwnerlessGenesisV1 = serde_json::from_slice(&draft).unwrap();
            assert_eq!(genesis.format_version, swarm_protocol::OWNERLESS_PUBLIC_FORMAT_V2);
            assert_eq!(genesis.policy, swarm_protocol::OWNERLESS_NO_OP_PUBLIC_V2);
            let draft_path = temp.path().join("draft.json");
            fs::write(&draft_path, draft).unwrap();
            let world = genesis.world_id().unwrap().to_string();
            let mut approvals = Vec::new();
            for (i, profile) in profiles[..3].iter().enumerate() {
                let approval = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
                    .arg("--data-dir")
                    .arg(profile)
                    .args(["ownerless", "approve", "--draft", s(&draft_path), "--expected-world", &world, "--confirm"])
                    .output()
                    .unwrap();
                let path = temp.path().join(format!("approval-{i}.json"));
                fs::write(&path, success(approval)).unwrap();
                approvals.push(path);
            }
            let assembled = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
                .arg("--data-dir")
                .arg(&organizer)
                .args([
                    "ownerless",
                    "assemble",
                    "--draft",
                    s(&draft_path),
                    "--expected-world",
                    &world,
                    "--approval",
                    s(&approvals[0]),
                    "--approval",
                    s(&approvals[1]),
                    "--approval",
                    s(&approvals[2]),
                ])
                .output()
                .unwrap();
            fs::write(&certificate, success(assembled)).unwrap();
            let permission = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
                .arg("--data-dir")
                .arg(&organizer)
                .args([
                    "ownerless",
                    "permission-profile",
                    "--certificate",
                    s(&certificate),
                    "--expected-world",
                    &world,
                    "--expected-compatibility",
                    &compatibility,
                ])
                .output()
                .unwrap();
            rejected(permission, "public reference identities cannot use the private #65");
            genesis
        } else {
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
            fs::write(
                &certificate,
                serde_json::to_vec(&OwnerlessCreationV1 { genesis: genesis.clone(), approvals }).unwrap(),
            )
            .unwrap();
            genesis
        };
        let world_id = genesis.world_id().unwrap();
        let world = world_id.to_string();
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
        assert_eq!(report["participation_mode"], if public { "public_authenticated" } else { "invite_only" });
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

impl Fixture {
    fn role_propose(&self, history: &Path, head: &str, name: &str, action: &[&str]) -> (PathBuf, String) {
        let mut args = vec![
            "participation",
            action[0],
            "--history",
            s(history),
            "--expected-world",
            &self.world,
            "--expected-head",
            head,
        ];
        args.extend(&action[1..]);
        let bytes = success(invoke(&self.organizer(), &args));
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let hash = v["proposal_hash"].as_str().unwrap().to_owned();
        let p = self.path(name);
        fs::write(&p, bytes).unwrap();
        (p, hash)
    }
    fn commit_role(
        &self,
        history: &Path,
        head: &str,
        proposal: &(PathBuf, String),
        name: &str,
        seq: u64,
        fence: u64,
    ) -> PathBuf {
        let v1 = self.vote(1, head, proposal, &self.initial, &format!("{name}-v1.json"));
        rejected(self.assemble(history, head, proposal, &[&v1]), "missing current validator majority");
        let v2 = self.vote(2, head, proposal, &self.initial, &format!("{name}-v2.json"));
        let h = self.certified(history, head, proposal, &[&v1, &v2], name);
        self.sync(&[1, 2, 3, 4], &h, &proposal.1, &self.initial, seq, fence);
        h
    }
    fn challenge(&self, verifier: usize, subject: usize, head: &str, name: &str, intent: &[&str]) -> (PathBuf, String) {
        let key = hex::encode(self.identities[subject].public_key());
        let mut args = vec![
            "participation",
            "challenge",
            "--expected-world",
            &self.world,
            "--expected-head",
            head,
            "--subject-key",
            &key,
        ];
        args.extend(intent);
        let bytes = success(invoke(&self.profiles[verifier], &args));
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let hash = v["challenge_hash"].as_str().unwrap().to_owned();
        let p = self.path(name);
        fs::write(&p, bytes).unwrap();
        (p, hash)
    }
    fn respond(
        &self,
        subject: usize,
        history: &Path,
        head: &str,
        challenge: &(PathBuf, String),
        consent: bool,
    ) -> Output {
        let mut args = vec![
            "participation",
            "respond",
            "--history",
            s(history),
            "--expected-world",
            &self.world,
            "--expected-head",
            head,
            "--request",
            s(&challenge.0),
            "--expected-challenge",
            &challenge.1,
            "--confirm",
        ];
        if consent {
            args.push("--confirm-volunteer");
        }
        invoke(&self.profiles[subject], &args)
    }
    fn response(
        &self,
        subject: usize,
        history: &Path,
        head: &str,
        challenge: &(PathBuf, String),
        consent: bool,
        name: &str,
    ) -> PathBuf {
        let p = self.path(name);
        fs::write(&p, success(self.respond(subject, history, head, challenge, consent))).unwrap();
        p
    }
    fn verify_role_cmd(&self, verifier: usize, head: &str, challenge: &(PathBuf, String), response: &Path) -> Command {
        cmd(
            &self.profiles[verifier],
            &[
                "participation",
                "verify",
                "--expected-world",
                &self.world,
                "--expected-head",
                head,
                "--expected-challenge",
                &challenge.1,
                "--response",
                s(response),
            ],
        )
    }
    fn verify_role(&self, verifier: usize, head: &str, challenge: &(PathBuf, String), response: &Path) -> Output {
        self.verify_role_cmd(verifier, head, challenge, response).output().unwrap()
    }
}

fn assert_role(report: Value, role: &str, already_validator: bool) {
    assert_eq!(report["role"], role);
    assert_eq!(report["validator_already_enrolled"], already_validator);
    for flag in [
        "validator_granted",
        "write_authority",
        "live_quorum_verified",
        "resource_enforcement_verified",
        "runtime_activation",
        "worker_admission",
        "player_admission",
    ] {
        assert_eq!(report[flag], false, "unexpected role grant: {flag}");
    }
}

#[test]
fn public_authenticated_play_never_enrolls_workers_or_validators() {
    let f = Fixture::with_mode(true);
    let head = &f.initial_head;
    let player_key = hex::encode(f.identities[3].public_key());
    let before = fs::read(f.journal(1)).unwrap();
    let play = f.challenge(1, 3, head, "play.json", &["--play"]);
    rejected(f.respond(4, &f.genesis, head, &play, false), "not the reviewed participation subject");
    rejected(f.respond(3, &f.genesis, head, &play, true), "play must not include it");
    let reply = f.response(3, &f.genesis, head, &play, false, "play-response.json");
    let report = json(f.verify_role(1, head, &play, &reply));
    assert_role(report, "reference_player", false);
    rejected(f.verify_role(1, head, &play, &reply), "already consumed or canceled");
    assert_eq!(fs::read(f.journal(1)).unwrap(), before, "play mutated authority journal");

    // A player can request volunteering but cannot manufacture eligibility.
    let offer_args = ["--memory-mib", "4096", "--cpu-millicores", "1000"];
    let offer = f.challenge(1, 3, head, "unapproved.json", &offer_args);
    rejected(f.respond(3, &f.genesis, head, &offer, false), "separate --confirm-volunteer");
    let reply = f.response(3, &f.genesis, head, &offer, true, "offer-response.json");
    rejected(f.verify_role(1, head, &offer, &reply), "no accepted eligibility");
    rejected(
        invoke(
            &f.organizer(),
            &[
                "participation",
                "propose-player",
                "--history",
                s(&f.genesis),
                "--expected-world",
                &f.world,
                "--expected-head",
                head,
                "--player-key",
                &player_key,
                "--allow",
            ],
        ),
        "public participation does not use private invitations",
    );

    let grant = f.role_propose(
        &f.genesis,
        head,
        "worker.json",
        &["propose-worker", "--worker-key", &player_key, "--memory-mib", "4096", "--cpu-millicores", "2000"],
    );
    rejected(f.vote_cmd(3, head, &grant, &f.initial).output().unwrap(), "not an eligible ownerless validator");
    let h1 = f.commit_role(&f.genesis, head, &grant, "h1.json", 1, 2);
    let head = &grant.1;
    assert_eq!(json(f.status(3))["local_validator"], false);
    // Old challenge was issued against the preceding head.
    rejected(f.verify_role(1, head, &offer, &reply), "differs from accepted head");
    let large = f.challenge(1, 3, head, "oversize.json", &["--memory-mib", "8192", "--cpu-millicores", "1000"]);
    let large_reply = f.response(3, &h1, head, &large, true, "large-response.json");
    rejected(f.verify_role(1, head, &large, &large_reply), "exceeds accepted worker cap");
    let offer = f.challenge(1, 3, head, "approved.json", &offer_args);
    let reply = f.response(3, &h1, head, &offer, true, "approved-response.json");
    let report = json(f.verify_role(1, head, &offer, &reply));
    assert_eq!(report["offered_budget"]["memory_mib"], 4096);
    assert_eq!(report["offered_budget"]["cpu_millicores"], 1000);
    assert_role(report, "reference_volunteer", false);
    let recovery = f.propose(&h1, head, "recovery.json", &["--recover"]);
    rejected(f.vote_cmd(3, head, &recovery, &f.initial).output().unwrap(), "not an eligible ownerless validator");

    let stale = f.challenge(1, 3, head, "pending-offer.json", &offer_args);
    let stale_reply = f.response(3, &h1, head, &stale, true, "pending-response.json");
    let revoke = f.role_propose(&h1, head, "revoke.json", &["propose-worker", "--worker-key", &player_key, "--revoke"]);
    let h2 = f.commit_role(&h1, head, &revoke, "h2.json", 2, 3);
    let head = &revoke.1;
    rejected(f.verify_role(1, head, &stale, &stale_reply), "differs from accepted head");
    let offer = f.challenge(1, 3, head, "removed.json", &offer_args);
    let reply = f.response(3, &h2, head, &offer, true, "removed-response.json");
    rejected(f.verify_role(1, head, &offer, &reply), "no accepted eligibility");
    let play = f.challenge(1, 3, head, "still-public.json", &["--play"]);
    let reply = f.response(3, &h2, head, &play, false, "still-public-response.json");
    assert_role(json(f.verify_role(1, head, &play, &reply)), "reference_player", false);
    f.assert_no_live_storage();
}

#[test]
fn invite_only_player_and_worker_roles_are_independent_and_revocable() {
    let f = Fixture::new();
    let head = &f.initial_head;
    let player_key = hex::encode(f.identities[3].public_key());
    let worker_key = hex::encode(f.identities[4].public_key());
    // Even a founding validator is not automatically invited.
    let denied = f.challenge(1, 0, head, "founder-play.json", &["--play"]);
    let reply = f.response(0, &f.genesis, head, &denied, false, "founder-reply.json");
    rejected(f.verify_role(1, head, &denied, &reply), "no accepted private invitation");
    let invite =
        f.role_propose(&f.genesis, head, "invite.json", &["propose-player", "--player-key", &player_key, "--allow"]);
    let h1 = f.commit_role(&f.genesis, head, &invite, "h1.json", 1, 2);
    let head = &invite.1;
    let play = f.challenge(1, 3, head, "invited.json", &["--play"]);
    let reply = f.response(3, &h1, head, &play, false, "invited-response.json");
    assert_role(json(f.verify_role(1, head, &play, &reply)), "reference_player", false);
    let worker = f.role_propose(
        &h1,
        head,
        "worker.json",
        &["propose-worker", "--worker-key", &worker_key, "--memory-mib", "4096", "--cpu-millicores", "1000"],
    );
    let h2 = f.commit_role(&h1, head, &worker, "h2.json", 2, 3);
    let head = &worker.1;
    let play = f.challenge(1, 4, head, "worker-play.json", &["--play"]);
    let reply = f.response(4, &h2, head, &play, false, "worker-play-response.json");
    rejected(f.verify_role(1, head, &play, &reply), "no accepted private invitation");
    let offer = f.challenge(1, 4, head, "worker-offer.json", &["--memory-mib", "512", "--cpu-millicores", "250"]);
    let reply = f.response(4, &h2, head, &offer, true, "worker-offer-response.json");
    assert_role(json(f.verify_role(1, head, &offer, &reply)), "reference_volunteer", false);
    let revoke =
        f.role_propose(&h2, head, "uninvite.json", &["propose-player", "--player-key", &player_key, "--revoke"]);
    let h3 = f.commit_role(&h2, head, &revoke, "h3.json", 3, 4);
    let head = &revoke.1;
    let status = json(f.status(1));
    assert_eq!(status["invited_player_count"], 0);
    assert_eq!(status["eligible_worker_count"], 1);
    let play = f.challenge(1, 3, head, "revoked.json", &["--play"]);
    let reply = f.response(3, &h3, head, &play, false, "revoked-response.json");
    rejected(f.verify_role(1, head, &play, &reply), "no accepted private invitation");
    f.assert_no_live_storage();
}

#[test]
fn participation_challenge_is_single_use_across_processes_and_output_failure() {
    let f = Fixture::with_mode(true);
    let head = &f.initial_head;
    let play = f.challenge(1, 3, head, "first.json", &["--play"]);
    let response = f.response(3, &f.genesis, head, &play, false, "first-response.json");
    let a = f.verify_role_cmd(1, head, &play, &response).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let b = f.verify_role_cmd(1, head, &play, &response).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let a = a.wait_with_output().unwrap();
    let b = b.wait_with_output().unwrap();
    assert_ne!(a.status.success(), b.status.success());
    let (winner, loser) = if a.status.success() { (a, b) } else { (b, a) };
    assert_role(json(winner), "reference_player", false);
    rejected(loser, "already consumed or canceled");
    let next = f.challenge(1, 3, head, "next.json", &["--play"]);
    let reply = f.response(3, &f.genesis, head, &next, false, "next-response.json");
    rejected(f.verify_role(1, head, &play, &response), "differs from expected pin");
    let full = OpenOptions::new().write(true).open("/dev/full").unwrap();
    let out =
        f.verify_role_cmd(1, head, &next, &reply).stdout(Stdio::from(full)).stderr(Stdio::piped()).output().unwrap();
    assert!(!out.status.success());
    rejected(f.verify_role(1, head, &next, &reply), "already consumed or canceled");
    let canceled = f.challenge(1, 3, head, "cancel.json", &["--play"]);
    let reply = f.response(3, &f.genesis, head, &canceled, false, "cancel-response.json");
    success(invoke(
        &f.profiles[1],
        &[
            "participation",
            "cancel",
            "--expected-world",
            &f.world,
            "--expected-head",
            head,
            "--expected-challenge",
            &canceled.1,
        ],
    ));
    rejected(f.verify_role(1, head, &canceled, &reply), "already consumed or canceled");
    let tampered = f.challenge(1, 3, head, "tamper.json", &["--play"]);
    let reply = f.response(3, &f.genesis, head, &tampered, false, "tamper-response.json");
    let path = f.journal(1).parent().unwrap().join("participation-challenge.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state["state"]["consumed"] = true.into();
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    rejected(f.verify_role(1, head, &tampered, &reply), "local participation challenge signature invalid");
    f.assert_no_live_storage();
}

struct ObservationListener {
    child: Option<std::process::Child>,
    address: String,
}

impl Fixture {
    fn decision_serve(&self, voter: usize, proposer: usize, head: &str, p: &(PathBuf, String)) -> Command {
        cmd(
            &self.profiles[voter],
            &[
                "decision",
                "serve-vote",
                "--expected-world",
                &self.world,
                "--expected-head",
                head,
                "--proposal",
                s(&p.0),
                "--expected-proposal",
                &p.1,
                "--from",
                &self.identities[proposer].peer_id().to_string(),
                "--bind",
                "127.0.0.1:0",
                "--confirm",
            ],
        )
    }

    fn decision_listener(
        &self,
        voter: usize,
        proposer: usize,
        head: &str,
        p: &(PathBuf, String),
    ) -> ObservationListener {
        use std::io::BufRead;
        let child = self
            .decision_serve(voter, proposer, head, p)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut listener = ObservationListener { child: Some(child), address: String::new() };
        let mut line = String::new();
        std::io::BufReader::new(listener.child.as_mut().unwrap().stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(!line.is_empty(), "decision listener failed before readiness");
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["classification"], "ownerless_reference_decision_listener");
        assert_eq!(ready["validator"], self.identities[voter].peer_id().to_string());
        assert_eq!(ready["proposal"], p.1);
        assert_eq!(ready["runtime_activation"], false);
        listener.address = ready["address"].as_str().unwrap().to_owned();
        listener
    }

    fn decision_request(
        &self,
        proposer: usize,
        voter: usize,
        head: &str,
        p: &(PathBuf, String),
        state: &Path,
        addr: &str,
    ) -> Output {
        invoke(
            &self.profiles[proposer],
            &[
                "decision",
                "request-vote",
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
                "--peer",
                &format!("{}@{}", self.identities[voter].peer_id(), addr),
                "--confirm",
            ],
        )
    }
}

// Inspect the actual framed TCP exchange without access to its ephemeral keys.
// Forward only the known message sequence, with optional corruption or loss.
fn decision_proxy(
    listener: std::net::TcpListener,
    target: String,
    checkpoint_chunks: usize,
    corrupt_checkpoint: bool,
    drop_vote: bool,
) -> Vec<u8> {
    use std::{
        io::{Read, Write},
        net::TcpStream,
        time::Duration,
    };
    fn relay(from: &mut TcpStream, to: &mut TcpStream, capture: &mut Vec<u8>, corrupt: bool, drop: bool) {
        let mut header = [0; 4];
        from.read_exact(&mut header).unwrap();
        let size = u32::from_be_bytes(header) as usize;
        assert!((1..=32784).contains(&size));
        let mut bytes = vec![0; size];
        from.read_exact(&mut bytes).unwrap();
        capture.extend_from_slice(&header);
        capture.extend_from_slice(&bytes);
        if corrupt {
            bytes[0] ^= 1;
        }
        if !drop {
            to.write_all(&header).unwrap();
            to.write_all(&bytes).unwrap();
        }
    }
    let (mut client, _) = listener.accept().unwrap();
    let mut server = TcpStream::connect(target).unwrap();
    for stream in [&client, &server] {
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.set_nodelay(true).unwrap();
    }
    let mut capture = Vec::new();
    relay(&mut client, &mut server, &mut capture, false, false); // Noise NN first message
    relay(&mut server, &mut client, &mut capture, false, false); // Noise NN second message
    relay(&mut server, &mut client, &mut capture, false, false); // responder auth
    relay(&mut client, &mut server, &mut capture, false, false); // initiator auth
    for _ in 0..3 {
        // proposal length, proposal, checkpoint length
        relay(&mut client, &mut server, &mut capture, false, false);
    }
    for i in 0..checkpoint_chunks {
        relay(&mut client, &mut server, &mut capture, corrupt_checkpoint && i + 1 == checkpoint_chunks, false);
    }
    if !corrupt_checkpoint {
        relay(&mut server, &mut client, &mut capture, false, drop_vote); // vote length
        relay(&mut server, &mut client, &mut capture, false, drop_vote); // vote
    }
    capture
}

#[test]
fn encrypted_decision_vote_loss_restart_and_exact_checkpoint() {
    use std::{net::TcpListener, thread};
    let f = Fixture::new();
    let founder = fs::read(f.journal(0)).unwrap();
    let marker = b"private-reference-checkpoint-content-66";
    // Exact upper bound exercises 32 authenticated chunks, not only tiny JSON.
    let mut payload = vec![0u8; swarm_protocol::MAX_OWNERLESS_CHECKPOINT_BYTES];
    payload[..marker.len()].copy_from_slice(marker);
    let state = f.state("network-state.bin", &payload);
    let p = f.propose(&f.genesis, &f.initial_head, "net-proposal.json", &["--checkpoint", s(&state)]);
    let before = fs::read(f.journal(2)).unwrap();
    let listener = f.decision_listener(2, 1, &f.initial_head, &p);
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = proxy.local_addr().unwrap().to_string();
    let target = listener.address.clone();
    let relay = thread::spawn(move || decision_proxy(proxy, target, 32, true, false));
    let result = f.decision_request(1, 2, &f.initial_head, &p, &state, &addr);
    assert!(!result.status.success());
    assert!(!listener.finish().status.success());
    let capture = relay.join().unwrap();
    assert!(!capture.windows(marker.len()).any(|w| w == marker));
    assert_eq!(fs::read(f.journal(2)).unwrap(), before);
    // Lose the response after the real responder durably records its vote.
    let listener = f.decision_listener(2, 1, &f.initial_head, &p);
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = proxy.local_addr().unwrap().to_string();
    let target = listener.address.clone();
    let relay = thread::spawn(move || decision_proxy(proxy, target, 32, false, true));
    let result = f.decision_request(1, 2, &f.initial_head, &p, &state, &addr);
    assert!(!result.status.success());
    success(listener.finish());
    let capture = relay.join().unwrap();
    assert!(!capture.windows(marker.len()).any(|w| w == marker));
    assert_eq!(json(f.status(2))["pending_proposal"], p.1);
    let locked = fs::read(f.journal(2)).unwrap();
    let conflict = f.propose(&f.genesis, &f.initial_head, "conflict.json", &["--recover"]);
    rejected(f.decision_serve(2, 1, &f.initial_head, &conflict).output().unwrap(), "already locked");
    let listener = f.decision_listener(2, 1, &f.initial_head, &p);
    let vote = success(f.decision_request(1, 2, &f.initial_head, &p, &state, &listener.address));
    success(listener.finish());
    assert_eq!(fs::read(f.journal(2)).unwrap(), locked);
    assert_eq!(vote, success(f.vote_cmd(2, &f.initial_head, &p, &state).output().unwrap()));
    let remote_vote = f.path("network-vote.json");
    fs::write(&remote_vote, vote).unwrap();
    let local_vote = f.vote(1, &f.initial_head, &p, &state, "local-vote.json");
    let history = f.certified(&f.genesis, &f.initial_head, &p, &[&remote_vote, &local_vote], "net-history.json");
    f.sync(&[1, 2], &history, &p.1, &state, 1, 1);
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder);
    f.assert_no_live_storage();
}

#[test]
fn encrypted_decision_pins_both_identities_and_rechecks_head() {
    let f = Fixture::new();
    let p = f.propose(&f.genesis, &f.initial_head, "recover.json", &["--recover"]);
    let before = fs::read(f.journal(2)).unwrap();
    rejected(f.decision_serve(2, 3, &f.initial_head, &p).output().unwrap(), "current validator");
    let listener = f.decision_listener(2, 1, &f.initial_head, &p);
    // The caller claims the endpoint belongs to a different accepted validator.
    let wrong = f.decision_request(1, 0, &f.initial_head, &p, &f.initial, &listener.address);
    assert!(!wrong.status.success());
    assert!(!listener.finish().status.success());
    let listener = f.decision_listener(2, 1, &f.initial_head, &p);
    let wrong = f.decision_request(0, 2, &f.initial_head, &p, &f.initial, &listener.address);
    assert!(!wrong.status.success());
    assert!(!listener.finish().status.success());
    assert_eq!(fs::read(f.journal(2)).unwrap(), before);
    let listener = f.decision_listener(2, 1, &f.initial_head, &p);
    let v0 = f.vote(0, &f.initial_head, &p, &f.initial, "v0.json");
    let v1 = f.vote(1, &f.initial_head, &p, &f.initial, "v1.json");
    let history = f.certified(&f.genesis, &f.initial_head, &p, &[&v0, &v1], "recovered.json");
    f.sync(&[2], &history, &p.1, &f.initial, 1, 2);
    let advanced = fs::read(f.journal(2)).unwrap();
    let late = f.decision_request(1, 2, &f.initial_head, &p, &f.initial, &listener.address);
    assert!(!late.status.success());
    rejected(listener.finish(), "accepted head differs");
    assert_eq!(fs::read(f.journal(2)).unwrap(), advanced);
    f.assert_no_live_storage();
}

#[test]
fn encrypted_decision_joint_roster_requires_entrant_vote() {
    let f = Fixture::new();
    let keys: Vec<_> = [1, 2, 3].iter().map(|&i| hex::encode(f.identities[i].public_key())).collect();
    let p = f.propose(
        &f.genesis,
        &f.initial_head,
        "joint.json",
        &["--validator-key", &keys[0], "--validator-key", &keys[1], "--validator-key", &keys[2]],
    );
    rejected(f.decision_serve(4, 1, &f.initial_head, &p).output().unwrap(), "not an eligible ownerless validator");
    let v1 = f.vote(1, &f.initial_head, &p, &f.initial, "joint-v1.json");
    let listener = f.decision_listener(2, 1, &f.initial_head, &p);
    let v2 = f.path("joint-v2.json");
    fs::write(&v2, success(f.decision_request(1, 2, &f.initial_head, &p, &f.initial, &listener.address))).unwrap();
    success(listener.finish());
    assert!(!f.assemble(&f.genesis, &f.initial_head, &p, &[&v1, &v2]).status.success());
    let listener = f.decision_listener(3, 1, &f.initial_head, &p);
    let v3 = f.path("joint-v3.json");
    fs::write(&v3, success(f.decision_request(1, 3, &f.initial_head, &p, &f.initial, &listener.address))).unwrap();
    success(listener.finish());
    let history = f.certified(&f.genesis, &f.initial_head, &p, &[&v1, &v2, &v3], "joint-history.json");
    f.sync(&[1, 2, 3], &history, &p.1, &f.initial, 1, 2);
    f.assert_no_live_storage();
}

impl ObservationListener {
    fn finish(mut self) -> Output {
        self.child.take().unwrap().wait_with_output().unwrap()
    }
}
impl Drop for ObservationListener {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Fixture {
    fn observation_serve(&self, peer: usize, head: &str) -> Command {
        cmd(
            &self.profiles[peer],
            &[
                "observe",
                "serve",
                "--expected-world",
                &self.world,
                "--expected-head",
                head,
                "--bind",
                "127.0.0.1:0",
                "--confirm",
            ],
        )
    }

    fn observe_listener(&self, peer: usize, head: &str) -> ObservationListener {
        use std::io::BufRead;
        let child = self.observation_serve(peer, head).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut listener = ObservationListener { child: Some(child), address: String::new() };
        let mut line = String::new();
        std::io::BufReader::new(listener.child.as_mut().unwrap().stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(
            !line.is_empty(),
            "listener failed before readiness: {:?}",
            listener.child.as_mut().unwrap().try_wait()
        );
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["classification"], "ownerless_reference_observation_listener");
        assert_eq!(ready["validator"], self.identities[peer].peer_id().to_string());
        assert_eq!(ready["head"], head);
        assert_eq!(ready["runtime_activation"], false);
        listener.address = ready["address"].as_str().unwrap().to_owned();
        listener
    }

    fn observation_collect(&self, verifier: usize, head: &str, peers: &[(usize, &str)]) -> Output {
        let endpoints: Vec<_> = peers.iter().map(|(i, a)| format!("{}@{}", self.identities[*i].peer_id(), a)).collect();
        let mut args =
            vec!["observe", "collect", "--expected-world", &self.world, "--expected-head", head, "--confirm"];
        for endpoint in &endpoints {
            args.extend(["--peer", endpoint]);
        }
        invoke(&self.profiles[verifier], &args)
    }

    fn verify_observation_report(&self, history: &Path, result: &Value) {
        use swarm_consensus::{
            ownerless_authority::verify_ownerless_history, ownerless_observation::verify_observation_majority,
        };
        let history: swarm_protocol::OwnerlessAuthorityHistoryV1 =
            serde_json::from_slice(&fs::read(history).unwrap()).unwrap();
        let head = verify_ownerless_history(self.world.parse().unwrap(), &history).unwrap();
        let request = serde_json::from_value(result["request"].clone()).unwrap();
        let responses: Vec<swarm_protocol::OwnerlessObservationResponseV1> =
            serde_json::from_value(result["responses"].clone()).unwrap();
        verify_observation_majority(&head, &request, &responses).unwrap();
        assert_eq!(result["classification"], "observed_recent_ownerless_reference_majority");
        assert_eq!(result["observation_count"], responses.len());
        assert_eq!(result["head"], head.hash().to_string());
        assert_eq!(result["checkpoint"], head.checkpoint().to_string());
        assert_eq!(result["reference_fence"], head.fence());
        assert_eq!(result["majority_threshold"], head.threshold());
        assert!(result["collection_elapsed_ms"].as_u64().unwrap() < result["collection_window_ms"].as_u64().unwrap());
        for key in [
            "freshness_lease",
            "latest_global_head_proven",
            "live_quorum_verified",
            "write_authority",
            "canonical_authority",
            "runtime_activation",
            "player_admission",
            "worker_admission",
        ] {
            assert_eq!(result[key], false, "observation granted {key}");
        }
    }
}

#[test]
fn actual_tcp_observation_survives_founder_absence_and_rejects_replay() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let f = Fixture::new();
    let founder = fs::read(f.journal(0)).unwrap();
    let before = [fs::read(f.journal(1)).unwrap(), fs::read(f.journal(2)).unwrap()];
    let remote = f.observe_listener(2, &f.initial_head);
    let first = json(f.observation_collect(1, &f.initial_head, &[(2, &remote.address)]));
    success(remote.finish());
    f.verify_observation_report(&f.genesis, &first);
    assert_eq!(first["observation_count"], 2);
    assert_eq!(first["unavailable_or_invalid_peers"], serde_json::json!([]));
    let remote = f.observe_listener(2, &f.initial_head);
    let second = json(f.observation_collect(1, &f.initial_head, &[(2, &remote.address)]));
    success(remote.finish());
    f.verify_observation_report(&f.genesis, &second);
    assert_ne!(first["request"]["context"]["nonce"], second["request"]["context"]["nonce"]);
    assert_ne!(first["responses"], second["responses"]);

    // A real TCP endpoint replays a valid prior signed remote response, not
    // a syntactically invalid or unsigned substitute.
    let cached = first["responses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            serde_json::from_value::<swarm_protocol::PeerId>(r["validator"].clone()).unwrap()
                == f.identities[2].peer_id()
        })
        .unwrap()
        .clone();
    let fake = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = fake.local_addr().unwrap().to_string();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = fake.accept().unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut size = [0; 4];
        stream.read_exact(&mut size).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(size) as usize];
        assert!(bytes.len() <= 8192);
        stream.read_exact(&mut bytes).unwrap();
        let fresh: Value = serde_json::from_slice(&bytes).unwrap();
        assert_ne!(fresh["context"]["nonce"], Value::Null);
        let bytes = serde_json::to_vec(&cached).unwrap();
        stream.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();
        stream.write_all(&bytes).unwrap();
    });
    rejected(f.observation_collect(1, &f.initial_head, &[(2, &addr)]), "missing observation majority");
    server.join().unwrap();

    // A genuinely signed response from the wrong targeted validator is
    // refused even though the signer is a member of the same roster.
    let wrong = f.observe_listener(2, &f.initial_head);
    rejected(f.observation_collect(1, &f.initial_head, &[(0, &wrong.address)]), "missing observation majority");
    success(wrong.finish());
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder);
    assert_eq!(fs::read(f.journal(1)).unwrap(), before[0]);
    assert_eq!(fs::read(f.journal(2)).unwrap(), before[1]);
    f.assert_no_live_storage();
}

#[test]
fn actual_tcp_observation_rechecks_pending_and_retired_heads_after_listen() {
    let f = Fixture::new();
    let remote = f.observe_listener(2, &f.initial_head);
    let recovery = f.propose(&f.genesis, &f.initial_head, "obs-recovery.json", &["--recover"]);
    let vote2 = f.vote(2, &f.initial_head, &recovery, &f.initial, "obs-vote2.json");
    rejected(f.observation_collect(1, &f.initial_head, &[(2, &remote.address)]), "missing observation majority");
    rejected(remote.finish(), "pending governance vote prevents observation");
    rejected(f.observation_serve(2, &f.initial_head).output().unwrap(), "pending governance vote prevents observation");
    let vote1 = f.vote(1, &f.initial_head, &recovery, &f.initial, "obs-vote1.json");
    let history = f.certified(&f.genesis, &f.initial_head, &recovery, &[&vote1, &vote2], "obs-h1.json");
    f.sync(&[1, 2], &history, &recovery.1, &f.initial, 1, 2);
    let remote = f.observe_listener(2, &recovery.1);
    let result = json(f.observation_collect(1, &recovery.1, &[(2, &remote.address)]));
    success(remote.finish());
    f.verify_observation_report(&history, &result);

    // Listener opened on an accepted head, then another majority decision
    // arrives before its first request. The old listener cannot sign.
    let remote = f.observe_listener(2, &recovery.1);
    let next = f.propose(&history, &recovery.1, "obs-next.json", &["--recover"]);
    let vote1 = f.vote(1, &recovery.1, &next, &f.initial, "obs-next-vote1.json");
    let vote2 = f.vote(2, &recovery.1, &next, &f.initial, "obs-next-vote2.json");
    let h2 = f.certified(&history, &recovery.1, &next, &[&vote1, &vote2], "obs-h2.json");
    f.sync(&[2], &h2, &next.1, &f.initial, 2, 3);
    // Founder has the prior certified head but no pending vote.
    f.sync(&[0], &history, &recovery.1, &f.initial, 1, 2);
    rejected(f.observation_collect(0, &recovery.1, &[(2, &remote.address)]), "missing observation majority");
    rejected(remote.finish(), "differs from expected pin");
    rejected(
        f.observation_collect(1, &recovery.1, &[(2, "127.0.0.1:1")]),
        "pending governance vote prevents observation",
    );
    f.sync(&[1], &h2, &next.1, &f.initial, 2, 3);
    let remote = f.observe_listener(2, &next.1);
    let result = json(f.observation_collect(1, &next.1, &[(2, &remote.address)]));
    success(remote.finish());
    f.verify_observation_report(&h2, &result);
    f.assert_no_live_storage();
}

#[test]
fn actual_tcp_observation_bounds_endpoints_frames_and_stalled_peer() {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        time::{Duration, Instant},
    };
    let f = Fixture::new();
    rejected(f.observation_collect(1, &f.initial_head, &[(3, "127.0.0.1:1")]), "not a current remote validator");
    rejected(f.observation_collect(1, &f.initial_head, &[(1, "127.0.0.1:1")]), "not a current remote validator");
    rejected(f.observation_collect(1, &f.initial_head, &[(2, "8.8.8.8:1")]), "explicit and private");
    rejected(
        f.observation_collect(1, &f.initial_head, &[(2, "127.0.0.1:1"), (2, "127.0.0.1:2")]),
        "duplicate observation validator",
    );
    rejected(
        f.observation_collect(1, &f.initial_head, &[(0, "127.0.0.1:1"), (2, "127.0.0.1:1")]),
        "duplicate observation validator",
    );
    rejected(f.observation_serve(3, &f.initial_head).output().unwrap(), "not a current ownerless validator");
    rejected(
        invoke(
            &f.profiles[2],
            &[
                "observe",
                "serve",
                "--expected-world",
                &f.world,
                "--expected-head",
                &f.initial_head,
                "--bind",
                "0.0.0.0:0",
                "--confirm",
            ],
        ),
        "explicit private or loopback",
    );
    rejected(
        invoke(
            &f.profiles[2],
            &[
                "observe",
                "serve",
                "--expected-world",
                &f.world,
                "--expected-head",
                &f.initial_head,
                "--bind",
                "127.0.0.1:0",
            ],
        ),
        "explicit --confirm",
    );

    let remote = f.observe_listener(2, &f.initial_head);
    let mut client = TcpStream::connect(&remote.address).unwrap();
    client.write_all(&8193u32.to_be_bytes()).unwrap();
    drop(client);
    rejected(remote.finish(), "invalid observation frame size");

    // A silent but connected endpoint cannot block collection of the
    // other valid remote voter. The network phase leaves time for final
    // local journal and signature verification inside the shared window.
    let remote = f.observe_listener(2, &f.initial_head);
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = silent.local_addr().unwrap().to_string();
    let stall = std::thread::spawn(move || {
        let (mut stream, _) = silent.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
        let mut size = [0; 4];
        stream.read_exact(&mut size).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(size) as usize];
        assert!(bytes.len() <= 8192);
        stream.read_exact(&mut bytes).unwrap();
        // Wait for the collector to close at its monotonic deadline.
        assert_eq!(stream.read(&mut size).unwrap(), 0);
    });
    let start = Instant::now();
    let report = json(f.observation_collect(1, &f.initial_head, &[(0, &addr), (2, &remote.address)]));
    success(remote.finish());
    stall.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(7));
    f.verify_observation_report(&f.genesis, &report);
    assert_eq!(report["observation_count"], 2);
    assert_eq!(report["unavailable_or_invalid_peers"], serde_json::json!([f.identities[0].peer_id().to_string()]));
    f.assert_no_live_storage();
}

#[test]
fn retained_network_observation_signatures_match_recovered_checkpoint() {
    use swarm_consensus::{
        ownerless_authority::verify_ownerless_history, ownerless_observation::verify_observation_majority,
    };
    use swarm_protocol::{OwnerlessObservationRequestV1, OwnerlessObservationResponseV1};
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../research/distributed-simulation/evidence/2026-11-26-ownerless-network-observation.json"
    ))
    .unwrap();
    let history = serde_json::from_value(evidence["history"].clone()).unwrap();
    let world = evidence["world_id"].as_str().unwrap().parse().unwrap();
    let head = verify_ownerless_history(world, &history).unwrap();
    let checkpoint = serde_json::to_vec(&evidence["accepted_checkpoint"]).unwrap();
    assert_eq!(ownerless_checkpoint_commitment(&checkpoint).unwrap(), head.checkpoint());
    assert_eq!(evidence["physical_machines"], 1);
    assert_eq!(evidence["founder_journal_unchanged"], true);
    assert_eq!(evidence["validator_journals_unchanged"], true);
    assert_eq!(evidence["all_listeners_exited"], true);
    assert_eq!(evidence["external_routes"], false);
    assert_eq!(evidence["partition"]["link_down"], true);
    assert_eq!(evidence["partition"]["refusal"], "missing observation majority");
    assert_eq!(head.sequence(), 1);
    assert_eq!(head.fence(), 2);
    let mut observed = Vec::new();
    for stage in ["first", "restored"] {
        let sample = &evidence[stage];
        assert_ne!(sample["collector_namespace"], sample["listener_namespace"]);
        assert_eq!(sample["listener_exitcode"], 0);
        let report = &sample["observation"];
        assert_eq!(report["head"], head.hash().to_string());
        assert_eq!(report["checkpoint"], head.checkpoint().to_string());
        assert_eq!(report["observation_count"], 2);
        assert_eq!(report["classification"], "observed_recent_ownerless_reference_majority");
        for flag in [
            "freshness_lease",
            "latest_global_head_proven",
            "live_quorum_verified",
            "write_authority",
            "canonical_authority",
            "runtime_activation",
            "player_admission",
            "worker_admission",
        ] {
            assert_eq!(report[flag], false);
        }
        let request: OwnerlessObservationRequestV1 = serde_json::from_value(report["request"].clone()).unwrap();
        let responses: Vec<OwnerlessObservationResponseV1> =
            serde_json::from_value(report["responses"].clone()).unwrap();
        verify_observation_majority(&head, &request, &responses).unwrap();
        observed.push((request, responses));
    }
    assert_ne!(observed[0].0.context.nonce, observed[1].0.context.nonce);
    assert!(verify_observation_majority(&head, &observed[1].0, &observed[0].1).is_err());
    // Timing and kernel namespace facts are retained local observations,
    // not cryptographic proof of a current live majority.
}
