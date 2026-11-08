#!/usr/bin/env python3
"""Remote SwarmCraft probe. Output is an allowlisted JSON record, never raw CLI output."""
from __future__ import annotations
import argparse
import hashlib
import hmac
import ipaddress
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time
import urllib.request


def fail(code):
    print(json.dumps({"ok": False, "error_code": code}))
    raise SystemExit(2)


def fp(salt, value):
    return hmac.new(bytes.fromhex(salt), str(value).encode(), hashlib.sha256).hexdigest()


def run(argv, timeout=15):
    return subprocess.run(argv, capture_output=True, text=True, timeout=timeout, check=False)


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1048576), b""):
            h.update(chunk)
    return h.hexdigest()


def field(output, key):
    found = re.search(r"^" + re.escape(key) + r":\s*(.+?)\s*$", output, re.M)
    return found.group(1) if found else None


def integer(value):
    try:
        n = int(value)
        return n if 0 <= n <= 2**63 - 1 else None
    except (ValueError, TypeError):
        return None


def preflight(a):
    package = Path(a.package).resolve(strict=True)
    binary = Path(a.binary).resolve(strict=True)
    cli = Path(a.cli).resolve(strict=True)
    if package.suffix != ".deb" or not all(os.access(p, os.X_OK) for p in (binary, cli)):
        fail("package_or_binary_invalid")
    meta = run(["dpkg-deb", "-f", str(package), "Package", "Version"])
    if meta.returncode or len(meta.stdout.splitlines()) != 2:
        fail("deb_metadata_invalid")
    pkg, version = meta.stdout.splitlines()
    installed = run(["dpkg-query", "-W", "-f=\x24{Status} \x24{Version}", pkg])
    owner = run(["dpkg-query", "-S", str(binary)])
    if (installed.returncode or installed.stdout.strip() != "install ok installed " + version
            or owner.returncode or not owner.stdout.startswith(pkg + ":")):
        fail("package_not_installed")
    machine_id = Path("/etc/machine-id").read_text().strip()
    if not machine_id:
        fail("machine_id_missing")
    try:
        with urllib.request.urlopen("https://api.ipify.org", timeout=12) as response:
            public_ip = response.read(80).decode("ascii").strip()
        ipaddress.ip_address(public_ip)
    except (OSError, TimeoutError, ValueError, UnicodeError):
        fail("public_egress_unavailable")
    return {
        "package_sha256": digest(package), "package_version": version,
        "machine_fingerprint": fp(a.salt, machine_id),
        "egress_fingerprint": fp(a.salt, public_ip),
        "app_installed": True, "cli_installed": True, "os": sys.platform,
    }


def observed(pid, terms):
    if pid is None:
        return False
    try:
        cmd = Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace").lower()
        return any(term in cmd for term in terms)
    except OSError:
        return False


def capture(a):
    if not re.fullmatch("[a-fA-F0-9]{64}", a.world):
        fail("world_id_invalid")
    root = str(Path(a.data_dir).resolve(strict=True))
    cli = str(Path(a.cli).resolve(strict=True))
    cmd = [cli, "--data-dir", root]
    status = run(cmd + ["world", "status", a.world])
    if status.returncode or field(status.stdout, "World ID") != a.world:
        fail("world_status_unavailable")
    identity = run(cmd + ["identity"])
    peer = field(identity.stdout, "Peer ID") if identity.returncode == 0 else None
    if not peer:
        fail("device_identity_missing")
    verify = run(cmd + ["world", "verify", a.world], timeout=45)
    migration = run(cmd + ["world", "migration-status", a.world, "--json"])
    readiness = run(cmd + ["world", "host-readiness", a.world, "--json"])
    if migration.returncode or readiness.returncode:
        fail("backend_status_unavailable")
    try:
        m, r = json.loads(migration.stdout), json.loads(readiness.stdout)
    except (TypeError, ValueError):
        fail("backend_status_invalid")
    authority = m.get("authority_peer_id")
    if not isinstance(authority, str) or not authority:
        fail("backend_authority_missing")
    successor = r.get("successor_peer_id")
    snapshot = m.get("snapshot_hash")
    return {
        "world_fingerprint": fp(a.salt, a.world),
        "identity_fingerprint": fp(a.salt, peer),
        "authority_fingerprint": fp(a.salt, authority),
        "successor_fingerprint": fp(a.salt, successor) if successor else None,
        "authorized_peers": integer(field(status.stdout, "Authorized peers")),
        "snapshot_number": integer(field(status.stdout, "Latest snapshot")),
        "snapshot_epoch": integer(field(status.stdout, "Epoch")),
        "state_root": field(status.stdout, "State root"),
        "snapshot_hash": snapshot if isinstance(snapshot, str) and re.fullmatch("[a-fA-F0-9]{64}", snapshot) else None,
        "snapshot_verify_passed": verify.returncode == 0,
        "canonical": field(status.stdout, "Safety") == "Canonical",
        "epoch": integer(m.get("epoch")),
        "fencing_token": integer(m.get("fencing_token")),
        "phase": m.get("phase") if isinstance(m.get("phase"), str) else None,
        "runtime_ready": m.get("runtime_ready") is True,
        "host_readiness_state": r.get("state") if isinstance(r.get("state"), str) else None,
        "safe_to_shutdown": r.get("safe_to_shutdown") is True,
        "desktop_process_observed": observed(a.desktop_pid, ("swarmcraft-desktop",)),
        "minecraft_client_process_observed": observed(a.client_pid, ("minecraft", "java")),
        "minecraft_server_process_observed": observed(a.server_pid, ("java",)),
    }


def crash(a):
    if a.confirm != "CRASH-ALICE-AUTHORITY":
        fail("crash_confirmation_missing")
    if not re.fullmatch("[a-fA-F0-9]{64}", a.world) or not a.pid or a.pid <= 1 or a.pid == os.getpid():
        fail("crash_target_invalid")
    root = Path(a.data_dir).resolve(strict=True)
    cwd = Path(f"/proc/{a.pid}/cwd").resolve(strict=True)
    cmd = Path(f"/proc/{a.pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace").lower()
    if not (cwd == root or root in cwd.parents or a.world.lower() in cmd.lower()):
        fail("target_not_bound_to_isolated_world")
    if "java" not in cmd and "swarmcraft" not in cmd:
        fail("target_not_swarmcraft_runtime")
    os.kill(a.pid, signal.SIGKILL)
    for _ in range(40):
        if not Path(f"/proc/{a.pid}").exists():
            return {"signal": "SIGKILL", "process_gone": True, "role": "alice"}
        time.sleep(0.25)
    fail("crashed_pid_not_reaped")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("operation", choices=["preflight", "capture", "crash"])
    for name in ("salt", "package", "binary", "cli", "data-dir", "world", "confirm"):
        p.add_argument("--" + name)
    for name in ("pid", "desktop-pid", "client-pid", "server-pid"):
        p.add_argument("--" + name, type=int)
    a = p.parse_args()
    if not a.salt or not re.fullmatch("[a-fA-F0-9]{64}", a.salt):
        fail("fingerprint_salt_invalid")
    try:
        data = {"ok": True, "operation": a.operation}
        data.update({"preflight": preflight, "capture": capture, "crash": crash}[a.operation](a))
        print(json.dumps(data, sort_keys=True))
    except (OSError, subprocess.TimeoutExpired, ValueError, TypeError):
        fail("remote_probe_failed")


if __name__ == "__main__":
    main()
