#!/usr/bin/env python3
"""Repeat signed replica crash/restart under actual kernel netem on isolated lo.

Use a freshly built parallel_replication test executable. Run inside a new
mapped-root user/network namespace; never change host qdisc settings.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import time

from profile_upgrade import sha

PROFILES = (
    {"delay_ms": 15, "jitter_ms": 3, "loss_percent": 0.25, "rate_mbit": 30},
    {"delay_ms": 30, "jitter_ms": 5, "loss_percent": 0.75, "rate_mbit": 15},
)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def qdisc():
    observed = json.loads(subprocess.check_output(["tc", "-s", "-j", "qdisc", "show", "dev", "lo"], text=True))
    matching = [row for row in observed if row.get("kind") == "netem"]
    require(len(matching) == 1, "exactly one kernel netem qdisc required")
    return matching[0]


def run(args, evidence):
    require(os.getuid() == 0 and int(Path("/proc/self/uid_map").read_text().split()[1]) != 0,
            "mapped-root user namespace required")
    require({name for _, name in socket.if_nameindex()} == {"lo"},
            "isolated loopback-only network namespace required")
    # A new namespace starts with an unconfigured lo. Refuse to touch even
    # the loopback qdisc if a preexisting impairment configuration is present.
    existing = json.loads(subprocess.check_output(["tc", "-j", "qdisc", "show", "dev", "lo"], text=True))
    require(all(row.get("kind") == "noqueue" for row in existing), "unexpected existing loopback qdisc")
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    env = {key: value for key, value in os.environ.items() if not key.startswith("SWARMCRAFT_")}
    env.update(RUST_LOG="info", NO_COLOR="1", TERM="dumb")
    for index, profile in enumerate(PROFILES, 1):
        subprocess.run(["tc", "qdisc", "add", "dev", "lo", "root", "netem",
                        "limit", "1000", "delay", f"{profile['delay_ms']}ms",
                        f"{profile['jitter_ms']}ms", "loss", f"{profile['loss_percent']}%",
                        "rate", f"{profile['rate_mbit']}mbit"], check=True)
        row = {"repetition": index, "configured": profile, "qdisc_before": qdisc()}
        evidence["runs"].append(row)
        try:
            start = time.monotonic()
            with (args.artifacts / f"run-{index}.log").open("w") as log:
                process = subprocess.Popen([str(args.test), "--ignored", "--exact",
                                            "impaired_replica_crash_stress_preserves_signed_head",
                                            "--nocapture", "--test-threads=1"],
                                           env=env, stdout=log, stderr=subprocess.STDOUT,
                                           start_new_session=True)
                try:
                    exit_code = process.wait(timeout=220)
                finally:
                    # Test daemons inherit this fresh process group. Kill
                    # only that group on timeout or abnormal harness exit.
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait(timeout=10)
            row["elapsed_seconds"] = round(time.monotonic() - start, 3)
            row["qdisc_after"] = qdisc()
            row["exit_code"] = exit_code
            output = (args.artifacts / f"run-{index}.log").read_text(errors="replace")
            require(exit_code == 0, "signed replica stress failed; see retained log")
            receipt = re.findall(r"stress_receipt=(\{[^\n]*\})", output)
            require(len(receipt) == 1 and "1 passed; 0 failed" in output, "missing non-skipped receipt")
            row["receipt"] = json.loads(receipt[0])
            require(row["receipt"]["old_and_new_restores_byte_exact"] and row["receipt"]["resumed_blobs"] > 0,
                    "signed restore or durable resume not proven")
            require(row["qdisc_after"]["packets"] > row["qdisc_before"]["packets"], "no actual impaired packets")
            require(row["qdisc_after"]["drops"] > row["qdisc_before"]["drops"], "no observed kernel packet drops")
            row["passed"] = True
            print(f"PASS impairment {index}: {profile}; signed checkpoint recovery and durable resume", flush=True)
        finally:
            subprocess.run(["tc", "qdisc", "del", "dev", "lo", "root"], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test", type=lambda p: Path(p).resolve(), required=True)
    parser.add_argument("--artifacts", type=lambda p: Path(p).resolve(), required=True)
    args = parser.parse_args()
    os.umask(0o077)
    args.artifacts.mkdir(mode=0o700, exist_ok=False)
    evidence = {"scope": "Linux loopback-only netem; real signed three-daemon replication, hard receiver/source loss, no Java",
                "kernel": os.uname().release, "test_sha256": sha(args.test), "runs": []}
    try:
        run(args, evidence)
    finally:
        (args.artifacts / "result.json").write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
