#!/usr/bin/env python3
"""Disposable real-daemon Sabathon P0 localhost gateway negative smoke.

Uses the ACTUAL compiled `swarmcraft` sidecar and its authenticated private
localhost command socket, not a fabricated IPC response or mock Ready.
Exercises no actual Minecraft player or Fabric JVM and CANNOT certify P0.

The test deliberately creates an unconsented, one-member (non-quorum) world;
every route must stay unavailable even though the gateway can bind 127.0.0.1.
All world files live under a new private TemporaryDirectory, and only daemons
started by this script are signaled on cleanup. No user's real worlds change.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import time
from typing import TextIO


ROOT = Path(__file__).resolve().parents[2]
WORLD_ID = re.compile(r"World ID:\s*(scworld:[0-9a-f]{64})\b")


def run_cli(binary: Path, data: Path, *args: str, timeout: float = 24.0) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(binary), "--data-dir", str(data), *args],
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
    )


def expect(result: subprocess.CompletedProcess[str], code: int, subject: str) -> str:
    if (result.returncode == 0) != (code == 0):
        raise AssertionError(
            f"{subject} exit={result.returncode} (expected {'success' if code == 0 else 'failure'}): "
            f"stdout={result.stdout[-1200:]!r} stderr={result.stderr[-1200:]!r}"
        )
    return result.stdout


def daemon(binary: Path, data: Path, log_path: Path) -> tuple[subprocess.Popen[str], TextIO]:
    # No shell, no global kill, no user daemon and no public TCP Minecraft port.
    log = open(log_path, "w", encoding="utf8")
    process = subprocess.Popen(
        [str(binary), "--data-dir", str(data), "daemon", "--listen", "/ip4/127.0.0.1/udp/0/quic-v1"],
        stdout=log,
        stderr=subprocess.STDOUT,
        text=True,
        close_fds=True,
    )
    return process, log


def stop(process: subprocess.Popen[str] | None, log: TextIO | None) -> None:
    if process is not None and process.poll() is None:
        process.terminate()  # This exact child PID belongs to this smoke test.
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=8)
    if log is not None:
        log.close()


def await_daemon(binary: Path, data: Path, world: str, process: subprocess.Popen[str]) -> None:
    deadline = time.monotonic() + 12
    last = ""
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"test daemon exited before its private status socket was ready: {process.returncode}")
        result = run_cli(binary, data, "world", "game-route-status", world, "--json", timeout=5)
        if result.returncode == 0:
            assert_unavailable(result.stdout, world)
            return
        last = result.stderr[-500:]
        time.sleep(0.2)
    raise AssertionError(f"live daemon never published its private status endpoint: {last}")


def assert_unavailable(raw: str, world: str, *, attempted: bool = False) -> None:
    report = json.loads(raw)
    if report.get("world_id") != world or report.get("state") != "unavailable":
        raise AssertionError(f"unconsented world claimed a positive route: {report!r}")
    for flag in ("quorum_verified", "hosting_consent_verified", "server_ready", "route_verified"):
        if report.get(flag) is not False:
            raise AssertionError(f"unconsented world claimed {flag}: {report!r}")
    for field in (
        "local_gateway", "authority_peer_id", "epoch", "fencing_token", "snapshot_hash",
        "verified_at_unix_ms", "valid_until_unix_ms",
    ):
        if report.get(field) is not None:
            raise AssertionError(f"unverified route leaked previous {field}: {report!r}")
    if report.get("diagnostic") not in {
        "gateway_unavailable", "host_proof_unavailable", "route_check_timed_out"
    }:
        raise AssertionError("unconsented world failed to report a typed negative diagnostic")
    if attempted:
        session = report.get("recent_session") or {}
        if session.get("state") not in {"route_refused", "route_timed_out"}:
            raise AssertionError(f"real rejected TCP attempt was not diagnosed: {session!r}")
        if not isinstance(session.get("age_ms"), int) or not 0 <= session["age_ms"] < 30000:
            raise AssertionError("game session diagnostic was missing its bounded age")


def reserved_port(world: str) -> int:
    return 30_000 + int(world.removeprefix("scworld:")[:4], 16) % 20_000


def assert_denied_gateway(port: int) -> None:
    with socket.create_connection(("127.0.0.1", port), timeout=3) as sock:
        sock.settimeout(21)
        try:
            sock.sendall(b"\0\x10not-a-valid-minecraft-session")
            # Admission must close the socket when no independently verified
            # quorum/Fabric proof exists. No Minecraft byte is forwarded.
            received = sock.recv(1)
        except (BrokenPipeError, ConnectionResetError):
            return  # A TCP RST is a valid fail-closed refusal.
        if received:
            raise AssertionError(f"unverified game gateway returned unexpected bytes: {received!r}")


def smoke(binary: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="swarmcraft-sabathon-p0-") as data_str:
        data = Path(data_str)
        process = None
        log = None
        try:
            created = run_cli(
                binary, data, "world", "create", "--name", "Sabathon Gateway Negative Mock",
                "--minecraft", "26.1.2", "--fabric-loader", "0.19.3",
                "--visibility", "private",
            )
            output = expect(created, 0, "local disposable world creation")
            match = WORLD_ID.search(output)
            if not match:
                raise AssertionError(f"world CLI did not provide a canonical world identity: {output[-700:]}")
            world = match.group(1)
            port = reserved_port(world)

            expect(
                run_cli(binary, data, "world", "game-route-status", world, "--json"),
                1, "status without a live daemon",
            )
            expect(run_cli(binary, data, "world", "game-route-select", world), 1, "selection without a live daemon")

            process, log = daemon(binary, data, data / "daemon-first.log")
            await_daemon(binary, data, world, process)

            # An existing app data directory may have just one live game-route
            # daemon. Its private endpoint/file lock must reject a second one.
            duplicate, duplicate_log = daemon(binary, data, data / "daemon-duplicate.log")
            try:
                try:
                    status = duplicate.wait(timeout=10)
                except subprocess.TimeoutExpired as error:
                    raise AssertionError("a second daemon was admitted to the same private route socket") from error
                if status == 0 or "another local game-route daemon" not in (data / "daemon-duplicate.log").read_text():
                    raise AssertionError("duplicate daemon did not fail on the live per-data-directory ownership lock")
            finally:
                stop(duplicate, duplicate_log)

            selected = run_cli(binary, data, "world", "game-route-select", world)
            assert "not yet verified" in expect(selected, 0, "select durable local game gateway")
            assert_unavailable(
                expect(run_cli(binary, data, "world", "game-route-status", world, "--json"), 0, "fresh status"), world,
            )
            assert_denied_gateway(port)
            assert_unavailable(
                expect(run_cli(binary, data, "world", "game-route-status", world, "--json"), 0, "failed session diagnosis"),
                world, attempted=True,
            )

            # Crash/restart of the daemon alone must never manufacture a
            # positive route from its saved port or migration status file.
            stop(process, log)
            process = None
            log = None
            expect(
                run_cli(binary, data, "world", "game-route-status", world, "--json"),
                1, "status after the live daemon stopped",
            )
            process, log = daemon(binary, data, data / "daemon-restarted.log")
            await_daemon(binary, data, world, process)
            expect(run_cli(binary, data, "world", "game-route-select", world), 0, "reselect same reserved gateway")
            assert_denied_gateway(port)
            assert_unavailable(
                expect(run_cli(binary, data, "world", "game-route-status", world, "--json"), 0, "post-restart status"), world,
                attempted=True,
            )
            print(f"PASS: real daemon/private CLI, no cached proof, duplicate-owner refusal, stable loopback port {port},")
            print("      closed unauthorized TCP sessions, restart and continued fail-closed status")
            print("NOTE: This is an unconsented one-peer negative MOCK, not real three-machine Minecraft acceptance.")
        finally:
            stop(process, log)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/swarmcraft")
    args = parser.parse_args()
    binary = args.cli.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error(f"build actual SwarmCraft CLI first: {binary}")
    smoke(binary)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
