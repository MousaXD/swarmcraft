#!/usr/bin/env python3
"""Local, offline package/profile upgrade rehearsal using a quiescent Java save.

Inputs are two already downloaded and verified published Debian packages, a
locally built candidate CLI and import sidecar, and an actual saved world.
All commands operate on new private test profiles; never point this at a live
profile, start a daemon/JVM, or run a historical binary on the upgraded root.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import time


def sha(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def inventory(root):
    result = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink() or not (path.is_dir() or path.is_file()):
            raise ValueError("unsafe fixture or profile entry")
        if path.is_file():
            result[str(path.relative_to(root))] = sha(path)
    return result


def run(binary, root, *args, check=True):
    env = {k: v for k, v in os.environ.items() if not k.startswith("SWARMCRAFT_")}
    env["RUST_LOG"] = "warn"
    return subprocess.run([str(binary), "--data-dir", str(root), *map(str, args)],
                          env=env, capture_output=True, text=True, timeout=90, check=check)


def value(binary, root, *args):
    return json.loads(run(binary, root, *args).stdout)


def extract(verified, label, output):
    record = json.loads((verified / f"{label}-verified.json").read_text())
    package, = [item for item in record["packages"] if item["platform"] == "linux"]
    path = verified / label / package["file"]
    assert sha(path) == package["sha256"], "published package digest mismatch"
    subprocess.run(["dpkg-deb", "-x", str(path), str(output)], check=True, timeout=90)
    bin_dir = output / "usr/bin"
    for name in ("swarmcraft", "swarmcraft-import"):
        assert (bin_dir / name).is_file() and not (bin_dir / name).is_symlink()
        assert os.access(bin_dir / name, os.X_OK)
    return bin_dir, {"tag": record["tag"], "package_sha256": package["sha256"],
                     "cli_sha256": sha(bin_dir / "swarmcraft"),
                     "import_sha256": sha(bin_dir / "swarmcraft-import")}


def main(args):
    if not __debug__:
        raise RuntimeError("acceptance assertions must be enabled")
    if args.artifacts.is_relative_to(args.save):
        raise ValueError("artifacts must not be inside the source save")
    os.umask(0o077)
    args.artifacts.mkdir(mode=0o700, parents=False, exist_ok=False)
    assert args.artifacts.stat().st_mode & 0o077 == 0
    assert (args.save / "level.dat").is_file() and (args.save / "session.lock").is_file()
    regions = [p for p in args.save.rglob("*.mca") if p.is_file() and p.stat().st_size > 8192]
    assert regions, "a real saved region is required"
    source = inventory(args.save)
    saved_bytes = sum((args.save / p).stat().st_size for p in source)
    legacy, older = extract(args.verified, "legacy", args.artifacts / "legacy")
    pinned, newer = extract(args.verified, "pinned", args.artifacts / "pinned")
    cli = args.candidate / "swarmcraft"
    importer = args.candidate / "swarmcraft-import"
    runtime = args.candidate / "swarmcraft-runtime"
    profile = args.artifacts / "upgrade-profile"
    profile.mkdir()
    root = profile / "data"
    root.mkdir()
    checks = {}
    result = {
        "scope": "actual published Debian sidecars and locally built candidate; genuine quiescent Java world; no network or Java during profile operations",
        "packages": {"legacy": older, "pinned": newer},
        "candidate_cli_sha256": sha(cli),
        "candidate_import_sha256": sha(importer),
        "saved_files": len(source), "saved_bytes": saved_bytes, "region_files": len(regions),
        "checks": checks,
    }
    try:
        imported = value(legacy / "swarmcraft-import", root, "--source", args.save,
                         "--name", "Upgrade acceptance", "--minecraft", "26.1.2",
                         "--fabric-loader", "0.19.3", "--no-server-mods")
        world = imported["world_id"]
        assert imported["files"] == len(source)
        baseline = run(legacy / "swarmcraft", root, "world", "verify", world).stdout
        identity = run(legacy / "swarmcraft", root, "identity").stdout
        result["original_checkpoint"] = imported["snapshot_hash"]
        result["original_state_root"] = imported["state_root"]
        assert imported["state_root"] in baseline
        checks["actual_legacy_import_verified"] = True
        print("PASS actual 0.5.0 saved-world import", flush=True)

        assert run(pinned / "swarmcraft", root, "world", "verify", world).stdout == baseline
        assert run(pinned / "swarmcraft", root, "identity").stdout == identity
        absent = value(pinned / "swarmcraft", root, "world", "hosting", "status", world, "--json")
        assert absent["consent_revision"] is None and not absent["consented"]
        enabled = value(pinned / "swarmcraft", root, "world", "hosting", "set", world, "--heap-mib", 512, "--json")
        assert enabled["consented"] and enabled["selected_heap_mib"] == 512
        revoked = value(pinned / "swarmcraft", root, "world", "hosting", "set", world, "--heap-mib", 0, "--json")
        assert not revoked["consented"] and revoked["selected_heap_mib"] == 0
        assert int(revoked["consent_revision"]) > int(enabled["consent_revision"])
        checks["actual_pinned_preview_preserves_identity_and_explicit_consent_transitions"] = True
        before = inventory(root)
        # No permission synthesis or canonical changes during the upgrade.
        preflight = value(cli, root, "profile", "check")
        assert preflight["compatibility"] is None and preflight["world_metadata_count"] == 1
        assert inventory(root) == before
        checks["read_only_preflight"] = True
        marker = root / "profile-compatibility.json"
        with (root / ".profile-compatibility.lock").open("a+b") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            process = subprocess.Popen([str(cli), "--data-dir", str(root), "world", "list"],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                time.sleep(0.5)
                assert process.poll() is None and not marker.exists()
                process.kill()
                process.communicate(timeout=10)
                assert process.returncode < 0
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
        assert all(inventory(root).get(k) == v for k, v in before.items())
        checks["real_killed_candidate_waiting_for_activation"] = True
        # Simulate a crash after an incomplete staging write, before rename.
        staged = root / ".atomic-profile-interrupted.tmp"
        staged.write_bytes(b'{"format_schema":')
        assert value(cli, root, "profile", "check")["compatibility"] is None
        assert world in run(cli, root, "world", "list").stdout
        floor = value(cli, root, "profile", "check")["compatibility"]
        assert floor == {"format_schema": 1, "minimum_reader_generation": 1,
                         "storage_schema_version": 1, "protocol_version": 1}
        assert staged.read_bytes() == b'{"format_schema":'
        checks["restart_activates_complete_floor"] = True
        for _ in range(2):
            assert run(cli, root, "identity").stdout == identity
            verify = run(cli, root, "world", "verify", world).stdout
            assert baseline.strip() in verify and imported["snapshot_hash"] in verify
            consent = value(cli, root, "world", "hosting", "status", world, "--json")
            assert not consent["consented"] and not consent["may_host_now"] and consent["selected_heap_mib"] == 0
            assert consent["consent_revision"] == revoked["consent_revision"]
            current = inventory(root)
            assert all(current.get(k) == v for k, v in before.items())
        result["compatibility_floor"] = floor
        result["consent_transitions"] = ["absent", "explicit_512", "explicit_withdrawal"]
        result["latest_revision_preserved"] = True
        checks["same_profile_upgrade_and_restart_preserve_exact_bytes"] = True
        print("PASS current upgrade, interrupted activation, saved bytes and revoked consent", flush=True)

        # The supported rollback preflight must refuse either old release
        # on this initialized profile. Do not invoke them after activation.
        for target in ("published-0.5.0", "published-0.5.1"):
            state = inventory(root)
            denied = run(cli, root, "profile", "check", "--target", target, check=False)
            assert denied.returncode != 0 and "unsupported downgrade into an initialized profile" in denied.stderr
            assert inventory(root) == state
            empty = args.artifacts / f"empty-{target}"
            assert not value(cli, empty, "profile", "check", "--target", target)["initialized"]
            assert not empty.exists()
        checks["both_historical_rollback_targets_refused"] = True

        # The legacy imported world predates certified genesis epoch 1.
        # Its signed payload remains recoverable without inventing authority.
        export = args.artifacts / "verified-export"
        run(cli, root, "world", "recover", world, "1", export)
        assert inventory(export) == source
        checks["verified_legacy_data_only_export_exact"] = True
        key = args.artifacts / "backup.key"
        bundle = args.artifacts / "new-copy.scbackup"
        value(cli, root, "backup", "keygen", "--destination", key)
        denied = run(cli, root, "backup", "export", world, "--checkpoint", imported["snapshot_hash"],
                     "--key-file", key, "--destination", args.artifacts / "legacy.scbackup", check=False)
        assert denied.returncode != 0 and not (args.artifacts / "legacy.scbackup").exists()
        checks["legacy_certified_history_export_refuses_unsupported_genesis"] = True
        # Verified files are imported into a separate fresh peer and world,
        # not rolled back into the original live identity.
        fresh = args.artifacts / "recovered-profile"
        copy = value(importer, fresh, "--source", export, "--name", "Recovered data only",
                     "--minecraft", "26.1.2", "--fabric-loader", "0.19.3", "--no-server-mods")
        assert copy["world_id"] != world
        assert run(cli, fresh, "identity").stdout != identity
        assert copy["state_root"] == imported["state_root"]
        status = value(cli, fresh, "world", "hosting", "status", copy["world_id"], "--json")
        assert not status["consented"] and status["consent_revision"] is None
        prepared = value(runtime, fresh, "status", copy["world_id"])
        assert not prepared["ready"] and not prepared["eula_accepted"]
        receipt = value(cli, fresh, "backup", "export", copy["world_id"], "--checkpoint", copy["snapshot_hash"],
                        "--key-file", key, "--destination", bundle)
        assert not receipt["authority_granted"]
        restored = args.artifacts / "encrypted-restored"
        recipient = args.artifacts / "untouched-recipient"
        receipt = value(cli, recipient, "backup", "import", "--expected-world", copy["world_id"],
                        "--expected-checkpoint", copy["snapshot_hash"], "--key-file", key,
                        "--bundle", bundle, "--destination", restored, "--confirm-data-only")
        assert not receipt["authority_granted"] and not receipt["registered_world"]
        assert not recipient.exists() and inventory(restored / "world") == source
        assert {p.name for p in restored.iterdir()} == {"world", "backup-receipt.json", "signed-manifest.postcard", "signed-config.postcard"}
        assert json.loads((restored / "backup-receipt.json").read_text()) == receipt
        assert inventory(args.save) == source
        final = inventory(root)
        assert all(final.get(k) == v for k, v in before.items())
        checks["new_identity_data_only_recovery_and_encrypted_roundtrip"] = True
        print("PASS guarded downgrade refusal, verified legacy export and encrypted fresh-copy roundtrip", flush=True)
    finally:
        (args.artifacts / "result.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("verified", "candidate", "save", "artifacts"):
        parser.add_argument("--" + name, type=lambda value: Path(value).resolve(), required=True)
    main(parser.parse_args())
