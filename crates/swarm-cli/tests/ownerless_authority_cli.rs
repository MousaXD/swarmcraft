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

    fn decision_collect(
        &self,
        proposer: usize,
        head: &str,
        p: &(PathBuf, String),
        state: &Path,
        peers: &[(usize, &str)],
    ) -> Output {
        let endpoints: Vec<_> = peers.iter().map(|(i, a)| format!("{}@{}", self.identities[*i].peer_id(), a)).collect();
        let mut args = vec![
            "decision",
            "collect",
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
        ];
        for endpoint in &endpoints {
            args.extend(["--peer", endpoint]);
        }
        invoke(&self.profiles[proposer], &args)
    }

    fn sync_serve(&self, recipient: usize, sender: usize, base: &str, head: &str) -> Command {
        cmd(
            &self.profiles[recipient],
            &[
                "decision",
                "serve-sync",
                "--expected-world",
                &self.world,
                "--expected-base",
                base,
                "--expected-head",
                head,
                "--from",
                &self.identities[sender].peer_id().to_string(),
                "--bind",
                "127.0.0.1:0",
                "--confirm",
            ],
        )
    }

    fn sync_listener(&self, recipient: usize, sender: usize, base: &str, head: &str) -> ObservationListener {
        use std::io::BufRead;
        let child = self
            .sync_serve(recipient, sender, base, head)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut listener = ObservationListener { child: Some(child), address: String::new() };
        let mut line = String::new();
        std::io::BufReader::new(listener.child.as_mut().unwrap().stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(!line.is_empty(), "sync listener failed before readiness");
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["classification"], "ownerless_reference_sync_listener");
        assert_eq!(ready["validator"], self.identities[recipient].peer_id().to_string());
        assert_eq!(ready["head"], head);
        assert_eq!(ready["runtime_activation"], false);
        listener.address = ready["address"].as_str().unwrap().to_owned();
        listener
    }

    fn sync_push(&self, sender: usize, recipient: usize, base: &str, head: &str, addr: &str) -> Output {
        invoke(
            &self.profiles[sender],
            &[
                "decision",
                "push-sync",
                "--expected-world",
                &self.world,
                "--expected-base",
                base,
                "--expected-head",
                head,
                "--peer",
                &format!("{}@{}", self.identities[recipient].peer_id(), addr),
                "--confirm",
            ],
        )
    }

    fn check_sync_receipt(&self, report: &Value, recipient: usize, base: &str, head: &str, seq: u64, fence: u64) {
        assert_eq!(report["classification"], "ownerless_reference_checkpoint_synced");
        let body = &report["receipt"]["body"];
        assert_eq!(
            body["world"],
            serde_json::to_value(self.world.parse::<swarm_protocol::OwnerlessWorldIdV1>().unwrap()).unwrap()
        );
        assert_eq!(body["base"], serde_json::to_value(base.parse::<Hash32>().unwrap()).unwrap());
        assert_eq!(body["head"], serde_json::to_value(head.parse::<Hash32>().unwrap()).unwrap());
        assert_eq!(body["recipient"], serde_json::to_value(self.identities[recipient].peer_id()).unwrap());
        assert_eq!(body["sequence"], seq);
        assert_eq!(body["fence"], fence);
        assert_eq!(
            body["checkpoint"],
            serde_json::to_value(
                json(self.status(recipient))["checkpoint"].as_str().unwrap().parse::<Hash32>().unwrap()
            )
            .unwrap()
        );
        for key in [
            "live_quorum_verified",
            "runtime_activation",
            "write_authority",
            "canonical_authority",
            "player_admission",
            "worker_admission",
        ] {
            assert_eq!(report[key], false);
        }
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

#[test]
fn encrypted_collection_and_sync_survive_lost_votes_and_receipts() {
    use std::{net::TcpListener, thread};
    let f = Fixture::new();
    let founder = fs::read(f.journal(0)).unwrap();
    let state = f.state("collect.bin", b"exact network-certified candidate");
    let p = f.propose(&f.genesis, &f.initial_head, "collect.json", &["--checkpoint", s(&state)]);
    let local_before = fs::read(f.journal(1)).unwrap();
    let remote_before = fs::read(f.journal(2)).unwrap();
    let unused = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = unused.local_addr().unwrap().to_string();
    rejected(f.decision_collect(1, &f.initial_head, &p, &state, &[(2, &addr), (2, &addr)]), "duplicate");
    assert_eq!(fs::read(f.journal(1)).unwrap(), local_before);
    drop(unused);
    // An unreachable majority locks the local vote but never accepts a head.
    rejected(f.decision_collect(1, &f.initial_head, &p, &state, &[(2, &addr)]), "missing current validator majority");
    let status = json(f.status(1));
    assert_eq!(status["head"], f.initial_head);
    assert_eq!(status["pending_proposal"], p.1);
    assert_eq!(fs::read(f.journal(2)).unwrap(), remote_before);
    let conflict = f.propose(&f.genesis, &f.initial_head, "collect-conflict.json", &["--recover"]);
    rejected(f.decision_collect(1, &f.initial_head, &conflict, &f.initial, &[(2, &addr)]), "already locked");
    let listener = f.decision_listener(2, 1, &f.initial_head, &p);
    // An optional validator's silent endpoint must not consume the final
    // local verification/commit budget when a real majority responds.
    let stalled = TcpListener::bind("127.0.0.1:0").unwrap();
    let stalled_addr = stalled.local_addr().unwrap().to_string();
    let accepted =
        success(f.decision_collect(1, &f.initial_head, &p, &state, &[(2, &listener.address), (0, &stalled_addr)]));
    drop(stalled);
    success(listener.finish());
    let h = f.path("collected.json");
    fs::write(&h, &accepted).unwrap();
    assert_eq!(json(f.status(1))["head"], p.1);
    assert_eq!(json(f.status(1))["pending_proposal"], Value::Null);
    assert_eq!(json(f.status(2))["pending_proposal"], p.1);
    let committed = fs::read(f.journal(1)).unwrap();
    // No peer reachable; exact local certificate recovery is idempotent.
    assert_eq!(success(f.decision_collect(1, &f.initial_head, &p, &state, &[(2, &addr)])), accepted);
    assert_eq!(fs::read(f.journal(1)).unwrap(), committed);
    let remote_pending = fs::read(f.journal(2)).unwrap();
    // Corrupt checkpoint data during sync: the receiving journal must not change.
    let listener = f.sync_listener(2, 1, &f.initial_head, &p.1);
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_addr = proxy.local_addr().unwrap().to_string();
    let target = listener.address.clone();
    let relay = thread::spawn(move || decision_proxy(proxy, target, 1, true, false));
    assert!(!f.sync_push(1, 2, &f.initial_head, &p.1, &proxy_addr).status.success());
    assert!(!listener.finish().status.success());
    relay.join().unwrap();
    assert_eq!(fs::read(f.journal(2)).unwrap(), remote_pending);
    // Lose the real durable publication receipt; retry from the same base.
    let listener = f.sync_listener(2, 1, &f.initial_head, &p.1);
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_addr = proxy.local_addr().unwrap().to_string();
    let target = listener.address.clone();
    let relay = thread::spawn(move || decision_proxy(proxy, target, 1, false, true));
    assert!(!f.sync_push(1, 2, &f.initial_head, &p.1, &proxy_addr).status.success());
    success(listener.finish());
    relay.join().unwrap();
    assert_eq!(json(f.status(2))["head"], p.1);
    assert_eq!(json(f.status(2))["pending_proposal"], Value::Null);
    let next = f.propose(&h, &p.1, "next-recover.json", &["--recover"]);
    f.vote(2, &p.1, &next, &state, "next-pending.json");
    let remote_committed = fs::read(f.journal(2)).unwrap();
    let listener = f.sync_listener(2, 1, &f.initial_head, &p.1);
    let receipt = json(f.sync_push(1, 2, &f.initial_head, &p.1, &listener.address));
    success(listener.finish());
    f.check_sync_receipt(&receipt, 2, &f.initial_head, &p.1, 1, 1);
    assert_eq!(fs::read(f.journal(2)).unwrap(), remote_committed);
    assert_eq!(json(f.status(2))["pending_proposal"], next.1);
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder);
    f.assert_no_live_storage();
}

#[test]
fn encrypted_collection_and_sync_enforce_joint_membership_and_exact_history() {
    let f = Fixture::new();
    let keys: Vec<_> = [1, 2, 3].iter().map(|&i| hex::encode(f.identities[i].public_key())).collect();
    let p = f.propose(
        &f.genesis,
        &f.initial_head,
        "joint-net.json",
        &["--validator-key", &keys[0], "--validator-key", &keys[1], "--validator-key", &keys[2]],
    );
    let before = fs::read(f.journal(1)).unwrap();
    rejected(
        f.decision_collect(1, &f.initial_head, &p, &f.initial, &[(2, "127.0.0.1:1")]),
        "missing configured entrant",
    );
    assert_eq!(fs::read(f.journal(1)).unwrap(), before);
    let b = f.decision_listener(2, 1, &f.initial_head, &p);
    let entrant = f.decision_listener(3, 1, &f.initial_head, &p);
    let h = f.path("joint-net-history.json");
    fs::write(
        &h,
        success(f.decision_collect(1, &f.initial_head, &p, &f.initial, &[(2, &b.address), (3, &entrant.address)])),
    )
    .unwrap();
    success(b.finish());
    success(entrant.finish());
    assert_eq!(json(f.status(1))["reference_fence"], 2);
    // Neither the departed founder nor an unrelated observer can receive
    // a certified checkpoint from the new accepted roster.
    for recipient in [0, 4] {
        rejected(f.sync_push(1, recipient, &f.initial_head, &p.1, "127.0.0.1:1"), "recipient not in target roster");
    }
    let listener = f.sync_listener(3, 1, &f.initial_head, &p.1);
    let receipt = json(f.sync_push(1, 3, &f.initial_head, &p.1, &listener.address));
    success(listener.finish());
    f.check_sync_receipt(&receipt, 3, &f.initial_head, &p.1, 1, 2);
    // Advance using the newly enrolled validator; peer 2 remains at genesis
    // with the joint vote pending until the full certified suffix arrives.
    let state = f.state("post-joint.bin", b"new accepted roster checkpoint");
    let p2 = f.propose(&h, &p.1, "post-joint.json", &["--checkpoint", s(&state)]);
    let listener = f.decision_listener(3, 1, &p.1, &p2);
    success(f.decision_collect(1, &p.1, &p2, &state, &[(3, &listener.address)]));
    success(listener.finish());
    let listener = f.sync_listener(2, 1, &f.initial_head, &p2.1);
    let receipt = json(f.sync_push(1, 2, &f.initial_head, &p2.1, &listener.address));
    success(listener.finish());
    f.check_sync_receipt(&receipt, 2, &f.initial_head, &p2.1, 2, 2);
    assert_eq!(json(f.status(2))["pending_proposal"], Value::Null);
    rejected(f.sync_serve(2, 1, &f.initial_head, &p.1).output().unwrap(), "local head is not pinned base or target");
    rejected(f.sync_push(1, 3, &f.initial_head, &p.1, "127.0.0.1:1"), "local head is not pinned base or target");
    f.assert_no_live_storage();
}

impl Fixture {
    fn recovery_serve(&self, responder: usize, requester: usize, base: &str, p: &(PathBuf, String)) -> Command {
        cmd(
            &self.profiles[responder],
            &[
                "decision",
                "serve-recovery",
                "--expected-world",
                &self.world,
                "--expected-head",
                base,
                "--proposal",
                s(&p.0),
                "--expected-proposal",
                &p.1,
                "--from",
                &self.identities[requester].peer_id().to_string(),
                "--bind",
                "127.0.0.1:0",
                "--confirm",
            ],
        )
    }

    fn recovery_listener(
        &self,
        responder: usize,
        requester: usize,
        base: &str,
        p: &(PathBuf, String),
    ) -> ObservationListener {
        use std::io::BufRead;
        let child = self
            .recovery_serve(responder, requester, base, p)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut listener = ObservationListener { child: Some(child), address: String::new() };
        let mut line = String::new();
        std::io::BufReader::new(listener.child.as_mut().unwrap().stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(!line.is_empty(), "recovery listener failed before readiness");
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["classification"], "ownerless_reference_recovery_listener");
        assert_eq!(ready["validator"], self.identities[responder].peer_id().to_string());
        assert_eq!(ready["proposal"], p.1);
        assert_eq!(ready["runtime_activation"], false);
        listener.address = ready["address"].as_str().unwrap().to_owned();
        listener
    }

    fn decision_recover(&self, requester: usize, base: &str, p: &(PathBuf, String), peers: &[(usize, &str)]) -> Output {
        let endpoints: Vec<_> = peers.iter().map(|(i, a)| format!("{}@{}", self.identities[*i].peer_id(), a)).collect();
        let mut args = vec![
            "decision",
            "recover",
            "--expected-world",
            &self.world,
            "--expected-head",
            base,
            "--proposal",
            s(&p.0),
            "--expected-proposal",
            &p.1,
            "--confirm",
        ];
        for endpoint in &endpoints {
            args.extend(["--peer", endpoint]);
        }
        invoke(&self.profiles[requester], &args)
    }
}

#[test]
fn encrypted_recovery_reconstructs_certificate_after_collector_loss() {
    use std::net::TcpListener;
    let f = Fixture::new();
    let state = f.state("recover-candidate.bin", b"recoverable checkpoint without collector");
    let p = f.propose(&f.genesis, &f.initial_head, "collector-loss.json", &["--checkpoint", s(&state)]);
    // Before any votes exist, recovery cannot create one.
    let before = fs::read(f.journal(1)).unwrap();
    rejected(f.recovery_serve(1, 2, &f.initial_head, &p).output().unwrap(), "no matching durable recovery evidence");
    assert_eq!(fs::read(f.journal(1)).unwrap(), before);
    let a = f.decision_listener(1, 0, &f.initial_head, &p);
    let b = f.decision_listener(2, 0, &f.initial_head, &p);
    success(f.decision_collect(0, &f.initial_head, &p, &state, &[(1, &a.address), (2, &b.address)]));
    success(a.finish());
    success(b.finish());
    // Founder/collector is never contacted again.
    let founder = fs::read(f.journal(0)).unwrap();
    let a_pending = fs::read(f.journal(1)).unwrap();
    let b_pending = fs::read(f.journal(2)).unwrap();
    let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = unavailable.local_addr().unwrap().to_string();
    drop(unavailable);
    rejected(f.decision_recover(1, &f.initial_head, &p, &[(2, &addr)]), "missing current validator majority");
    assert_eq!(fs::read(f.journal(1)).unwrap(), a_pending);
    let listener = f.recovery_listener(2, 1, &f.initial_head, &p);
    // A live endpoint with the wrong pinned identity cannot provide evidence.
    rejected(
        f.decision_recover(1, &f.initial_head, &p, &[(0, &listener.address)]),
        "missing current validator majority",
    );
    assert!(!listener.finish().status.success());
    assert_eq!(fs::read(f.journal(1)).unwrap(), a_pending);
    assert_eq!(fs::read(f.journal(2)).unwrap(), b_pending);
    let listener = f.recovery_listener(2, 1, &f.initial_head, &p);
    let accepted = success(f.decision_recover(1, &f.initial_head, &p, &[(2, &listener.address)]));
    success(listener.finish());
    let history: swarm_protocol::OwnerlessAuthorityHistoryV1 = serde_json::from_slice(&accepted).unwrap();
    let votes = &history.records.last().unwrap().votes;
    assert_eq!(votes.len(), 2);
    for i in [1, 2] {
        assert!(votes.iter().any(|v| v.validator == f.identities[i].peer_id()));
    }
    assert_eq!(json(f.status(1))["head"], p.1);
    assert_eq!(json(f.status(1))["pending_proposal"], Value::Null);
    assert_eq!(fs::read(f.journal(2)).unwrap(), b_pending);
    let accepted_journal = fs::read(f.journal(1)).unwrap();
    assert_eq!(success(f.decision_recover(1, &f.initial_head, &p, &[(2, &addr)])), accepted);
    assert_eq!(fs::read(f.journal(1)).unwrap(), accepted_journal);
    let listener = f.sync_listener(2, 1, &f.initial_head, &p.1);
    let receipt = json(f.sync_push(1, 2, &f.initial_head, &p.1, &listener.address));
    success(listener.finish());
    f.check_sync_receipt(&receipt, 2, &f.initial_head, &p.1, 1, 1);
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder);
    f.assert_no_live_storage();
}

#[test]
fn encrypted_recovery_accepts_surviving_certificate_without_fresh_votes() {
    let f = Fixture::new();
    let state = f.state("certified-candidate.bin", b"surviving certified checkpoint");
    let p = f.propose(&f.genesis, &f.initial_head, "surviving-certificate.json", &["--checkpoint", s(&state)]);
    let conflict = f.propose(&f.genesis, &f.initial_head, "different-pending.json", &["--recover"]);
    f.vote(2, &f.initial_head, &conflict, &f.initial, "conflicting-vote.json");
    let listener = f.decision_listener(1, 0, &f.initial_head, &p);
    let original = success(f.decision_collect(0, &f.initial_head, &p, &state, &[(1, &listener.address)]));
    success(listener.finish());
    let listener = f.sync_listener(1, 0, &f.initial_head, &p.1);
    success(f.sync_push(0, 1, &f.initial_head, &p.1, &listener.address));
    success(listener.finish());
    let founder = fs::read(f.journal(0)).unwrap();
    let survivor = fs::read(f.journal(1)).unwrap();
    let pending = fs::read(f.journal(2)).unwrap();
    // A wrong proposal cannot be retrieved from the certified profile.
    rejected(
        f.recovery_serve(1, 2, &f.initial_head, &conflict).output().unwrap(),
        "local head is not pinned base or target",
    );
    assert_eq!(fs::read(f.journal(2)).unwrap(), pending);
    let listener = f.recovery_listener(1, 2, &f.initial_head, &p);
    let recovered = success(f.decision_recover(2, &f.initial_head, &p, &[(1, &listener.address)]));
    success(listener.finish());
    assert_eq!(recovered, original);
    assert_eq!(json(f.status(2))["head"], p.1);
    assert_eq!(json(f.status(2))["pending_proposal"], Value::Null);
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder);
    assert_eq!(fs::read(f.journal(1)).unwrap(), survivor);
    f.assert_no_live_storage();
}

#[test]
fn encrypted_recovery_requires_joint_majority_and_every_entrant() {
    let f = Fixture::new();
    let keys: Vec<_> = [1, 2, 3].iter().map(|&i| hex::encode(f.identities[i].public_key())).collect();
    let p = f.propose(
        &f.genesis,
        &f.initial_head,
        "recovery-joint.json",
        &["--validator-key", &keys[0], "--validator-key", &keys[1], "--validator-key", &keys[2]],
    );
    let a = f.decision_listener(0, 1, &f.initial_head, &p);
    let b = f.decision_listener(2, 1, &f.initial_head, &p);
    let entrant = f.decision_listener(3, 1, &f.initial_head, &p);
    success(f.decision_collect(
        1,
        &f.initial_head,
        &p,
        &f.initial,
        &[(0, &a.address), (2, &b.address), (3, &entrant.address)],
    ));
    success(a.finish());
    success(b.finish());
    success(entrant.finish());
    let collector = fs::read(f.journal(1)).unwrap();
    let pending = fs::read(f.journal(2)).unwrap();
    rejected(f.recovery_serve(3, 0, &f.initial_head, &p).output().unwrap(), "requester must remain in target roster");
    rejected(
        f.decision_recover(0, &f.initial_head, &p, &[(2, "127.0.0.1:1")]),
        "requester must remain in target roster",
    );
    let entrant = f.recovery_listener(3, 2, &f.initial_head, &p);
    rejected(
        f.decision_recover(2, &f.initial_head, &p, &[(3, &entrant.address)]),
        "missing current validator majority",
    );
    success(entrant.finish());
    assert_eq!(fs::read(f.journal(2)).unwrap(), pending);
    let a = f.recovery_listener(0, 2, &f.initial_head, &p);
    rejected(f.decision_recover(2, &f.initial_head, &p, &[(0, &a.address)]), "missing proposed validator majority");
    success(a.finish());
    assert_eq!(fs::read(f.journal(2)).unwrap(), pending);
    let a = f.recovery_listener(0, 2, &f.initial_head, &p);
    let entrant = f.recovery_listener(3, 2, &f.initial_head, &p);
    success(f.decision_recover(2, &f.initial_head, &p, &[(0, &a.address), (3, &entrant.address)]));
    success(a.finish());
    success(entrant.finish());
    assert_eq!(json(f.status(2))["head"], p.1);
    assert_eq!(json(f.status(2))["reference_fence"], 2);
    assert_eq!(fs::read(f.journal(1)).unwrap(), collector);
    let listener = f.sync_listener(3, 2, &f.initial_head, &p.1);
    success(f.sync_push(2, 3, &f.initial_head, &p.1, &listener.address));
    success(listener.finish());
    rejected(f.sync_push(2, 0, &f.initial_head, &p.1, "127.0.0.1:1"), "recipient not in target roster");
    f.assert_no_live_storage();
}

impl Fixture {
    fn head_serve(&self, responder: usize, requester: usize, base: &str) -> Command {
        cmd(
            &self.profiles[responder],
            &[
                "decision",
                "serve-head",
                "--expected-world",
                &self.world,
                "--expected-base",
                base,
                "--from",
                &self.identities[requester].peer_id().to_string(),
                "--bind",
                "127.0.0.1:0",
                "--confirm",
            ],
        )
    }

    fn head_listener(&self, responder: usize, requester: usize, base: &str) -> ObservationListener {
        use std::io::BufRead;
        let child =
            self.head_serve(responder, requester, base).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut listener = ObservationListener { child: Some(child), address: String::new() };
        let mut line = String::new();
        std::io::BufReader::new(listener.child.as_mut().unwrap().stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(!line.is_empty(), "certified head listener failed before readiness");
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["classification"], "ownerless_reference_head_listener");
        assert_eq!(ready["validator"], self.identities[responder].peer_id().to_string());
        assert_eq!(ready["base"], base);
        assert_eq!(ready["runtime_activation"], false);
        listener.address = ready["address"].as_str().unwrap().to_owned();
        listener
    }

    fn pull_head_cmd(&self, requester: usize, base: &str, peers: &[(usize, &str)]) -> Command {
        let mut command = cmd(
            &self.profiles[requester],
            &["decision", "pull-head", "--expected-world", &self.world, "--expected-base", base, "--confirm"],
        );
        for (peer, addr) in peers {
            command.args(["--peer", &format!("{}@{}", self.identities[*peer].peer_id(), addr)]);
        }
        command
    }

    fn pull_head(&self, requester: usize, base: &str, peers: &[(usize, &str)]) -> Output {
        self.pull_head_cmd(requester, base, peers).output().unwrap()
    }

    fn check_pulled_head(&self, result: &Value, base: &str, head: &str, sequence: u64, advanced: bool) {
        assert_eq!(result["classification"], "ownerless_reference_head_pulled");
        assert_eq!(result["base"], base);
        assert_eq!(result["head"], head);
        assert_eq!(result["sequence"], sequence);
        assert_eq!(result["advanced"], advanced);
        for key in [
            "live_quorum_verified",
            "globally_latest_verified",
            "runtime_activation",
            "canonical_authority",
            "write_authority",
            "player_admission",
            "worker_admission",
            "v1_canonical_world_created",
        ] {
            assert_eq!(result[key], false, "{key}");
        }
    }
}

#[test]
fn encrypted_head_discovery_selects_certified_suffix_and_preserves_same_head_vote() {
    let f = Fixture::new();
    let state1 = f.state("discovery-first.bin", b"first certified discovery checkpoint");
    let p1 = f.propose(&f.genesis, &f.initial_head, "discovery-first.json", &["--checkpoint", s(&state1)]);
    let conflict = f.propose(&f.genesis, &f.initial_head, "discovery-pending.json", &["--recover"]);
    f.vote(2, &f.initial_head, &conflict, &f.initial, "discovery-pending-vote.json");
    let a = f.vote(0, &f.initial_head, &p1, &state1, "discovery-first-a.json");
    let b = f.vote(1, &f.initial_head, &p1, &state1, "discovery-first-b.json");
    let h1 = f.certified(&f.genesis, &f.initial_head, &p1, &[&a, &b], "discovery-h1.json");
    f.sync(&[0, 1], &h1, &p1.1, &state1, 1, 1);
    let state2 = f.state("discovery-second.bin", b"second certified discovery checkpoint");
    let p2 = f.propose(&h1, &p1.1, "discovery-second.json", &["--checkpoint", s(&state2)]);
    let a = f.vote(0, &p1.1, &p2, &state2, "discovery-second-a.json");
    let b = f.vote(1, &p1.1, &p2, &state2, "discovery-second-b.json");
    let h2 = f.certified(&h1, &p1.1, &p2, &[&a, &b], "discovery-h2.json");
    f.sync(&[1], &h2, &p2.1, &state2, 2, 1);
    let founder = fs::read(f.journal(0)).unwrap();
    let survivor = fs::read(f.journal(1)).unwrap();
    let pending = fs::read(f.journal(2)).unwrap();
    let a = f.head_listener(0, 2, &f.initial_head);
    let b = f.head_listener(1, 2, &f.initial_head);
    // No target hash or proposal file provided. Both heads are certified,
    // but only the longest compatible suffix should be installed.
    let result = json(f.pull_head(2, &f.initial_head, &[(0, &a.address), (1, &b.address)]));
    success(a.finish());
    success(b.finish());
    f.check_pulled_head(&result, &f.initial_head, &p2.1, 2, true);
    assert_eq!(result["responded"].as_array().unwrap().len(), 2);
    assert_eq!(result["pending_proposal"], Value::Null);
    assert_ne!(fs::read(f.journal(2)).unwrap(), pending);
    let copy = f.path("discovered-checkpoint.bin");
    success(invoke(
        &f.profiles[2],
        &["recovery-copy", "--expected-world", &f.world, "--expected-head", &p2.1, "--output", s(&copy)],
    ));
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&state2).unwrap());
    let next = f.propose(&h2, &p2.1, "discovery-next.json", &["--recover"]);
    f.vote(2, &p2.1, &next, &state2, "discovery-next-vote.json");
    let committed = fs::read(f.journal(2)).unwrap();
    rejected(f.pull_head(2, &f.initial_head, &[(1, "127.0.0.1:1")]), "differs from expected pin");
    let b = f.head_listener(1, 2, &p2.1);
    let result = json(f.pull_head(2, &p2.1, &[(1, &b.address)]));
    success(b.finish());
    f.check_pulled_head(&result, &p2.1, &p2.1, 2, false);
    assert_eq!(result["pending_proposal"], next.1);
    assert_eq!(fs::read(f.journal(2)).unwrap(), committed);
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder);
    assert_eq!(fs::read(f.journal(1)).unwrap(), survivor);
    f.assert_no_live_storage();
}

#[test]
fn encrypted_head_discovery_refuses_wrong_identity_and_retirement_during_handshake() {
    let f = Fixture::new();
    let before = fs::read(f.journal(2)).unwrap();
    rejected(f.pull_head(2, &f.initial_head, &[(1, "127.0.0.1:1"), (1, "127.0.0.1:2")]), "duplicate");
    rejected(f.pull_head(2, &f.initial_head, &[(0, "127.0.0.1:1"), (1, "127.0.0.1:1")]), "duplicate");
    rejected(f.pull_head(2, &f.initial_head, &[(3, "127.0.0.1:1")]), "current validator");
    rejected(f.pull_head(3, &f.initial_head, &[(1, "127.0.0.1:1")]), "current validator");
    rejected(f.head_serve(1, 3, &f.initial_head).output().unwrap(), "current validator");
    let listener = f.head_listener(1, 2, &f.initial_head);
    rejected(f.pull_head(2, &f.initial_head, &[(0, &listener.address)]), "no verified certified head response");
    assert!(!listener.finish().status.success());
    // A once-eligible requester is retired while the listener is ready.
    let listener = f.head_listener(1, 2, &f.initial_head);
    let keys: Vec<_> = [0, 1, 3].iter().map(|&i| hex::encode(f.identities[i].public_key())).collect();
    let joint = f.propose(
        &f.genesis,
        &f.initial_head,
        "discovery-joint.json",
        &["--validator-key", &keys[0], "--validator-key", &keys[1], "--validator-key", &keys[2]],
    );
    let a = f.vote(0, &f.initial_head, &joint, &f.initial, "discovery-joint-a.json");
    let b = f.vote(1, &f.initial_head, &joint, &f.initial, "discovery-joint-b.json");
    let c = f.vote(3, &f.initial_head, &joint, &f.initial, "discovery-joint-c.json");
    let h = f.certified(&f.genesis, &f.initial_head, &joint, &[&a, &b, &c], "discovery-joint-history.json");
    f.sync(&[1, 3], &h, &joint.1, &f.initial, 1, 2);
    rejected(f.pull_head(2, &f.initial_head, &[(1, &listener.address)]), "no verified certified head response");
    rejected(listener.finish(), "retired or not in target roster");
    rejected(f.head_serve(1, 2, &f.initial_head).output().unwrap(), "retired or not in target roster");
    rejected(f.head_serve(3, 0, &f.initial_head).output().unwrap(), "current validator");
    assert_eq!(fs::read(f.journal(2)).unwrap(), before);
    f.assert_no_live_storage();
}

#[test]
fn encrypted_head_discovery_refuses_conflicting_certified_chains() {
    use swarm_protocol::{
        OwnerlessAuthorityCertificateV1, OwnerlessAuthorityHistoryV1, OwnerlessAuthorityProposalV1,
        OwnerlessAuthorityVoteV1,
    };
    let f = Fixture::new();
    let state = f.state("discovery-fork.bin", b"test only conflicting certified checkpoint");
    let a = f.propose(&f.genesis, &f.initial_head, "discovery-fork-a.json", &["--checkpoint", s(&state)]);
    let b = f.propose(&f.genesis, &f.initial_head, "discovery-fork-b.json", &["--recover"]);
    // Deliberately bypass the honest-voter journal ONLY in this adversarial
    // fixture. Conflicting majority certificates cannot be produced by
    // compliant durable voting, but valid signatures alone cannot rule
    // out a cloned/compromised profile. Pull must not choose either chain.
    for (i, proposal, bytes) in [(0, &a, &state), (1, &b, &f.initial)] {
        let document: Value = serde_json::from_slice(&fs::read(&proposal.0).unwrap()).unwrap();
        let p: OwnerlessAuthorityProposalV1 = serde_json::from_value(document["proposal"].clone()).unwrap();
        let mut votes: Vec<_> = [0, 1]
            .iter()
            .map(|&v| {
                let signer = &f.identities[v];
                let validator = signer.peer_id();
                OwnerlessAuthorityVoteV1 {
                    proposal_hash: p.proposal_hash().unwrap(),
                    validator,
                    signature: signer.sign(&p.vote_bytes(validator).unwrap()),
                }
            })
            .collect();
        votes.sort_by_key(|v| v.validator);
        let mut history: OwnerlessAuthorityHistoryV1 = serde_json::from_slice(&fs::read(&f.genesis).unwrap()).unwrap();
        history.records.push(OwnerlessAuthorityCertificateV1 { proposal: p, votes });
        let path = f.path(&format!("discovery-conflict-{i}.json"));
        fs::write(&path, serde_json::to_vec(&history).unwrap()).unwrap();
        success(f.accept(i, &path, &proposal.1, bytes));
    }
    let before = fs::read(f.journal(2)).unwrap();
    let a = f.head_listener(0, 2, &f.initial_head);
    let b = f.head_listener(1, 2, &f.initial_head);
    rejected(
        f.pull_head(2, &f.initial_head, &[(0, &a.address), (1, &b.address)]),
        "conflicting certified discovery heads",
    );
    success(a.finish());
    success(b.finish());
    assert_eq!(fs::read(f.journal(2)).unwrap(), before);
    f.assert_no_live_storage();
}

// Forward the actual framed Noise exchange without possessing either
// identity or any transport key. The fixture's history is one chunk.
fn discovery_proxy(
    listener: std::net::TcpListener,
    target: String,
    checkpoint_chunks: usize,
    corrupt: bool,
    before_last: impl FnOnce(),
) -> Vec<u8> {
    use std::{
        io::{Read, Write},
        net::TcpStream,
        time::Duration,
    };
    fn frame(from: &mut TcpStream, captured: &mut Vec<u8>) -> (Vec<u8>, Vec<u8>) {
        let mut header = vec![0; 4];
        from.read_exact(&mut header).unwrap();
        let size = u32::from_be_bytes(header.as_slice().try_into().unwrap()) as usize;
        assert!((1..=32784).contains(&size));
        let mut bytes = vec![0; size];
        from.read_exact(&mut bytes).unwrap();
        captured.extend_from_slice(&header);
        captured.extend_from_slice(&bytes);
        (header, bytes)
    }
    fn relay(from: &mut TcpStream, to: &mut TcpStream, captured: &mut Vec<u8>) {
        let (header, bytes) = frame(from, captured);
        to.write_all(&header).unwrap();
        to.write_all(&bytes).unwrap();
    }
    let (mut client, _) = listener.accept().unwrap();
    let mut server = TcpStream::connect(target).unwrap();
    for stream in [&client, &server] {
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.set_nodelay(true).unwrap();
    }
    let mut capture = Vec::new();
    relay(&mut client, &mut server, &mut capture); // Noise first message
    relay(&mut server, &mut client, &mut capture); // Noise second message
    relay(&mut server, &mut client, &mut capture); // responder auth
    relay(&mut client, &mut server, &mut capture); // initiator auth
    for _ in 0..3 {
        relay(&mut server, &mut client, &mut capture); // history length/data, checkpoint length
    }
    for _ in 1..checkpoint_chunks {
        relay(&mut server, &mut client, &mut capture);
    }
    let (header, mut bytes) = frame(&mut server, &mut capture);
    before_last();
    if corrupt {
        bytes[0] ^= 1;
    }
    client.write_all(&header).unwrap();
    client.write_all(&bytes).unwrap();
    capture
}

#[test]
fn encrypted_head_discovery_rejects_corruption_and_concurrent_local_advance() {
    use std::{net::TcpListener, sync::mpsc, thread, time::Duration};
    let f = Fixture::new();
    let marker = b"private discovery checkpoint plaintext sentinel";
    let mut contents = vec![42; swarm_protocol::MAX_OWNERLESS_CHECKPOINT_BYTES];
    contents[..marker.len()].copy_from_slice(marker);
    let state = f.state("discovery-max.bin", &contents);
    let p = f.propose(&f.genesis, &f.initial_head, "discovery-max.json", &["--checkpoint", s(&state)]);
    let a = f.vote(0, &f.initial_head, &p, &state, "discovery-max-a.json");
    let b = f.vote(1, &f.initial_head, &p, &state, "discovery-max-b.json");
    let history = f.certified(&f.genesis, &f.initial_head, &p, &[&a, &b], "discovery-max-history.json");
    assert!(fs::metadata(&history).unwrap().len() < 32768);
    f.sync(&[1], &history, &p.1, &state, 1, 1);
    let before = fs::read(f.journal(2)).unwrap();
    let responder = fs::read(f.journal(1)).unwrap();
    let founder = fs::read(f.journal(0)).unwrap();
    let listener = f.head_listener(1, 2, &f.initial_head);
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = proxy.local_addr().unwrap().to_string();
    let target = listener.address.clone();
    let relay = thread::spawn(move || discovery_proxy(proxy, target, 32, true, || {}));
    rejected(f.pull_head(2, &f.initial_head, &[(1, &address)]), "no verified certified head response");
    // The responder only sent historical bytes; it cannot claim that the
    // recipient accepted them, even if its own transport writes succeeded.
    success(listener.finish());
    let capture = relay.join().unwrap();
    assert!(!capture.windows(marker.len()).any(|window| window == marker));
    assert_eq!(fs::read(f.journal(2)).unwrap(), before);

    let listener = f.head_listener(1, 2, &f.initial_head);
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = proxy.local_addr().unwrap().to_string();
    let target = listener.address.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let relay = thread::spawn(move || {
        discovery_proxy(proxy, target, 32, false, || {
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        })
    });
    let mut command = f.pull_head_cmd(2, &f.initial_head, &[(1, &address)]);
    let requester = thread::spawn(move || command.output().unwrap());
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    // Complete the same certified decision locally while the remote
    // checkpoint's final frame is withheld. A stale pull must not
    // replace that local journal or a pending vote for its successor.
    f.sync(&[2], &history, &p.1, &state, 1, 1);
    let next = f.propose(&history, &p.1, "discovery-race-next.json", &["--recover"]);
    f.vote(2, &p.1, &next, &state, "discovery-race-next-vote.json");
    let committed = fs::read(f.journal(2)).unwrap();
    release_tx.send(()).unwrap();
    relay.join().unwrap();
    rejected(requester.join().unwrap(), "discovery requester base or identity changed");
    success(listener.finish());
    assert_eq!(fs::read(f.journal(2)).unwrap(), committed);
    assert_eq!(fs::read(f.journal(1)).unwrap(), responder);
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder);
    f.assert_no_live_storage();
}

#[test]
fn encrypted_head_discovery_accepts_surviving_certificate_with_silent_optional_peer() {
    use std::net::TcpListener;
    let f = Fixture::new();
    let p = f.propose(&f.genesis, &f.initial_head, "discovery-survivor.json", &["--recover"]);
    let a = f.vote(0, &f.initial_head, &p, &f.initial, "discovery-survivor-a.json");
    let b = f.vote(1, &f.initial_head, &p, &f.initial, "discovery-survivor-b.json");
    let h = f.certified(&f.genesis, &f.initial_head, &p, &[&a, &b], "discovery-survivor-history.json");
    f.sync(&[1], &h, &p.1, &f.initial, 1, 2);
    let founder = fs::read(f.journal(0)).unwrap();
    let survivor = fs::read(f.journal(1)).unwrap();
    let b = f.head_listener(1, 2, &f.initial_head);
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = silent.local_addr().unwrap().to_string();
    let result = json(f.pull_head(2, &f.initial_head, &[(0, &address), (1, &b.address)]));
    drop(silent);
    success(b.finish());
    f.check_pulled_head(&result, &f.initial_head, &p.1, 1, true);
    assert_eq!(result["reference_fence"], 2);
    assert_eq!(result["responded"], serde_json::json!([f.identities[1].peer_id().to_string()]));
    assert_eq!(result["unavailable"], serde_json::json!([f.identities[0].peer_id().to_string()]));
    assert_eq!(fs::read(f.journal(0)).unwrap(), founder);
    assert_eq!(fs::read(f.journal(1)).unwrap(), survivor);
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
