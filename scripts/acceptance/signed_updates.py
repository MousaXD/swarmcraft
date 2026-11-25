#!/usr/bin/env python3
"""Offline end-to-end update delivery using ephemeral channel keys.

The newer Debian package is an inert, valid archive, not a released build.
Other platform bytes are explicit signing fixtures. No installer runs, no
production credentials are used, and no network service is contacted.
"""
from __future__ import annotations

import argparse
import copy
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from profile_upgrade import inventory, sha

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ci-release"))
import sign_updates
import verify_artifacts


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def call(binary, root, *args, error=None):
    env = {k: v for k, v in os.environ.items() if not k.startswith("SWARMCRAFT_")}
    env.update(RUST_LOG="warn", NO_COLOR="1")
    result = subprocess.run([str(binary), "--data-dir", str(root), *map(str, args)],
                            env=env, capture_output=True, text=True, timeout=90)
    if error is not None:
        require(result.returncode != 0 and error in result.stderr, f"expected refusal {error}: {result.stderr[-800:]}")
        return result.stderr
    require(result.returncode == 0, f"CLI failed: {result.stderr[-800:]}")
    return result.stdout


def value(binary, root, *args):
    return json.loads(call(binary, root, *args))


def keygen(path):
    subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(path)],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
    raw = sign_updates.public_key(path)
    return raw.hex(), hashlib.sha256(raw).hexdigest()


def make_fixture(base, version, commit):
    groups = verify_artifacts.expected_groups("preview", commit)
    source = base / "groups"
    fixture = {
        "linux": f"SwarmCraft_{version}_amd64.deb",
        "windows": f"SwarmCraft_{version}_x64-setup.exe",
        "macos-arm64": f"SwarmCraft_{version}_arm64.dmg",
        "macos-x86_64": f"SwarmCraft_{version}_x86_64.dmg",
        "fabric": f"swarmcraft-fabric-{version}.jar",
    }
    for platform, name in fixture.items():
        folder = source / groups[platform]
        folder.mkdir(parents=True)
        path = folder / name
        if platform == "linux":
            pkg = base / "deb-tree"
            (pkg / "DEBIAN").mkdir(parents=True)
            (pkg / "DEBIAN/control").write_text(
                f"Package: swarm-craft\nVersion: {version}\nArchitecture: amd64\n"
                "Maintainer: Offline Test <fixture@example.invalid>\nDescription: Inert test archive; do not install\n")
            (pkg / "usr/share/swarmcraft-update-fixture").mkdir(parents=True)
            (pkg / "usr/share/swarmcraft-update-fixture/README").write_text("Inert update verification fixture.\n")
            # Native Debian metadata and archive members need their normal
            # installation modes; the enclosing lab is still private.
            for member in [pkg, *pkg.rglob("*")]:
                member.chmod(0o755 if member.is_dir() else 0o644)
            subprocess.run(["dpkg-deb", "--build", "--root-owner-group", str(pkg), str(path)],
                           check=True, stdout=subprocess.DEVNULL, timeout=30)
            require(subprocess.check_output(["dpkg-deb", "-f", str(path), "Version"], text=True).strip() == version,
                    "fixture package version mismatch")
        else:
            path.write_bytes(f"INERT OFFLINE SIGNING FIXTURE {platform} {version}\n".encode())
        (folder / f"{name}.sha256").write_text(f"{sha(path)}  {name}\n")
    artifacts = base / "verified-fixture"
    verify_artifacts.verify(source, artifacts, "preview", commit, version)
    return artifacts


def sign_index(artifacts, output, key, fingerprint, channel, version, commit, sequence, installed):
    args = argparse.Namespace(
        artifacts=artifacts, output=output, key=key, expected_fingerprint=fingerprint,
        channel=channel, expected_commit=commit, expected_version=version,
        minimum_client_version=installed, sequence=sequence, valid_days=7,
        protocol_version=1, storage_schema_version=1, reader_generation=1)
    return sign_updates.sign(args)


def rewrite_manifest(feed, manifest, key, scratch, resign_packages=True):
    if resign_packages:
        for package in manifest["packages"]:
            package["signature"] = sign_updates.signature(key, sign_updates.package_message(manifest, package), scratch)
    raw = (json.dumps(manifest, indent=2) + "\n").encode()
    (feed / "update-manifest.json").write_bytes(raw)
    sig = sign_updates.signature(key, sign_updates.MANIFEST_DOMAIN + raw, scratch)
    (feed / "update-manifest.sig").write_text(sig + "\n")


def run_test(args, scratch, report):
    cli = args.bin / "swarmcraft"
    importer = args.bin / "swarmcraft-import"
    checks = report["checks"]
    preview_key, stable_key = scratch / "preview.pem", scratch / "stable.pem"
    preview_public, preview_fp = keygen(preview_key)
    stable_public, stable_fp = keygen(stable_key)
    profile = scratch / "profile"
    empty = scratch / "never-initialized"
    require(value(cli, empty, "update", "status")["preferences"]["channel"] == "off", "default opt-out missing")
    call(cli, empty, "update", "check", "--online", error="updates are disabled")
    call(cli, empty, "update", "stage", "--feed-dir", scratch / "missing", error="updates are disabled")
    require(not empty.exists(), "disabled update mutated empty profile")
    checks["default_off_no_network_or_profile_creation"] = True
    imported = value(importer, profile, "--source", args.save, "--name", "Update acceptance",
                     "--minecraft", "26.1.2", "--fabric-loader", "0.19.3", "--no-server-mods")
    world = imported["world_id"]
    checkpoint = call(cli, profile, "world", "verify", world)
    identity = call(cli, profile, "identity")
    value(cli, profile, "world", "hosting", "set", world, "--heap-mib", "512", "--json")
    consent = value(cli, profile, "world", "hosting", "set", world, "--heap-mib", "0", "--json")
    require(not consent["consented"] and consent["consent_revision"] is not None, "consent setup invalid")
    baseline = inventory(profile)
    installed = value(cli, profile, "update", "status")["application_version"]
    major, minor, patch = map(int, installed.split("."))
    newer = f"{major}.{minor}.{patch + 1}"
    commit = "54" * 20
    call(cli, profile, "update", "configure", "--channel", "preview", error="first opt-in needs")
    call(cli, profile, "update", "configure", "--channel", "preview", "--public-key", preview_public,
         "--expected-fingerprint", stable_fp, error="independently authenticated fingerprint")
    value(cli, profile, "update", "configure", "--channel", "preview", "--public-key", preview_public,
          "--expected-fingerprint", preview_fp)
    call(cli, profile, "update", "configure", "--channel", "preview", "--public-key", stable_public,
         "--expected-fingerprint", stable_fp, error="pin replacement is not supported")
    call(cli, profile, "update", "configure", "--channel", "stable", "--public-key", preview_public,
         "--expected-fingerprint", preview_fp, error="distinct trust keys")
    checks["explicit_pin_and_independent_channels"] = True

    # Actual old published assets from #50: sign locally using a disposable
    # key solely to prove that trusted metadata cannot override version policy.
    published = json.loads((args.published / "release-provenance.json").read_text())
    old_index = scratch / "old-index"
    old = sign_index(args.published, old_index, preview_key, preview_fp, "preview",
                     published["version"], published["commit"], 1, "0.5.0")
    call(cli, profile, "update", "check", "--feed-dir", old_index, error="strictly newer")
    report["actual_published_preview"] = {"commit": old["commit"], "version": old["version"],
                                         "verified_package_records": old["packages"], "equal_or_older_version_denied": True}
    checks["actual_published_package_provenance_signed_but_not_upgradeable"] = True

    base = scratch / "fixture"
    base.mkdir()
    assets = make_fixture(base, newer, commit)
    index = scratch / "new-index"
    signed = sign_index(assets, index, preview_key, preview_fp, "preview", newer, commit, 2, installed)
    feed = scratch / "feed"
    shutil.copytree(index, feed)
    original = json.loads((index / "update-manifest.json").read_text())
    linux = next(p for p in original["packages"] if p["target"] == "linux-x86_64-deb")
    package = feed / linux["file"]
    shutil.copyfile(assets / linux["file"], package)
    original_bytes = package.read_bytes()
    require(signed["published"] is False, "signing tool attempted publication")
    raw = (feed / "update-manifest.json").read_bytes()
    sig = (feed / "update-manifest.sig").read_bytes()
    state_file = profile / "updates/state.json"
    def restore_feed():
        (feed / "update-manifest.json").write_bytes(raw)
        (feed / "update-manifest.sig").write_bytes(sig)
        package.write_bytes(original_bytes)
    (feed / "update-manifest.json").write_bytes(raw + b" ")
    call(cli, profile, "update", "check", "--feed-dir", feed, error="untrusted update manifest")
    restore_feed()
    (feed / "update-manifest.sig").write_text("00" * 64)
    call(cli, profile, "update", "check", "--feed-dir", feed, error="untrusted update manifest")
    restore_feed()
    bad = copy.deepcopy(original)
    bad["packages"][0]["signature"] = "00" * 64
    rewrite_manifest(feed, bad, preview_key, scratch, resign_packages=False)
    call(cli, profile, "update", "check", "--feed-dir", feed, error="invalid detached package signature")
    restore_feed()
    for field in ("protocol_version", "storage_schema_version", "reader_generation"):
        bad = copy.deepcopy(original)
        bad[field] = 2
        rewrite_manifest(feed, bad, preview_key, scratch)
        call(cli, profile, "update", "check", "--feed-dir", feed, error="incompatible protocol, storage schema or profile reader")
    bad = copy.deepcopy(original)
    bad["expires_unix"] = int(time.time()) - 1
    rewrite_manifest(feed, bad, preview_key, scratch)
    call(cli, profile, "update", "check", "--feed-dir", feed, error="expired")
    restore_feed()
    require(value(cli, profile, "update", "status")["preferences"]["preview"]["high_water"] is None,
            "untrusted metadata advanced replay record")
    checks["invalid_signatures_and_incompatible_or_expired_signed_metadata_denied"] = True

    accepted = value(cli, profile, "update", "check", "--feed-dir", feed)
    require(accepted["manifest_signature_verified"] and accepted["detached_package_signature_verified"]
            and not accepted["package_bytes_verified"] and accepted["staged_package"] is None,
            "manifest-only check claimed package verification")
    require(accepted["manifest_sha256"] == signed["manifest_sha256"], "client/signer digest differs")
    checks["actual_openssl_signed_manifest_verified_by_rust_client"] = True
    for bad_bytes in (original_bytes[:-1], original_bytes + b"x", bytes([original_bytes[0] ^ 1]) + original_bytes[1:]):
        package.write_bytes(bad_bytes)
        call(cli, profile, "update", "stage", "--feed-dir", feed, error="update package")
        require(not any((profile / "updates/staged").iterdir()), "invalid bytes were staged or left scratch")
    restore_feed()
    package.unlink()
    call(cli, profile, "update", "stage", "--feed-dir", feed, error="missing or unsafe local update asset")
    require(not any((profile / "updates/staged").iterdir()), "missing asset left approved staging")
    package.write_bytes(original_bytes)
    checks["corrupt_truncated_oversized_missing_package_never_published"] = True
    stage = value(cli, profile, "update", "stage", "--feed-dir", feed)
    staged = Path(stage["staged_package"])
    require(staged.read_bytes() == original_bytes and stage["package_bytes_verified"] and not stage["automatic_installation"],
            "staged package receipt incorrect")
    require(staged.stat().st_mode & 0o111 == 0, "staged executable was made executable")
    require(value(cli, profile, "update", "verify-staged")["package_sha256"] == sha(staged),
            "staged re-verification failed")
    staged.write_bytes(b"tampered local package")
    call(cli, profile, "update", "verify-staged", error="update package")
    call(cli, profile, "update", "stage", "--feed-dir", feed, error="update package")
    staged.write_bytes(original_bytes)
    require(value(cli, profile, "update", "verify-staged")["package_bytes_verified"], "restored exact fixture not verified")
    checks["verified_inert_deb_stage_and_local_tampering_denial"] = True

    old_state = state_file.read_bytes()
    with (profile / "updates/state.lock").open("r+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        call(cli, profile, "update", "configure", "--channel", "off", error="another update operation")
    require(state_file.read_bytes() == old_state, "concurrent preference update changed state")
    bad = copy.deepcopy(original)
    bad["sequence"] = 1
    rewrite_manifest(feed, bad, preview_key, scratch)
    call(cli, profile, "update", "check", "--feed-dir", feed, error="sequence rollback")
    bad = copy.deepcopy(original)
    bad["expires_unix"] += 1
    rewrite_manifest(feed, bad, preview_key, scratch)
    call(cli, profile, "update", "check", "--feed-dir", feed, error="sequence reused")
    restore_feed()
    value(cli, profile, "update", "configure", "--channel", "off")
    call(cli, profile, "update", "verify-staged", error="updates are disabled")
    call(cli, profile, "update", "check", "--online", error="updates are disabled")
    value(cli, profile, "update", "configure", "--channel", "preview")
    require(value(cli, profile, "update", "verify-staged")["package_sha256"] == linux["sha256"],
            "opt-out/re-enable lost replay state")
    checks["durable_replay_lock_and_opt_out"] = True

    # A stable signed offer cannot reuse the preview pin. Stable positive
    # cryptographic acceptance uses an explicitly synthetic fixture manifest,
    # not a claim of a real native signed production release.
    value(cli, profile, "update", "configure", "--channel", "stable", "--public-key", stable_public,
          "--expected-fingerprint", stable_fp)
    call(cli, profile, "update", "check", "--feed-dir", feed, error="untrusted update manifest")
    stable = copy.deepcopy(original)
    stable["channel"] = "stable"
    stable["release_tag"] = f"v{newer}"
    rewrite_manifest(feed, stable, stable_key, scratch)
    stable_stage = value(cli, profile, "update", "stage", "--feed-dir", feed)
    require(stable_stage["channel"] == "stable" and stable_stage["package_bytes_verified"]
            and stable_stage["staged_package"] != stage["staged_package"], "stable and preview staging mixed")
    value(cli, profile, "update", "configure", "--channel", "preview")
    call(cli, profile, "update", "check", "--feed-dir", feed, error="untrusted update manifest")
    require(value(cli, profile, "update", "verify-staged")["channel"] == "preview", "preview state lost")
    restore_feed()
    checks["stable_and_preview_independent_signed_staging"] = True

    # A more recent accepted offer supersedes the old stage, even if its
    # download then fails. No fallback to an older signed artifact.
    next_manifest = copy.deepcopy(original)
    next_manifest["sequence"] = 3
    next_manifest["expires_unix"] += 10
    rewrite_manifest(feed, next_manifest, preview_key, scratch)
    package.unlink()
    call(cli, profile, "update", "stage", "--feed-dir", feed, error="missing or unsafe local update asset")
    call(cli, profile, "update", "verify-staged", error="no staged package")
    restore_feed()
    call(cli, profile, "update", "check", "--feed-dir", feed, error="sequence rollback")
    package.write_bytes(original_bytes)
    rewrite_manifest(feed, next_manifest, preview_key, scratch)
    latest = value(cli, profile, "update", "stage", "--feed-dir", feed)
    require(latest["sequence"] == "3" and value(cli, profile, "update", "verify-staged")["sequence"] == "3",
            "newer staged offer not current")
    checks["accepted_newer_offer_cannot_fall_back_after_failed_download"] = True

    # In the isolated network namespace, no external host is reachable.
    before = state_file.read_bytes()
    call(cli, profile, "update", "check", "--online", error="error sending request")
    require(state_file.read_bytes() == before, "offline failure altered trust/replay state")
    checks["explicit_online_transport_failure_preserves_trust_state"] = True

    future = scratch / "future-profile"
    future.mkdir(mode=0o700)
    value(cli, future, "update", "configure", "--channel", "preview", "--public-key", preview_public,
          "--expected-fingerprint", preview_fp)
    marker = future / "profile-compatibility.json"
    marker.write_text('{"format_schema":1,"minimum_reader_generation":2,"storage_schema_version":1,"protocol_version":1}')
    state_before = inventory(future)
    call(cli, future, "update", "stage", "--feed-dir", feed, error="minimum reader generation 2")
    require(inventory(future) == state_before, "future profile mutated")
    checks["incompatible_live_profile_preflight_read_only"] = True

    # Publisher must not sign changed bytes or preview provenance as stable.
    corrupt_output = scratch / "corrupt-index"
    target = assets / linux["file"]
    target.write_bytes(original_bytes + b"x")
    try:
        sign_index(assets, corrupt_output, preview_key, preview_fp, "preview", newer, commit, 4, installed)
        raise RuntimeError("corrupted release asset signed")
    except ValueError as error:
        require("size mismatch" in str(error) and not corrupt_output.exists(), "wrong publisher refusal")
    target.write_bytes(original_bytes)
    try:
        sign_index(assets, scratch / "false-stable-index", stable_key, stable_fp, "stable", newer, commit, 4, installed)
        raise RuntimeError("preview provenance signed as stable")
    except ValueError as error:
        require("provenance does not match" in str(error), "wrong stable publisher refusal")
    # A new signed checksum for a wrongly labelled native archive must not
    # defeat the package identity check. Rebuild the inert fixture with
    # a different Debian architecture and update its fixture provenance.
    deb_tree = base / "deb-tree"
    control = deb_tree / "DEBIAN/control"
    control.write_text(control.read_text().replace("Architecture: amd64", "Architecture: arm64"))
    subprocess.run(["dpkg-deb", "--build", "--root-owner-group", str(deb_tree), str(target)],
                   check=True, stdout=subprocess.DEVNULL, timeout=30)
    (assets / f"{target.name}.sha256").write_text(f"{sha(target)}  {target.name}\n")
    provenance_path = assets / "release-provenance.json"
    provenance = json.loads(provenance_path.read_text())
    row = next(row for row in provenance["assets"] if row["file"] == target.name)
    row.update(bytes=target.stat().st_size, sha256=sha(target))
    provenance_path.write_text(json.dumps(provenance))
    try:
        sign_index(assets, scratch / "wrong-arch-index", preview_key, preview_fp, "preview", newer, commit, 4, installed)
        raise RuntimeError("publisher signed wrong Debian architecture")
    except ValueError as error:
        require("Debian package identity/version/architecture mismatch" in str(error), "wrong architecture refusal")
    checks["publisher_refuses_changed_bytes_and_channel_substitution"] = True
    checks["publisher_checks_debian_identity_not_only_filename"] = True

    require(call(cli, profile, "world", "verify", world) == checkpoint
            and call(cli, profile, "identity") == identity
            and value(cli, profile, "world", "hosting", "status", world, "--json")["consent_revision"] == consent["consent_revision"],
            "update changed canonical identity/checkpoint or consent")
    final = {k: v for k, v in inventory(profile).items() if not k.startswith("updates/")}
    require(final == baseline, "update operations changed non-update profile bytes")
    report.update(actual_saved_checkpoint=imported["snapshot_hash"], original_save_files=imported["files"],
                  consent_withdrawal_preserved=True, candidate_cli_sha256=sha(cli),
                  fixture_version=newer, fixture_package_sha256=linux["sha256"], fixture_package_bytes=linux["bytes"],
                  preview_manifest_sha256=signed["manifest_sha256"], latest_sequence=latest["sequence"])
    checks["real_saved_world_and_consent_bytes_unchanged"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("bin", "save", "published", "artifacts"):
        parser.add_argument("--" + name, required=True, type=lambda p: Path(p).resolve())
    args = parser.parse_args()
    import socket
    require({name for _, name in socket.if_nameindex()} == {"lo"}, "isolated loopback-only network namespace required")
    require(not args.artifacts.is_relative_to(args.save) and not args.artifacts.is_relative_to(args.published),
            "output must be outside source inputs")
    os.umask(0o077)
    args.artifacts.mkdir(mode=0o700, exist_ok=False)
    report = {"scope": "offline test-only keys, real current CLI, actual old published preview, inert next-version Debian archive and synthetic other platform artifacts; no installation",
              "checks": {}}
    try:
        with tempfile.TemporaryDirectory(prefix="swarmcraft-update-lab-", dir=args.artifacts) as folder:
            run_test(args, Path(folder), report)
        report["complete"] = True
        print(f"PASS {len(report['checks'])} signed update delivery checks", flush=True)
    except Exception as error:
        report["failure"] = str(error)[:400]
        raise
    finally:
        (args.artifacts / "result.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
