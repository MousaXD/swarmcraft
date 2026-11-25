#!/usr/bin/env python3
"""Prepare an offline signed update index from exact verified release artifacts.

This is not a publisher or release approval. It performs no network calls.
The offline signing key stays outside the checkout and staging/output folders.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

from verify_artifacts import (expected_groups, expected_status_file, verify_native_signing_receipt,
                              verify_checksum, unique_object)

ROOT = Path(__file__).resolve().parents[2]
MANIFEST_DOMAIN = b"SwarmCraft update manifest v1\0"
PACKAGE_DOMAIN = "SwarmCraft update package v1\n"
ED25519_SPKI = bytes.fromhex("302a300506032b6570032100")
MAX_PACKAGE = 1024**3


def require(condition, message):
    if not condition:
        raise ValueError(message)


def version(text):
    require(isinstance(text, str) and re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", text)
            and len(text) <= 32, "invalid exact application version")
    result = tuple(map(int, text.split(".")))
    require(all(x < 2**32 for x in result), "application version component overflow")
    return result


def safe_file(root, name):
    require(isinstance(name, str) and re.fullmatch(r"[a-zA-Z0-9_.-]{1,180}", name)
            and name not in {".", ".."}, "unsafe release asset filename")
    path = root / name
    require(path.is_file() and not path.is_symlink(), "missing or unsafe release asset")
    return path


def public_key(key):
    der = subprocess.check_output(["openssl", "pkey", "-in", str(key), "-pubout", "-outform", "DER"],
                                  stdin=subprocess.DEVNULL, timeout=30)
    require(len(der) == len(ED25519_SPKI) + 32 and der.startswith(ED25519_SPKI), "expected an Ed25519 key")
    return der[len(ED25519_SPKI):]


def signature(key, message, scratch):
    # OpenSSL one-shot Ed25519 expects a regular file with a known size.
    path = scratch / "message"
    path.write_bytes(message)
    output = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(key), "-in", str(path)],
                                     stdin=subprocess.DEVNULL, timeout=30)
    require(len(output) == 64, "unexpected Ed25519 signature size")
    return output.hex()


def package_target(platform, filename):
    if platform == "linux":
        if filename.endswith(".deb"):
            return "linux-x86_64-deb"
        if filename.endswith(".AppImage"):
            return "linux-x86_64-appimage"
    elif platform == "windows" and filename.endswith(".exe"):
        return "windows-x86_64-nsis"
    elif platform == "macos-arm64" and filename.endswith(".dmg"):
        return "macos-aarch64-dmg"
    elif platform == "macos-x86_64" and filename.endswith(".dmg"):
        return "macos-x86_64-dmg"
    raise ValueError("unknown release platform/installer format")


def package_message(manifest, package):
    values = (manifest["channel"], manifest["version"], manifest["commit"], manifest["release_tag"],
              package["target"], package["file"], package["bytes"], package["sha256"])
    return (PACKAGE_DOMAIN + "".join(f"{value}\n" for value in values)).encode("ascii")


def inventory(args):
    provenance_file = safe_file(args.artifacts, "release-provenance.json")
    require(provenance_file.stat().st_size <= 64 * 1024, "oversized release provenance")
    data = json.loads(provenance_file.read_text(), object_pairs_hook=unique_object)
    require(set(data) == {"commit", "version", "channel", "scope", "assets"}, "invalid release provenance schema")
    expected_channel = "production" if args.channel == "stable" else "preview"
    require(data["commit"] == args.expected_commit and data["version"] == args.expected_version
            and data["channel"] == expected_channel, "provenance does not match separately reviewed release")
    groups = expected_groups(expected_channel, args.expected_commit)
    if args.channel == "stable":
        safe_file(args.artifacts, expected_status_file("linux"))
    expected = {"linux-x86_64-deb", "windows-x86_64-nsis", "macos-aarch64-dmg", "macos-x86_64-dmg"}
    if args.channel == "stable":
        expected.add("linux-x86_64-appimage")
    require(isinstance(data["assets"], list) and len(data["assets"]) == len(expected) + 1,
            "incomplete or ambiguous exact-run release inventory")
    seen = set()
    names = set()
    packages = []
    for row in data["assets"]:
        require(isinstance(row, dict) and {"platform", "file", "bytes", "sha256", "source_artifact"} <= row.keys(),
                "invalid provenance asset")
        require(row["file"].lower() not in names, "duplicate release asset")
        names.add(row["file"].lower())
        path = safe_file(args.artifacts, row["file"])
        require(type(row["bytes"]) is int and 0 < row["bytes"] <= MAX_PACKAGE
                and path.stat().st_size == row["bytes"], "release asset size mismatch")
        checksum = safe_file(args.artifacts, f"{path.name}.sha256")
        require(checksum.stat().st_size <= 1024, "oversized release checksum")
        digest = verify_checksum(path, checksum)
        require(digest == row["sha256"], "release asset digest differs from provenance")
        platform = row["platform"]
        require(platform in groups and row["source_artifact"] == groups[platform],
                "release artifact not bound to reviewed exact-run group")
        if platform == "fabric":
            require(path.name == f"swarmcraft-fabric-{args.expected_version}.jar" and "fabric" not in seen,
                    "unexpected Fabric artifact")
            seen.add("fabric")
            continue
        target = package_target(platform, path.name)
        require(target in expected and target not in seen, "ambiguous or unsupported release installer target")
        seen.add(target)
        lower = path.name.lower()
        require(lower.startswith("swarmcraft") and f"_{args.expected_version}_" in path.name,
                "installer filename version mismatch")
        require(
            (target.startswith("linux-") and ("amd64" in lower or "x86_64" in lower))
            or (target == "windows-x86_64-nsis" and "x64" in lower and lower.endswith("-setup.exe"))
            or (target == "macos-aarch64-dmg" and ("arm64" in lower or "aarch64" in lower))
            or (target == "macos-x86_64-dmg" and ("x86_64" in lower or "x64" in lower)),
            "installer filename architecture mismatch",
        )
        # Inspect Linux package identity without executing any packaged code
        # or maintainer script. Other native checks remain bound to the
        # already reviewed signing receipts.
        if target == "linux-x86_64-deb":
            fields = {}
            text = subprocess.check_output(["dpkg-deb", "-f", str(path), "Package", "Version", "Architecture"],
                                           text=True, timeout=30)
            for line in text.splitlines():
                name, _, value = line.partition(": ")
                require(name not in fields and bool(value), "ambiguous Debian package identity")
                fields[name] = value
            require(fields == {"Package": "swarm-craft", "Version": args.expected_version, "Architecture": "amd64"},
                    "Debian package identity/version/architecture mismatch")
        if target == "linux-x86_64-appimage":
            with path.open("rb") as file:
                header = file.read(64)
            require(len(header) == 64 and header[:6] == b"\x7fELF\x02\x01"
                    and header[8:11] == b"AI\x02" and header[18:20] == b"\x3e\x00",
                    "expected x86-64 ELF AppImage type 2 package")
        if args.channel == "stable" and platform != "linux":
            receipt = row.get("native_signing_evidence")
            require(isinstance(receipt, dict) and set(receipt) == {"file", "sha256"},
                    "stable release lacks bound native signing receipt")
            require(receipt["file"] == expected_status_file(platform), "wrong native signing receipt filename")
            evidence_path = safe_file(args.artifacts, receipt["file"])
            actual = verify_native_signing_receipt(evidence_path, platform, args.expected_commit, path, digest)
            require(actual == receipt, "stable release signing receipt digest mismatch")
        packages.append({"target": target, "file": path.name, "bytes": row["bytes"], "sha256": digest})
    require(seen == expected | {"fabric"}, "release inventory missing required target")
    return sorted(packages, key=lambda row: row["target"])


def sign(args):
    require(args.artifacts.is_dir() and not args.artifacts.is_symlink(), "invalid artifact root")
    require(re.fullmatch(r"[0-9a-f]{40}", args.expected_commit), "expected full lowercase commit SHA")
    require(version(args.minimum_client_version) < version(args.expected_version), "incompatible version range")
    require(0 < args.sequence < 2**64 and 1 <= args.valid_days <= 30, "invalid update sequence or lifetime")
    require(args.protocol_version == 1 and args.storage_schema_version == 1 and args.reader_generation == 1,
            "incompatible update schema; reviewed migration required")
    key = args.key
    require(key.is_file() and not key.is_symlink(), "missing private signing key")
    require(not key.resolve().is_relative_to(ROOT) and not key.resolve().is_relative_to(args.artifacts.resolve())
            and not key.resolve().is_relative_to(args.output.resolve()), "signing key must be outside repository and delivery folders")
    if os.name == "posix":
        require(key.stat().st_mode & 0o077 == 0 and key.stat().st_uid == os.getuid()
                and key.stat().st_nlink == 1, "signing key must be private and singly linked")
    require(not args.output.exists() and not args.output.is_symlink(), "new output directory required")
    require(not args.output.resolve().is_relative_to(args.artifacts.resolve()), "index output must be separate from artifact input")
    os.umask(0o077)
    packages = inventory(args)
    pub = public_key(key)
    fingerprint = hashlib.sha256(pub).hexdigest()
    require(fingerprint == args.expected_fingerprint, "signing key differs from reviewed channel fingerprint")
    now = int(time.time())
    manifest = {
        "schema": 1, "channel": args.channel, "sequence": args.sequence,
        "issued_unix": now, "expires_unix": now + args.valid_days * 86400,
        "version": args.expected_version, "minimum_client_version": args.minimum_client_version,
        "commit": args.expected_commit,
        "release_tag": f"v{args.expected_version}" if args.channel == "stable" else f"main-snapshot-{args.expected_commit}",
        "protocol_version": args.protocol_version, "storage_schema_version": args.storage_schema_version,
        "reader_generation": args.reader_generation, "packages": packages,
    }
    with tempfile.TemporaryDirectory(prefix=".swarmcraft-update-sign-", dir=args.output.parent) as temp:
        scratch = Path(temp)
        for package in packages:
            package["signature"] = signature(key, package_message(manifest, package), scratch)
        raw = (json.dumps(manifest, indent=2, ensure_ascii=True) + "\n").encode("ascii")
        signed = signature(key, MANIFEST_DOMAIN + raw, scratch)
        (scratch / "message").unlink()
        (scratch / "update-manifest.json").write_bytes(raw)
        (scratch / "update-manifest.sig").write_text(signed + "\n", encoding="ascii")
        # Only the two signed index files are delivered. Package assets remain
        # attached to their independently reviewed immutable release tag.
        scratch.rename(args.output)
    return {"channel": args.channel, "version": args.expected_version, "commit": args.expected_commit,
            "sequence": str(args.sequence), "public_key_fingerprint": fingerprint,
            "manifest_sha256": hashlib.sha256(raw).hexdigest(), "packages": len(packages),
            "published": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--key", required=True, type=Path)
    parser.add_argument("--expected-fingerprint", required=True)
    parser.add_argument("--channel", required=True, choices=("preview", "stable"))
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--minimum-client-version", required=True)
    parser.add_argument("--sequence", required=True, type=int)
    parser.add_argument("--valid-days", required=True, type=int)
    for name in ("protocol-version", "storage-schema-version", "reader-generation"):
        parser.add_argument("--" + name, required=True, type=int)
    args = parser.parse_args()
    print(json.dumps(sign(args), indent=2))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError) as error:
        raise SystemExit(f"update signing refused: {error}") from None
