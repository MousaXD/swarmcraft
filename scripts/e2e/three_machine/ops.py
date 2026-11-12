#!/usr/bin/env python3
"""Optional, explicitly approved three-machine package staging/installation.

Never creates user identities/worlds or removes installed apps/player saves.
No passwords on stdin; remote sudo must already be appropriately authorized.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shlex
import subprocess
import sys

import run as gate


def checked_destination(path):
    candidate = PurePosixPath(path)
    # Fixed dedicated area: no staging into home, arbitrary /tmp or user saves.
    gate.require(str(candidate).startswith("/opt/swarmcraft-e2e/"), "installer_must_be_under_dedicated_opt_dir")
    gate.require(candidate.name.endswith(".deb") and
                 all(part not in ("..", ".") for part in candidate.parts),
                 "installer_path_invalid")
    gate.require(all(ch.isalnum() or ch in "_-./" for ch in str(candidate)), "installer_path_unsafe")
    return str(candidate)


def run_command(argv, timeout):
    try:
        completed = subprocess.run(argv, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise gate.Blocked("remote_provision_command_unavailable") from exc
    # Intentionally do not forward STDERR, SSH banners, machine names or
    # dpkg post-install output to public evidence.
    gate.require(completed.returncode == 0, "remote_provision_command_failed")
    return completed.stdout.decode("utf-8", errors="replace").strip()


def ssh(role, config, script, timeout=90):
    target = config["nodes"][role]["ssh"]
    return run_command(["ssh", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
                        "-o", "ConnectTimeout=12", target, script], timeout)


def checksum(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1048576), b""):
            h.update(chunk)
    return h.hexdigest()


def provision(config, deb, output, confirmation):
    gate.require(confirmation == "INSTALL-THREE-REAL-TEST-MACHINES", "explicit_install_approval_required")
    package = Path(deb).resolve(strict=True)
    gate.require(package.is_file() and package.suffix == ".deb", "real_deb_file_required")
    gate.require(checksum(package) == config["package_sha256"], "local_candidate_package_sha_mismatch")
    installed = []
    for role in gate.ROLES:
        destination = checked_destination(config["nodes"][role]["package"])
        target = config["nodes"][role]["ssh"] + ":" + destination
        run_command(["scp", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
                     "-o", "ConnectTimeout=12", str(package), target], timeout=180)
        observed = ssh(role, config, "sha256sum -- " + shlex.quote(destination), timeout=30).split()
        gate.require(observed and observed[0].lower() == config["package_sha256"].lower(),
                     "remote_package_sha_mismatch_" + role)
        # Never attempt sudo password prompts, automatic dependency upgrades,
        # reboot, package purge, or modification of user data.
        ssh(role, config, "sudo -n dpkg -i -- " + shlex.quote(destination), timeout=180)
        installed.append(role)
        gate.save(output / "provision.json", {
            "commit": config["commit"], "package_sha256": config["package_sha256"],
            "independently_installed_roles": installed,
            "scope": "test_debian_package_only",
        })


def cleanup_stage(config, output, confirmation):
    gate.require(confirmation == "DELETE-ONLY-STAGED-E2E-DEBS", "explicit_cleanup_approval_required")
    removed = []
    for role in gate.ROLES:
        destination = checked_destination(config["nodes"][role]["package"])
        expected = config["package_sha256"].lower()
        observed = ssh(role, config, "sha256sum -- " + shlex.quote(destination), timeout=30).split()
        gate.require(observed and observed[0].lower() == expected, "staged_package_sha_mismatch_" + role)
        ssh(role, config, "rm -- " + shlex.quote(destination), timeout=30)
        removed.append(role)
        gate.save(output / "staged-package-cleanup.json", {
            "commit": config["commit"], "removed_staged_debs": removed,
            "installed_apps_removed": False, "world_or_player_data_removed": False,
        })


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["provision", "cleanup-stage"])
    p.add_argument("--config", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--deb")
    p.add_argument("--confirm")
    args = p.parse_args()
    try:
        cfg = gate.config(args.config)
        out = Path(args.output)
        if args.action == "provision":
            gate.require(args.deb, "local_candidate_deb_required")
            provision(cfg, args.deb, out, args.confirm)
        else:
            cleanup_stage(cfg, out, args.confirm)
        print(json.dumps({"action": args.action, "verdict": "STAGING_COMPLETE_NOT_CERTIFIED"}))
    except (gate.Blocked, OSError, ValueError, KeyError) as error:
        reason = str(error) if isinstance(error, gate.Blocked) else "staging_failed"
        print(json.dumps({"verdict": "BLOCKED", "reason": reason}), file=sys.stderr)
        raise SystemExit(2) from None


if __name__ == "__main__":
    main()
