#!/usr/bin/env python3
"""Qualify the exact offline package pin using a retained, stopped test server.

Copy only the supported package layout; deliberately omit the test-only
permission probe, old generated Fabric cache, and server configuration.
No JVM is executed. Never use a live or production save for this gate.
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

from verify_core_runtime import digest, require, tree


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cli", "server-fixture", "toolchain-fixture"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--confirm-stopped-test-fixture", required=True, action="store_true")
    args = parser.parse_args()
    os.umask(0o077)
    cli = args.cli.resolve(strict=True)
    server = args.server_fixture.resolve(strict=True)
    toolchain = args.toolchain_fixture.resolve(strict=True)
    source = server / "world"
    no_authority = ("canonical_authority", "write_authority", "runtime_activation", "player_admission",
                    "installed_runtime_verified", "java_runtime_verified", "complete_save_semantics_verified",
                    "save_barrier_verified", "resource_enforcement_verified", "current_grant_verified",
                    "handoff_receipt_issued", "minecraft_gameplay_verified", "artifact_publisher_verified",
                    "java_executed")

    def run(profile, *parts, refusal=None):
        proc = subprocess.run([str(cli), "--data-dir", str(profile), *map(str, parts)],
                              capture_output=True, text=True, timeout=180)
        if refusal is not None:
            require(proc.returncode != 0 and not proc.stdout and refusal in proc.stderr,
                    f"expected {refusal!r}, got {proc.stderr!r}")
            return None
        require(proc.returncode == 0, proc.stderr)
        return proc.stdout

    def check(report):
        require(all(report.get(flag) is False for flag in no_authority), "unexpected positive authority claim")
        require(report["package_inventory_verified"] and report["declared_dependency_files_verified"], "package not verified")

    with tempfile.TemporaryDirectory(prefix="sc8-runtime-package-") as tmp:
        root = Path(tmp)
        package = root / "package"
        package.mkdir()
        for original, dest in ((toolchain / "jdk", "java"), (toolchain / "loader-libs", "fabric-libraries"),
                               (server / "versions", "versions"), (server / "libraries", "libraries")):
            shutil.copytree(original, package / dest, symlinks=True)
        for original, dest in ((server / "server.jar", "server.jar"),
                               (toolchain / "fabric-server-profile.json", "fabric-profile.json"),
                               (server / "mods/swarmcraft-fabric-0.5.1.jar", "mods/swarmcraft-fabric.jar")):
            target = package / dest
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(original, target)
            require(digest(original) == digest(target), "package copy changed")
        for item in [package, *package.rglob("*")]:
            if item.is_symlink():
                continue
            item.chmod(0o700 if item.is_dir() else 0o600 | (item.stat().st_mode & 0o111))
        copied = root / "world"
        lock_fd = os.open(source / "session.lock", os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(lock_fd, "r+b") as lock:
            require(stat.S_ISREG(os.fstat(lock.fileno()).st_mode), "unsafe fixture lock")
            fcntl.lockf(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            original = tree(source)
            shutil.copytree(source, copied, ignore=lambda directory, names: ["session.lock"] if Path(directory) == source else [])
            require(original == tree(source) == tree(copied), "save changed during copy")
            (copied / "session.lock").write_bytes(lock.read())
        for item in [copied, *copied.rglob("*")]:
            item.chmod(0o700 if item.is_dir() else 0o600)
        reviewer = root / "reviewer"
        save = lambda profile, *parts, **kw: run(profile, "exclusive-turn", "save", *parts, **kw)
        candidate = json.loads(save(reviewer, "runtime-package-pin", "--package", package))
        check(candidate)
        require(candidate["minecraft_library_count"] == 39 and candidate["fabric_library_count"] == 7,
                "actual pinned fixture dependency count differs")
        require(candidate["compatibility"] != candidate["core_compatibility"], "package silently reused old core pin")
        relocated = root / "relocated-package"
        package.rename(relocated)
        package = relocated
        bundle = root / "bundle"
        capture = json.loads(save(reviewer, "capture", "--source", copied, "--destination", bundle,
                                  "--compatibility", candidate["compatibility"], "--confirm"))
        pins = ["--expected-manifest", capture["manifest_hash"], "--expected-state-root", capture["state_root"],
                "--expected-compatibility", candidate["compatibility"]]
        offline = json.loads(save(reviewer, "verify-runtime-package", "--bundle", bundle, *pins, "--package", package))
        check(offline)
        require(offline["compatibility"] == candidate["compatibility"] and offline["level_data_version"] == 4790,
                "relocated package or data version mismatch")

        profiles = [root / "device-a", root / "device-b"]
        ids = [dict(line.split(": ", 1) for line in run(p, "identity").splitlines()) for p in profiles]
        draft = root / "draft.json"
        draft.write_text(save(reviewer, "draft", "--device-key", ids[0]["Public key"],
                              "--device-key", ids[1]["Public key"], "--initial-holder", ids[0]["Peer ID"], "--bundle", bundle))
        review = json.loads(run(reviewer, "exclusive-turn", "inspect-draft", "--draft", draft))
        world, grant = review["world_id"], review["initial_grant"]
        approvals = [root / "approval-a.json", root / "approval-b.json"]
        for profile, approval in zip(profiles, approvals):
            approval.write_text(save(profile, "approve", "--draft", draft, "--expected-world", world,
                                     "--bundle", bundle, "--confirm"))
        history = root / "history.json"
        history.write_text(run(reviewer, "exclusive-turn", "assemble", "--draft", draft, "--expected-world", world,
                               "--approval", approvals[0], "--approval", approvals[1]))
        base = ["--expected-world", world, "--expected-grant", grant]
        for profile in profiles:
            save(profile, "init", *base, "--history", history, "--bundle", bundle, "--confirm")
        journals = [p / "exclusive-turn-reference" / world.removeprefix("scpass:") / "journal.postcard" for p in profiles]
        before = [digest(path) for path in journals]
        held_args = [*base, "--expected-checkpoint", capture["reference_commitment"], "--package", package]
        held = json.loads(save(profiles[0], "verify-held-runtime-package", *held_args))
        check(held)
        require(held["signed_compatibility_verified"] and held["local_held_reference_verified"], "held package not verified")
        save(profiles[1], "verify-held-runtime-package", *held_args, refusal="requires exact held")

        dependency = next((package / "libraries").rglob("gson-*.jar"))
        saved = root / "saved-library"
        dependency.rename(saved)
        save(profiles[0], "verify-held-runtime-package", *held_args, refusal="required nonempty runtime file missing")
        saved.rename(dependency)
        original_dep = dependency.read_bytes()
        dependency.write_bytes(original_dep + b"changed")
        save(profiles[0], "verify-held-runtime-package", *held_args, refusal="Minecraft dependency checksum differs")
        dependency.write_bytes(original_dep)
        extra = package / "mods/unreviewed.jar"
        extra.write_bytes(b"unreviewed mod")
        save(profiles[0], "verify-held-runtime-package", *held_args, refusal="missing or unexpected")
        extra.unlink()
        java_file = package / "java/bin/java"
        original_java = java_file.read_bytes()
        java_file.write_bytes(original_java + b"changed")
        save(profiles[0], "verify-held-runtime-package", *held_args, refusal="runtime package pin differs")
        java_file.write_bytes(original_java)
        final = json.loads(save(profiles[0], "verify-held-runtime-package", *held_args))
        require(final == held and before == [digest(path) for path in journals], "restored package or journal differs")
        require(not reviewer.exists(), "offline review created a profile")
        for profile in profiles:
            require(not any((profile / "worlds").iterdir()), "canonical world created")
            require(not (profile / "control").exists() and not (profile / "runtime").exists(), "runtime state created")
        print(json.dumps({"classification": "actual_stopped_fixture_runtime_package_preflight", "candidate": candidate,
                          "offline": offline, "held": held, "source_entries": len(original),
                          "source_tree_sha256": hashlib.sha256(json.dumps(original, sort_keys=True).encode()).hexdigest(),
                          "relocation_verified": True, "missing_dependency_refused": True, "changed_dependency_refused": True,
                          "unexpected_mod_refused": True, "changed_java_refused": True, "replica_refused": True,
                          "journals_unchanged": True, "exact_retry_passed": True, "no_runtime_started": True}, sort_keys=True))


if __name__ == "__main__":
    main()
