#!/usr/bin/env python3
"""Verify extracted package sidecars and redacted support on a fresh real-save import.

No live profile, Java process, installer or external network is used.
Only the caller's new artifact directory is written. The source save must
already be quiescent; it is read but never modified.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess

from profile_upgrade import inventory, run, sha, value


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main(args):
    os.umask(0o077)
    require(not args.artifacts.is_relative_to(args.save), "artifacts inside save")
    require((args.save / "level.dat").is_file()
            and any(p.stat().st_size > 8192 for p in args.save.rglob("*.mca")),
            "a genuine quiescent Java save is required")
    args.artifacts.mkdir(mode=0o700, exist_ok=False)
    package = args.artifacts / "package"
    subprocess.run(["dpkg-deb", "-x", str(args.deb), str(package)], check=True, timeout=90)
    bin_dir = package / "usr/bin"
    names = ("swarmcraft", "swarmcraft-host", "swarmcraft-runtime", "swarmcraft-import")
    hashes = {name: sha(bin_dir / name) for name in names}
    require(all(hashes[name] == sha(args.bin / name) for name in names),
            "packaged sidecars are not the tested build")
    cli, importer, runtime = (bin_dir / name for name in
                              ("swarmcraft", "swarmcraft-import", "swarmcraft-runtime"))
    result = {"scope": "extracted local Linux Debian package, fresh real Java save import, offline operations",
              "package_sha256": sha(args.deb), "sidecar_sha256": hashes, "checks": {}}
    checks = result["checks"]
    scratch = args.artifacts
    fresh = scratch / "absent-profile"
    require(not value(cli, fresh, "support", "status")["collection_enabled"], "default enabled")
    default = value(cli, fresh, "support", "report", "--confirm")
    require(default["retained_count"] == 0 and not fresh.exists(), "report initialized profile")
    refused = run(cli, fresh, "support", "report", check=False)
    require(refused.returncode != 0 and not refused.stdout and not fresh.exists(), "unconfirmed export")
    checks["default_off_and_explicit_report_without_initialization"] = True

    profile = scratch / "profile"
    source = inventory(args.save)
    imported = value(importer, profile, "--source", args.save, "--name", "PRIVATE_WORLD_NAME_CANARY",
                     "--minecraft", "26.1.2", "--fabric-loader", "0.19.3", "--no-server-mods")
    world = imported["world_id"]
    checkpoint = run(cli, profile, "world", "verify", world).stdout
    identity = run(cli, profile, "identity").stdout
    value(cli, profile, "world", "hosting", "set", world, "--heap-mib", 512, "--json")
    consent = value(cli, profile, "world", "hosting", "set", world, "--heap-mib", 0, "--json")
    require(not consent["consented"], "failed to withdraw consent")
    baseline = inventory(profile)
    require(value(cli, profile, "support", "report", "--confirm", "--world", world)["retained_count"] == 0,
            "disabled history recorded operations")
    value(cli, profile, "support", "configure", "--collection", "on")
    run(cli, profile, "world", "verify", world)
    restored = scratch / "PRIVATE_RESTORE_PATH_CANARY"
    run(cli, profile, "world", "recover", world, 1, restored)
    require(inventory(restored) == source, "restored files differ from real save")
    invalid = scratch / "PRIVATE_FAILED_RESTORE_CANARY"
    require(run(cli, profile, "world", "recover", world, 99999, invalid, check=False).returncode != 0
            and not invalid.exists(), "invalid restore accepted")
    # A nonexistent world is an actual sidecar validation failure; no
    # downloading or runtime readiness is claimed.
    missing_world = "scworld:" + "aa" * 32
    require(run(runtime, profile, "verify", missing_world, check=False).returncode != 0,
            "missing runtime world accepted")
    before_report = inventory(profile)
    text = run(cli, profile, "support", "report", "--confirm", "--world", world).stdout
    report = json.loads(text)
    require(inventory(profile) == before_report, "report mutated profile")
    require(report["summary"]["checkpoint_restore"]["outcomes"] == {"completed": 1, "failed": 1},
            "missing restore outcomes")
    require(report["summary"]["runtime_prepare"]["outcomes"] == {"failed": 1},
            "missing sidecar failure")
    require(report["summary"]["checkpoint_verify"]["outcomes"] == {"completed": 1},
            "missing verification")
    require(report["profile_marker_only"] == "observed" and not report["authorizes_hosting_or_play"]
            and not report["automatically_uploaded"], "invalid report boundary")
    secrets = [world, world.removeprefix("scworld:"), missing_world, "aa" * 32, str(profile),
               str(restored), str(invalid), "PRIVATE_", imported["snapshot_hash"], imported["state_root"]]
    secrets.extend(line.partition(": ")[2].strip() for line in identity.splitlines()
                   if line.startswith(("Peer ID:", "Public key:")))
    require(all(secret and secret not in text for secret in secrets), "redaction canary leaked")
    require(len(text.encode()) <= 256 * 1024, "report exceeded size limit")
    checks["packaged_cli_and_runtime_observations_redacted"] = True
    checks["real_save_restored_byte_exact_and_report_read_only"] = True
    result["report"] = report
    result["real_save_files"] = len(source)
    result["real_save_bytes"] = sum(p.stat().st_size for p in args.save.rglob("*") if p.is_file())

    value(cli, profile, "support", "configure", "--collection", "off")
    retained = value(cli, profile, "support", "status")["retained_count"]
    run(cli, profile, "world", "verify", world)
    require(value(cli, profile, "support", "status")["retained_count"] == retained,
            "paused recorder appended")
    require(run(cli, profile, "support", "clear", check=False).returncode != 0,
            "unconfirmed clear accepted")
    value(cli, profile, "support", "clear", "--confirm")
    require(value(cli, profile, "support", "status")["retained_count"] == 0, "clear failed")
    after = inventory(profile)
    require({k: v for k, v in after.items() if not k.startswith("support-metrics-v1")}
            == baseline, "non-support profile changed")
    require(inventory(args.save) == source, "source save changed")
    require(run(cli, profile, "world", "verify", world).stdout == checkpoint, "signed checkpoint changed")
    require(run(cli, profile, "identity").stdout == identity, "identity changed")
    # Hosting status includes live host memory/readiness measurements.
    # Compare durable consent, not the unrelated fluctuating telemetry.
    current_consent = value(cli, profile, "world", "hosting", "status", world, "--json")
    require(not current_consent["consented"] and current_consent["selected_heap_mib"] == 0
            and current_consent["consent_revision"] == consent["consent_revision"],
            "withdrawn consent changed")
    checks["pause_clear_preserve_canonical_state_and_withdrawn_consent"] = True

    future = scratch / "future-profile"
    future.mkdir()
    (future / "profile-compatibility.json").write_text(json.dumps(
        {"format_schema": 1, "minimum_reader_generation": 2,
         "storage_schema_version": 1, "protocol_version": 1}))
    future_before = inventory(future)
    require(value(cli, future, "support", "report", "--confirm")["profile_marker_only"] == "incompatible",
            "future reader not reported")
    require(run(cli, future, "support", "configure", "--collection", "on", check=False).returncode != 0
            and inventory(future) == future_before, "future profile mutated")
    checks["incompatible_profile_report_without_mutation"] = True
    broken = scratch / "invalid-support"
    broken.mkdir()
    invalid_store = broken / "support-metrics-v1.json"
    invalid_store.write_text("PRIVATE_INVALID_STORE_CANARY")
    refused = run(cli, broken, "support", "report", "--confirm", check=False)
    require(refused.returncode != 0 and not refused.stdout and "PRIVATE_" not in refused.stderr
            and invalid_store.read_text() == "PRIVATE_INVALID_STORE_CANARY", "malformed store exported or overwritten")
    checks["malformed_store_refused_without_raw_content"] = True
    (scratch / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"PASS {len(checks)} packaged operational support checks; {len(source)} saved files restored exactly")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("deb", "bin", "save", "artifacts"):
        parser.add_argument(f"--{name}", type=lambda p: Path(p).resolve(), required=True)
    main(parser.parse_args())
