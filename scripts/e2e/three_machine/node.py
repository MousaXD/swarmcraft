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


def canonical_world_id(value):
    """Normalize exact SwarmCraft CLI WorldId display (scworld:<hex>)."""
    if not isinstance(value, str):
        return None
    raw = value.removeprefix("scworld:")
    if not re.fullmatch(r"[0-9a-fA-F]{64}", raw):
        return None
    return raw.lower()


def canonical_peer_id(value):
    """Normalize a typed PeerId only, disallow unsanitized CLI text."""
    if not isinstance(value, str):
        return None
    raw = value.removeprefix("scpeer:")
    if not re.fullmatch(r"[0-9a-fA-F]{64}", raw):
        return None
    return "scpeer:" + raw.lower()


def packaged_sidecars_present(file_paths):
    """All four actual Tauri-packaged sidecars must remain executable."""
    required = ("swarmcraft-host", "swarmcraft-runtime", "swarmcraft-import")
    for name in required:
        if not any(Path(path).name.startswith(name) and
                   Path(path).is_file() and os.access(path, os.X_OK)
                   for path in file_paths):
            return False
    return True


def deb_package_metadata(package):
    """Read one Debian field per invocation; querying multiple labels prints
    'Package: ...' and 'Version: ...', not bare values.
    """
    pkg = run(["dpkg-deb", "-f", str(package), "Package"])
    version = run(["dpkg-deb", "-f", str(package), "Version"])
    if pkg.returncode or version.returncode:
        return None
    package_name, package_version = pkg.stdout.strip(), version.stdout.strip()
    if (not re.fullmatch(r"[a-z0-9][a-z0-9+.-]{1,127}", package_name) or
            not re.fullmatch(r"[0-9][0-9A-Za-z.+:~_-]{0,127}", package_version)):
        return None
    return package_name, package_version


def installed_package_integrity(package_name):
    """Reject locally modified dpkg-owned package files.

    dpkg --verify checks installed files against the installed package
    database. This does NOT cryptographically bind the installation to the
    separate pinned candidate .deb; independent artifact review is required.
    """
    verified = run(["dpkg", "--verify", package_name], timeout=60)
    return (verified.returncode == 0 and not verified.stdout.strip() and
            not verified.stderr.strip())


def preflight(a):
    package = Path(a.package).resolve(strict=True)
    binary = Path(a.binary).resolve(strict=True)
    cli = Path(a.cli).resolve(strict=True)
    if package.suffix != ".deb" or not all(os.access(p, os.X_OK) for p in (binary, cli)):
        fail("package_or_binary_invalid")
    metadata = deb_package_metadata(package)
    if metadata is None:
        fail("deb_metadata_invalid")
    pkg, version = metadata
    installed = run(["dpkg-query", "-W", "-f=\x24{Status} \x24{Version}", pkg])
    owner = run(["dpkg-query", "-S", str(binary)])
    cli_owner = run(["dpkg-query", "-S", str(cli)])
    if (installed.returncode or installed.stdout.strip() != "install ok installed " + version
            or owner.returncode or not owner.stdout.startswith(pkg + ":")
            or cli_owner.returncode or not cli_owner.stdout.startswith(pkg + ":")):
        fail("package_not_installed")
    if not installed_package_integrity(pkg):
        fail("installed_package_integrity_failed")
    files = run(["dpkg-query", "-L", pkg])
    if files.returncode or not packaged_sidecars_present(files.stdout.splitlines()):
        fail("required_installed_tauri_sidecars_missing")
    profile = Path(a.data_dir)
    if profile.exists() and any(profile.iterdir()):
        fail("preflight_requires_fresh_profile")
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
        "app_installed": True, "cli_installed": True,
        "all_required_sidecars_installed": True, "os": sys.platform,
    }


def fabric_client_process(pid, server_pid=None):
    """Require a real Fabric Minecraft client JVM, never an argv substring.

    JVM provenance is necessary but not sufficient: independent field review
    must still prove genuine game login, joins and shared marker state.
    """
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 1 or pid == server_pid:
        return False
    try:
        executable = Path(f"/proc/{pid}/exe").resolve(strict=True)
        if executable.name.lower() not in ("java", "javaw"):
            return False
        raw = Path(f"/proc/{pid}/cmdline").read_bytes()
        if not raw or len(raw) > 131072:
            return False
        argv = [arg.decode("utf-8", errors="replace") for arg in raw.split(b"\0") if arg]
        if len(argv) < 3 or not any(arg in ("-cp", "-classpath", "--class-path") for arg in argv):
            return False
        return ("net.fabricmc.loader.impl.launch.knot.KnotClient" in argv or
                ("org.prismlauncher.EntryPoint" in argv and
                 any("fabric-loader" in arg.lower() for arg in argv)))
    except (OSError, ValueError, UnicodeError):
        return False


def canonical_head_digest(data_dir, world):
    """Digest the exact persisted canonical head from SwarmCraft Storage.

    The Rust CLI's world verify must separately succeed; hashing a file
    alone never authenticates a signed snapshot or proves it is current.
    Replica-only peers may correctly have no migration snapshot_hash.
    """
    path = (Path(data_dir) / "worlds" / world / "metadata" /
            "canonical-snapshot-head.postcard")
    try:
        if path.is_symlink() or not path.is_file():
            return None
        if not 40 <= path.stat().st_size <= 16384:
            return None
        return digest(path)
    except (OSError, ValueError):
        return None


def log_digests(data_dir):
    """Hash bounded test-profile log files; never return log text or filenames."""
    folder = Path(data_dir) / "logs"
    if not folder.is_dir():
        return []
    records = []
    for file in sorted(folder.glob("*.log"))[:16]:
        try:
            if file.is_symlink() or not file.is_file():
                continue
            size = file.stat().st_size
            if size > 20 * 1024 * 1024:
                continue
            records.append({"sha256": digest(file), "bytes": size})
        except OSError:
            continue
    return records


def connectivity_summary(data_dir):
    """Optional backend JSON diagnostics; exclude addresses, relays, peer IDs and errors."""
    path = Path(data_dir) / "connectivity-diagnostics-v1.json"
    try:
        if not path.is_file() or path.is_symlink() or path.stat().st_size > 65536:
            return {"available": False}
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            return {"available": False}
        state = data.get("state")
        allowed = {
            "nat_status_unknown", "direct_reachable", "hole_punched",
            "relay_connected", "relay_required", "private_unreachable",
            "bootstrap_unavailable", "no_viable_path",
        }
        nat = data.get("nat_status")
        punch = data.get("hole_punch")
        if isinstance(punch, dict):
            punch = "failed" if "failed" in punch else "other"
        return {
            "available": True,
            "state": state if state in allowed else "unknown",
            "nat_status": nat if nat in ("public", "private", "unknown") else "unknown",
            "direct_connectivity": data.get("direct_connectivity") is True,
            "relay_connectivity": data.get("relay_connectivity") is True,
            "hole_punch": punch if punch in ("idle", "attempting", "succeeded", "failed") else "unknown",
            "ipv4_available": data.get("ipv4_available") is True,
            "ipv6_available": data.get("ipv6_available") is True,
        }
    except (OSError, UnicodeError, ValueError, TypeError):
        return {"available": False}


def typed_id_bytes(value):
    """Decode SwarmCraft's fixed 32-byte Serde ID array, not an untrusted text alias."""
    if not isinstance(value, list) or len(value) != 32:
        return None
    if any(type(part) is not int or not 0 <= part <= 255 for part in value):
        return None
    return bytes(value)


def hosting_consent(data_dir, world, peer):
    """Read Agent 03's signed-world-bound, *local* opt-in record if integrated.

    File availability is not permission to run a JVM; authority still depends
    on Rust admission, current memory headroom, signed policy and quorum.
    """
    path = Path(data_dir) / "control" / world / "hosting-consent.json"
    try:
        if not path.is_file() or path.is_symlink() or path.stat().st_size > 16384:
            return {"available": False}
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, dict) or data.get("schema_version") != 1:
            return {"available": False}
        world_id = typed_id_bytes(data.get("world_id"))
        peer_id = typed_id_bytes(data.get("peer_id"))
        expected_peer = peer.removeprefix("scpeer:")
        if world_id is None or peer_id is None or world_id.hex() != world.lower() or peer_id.hex() != expected_peer:
            return {"available": False}
        enabled = data.get("enabled")
        heap = data.get("heap_mib")
        revision = data.get("revision")
        if (type(enabled) is not bool or type(heap) is not int or type(revision) is not int
                or revision <= 0 or not 0 <= heap <= 65536 or (enabled and heap < 512)
                or (not enabled and heap != 0)):
            return {"available": False}
        return {
            "available": True, "enabled": enabled,
            "heap_mib": heap, "revision": revision,
            "world_bound": True, "identity_bound": True,
        }
    except (OSError, TypeError, ValueError, UnicodeError):
        return {"available": False}


def runtime_heap_mib(pid, root, world, binary, cli):
    if not isolated_process(pid, "server", root, world, binary, cli):
        return None
    try:
        arguments = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
        for arg in arguments:
            try:
                value = arg.decode("ascii", errors="strict")
            except UnicodeDecodeError:
                # Localized executable/world paths may be non-ASCII. That
                # must not conceal a valid standalone ASCII -Xmx argument.
                continue
            match = re.fullmatch(r"-Xmx([1-9][0-9]*)([mMgG])", value)
            if match:
                number = int(match.group(1))
                mib = number * (1024 if match.group(2).lower() == "g" else 1)
                return mib if 0 < mib <= 65536 else None
    except (OSError, UnicodeError, ValueError):
        pass
    return None


VERSION = re.compile(r"^[0-9][0-9A-Za-z._+-]{0,47}$")


def safe_version(text):
    if isinstance(text, str):
        text = text.strip()
        if VERSION.fullmatch(text):
            return text
    return None


def java_runtime_version(pid, root, world, binary, cli):
    """Report version of the exact verified JVM executable, not PATH's java."""
    if not isolated_process(pid, "server", root, world, binary, cli):
        return None
    try:
        executable = Path(f"/proc/{pid}/exe").resolve(strict=True)
        version = run([str(executable), "-version"], timeout=8)
        if version.returncode:
            return None
        # OpenJDK uses stderr, e.g. 'openjdk version "25.0.1"'.
        combined = (version.stderr + "\n" + version.stdout)[:1024]
        match = re.search(r"""(?:openjdk|java) version ["']([0-9][0-9A-Za-z._+-]{0,47})["']""", combined, re.I)
        if not match:
            match = re.search(r'openjdk ([0-9][0-9A-Za-z._+-]{0,47})', combined, re.I)
        return safe_version(match.group(1)) if match else None
    except (OSError, subprocess.TimeoutExpired, UnicodeError, ValueError):
        return None


def capture(a):
    if not re.fullmatch("[a-fA-F0-9]{64}", a.world):
        fail("world_id_invalid")
    root = str(Path(a.data_dir).resolve(strict=True))
    cli = str(Path(a.cli).resolve(strict=True))
    cmd = [cli, "--data-dir", root]
    status = run(cmd + ["world", "status", a.world])
    observed_world = canonical_world_id(field(status.stdout, "World ID"))
    if status.returncode or observed_world is None or observed_world != a.world.lower():
        fail("world_status_unavailable")
    identity = run(cmd + ["identity"])
    peer = canonical_peer_id(field(identity.stdout, "Peer ID")) if identity.returncode == 0 else None
    if peer is None:
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
    if canonical_world_id(m.get("world_id")) != observed_world or canonical_world_id(r.get("world_id")) != observed_world:
        fail("backend_world_reference_mismatch")
    authority = canonical_peer_id(m.get("authority_peer_id"))
    if authority is None:
        fail("backend_authority_missing")
    successor = canonical_peer_id(r.get("successor_peer_id"))
    snapshot = m.get("snapshot_hash")
    return {
        "world_fingerprint": fp(a.salt, a.world),
        "identity_fingerprint": fp(a.salt, peer),
        "authority_fingerprint": fp(a.salt, authority),
        "successor_fingerprint": fp(a.salt, successor) if successor else None,
        "authorized_peers": integer(field(status.stdout, "Authorized peers")),
        "minecraft_version": safe_version(field(status.stdout, "Minecraft")),
        "fabric_loader_version": safe_version(field(status.stdout, "Fabric loader")),
        "snapshot_number": integer(field(status.stdout, "Latest snapshot")),
        "snapshot_epoch": integer(field(status.stdout, "Epoch")),
        "state_root": field(status.stdout, "State root"),
        # The migration supervisor can be blocked (and omit snapshot_hash)
        # on a deliberately player-only replica; signed storage owns the
        # canonical head, not this optional runtime status.
        "snapshot_hash": snapshot if isinstance(snapshot, str) and re.fullmatch("[a-fA-F0-9]{64}", snapshot) else None,
        "canonical_head_sha256": canonical_head_digest(root, observed_world),
        "snapshot_verify_passed": verify.returncode == 0,
        "canonical": field(status.stdout, "Safety") == "Canonical",
        "safety_level": field(status.stdout, "Safety") if field(status.stdout, "Safety") in
                        ("Canonical", "SoloUnreplicated", "Conflict") else "Unknown",
        "epoch": integer(m.get("epoch")),
        "fencing_token": integer(m.get("fencing_token")),
        "phase": m.get("phase") if isinstance(m.get("phase"), str) else None,
        "runtime_ready": m.get("runtime_ready") is True,
        "host_readiness_state": r.get("state") if isinstance(r.get("state"), str) else None,
        "safe_to_shutdown": r.get("safe_to_shutdown") is True,
        "desktop_process_observed": isolated_process(
            a.desktop_pid, "desktop", Path(root), a.world,
            Path(a.binary).resolve(strict=True), Path(a.cli).resolve(strict=True)),
        "minecraft_client_process_observed": fabric_client_process(a.client_pid, a.server_pid),
        "minecraft_server_process_observed": isolated_process(
            a.server_pid, "server", Path(root), a.world,
            Path(a.binary).resolve(strict=True), Path(a.cli).resolve(strict=True)),
        "hosting_consent": hosting_consent(root, a.world, peer),
        "java_runtime_version": java_runtime_version(
            a.server_pid, Path(root), a.world,
            Path(a.binary).resolve(strict=True), Path(a.cli).resolve(strict=True)),
        "minecraft_server_heap_mib": runtime_heap_mib(
            a.server_pid, Path(root), a.world,
            Path(a.binary).resolve(strict=True), Path(a.cli).resolve(strict=True)),
        "test_profile_log_digests": log_digests(root),
        "connectivity_diagnostics": connectivity_summary(root),
    }


def process_command(pid):
    return Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace").lower()


def isolated_process(pid, kind, root, world, binary, cli):
    """Verify PID belongs to this exact ephemeral world before SIGSTOP/SIGKILL."""
    if not pid or pid <= 1 or pid == os.getpid():
        return False
    try:
        command = process_command(pid)
        environ = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
        profile = ("SWARMCRAFT_DATA_DIR=" + str(root)).encode()
        matches_profile = profile in environ or (
            "--data-dir" in command and str(root).lower() in command
        )
        executable = Path(f"/proc/{pid}/exe").resolve(strict=True)
        cwd = Path(f"/proc/{pid}/cwd").resolve(strict=True)
        if kind == "desktop":
            return matches_profile and executable == binary and "swarmcraft" in command
        if kind == "daemon":
            return matches_profile and "swarmcraft" in executable.name.lower() and "daemon" in command
        if kind == "server":
            return (
                cwd == root / "runtime" / world
                and "java" in executable.name.lower()
                and "-jar" in command
                and ("fabric" in command or ".jar" in command)
            )
    except (OSError, UnicodeError, ValueError):
        return False
    return False


def process_exited(pid):
    location = Path(f"/proc/{pid}")
    if not location.exists():
        return True
    try:
        # A zombie has already lost all file handles/listeners and cannot
        # regain authority; a parent may reap it after the SSH call ends.
        stat = (location / "stat").read_text()
        return stat.rsplit(") ", 1)[1].split(" ", 1)[0] in ("Z", "X")
    except (OSError, IndexError):
        return not location.exists()


def crash(a):
    if a.confirm != "CRASH-ALICE-AUTHORITY":
        fail("crash_confirmation_missing")
    if not a.world or not re.fullmatch("[a-fA-F0-9]{64}", a.world):
        fail("crash_world_invalid")
    root = Path(a.data_dir).resolve(strict=True)
    targets = [
        ("desktop", a.desktop_pid),
        ("daemon", a.daemon_pid),
        ("server", a.server_pid),
    ]
    pids = [pid for _, pid in targets]
    if any(not pid or pid <= 1 for pid in pids) or len(set(pids)) != 3:
        fail("three_distinct_crash_targets_required")
    binary = Path(a.binary).resolve(strict=True)
    cli = Path(a.cli).resolve(strict=True)
    if any(not isolated_process(pid, kind, root, a.world, binary, cli) for kind, pid in targets):
        fail("crash_targets_not_isolated_test_owned_processes")
    # Freeze all three before killing any to prevent Tauri or Rust from
    # repairing a deliberately crashed child during partial shutdown.
    frozen = []
    try:
        for _, pid in targets:
            os.kill(pid, signal.SIGSTOP)
            frozen.append(pid)
        for _, pid in targets:
            os.kill(pid, signal.SIGKILL)
    except OSError:
        for pid in frozen:
            try:
                os.kill(pid, signal.SIGCONT)
            except OSError:
                pass
        fail("crash_injection_partial_failure")
    for _ in range(40):
        if all(process_exited(pid) for pid in pids):
            return {
                "signal": "SIGKILL", "process_gone": True, "role": "alice",
                "crashed_components": ["desktop", "daemon", "server"],
                "crash_scope": "isolated_application_processes_not_machine_power",
            }
        time.sleep(0.25)
    fail("crash_processes_still_alive")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("operation", choices=["preflight", "capture", "crash"])
    for name in ("salt", "package", "binary", "cli", "data-dir", "world", "confirm"):
        p.add_argument("--" + name)
    for name in ("pid", "desktop-pid", "daemon-pid", "client-pid", "server-pid"):
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
