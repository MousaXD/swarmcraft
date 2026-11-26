#!/usr/bin/env python3
"""Disposable actual Fabric save/shutdown + two signed offline reference journals.

The ignored Rust gate owns the real cgroup and staged package. Only its
private network namespace can reach Minecraft. No production launcher,
hosting permit, real player, or live transfer is created by this test.
"""
import argparse
import fcntl
import gzip
import json
import os
from pathlib import Path
import select
import shutil
import stat
import struct
import subprocess
import tempfile
import uuid

from verify_core_runtime import digest, require, tree


def copy_package(server, toolchain, package):
    package.mkdir()
    for src, dest in ((toolchain / "jdk", "java"), (toolchain / "loader-libs", "fabric-libraries"),
                      (server / "versions", "versions"), (server / "libraries", "libraries")):
        shutil.copytree(src, package / dest, symlinks=True)
    for src, dest in ((server / "server.jar", "server.jar"),
                      (toolchain / "fabric-server-profile.json", "fabric-profile.json"),
                      (server / "mods/swarmcraft-fabric-0.5.1.jar", "mods/swarmcraft-fabric.jar")):
        target = package / dest
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, target)
        require(digest(src) == digest(target), "runtime package copy differs")
    for item in [package, *package.rglob("*")]:
        if not item.is_symlink():
            item.chmod(0o700 if item.is_dir() else 0o600 | (item.stat().st_mode & 0o111))


def copy_stopped_world(source, copied):
    fd = os.open(source / "session.lock", os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "r+b") as lock:
        require(stat.S_ISREG(os.fstat(lock.fileno()).st_mode), "unsafe fixture session lock")
        fcntl.lockf(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        original = tree(source)
        shutil.copytree(source, copied, ignore=lambda d, _: ["session.lock"] if Path(d) == source else [])
        require(original == tree(source) == tree(copied), "stopped test world changed during copy")
        (copied / "session.lock").write_bytes(lock.read())
    for item in [copied, *copied.rglob("*")]:
        item.chmod(0o700 if item.is_dir() else 0o600)
    return original


def marker_count(world):
    """Independently parse bounded gzip NBT, not a substring/log assertion."""
    file = world / "data/minecraft/scoreboard.dat"
    require(file.is_file() and file.stat().st_size <= 1024 * 1024, "scoreboard fixture missing or oversized")
    with gzip.open(file, "rb") as stream:
        data = stream.read(1024 * 1024 + 1)
    require(len(data) <= 1024 * 1024, "scoreboard NBT exceeds bound")
    pos = 0
    nodes = 0

    def take(size):
        nonlocal pos
        require(0 <= size <= len(data) - pos, "truncated scoreboard NBT")
        value = data[pos:pos + size]
        pos += size
        return value

    def num(fmt):
        return struct.unpack(">" + fmt, take(struct.calcsize(fmt)))[0]

    def text():
        return take(num("H")).decode("utf-8", errors="strict")

    def value(tag, depth):
        nonlocal nodes
        nodes += 1
        require(depth <= 32 and nodes <= 100_000, "scoreboard NBT structure exceeds bounds")
        if tag in (1, 2, 3, 4, 5, 6):
            return num({1: "b", 2: "h", 3: "i", 4: "q", 5: "f", 6: "d"}[tag])
        if tag == 8:
            return text()
        if tag in (7, 11, 12):
            length = num("i")
            return take(length * {7: 1, 11: 4, 12: 8}[tag])
        if tag == 9:
            child, length = num("B"), num("i")
            require(0 <= length <= 100_000 and (length == 0 or 1 <= child <= 12), "invalid scoreboard list")
            return [value(child, depth + 1) for _ in range(length)]
        if tag == 10:
            result = {}
            while (child := num("B")) != 0:
                name = text()
                require(name not in result, "duplicate scoreboard NBT key")
                result[name] = value(child, depth + 1)
            return result
        raise ValueError(f"invalid scoreboard NBT tag {tag}")

    require(num("B") == 10, "scoreboard root not a compound")
    text()
    root = value(10, 0)
    require(pos == len(data) and root.get("DataVersion") == 4790, "scoreboard NBT version or trailing data differs")

    def count(node):
        if isinstance(node, dict):
            fields = {key.lower(): item for key, item in node.items()}
            present = fields.get("name") == "sc8_checkpoint" and fields.get("objective") == "sc8marker" and fields.get("score") == 2601
            return int(present) + sum(count(item) for item in node.values())
        if isinstance(node, list):
            return sum(count(item) for item in node)
        return 0

    return count(root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cli", "library-test-binary", "server-fixture", "toolchain-fixture"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--confirm-disposable-test-artifacts-and-eula", required=True, action="store_true")
    args = parser.parse_args()
    os.umask(0o077)
    cli, binary = args.cli.resolve(strict=True), args.library_test_binary.resolve(strict=True)
    server, toolchain = args.server_fixture.resolve(strict=True), args.toolchain_fixture.resolve(strict=True)
    repo = Path(__file__).resolve().parents[2]
    root = Path(tempfile.mkdtemp(prefix="sc8-fabric-lifecycle-"))

    def command(profile, *parts):
        return [str(cli), "--data-dir", str(profile), *map(str, parts)]

    def run(profile, *parts, refusal=None):
        proc = subprocess.run(command(profile, *parts), capture_output=True, text=True, timeout=150)
        if refusal is not None:
            require(proc.returncode != 0 and not proc.stdout and refusal in proc.stderr, f"expected refusal {refusal}: {proc.stderr}")
            return None
        require(proc.returncode == 0, proc.stderr)
        return proc.stdout

    def ref(profile, *parts, **kw):
        return run(profile, "exclusive-turn", *parts, **kw)

    def save(profile, *parts, **kw):
        return ref(profile, "save", *parts, **kw)

    def transfer(source, receive, expected):
        # Exact one-shot reference listeners; no port probing that could
        # consume the authentication attempt. No Minecraft route is used.
        with subprocess.Popen(source, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as listener:
            try:
                require(select.select([listener.stdout], [], [], 10)[0], "reference listener readiness timed out")
                line = listener.stdout.readline()
                require(line, "reference listener did not publish readiness")
                ready = json.loads(line)
                require(ready["classification"] == expected and ready["address"].startswith("127.0.0.1:"), "reference listener identity differs")
                result = receive(ready["address"])
                rest, err = listener.communicate(timeout=20)
                require(listener.returncode == 0, err + rest)
                return json.loads(result)
            finally:
                if listener.poll() is None:
                    listener.kill()
                    listener.wait(timeout=5)

    try:
        package = root / "package"
        copy_package(server, toolchain, package)
        original = root / "initial-world"
        source_tree = copy_stopped_world(server / "world", original)
        require(marker_count(original) == 0, "fixture already contains qualification marker")
        reviewer = root / "reviewer"
        candidate = json.loads(save(reviewer, "runtime-package-pin", "--package", package))
        pin = candidate["compatibility"]
        initial_bundle = root / "initial-bundle"
        initial = json.loads(save(reviewer, "capture", "--source", original, "--destination", initial_bundle,
                                  "--compatibility", pin, "--confirm"))
        checkpoint = initial["reference_commitment"]
        profiles = [root / "device-a", root / "device-b"]
        ids = [dict(line.split(": ", 1) for line in run(profile, "identity").splitlines()) for profile in profiles]
        draft = root / "draft.json"
        draft.write_text(save(reviewer, "draft", "--device-key", ids[0]["Public key"], "--device-key", ids[1]["Public key"],
                              "--initial-holder", ids[0]["Peer ID"], "--bundle", initial_bundle))
        review = json.loads(ref(reviewer, "inspect-draft", "--draft", draft))
        world, grant = review["world_id"], review["initial_grant"]
        approvals = [root / "approval-a.json", root / "approval-b.json"]
        for profile, approval in zip(profiles, approvals):
            approval.write_text(save(profile, "approve", "--draft", draft, "--expected-world", world, "--bundle", initial_bundle, "--confirm"))
        history = root / "history.json"
        history.write_text(ref(reviewer, "assemble", "--draft", draft, "--expected-world", world, "--approval", approvals[0], "--approval", approvals[1]))
        base = ["--expected-world", world, "--expected-grant", grant]
        for profile in profiles:
            save(profile, "init", *base, "--history", history, "--bundle", initial_bundle, "--confirm")
        journals = [p / "exclusive-turn-reference" / world.removeprefix("scpass:") / "journal.postcard" for p in profiles]
        before = [digest(j) for j in journals]
        lab = root / "lab"
        lab.mkdir()
        (lab / "runtime").mkdir()
        restored = lab / "runtime/world"
        save(profiles[0], "restore-held", *base, "--expected-checkpoint", checkpoint, "--destination", restored, "--confirm")
        require(tree(restored) == source_tree, "initial held save restoration differs")
        save(profiles[1], "restore-held", *base, "--expected-checkpoint", checkpoint, "--destination", lab / "not-holder", "--confirm", refusal="requires exact held")
        unit = f"swarmcraft-memory-cap-test-fabric-{uuid.uuid4().hex}"
        gate = ["systemd-run", "--user", "--scope", "--quiet", f"--unit={unit}", "-p", "Delegate=yes", "env",
                "SWARMCRAFT_TEST_CPU_DELEGATION=1", f"SC8_LAB_ROOT={lab}", f"SC8_LAB_PACKAGE={package}",
                f"SC8_LAB_COMPATIBILITY={pin}", "bash", str(repo / "scripts/e2e/linux_cgroup_memory_gate.sh"), str(binary),
                "--exact", "exclusive_turn_fabric_lab::real_fabric_lifecycle_isolated_under_exact_package_and_cgroup",
                "--ignored", "--nocapture", "--test-threads=1"]
        print(f"Running disposable Fabric lifecycle fixture: {lab}", flush=True)
        try:
            subprocess.run(gate, cwd=repo, check=True, timeout=420)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            # Only the unique fixture-owned scope; never erase account records.
            subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], check=False, timeout=30)
            raise
        lifecycle = json.loads((lab / "fabric-lifecycle.json").read_text())
        supervision = json.loads((lab / "fabric-supervision.json").read_text())
        require(all(lifecycle.get(key) is True for key in ("authenticated_world_info", "missing_permit_readiness_refused", "active_session_lock_verified", "save_barrier_acknowledged", "shutdown_barrier_acknowledged", "clean_java_exit", "offline_session_lock_verified")), "actual lifecycle evidence incomplete")
        require(all(supervision.get(key) is True for key in ("exact_staged_package_reverified", "process_and_descendants_quiescent", "ownership_records_retired")), "kernel supervision evidence incomplete")
        require(marker_count(restored) == 1, "actual scoreboard marker missing or ambiguous after shutdown")
        require(before == [digest(j) for j in journals], "disposable lifecycle changed signed reference history")
        # Only now capture and publish the real changed offline save. This
        # test intentionally does not claim a single atomic runtime adapter.
        final_bundle = root / "final-bundle"
        final = json.loads(save(reviewer, "capture", "--source", restored, "--destination", final_bundle, "--compatibility", pin, "--confirm"))
        changed = final["reference_commitment"]
        require(changed != checkpoint, "server did not produce changed checkpoint")
        save(profiles[0], "checkpoint", *base, "--expected-checkpoint", checkpoint, "--bundle", final_bundle, "--confirm")
        offer_file = root / "offer.json"
        offer_file.write_text(ref(profiles[0], "prepare", *base, "--expected-checkpoint", changed, "--confirm"))
        prepared = json.loads(ref(profiles[0], "status", "--expected-world", world))
        offer = prepared["prepared_offer"]
        require(prepared["local_phase"] == "prepared" and offer, "source not durably prepared")
        review_args = [*base, "--expected-offer", offer, "--confirm"]
        transfer(command(profiles[0], "exclusive-turn", "save", "serve", *review_args, "--bind", "127.0.0.1:0"),
                 lambda address: save(profiles[1], "fetch", *review_args, "--offer", offer_file, "--peer", f"{ids[0]['Peer ID']}@{address}"),
                 "exclusive_turn_save_listener")
        transfer(command(profiles[1], "exclusive-turn", "network", "serve-handoff", *review_args, "--offer", offer_file,
                         "--from", ids[0]["Peer ID"], "--bind", "127.0.0.1:0"),
                 lambda address: ref(profiles[0], "network", "handoff", *review_args, "--peer", f"{ids[1]['Peer ID']}@{address}"),
                 "exclusive_turn_reference_handoff_listener")
        a, b = [json.loads(ref(p, "status", "--expected-world", world)) for p in profiles]
        require(a["local_phase"] == "released" and b["local_phase"] == "held" and a["grant"] == b["grant"]
                and a["reference_epoch"] == b["reference_epoch"] == 2, "signed reference handoff differs")
        new_base = ["--expected-world", world, "--expected-grant", b["grant"]]
        save(profiles[0], "restore-held", *new_base, "--expected-checkpoint", changed, "--destination", root / "released-restore", "--confirm", refusal="requires exact held")
        recipient = root / "recipient-restored"
        save(profiles[1], "restore-held", *new_base, "--expected-checkpoint", changed, "--destination", recipient, "--confirm")
        require(tree(recipient) == tree(restored) and marker_count(recipient) == 1, "recipient exact saved scoreboard differs")
        require(tree(server / "world") == source_tree, "retained source fixture changed")
        for profile in profiles:
            require(not any((profile / "worlds").iterdir()) and not (profile / "control").exists() and not (profile / "runtime").exists(), "canonical world or live profile runtime created")
        print(json.dumps({"classification": "isolated_real_fabric_shutdown_then_offline_signed_handoff", "lifecycle": lifecycle,
                          "supervision": supervision, "initial_commitment": checkpoint, "final_commitment": changed,
                          "final_save_bytes": final["total_bytes"], "marker": {"name": "sc8_checkpoint", "objective": "sc8marker", "score": 2601},
                          "recipient_exact_restore": True, "source_released": True, "recipient_reference_held": True,
                          "reference_epoch": 2, "source_fixture_unchanged": True, "canonical_authority": False,
                          "runtime_activation": False, "player_admission": False, "minecraft_gameplay_verified": False}, sort_keys=True), flush=True)
        shutil.rmtree(root)
    except BaseException:
        print(f"Retained failed disposable fixture for review: {root}", flush=True)
        raise


if __name__ == "__main__":
    main()
