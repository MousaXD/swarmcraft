#!/usr/bin/env python3
"""Local qualification of core-artifact preflight against a stopped test save.

Never launch a JVM, grant authority, or operate on a live/production world.
Use an isolated retained fixture from a real prior server test. All profiles,
JAR copies, and the captured bundle are disposable.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import zipfile


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def tree(root):
    result = {}
    for item in sorted(root.rglob("*")):
        name = item.relative_to(root).as_posix()
        if name == "session.lock":
            continue
        if item.is_symlink() or not (item.is_dir() or item.is_file()):
            raise ValueError(f"unsafe fixture entry: {name}")
        result[name] = "directory" if item.is_dir() else digest(item)
    return result


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cli", "source", "minecraft-jar", "fabric-loader-jar", "bridge-jar"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--confirm-stopped-test-fixture", required=True, action="store_true")
    args = parser.parse_args()
    os.umask(0o077)
    cli = args.cli.resolve(strict=True)
    source = args.source.resolve(strict=True)
    inputs = [args.minecraft_jar, args.fabric_loader_jar, args.bridge_jar]
    no_authority = ("canonical_authority", "write_authority", "runtime_activation", "player_admission",
                    "installed_runtime_verified", "java_runtime_verified", "complete_save_semantics_verified",
                    "save_barrier_verified", "resource_enforcement_verified", "current_grant_verified",
                    "handoff_receipt_issued", "minecraft_gameplay_verified")

    def run(profile, *parts, refusal=None):
        proc = subprocess.run([str(cli), "--data-dir", str(profile), *map(str, parts)],
                              capture_output=True, text=True, timeout=120)
        if refusal is not None:
            require(proc.returncode != 0 and not proc.stdout and refusal in proc.stderr,
                    f"unexpected refusal result: {proc.stderr}")
            return None
        require(proc.returncode == 0, proc.stderr)
        return proc.stdout

    def check_flags(report):
        require(all(report.get(flag) is False for flag in no_authority), "authority flag unexpectedly true or missing")

    with tempfile.TemporaryDirectory(prefix="sc8-core-runtime-") as tmp:
        root = Path(tmp)
        copied = root / "world"
        # Take the same POSIX record lock family as Java FileChannel. Do
        # not open/close another descriptor to session.lock in this process.
        lock_fd = os.open(source / "session.lock", os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(lock_fd, "r+b") as lock:
            require(stat.S_ISREG(os.fstat(lock.fileno()).st_mode), "unsafe fixture session.lock")
            fcntl.lockf(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            original = tree(source)
            shutil.copytree(source, copied, ignore=lambda directory, names:
                            ["session.lock"] if Path(directory) == source else [])
            require(tree(source) == original == tree(copied), "source save changed during fixture copy")
            (copied / "session.lock").write_bytes(lock.read())
        for item in [copied, *copied.rglob("*")]:
            item.chmod(0o700 if item.is_dir() else 0o600)
        jars = []
        provenance = {}
        for i, artifact in enumerate(inputs):
            dest = root / f"artifact-{i}.jar"
            before = digest(artifact)
            shutil.copyfile(artifact, dest)
            require(before == digest(dest) == digest(artifact), "artifact changed during copy")
            provenance[artifact.name] = before
            jars.append(dest)
        artifact_args = ["--minecraft-jar", jars[0], "--fabric-loader-jar", jars[1], "--bridge-jar", jars[2]]
        reviewer = root / "reviewer"
        save = lambda profile, *parts, **kw: run(profile, "exclusive-turn", "save", *parts, **kw)
        candidate = json.loads(save(reviewer, "runtime-pin", *artifact_args))
        check_flags(candidate)
        compatibility = candidate["compatibility"]
        bundle = root / "bundle"
        capture = json.loads(save(reviewer, "capture", "--source", copied, "--destination", bundle,
                                  "--compatibility", compatibility, "--confirm"))
        expected = ["--expected-manifest", capture["manifest_hash"], "--expected-state-root", capture["state_root"],
                    "--expected-compatibility", compatibility]
        offline = json.loads(save(reviewer, "verify-runtime", "--bundle", bundle, *expected, *artifact_args))
        check_flags(offline)
        require(offline["level_data_version_verified"] and offline["independent_checkpoint_pins_verified"],
                "offline preflight did not verify data version and pins")
        profiles = [root / "device-a", root / "device-b"]
        ids = [dict(line.split(": ", 1) for line in run(p, "identity").splitlines()) for p in profiles]
        draft = root / "draft.json"
        draft.write_text(save(reviewer, "draft", "--device-key", ids[0]["Public key"],
                              "--device-key", ids[1]["Public key"], "--initial-holder", ids[0]["Peer ID"],
                              "--bundle", bundle))
        review = json.loads(run(reviewer, "exclusive-turn", "inspect-draft", "--draft", draft))
        world, grant = review["world_id"], review["initial_grant"]
        approvals = [root / "approval-a.json", root / "approval-b.json"]
        for profile, approval in zip(profiles, approvals):
            approval.write_text(save(profile, "approve", "--draft", draft, "--expected-world", world,
                                     "--bundle", bundle, "--confirm"))
        history = root / "history.json"
        history.write_text(run(reviewer, "exclusive-turn", "assemble", "--draft", draft, "--expected-world", world,
                               "--approval", approvals[0], "--approval", approvals[1]))
        pins = ["--expected-world", world, "--expected-grant", grant]
        for profile in profiles:
            save(profile, "init", *pins, "--history", history, "--bundle", bundle, "--confirm")
        checkpoint = capture["reference_commitment"]
        journals = [profile / "exclusive-turn-reference" / world.removeprefix("scpass:") / "journal.postcard"
                    for profile in profiles]
        before = [digest(path) for path in journals]
        held_args = [*pins, "--expected-checkpoint", checkpoint, *artifact_args]
        held = json.loads(save(profiles[0], "verify-held-runtime", *held_args))
        check_flags(held)
        require(held["signed_compatibility_verified"] and held["local_held_reference_verified"]
                and held["level_data_version_verified"], "held preflight failed to bind signed state")
        save(profiles[1], "verify-held-runtime", *held_args, refusal="requires exact held")
        with zipfile.ZipFile(jars[2], "a") as archive:
            archive.comment = b"different exact JAR bytes, unchanged metadata"
        save(profiles[0], "verify-held-runtime", *held_args, refusal="core runtime artifact pin differs")
        require(before == [digest(path) for path in journals], "preflight mutated the journals")
        require(not reviewer.exists(), "offline review created a profile")
        for profile in profiles:
            require(not any((profile / "worlds").iterdir()), "canonical world created")
            require(not (profile / "control").exists() and not (profile / "runtime").exists(), "runtime state created")
        print(json.dumps({"classification": "actual_stopped_fixture_core_artifact_preflight",
                          "artifact_sha256": provenance, "source_entries": len(original),
                          "source_tree_sha256": hashlib.sha256(json.dumps(original, sort_keys=True).encode()).hexdigest(),
                          "candidate": candidate, "offline": offline, "held": held,
                          "replica_refused": True, "modified_artifact_refused": True,
                          "journals_unchanged": True, "no_runtime_started": True}, sort_keys=True))


if __name__ == "__main__":
    main()
