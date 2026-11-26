#!/usr/bin/env python3
"""Run the real encrypted relay duration gate under isolated Linux netem.

No host traffic shaping, sudo, real-world state, or physical-network claim.
Build the game_stream_relay test executable locally before invoking this gate.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import time

TEST = "private_relay_expires_staggered_active_circuits_without_cross_disconnect"
PROFILES = ((4, 5), (1, 15))  # shared loopback Mbit/s, per-packet delay ms


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def qdiscs():
    return json.loads(subprocess.check_output(
        ["tc", "-j", "-s", "qdisc", "show", "dev", "lo"], text=True))


def netem(rate, delay):
    rows = qdiscs()
    require(len(rows) == 1 and rows[0]["kind"] == "netem" and rows[0]["root"],
            "exact root netem qdisc missing")
    row = rows[0]
    require(row["options"]["rate"]["rate"] == rate * 125000,
            "observed kernel rate differs from requested rate")
    require(abs(row["options"]["delay"]["delay"] * 1000 - delay) < 0.001,
            "observed kernel delay differs from requested delay")
    return row


def parse_receipt(output):
    require(re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output),
            "test did not execute exactly one passing, nonignored case")
    rows = re.findall(r"relay_duration ([^\n]+)", output)
    require(len(rows) == 3, "missing or duplicate duration receipts")
    result = {}
    expected = {"acknowledged_bytes", "samples", "overlap_samples", "dial_to_close_ms",
                "authenticated_to_close_ms", "payload_kib_per_s", "p95_echo_ms"}
    for name, chunk in (("bulk", 8192), ("interactive", 64)):
        matches = [row for row in rows if row.startswith(name + " ")]
        require(len(matches) == 1, "missing circuit receipt: " + name)
        fields = dict(item.split("=", 1) for item in matches[0].split()[1:])
        require(fields.keys() == expected, "unexpected circuit receipt schema")
        values = {key: float(value) if key in ("payload_kib_per_s", "p95_echo_ms") else int(value)
                  for key, value in fields.items()}
        require(values["acknowledged_bytes"] == values["samples"] * chunk
                and values["samples"] >= values["overlap_samples"] >= 10,
                "inconsistent exact echo receipt")
        require(19000 <= values["dial_to_close_ms"] <= 30000
                and 0 < values["authenticated_to_close_ms"] <= 25000,
                "duration receipt outside bounds")
        require(0 < values["payload_kib_per_s"] and 0 < values["p95_echo_ms"] <= 5000,
                "invalid throughput or latency")
        result[name] = values
    summary = dict(item.split("=", 1) for item in next(
        row for row in rows if row.startswith("configured_seconds=")).split())
    require(summary.keys() == {"configured_seconds", "byte_budget", "overlap_ms",
                               "independent_post_expiry_samples", "authentication_revoked_both_pairs"},
            "unexpected summary schema")
    require(summary["configured_seconds"] == "20" and summary["byte_budget"] == "67108864"
            and summary["authentication_revoked_both_pairs"] == "true"
            and int(summary["overlap_ms"]) > 0
            and int(summary["independent_post_expiry_samples"]) >= 5,
            "duration, overlap or authentication retirement proof missing")
    result["summary"] = {key: True if key == "authentication_revoked_both_pairs" else int(value)
                         for key, value in summary.items()}
    return result


def isolated_run(args):
    # Never touch even the loopback qdisc outside a fresh mapped-root user
    # namespace with a different netns and no non-loopback interfaces.
    mapping = Path("/proc/self/uid_map").read_text().split()
    require(os.getuid() == 0 and int(mapping[0]) == 0 and int(mapping[1]) != 0,
            "mapped non-root user namespace required")
    require(os.readlink("/proc/self/ns/net") != args.host_netns,
            "refusing original network namespace")
    require({name for _, name in socket.if_nameindex()} == {"lo"},
            "loopback-only network namespace required")
    require(all(row["kind"] == "noqueue" for row in qdiscs()), "preexisting qdisc refused")
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    evidence = {"classification": "local_shared_loopback_netem_relay", "kernel": os.uname().release,
                "test_sha256": sha(args.test), "profiles": [], "physical_network_verified": False,
                "actual_minecraft_worlds_verified": False, "bandwidth_fairness_guaranteed": False}
    env = {key: value for key, value in os.environ.items() if not key.startswith("SWARMCRAFT_")}
    env.update(SWARMCRAFT_DISCOVERY_MDNS="0", RUST_LOG="off", NO_COLOR="1")
    try:
        for rate, delay in PROFILES:
            row = {"rate_mbit": rate, "delay_ms": delay, "passed": False}
            evidence["profiles"].append(row)
            subprocess.run(["tc", "qdisc", "add", "dev", "lo", "root", "netem", "limit", "1000",
                            "delay", f"{delay}ms", "rate", f"{rate}mbit"], check=True)
            try:
                row["before"] = netem(rate, delay)
                log_path = args.artifacts / f"rate-{rate}mbit.log"
                started = time.monotonic()
                with log_path.open("x") as log:
                    result = subprocess.run([str(args.test), "--exact", TEST, "--nocapture", "--test-threads=1"],
                                            stdout=log, stderr=subprocess.STDOUT, env=env, timeout=100)
                row["elapsed_seconds"] = round(time.monotonic() - started, 3)
                row["after"] = netem(rate, delay)
                row["exit_code"] = result.returncode
                require(result.returncode == 0, f"relay test failed; inspect {log_path}")
                row["receipt"] = parse_receipt(log_path.read_text())
                row["observed_bytes"] = row["after"]["bytes"] - row["before"]["bytes"]
                row["observed_packets"] = row["after"]["packets"] - row["before"]["packets"]
                require(row["observed_packets"] > 0 and row["observed_bytes"] >= 2 * sum(
                    row["receipt"][name]["acknowledged_bytes"] for name in ("bulk", "interactive")),
                    "kernel counters do not cover acknowledged bidirectional traffic")
                row["passed"] = True
                print(f"PASS {rate} Mbit/s, {delay} ms: {row['observed_packets']} kernel packets; "
                      f"bulk {row['receipt']['bulk']['payload_kib_per_s']} KiB/s; "
                      f"interactive p95 {row['receipt']['interactive']['p95_echo_ms']} ms", flush=True)
            finally:
                subprocess.run(["tc", "qdisc", "del", "dev", "lo", "root"], check=True)
    finally:
        (args.artifacts / "result.json").write_text(json.dumps(evidence, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test", type=lambda value: Path(value).resolve(strict=True), required=True)
    parser.add_argument("--artifacts", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--host-netns", help=argparse.SUPPRESS)
    args = parser.parse_args()
    os.umask(0o077)
    if args.host_netns is not None:
        isolated_run(args)
        return
    require(os.getuid() != 0, "run as ordinary user, never sudo")
    require(args.test.is_file() and os.access(args.test, os.X_OK), "test executable required")
    args.artifacts.mkdir(mode=0o700, exist_ok=False)
    subprocess.run(["unshare", "--user", "--map-root-user", "--net", "--fork", "--kill-child", "--",
                    sys.executable, str(Path(__file__).resolve()), "--test", str(args.test),
                    "--artifacts", str(args.artifacts), "--host-netns", os.readlink("/proc/self/ns/net")],
                   check=True, timeout=230)


if __name__ == "__main__":
    main()
