#!/usr/bin/env python3
"""Bounded Linux stress lab: real ENOSPC, RLIMIT_FSIZE, SIGKILL and signed restore.

Run in a new unprivileged user/mount namespace, with a quiescent disposable
Minecraft Java save. Never mount/fill the host filesystem, touch a real profile,
or launch Minecraft. Every repetition creates its own peer/world.
"""
from __future__ import annotations

import argparse
import errno
import json
import os
from pathlib import Path
import random
import re
import resource
import shutil
import signal
import subprocess
import tempfile
import time

from profile_upgrade import inventory, sha

MIB = 1024 * 1024
FS_BYTES = 96 * MIB
RESERVE = 64 * MIB
FILE_CAP = 1 * MIB


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def available(root):
    info = os.statvfs(root)
    return info.f_bavail * info.f_frsize


def memory_available():
    return int(re.search(r"^MemAvailable:\s+(\d+) kB", Path("/proc/meminfo").read_text(), re.M)[1]) * 1024


def command(cli, root, *args, expect=True, file_cap=None):
    env = {k: v for k, v in os.environ.items() if not k.startswith("SWARMCRAFT_")}
    env.update(RUST_LOG="warn", NO_COLOR="1")
    def limit():
        # Ignore the signal in the child so the kernel EFBIG is observable
        # as a handled write error, not merely a killed process.
        signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
        resource.setrlimit(resource.RLIMIT_FSIZE, (file_cap, file_cap))
    result = subprocess.run(
        [str(cli), "--data-dir", str(root), *map(str, args)],
        env=env, capture_output=True, text=True, timeout=90,
        preexec_fn=limit if file_cap is not None else None,
    )
    if expect:
        require(result.returncode == 0, f"CLI failed: {result.stderr[-600:]}")
    else:
        require(result.returncode != 0, "faulted operation unexpectedly succeeded")
    return result


def checkpoints(cli, root, world):
    output = command(cli, root, "world", "verify", world).stdout
    hashes = re.findall(r"^Verified checkpoint: ([a-f0-9]{64})$", output, re.M)
    require(bool(hashes), "signed checkpoint verification unavailable")
    return hashes


def restore(cli, root, world, number, destination, expected):
    command(cli, root, "world", "recover", world, number, destination)
    require(inventory(destination) == expected, "verified restore differs from original source")


def debris(root):
    return {str(p.relative_to(root)): p.stat().st_size for p in root.rglob("*")
            if p.is_file() and p.name.startswith((".atomic-", ".blob-"))}


def random_file(path, size, seed):
    rng = random.Random(seed)
    with path.open("wb") as f:
        for offset in range(0, size, MIB):
            f.write(rng.randbytes(min(MIB, size - offset)))


def kill_partial(cli, root, world, source, blob_dir, baseline):
    env = {k: v for k, v in os.environ.items() if not k.startswith("SWARMCRAFT_")}
    env.update(RUST_LOG="warn", NO_COLOR="1")
    proc = subprocess.Popen(
        [str(cli), "--data-dir", str(root), "world", "snapshot", world, str(source),
         "--epoch", "1", "--sequence", "2"], env=env, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline and proc.poll() is None:
            complete = {p.name for p in blob_dir.glob("*.zst") if not p.name.startswith(".")} - baseline
            if complete:
                for tmp in blob_dir.glob(".blob-*.zst"):
                    try:
                        observed = tmp.stat().st_size
                    except FileNotFoundError:
                        continue
                    if observed >= 128 * 1024:
                        # SIGSTOP prevents further publication while we inspect
                        # the actual in-progress temporary payload. Never signal
                        # a process other than the child we created.
                        proc.send_signal(signal.SIGSTOP)
                        pid, status = os.waitpid(proc.pid, os.WUNTRACED)
                        require(pid == proc.pid and os.WIFSTOPPED(status), "snapshot writer completed before stop")
                        require(tmp.is_file() and 0 < tmp.stat().st_size < 24 * MIB,
                                "no actual incomplete compression observed")
                        partial = tmp.stat().st_size
                        proc.kill()
                        proc.communicate(timeout=10)
                        require(proc.returncode == -signal.SIGKILL, "expected real SIGKILL")
                        return {"signal": "SIGKILL", "partial_bytes": partial,
                                "completed_uncommitted_blobs": len(complete)}
            time.sleep(0.001)
        raise RuntimeError("failed to observe live partial snapshot publication")
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.communicate(timeout=10)


def repetition(args, out, index):
    cli = args.bin / "swarmcraft"
    importer = args.bin / "swarmcraft-import"
    baseline_source = inventory(args.save)
    seed = out / "seed"
    imported = json.loads(command(importer, seed, "--source", args.save, "--name",
                                  "Stress acceptance", "--minecraft", "26.1.2",
                                  "--fabric-loader", "0.19.3", "--no-server-mods").stdout)
    world = imported["world_id"]
    original_hash = imported["snapshot_hash"]
    require(checkpoints(cli, seed, world) == [original_hash], "initial signed save invalid")
    command(cli, seed, "world", "storage-policy", world, "--unlimited",
            "--reserve-free-bytes", RESERVE)
    source = out / "candidate-save"
    shutil.copytree(args.save, source)
    random_file(source / "000-stress-orphan.bin", 2 * MIB, 1000 + index)
    random_file(source / "001-stress-large.bin", 24 * MIB, 2000 + index)
    replacement = out / "retry-save"
    shutil.copytree(args.save, replacement)
    random_file(replacement / "stress-retry.bin", 2 * MIB, 3000 + index)
    replacement_inventory = inventory(replacement)
    mount = out / "bounded-filesystem"
    mount.mkdir()
    require(memory_available() >= 1536 * MIB, "insufficient host memory for isolated tmpfs lab")
    subprocess.run(["mount", "-t", "tmpfs", "-o",
                    "size=96m,nr_inodes=32768,mode=0700", "tmpfs", str(mount)], check=True)
    report = {"repetition": index, "initial_checkpoint": original_hash,
              "filesystem_capacity_bytes": FS_BYTES, "reserve_bytes": RESERVE,
              "file_size_limit_bytes": FILE_CAP, "stages": {}}
    try:
        actual = os.statvfs(mount)
        require(actual.f_blocks * actual.f_frsize == FS_BYTES, "tmpfs limit differs from requested capacity")
        root = mount / "data"
        shutil.copytree(seed, root)
        world_dir = root / "worlds" / world.removeprefix("scworld:")
        policy = world_dir / "metadata/local-storage-policy.json"
        policy_before = policy.read_bytes()
        initial = inventory(root)
        blob_dir = world_dir / "blobs"
        before_blobs = {p.name for p in blob_dir.glob("*.zst") if not p.name.startswith(".")}
        before_debris = debris(root)

        # Exhaust only this dedicated tmpfs. Verify actual kernel errno
        # separately from the application's reserve admission refusal.
        filler = mount / "filler"
        filler_bytes = available(mount)
        with filler.open("wb") as f:
            os.posix_fallocate(f.fileno(), 0, filler_bytes)
        require(available(mount) == 0, "filesystem not full")
        try:
            with (mount / "enospc-probe").open("wb", buffering=0) as f:
                f.write(b"test")
            raise RuntimeError("kernel accepted bytes on full tmpfs")
        except OSError as error:
            require(error.errno == errno.ENOSPC, f"expected ENOSPC, got errno {error.errno}")
        for _ in range(3):
            result = command(cli, root, "world", "storage-policy", world, "--max-world-bytes",
                             100 * MIB, "--reserve-free-bytes", RESERVE, expect=False)
            require("No space left on device" in result.stderr, "actual metadata ENOSPC not reported")
            require(policy.read_bytes() == policy_before and debris(root) == before_debris,
                    "failed metadata write changed policy or accumulated scratch")
        result = command(cli, root, "world", "snapshot", world, source, "--epoch", 1,
                         "--sequence", 2, expect=False)
        require("filesystem space" in result.stderr or "No space left on device" in result.stderr,
                "wrong reason for full-filesystem snapshot refusal")
        require(checkpoints(cli, root, world) == [original_hash], "full filesystem changed head")
        restore(cli, root, world, 1, out / "full-disk-restore", baseline_source)
        require(all(inventory(root).get(k) == v for k, v in initial.items()),
                "full disk changed preexisting profile bytes")
        report["stages"]["enospc"] = {
            "available_bytes": 0, "filler_bytes": filler_bytes, "kernel_errno": errno.ENOSPC,
            "metadata_attempts": 3, "snapshot_denied": True, "last_accepted_checkpoint": original_hash,
            "restored_all_original_files": True, "new_atomic_debris": 0,
        }
        filler.unlink()

        # Real per-process file-size limit, not an artificial quota.
        result = command(cli, root, "world", "snapshot", world, source, "--epoch", 1,
                         "--sequence", 2, expect=False, file_cap=FILE_CAP)
        require("File too large" in result.stderr or "os error 27" in result.stderr,
                "expected kernel EFBIG under RLIMIT_FSIZE")
        require(debris(root) == before_debris, "handled file-size failure left staging debris")
        require(checkpoints(cli, root, world) == [original_hash], "file limit changed head")
        restore(cli, root, world, 1, out / "file-limit-restore", baseline_source)
        report["stages"]["file_size_limit"] = {
            "kernel_errno": errno.EFBIG, "file_size_limit_bytes": FILE_CAP,
            "exit_code": result.returncode, "last_accepted_checkpoint": original_hash,
            "restored_all_original_files": True,
        }

        fault = kill_partial(cli, root, world, source, blob_dir, before_blobs)
        require(checkpoints(cli, root, world) == [original_hash], "killed publication changed head")
        restore(cli, root, world, 1, out / "crash-restore", baseline_source)
        crash_debris = debris(root)
        require(len(crash_debris) > len(before_debris), "crash fixture did not leave real evidence")
        report["stages"]["crash"] = {
            **fault, "last_accepted_checkpoint": original_hash,
            "retained_scratch_bytes": sum(crash_debris.values()), "restored_all_original_files": True,
        }

        cleanup = json.loads(command(cli, root, "world", "storage-cleanup", world, "--confirm").stdout)
        require(cleanup["removed_blobs"] >= 1 and cleanup["removed_snapshots"] == [],
                "retention did not reclaim only unreferenced complete payload")
        require(debris(root) == crash_debris, "cleanup discarded unproven crash evidence")
        require(checkpoints(cli, root, world) == [original_hash], "GC changed signed history")
        command(cli, root, "world", "snapshot", world, replacement, "--epoch", 1, "--sequence", 2)
        hashes = checkpoints(cli, root, world)
        require(len(hashes) == 2 and hashes[0] == original_hash, "retry did not extend old head exactly once")
        cleanup_again = json.loads(command(cli, root, "world", "storage-cleanup", world, "--confirm").stdout)
        require(cleanup_again["removed_snapshots"] == [], "retention removed signed history")
        restore(cli, root, world, 1, out / "retained-restore", baseline_source)
        restore(cli, root, world, 2, out / "retry-restore", replacement_inventory)
        require(checkpoints(cli, root, world) == hashes and inventory(args.save) == baseline_source,
                "retention/retry changed source or checkpoint chain")
        report["stages"]["retention_and_retry"] = {
            "reclaimed_blobs": cleanup["removed_blobs"], "reclaimed_bytes": cleanup["reclaimed_bytes"],
            "removed_snapshots": 0, "last_accepted_checkpoint": hashes[-1],
            "retained_signed_checkpoints": hashes, "both_restores_byte_exact": True,
            "crash_scratch_preserved": True,
        }
        report["passed"] = True
        print(f"PASS repetition {index}: ENOSPC, EFBIG, SIGKILL, retention and exact signed recovery", flush=True)
    finally:
        subprocess.run(["umount", str(mount)], check=True)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", type=lambda p: Path(p).resolve(), required=True)
    parser.add_argument("--save", type=lambda p: Path(p).resolve(), required=True)
    parser.add_argument("--artifacts", type=lambda p: Path(p).resolve(), required=True)
    parser.add_argument("--repetitions", type=int, choices=range(1, 6), default=3)
    args = parser.parse_args()
    require(not args.artifacts.is_relative_to(args.save), "artifacts must not be within the source save")
    require(os.getuid() == 0 and int(Path("/proc/self/uid_map").read_text().split()[1]) != 0,
            "run in unprivileged mapped-root user namespace")
    require(subprocess.check_output(["findmnt", "-n", "-o", "PROPAGATION", "--target", "/"], text=True).strip() == "private",
            "private mount propagation required")
    require((args.save / "level.dat").is_file() and (args.save / "session.lock").is_file()
            and any(args.save.rglob("*.mca")), "real quiescent Java save required")
    os.umask(0o077)
    args.artifacts.mkdir(mode=0o700, exist_ok=False)
    evidence = {"scope": "Linux disposable 96 MiB tmpfs; real kernel ENOSPC/EFBIG; SIGKILL of actual snapshot writer; no Java or daemon",
                "kernel": os.uname().release, "cli_sha256": sha(args.bin / "swarmcraft"),
                "source_files": len(inventory(args.save)), "repetitions_requested": args.repetitions,
                "runs": []}
    try:
        for index in range(1, args.repetitions + 1):
            # Own disposable profiles/files only; retain the receipts, not
            # private identity keys or large test worlds.
            with tempfile.TemporaryDirectory(prefix=f"run-{index}-", dir=args.artifacts) as scratch:
                evidence["runs"].append(repetition(args, Path(scratch), index))
    finally:
        (args.artifacts / "result.json").write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
