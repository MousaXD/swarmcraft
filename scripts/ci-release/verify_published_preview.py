#!/usr/bin/env python3
"""Read-only verification of downloaded, published SwarmCraft preview bytes.

The caller independently resolves the expected Git tag target and retrieves
GitHub's release asset metadata. The local bytes must agree with every
remote size/digest and every package checksum. A commit-pinned manifest
binds the current preview to source; the legacy channel has no such
manifest and must be explicitly requested. Neither is a signing attestation.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import zipfile

from verify_artifacts import COMMIT, expected_groups, sha256_file, verify_checksum


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise ValueError("published preview verification failed: " + reason)


def packages(version: str) -> dict[str, str]:
    require(bool(re.fullmatch(r"\d+\.\d+\.\d+", version)), "invalid version")
    return {
        "linux": f"SwarmCraft_{version}_amd64.deb",
        "windows": f"SwarmCraft_{version}_x64-setup.exe",
        "macos-arm64": f"SwarmCraft_{version}_arm64.dmg",
        "macos-x86_64": f"SwarmCraft_{version}_x86_64.dmg",
        "fabric": f"swarmcraft-fabric-{version}.jar",
    }


def strict_json(data: str):
    def pairs(entries):
        result = {}
        for key, value in entries:
            require(key not in result, "duplicate JSON field")
            result[key] = value
        return result
    return json.loads(data, object_pairs_hook=pairs)


def verify(directory: Path, release: dict, commit: str, version: str, *, legacy: bool = False) -> dict:
    require(bool(COMMIT.fullmatch(commit)), "expected exact commit")
    tag = "main-latest" if legacy else "main-snapshot-" + commit
    require(release.get("tag_name") == tag and release.get("tag_target") == commit,
            "release tag or independently resolved target mismatch")
    require(release.get("draft") is False and release.get("prerelease") is True,
            "not a published prerelease")
    package_names = packages(version)
    expected = set(package_names.values()) | {name + ".sha256" for name in package_names.values()}
    if not legacy:
        expected.add("release-provenance.json")
    require(directory.is_dir() and not directory.is_symlink(), "unsafe download directory")
    files = {path.name: path for path in directory.iterdir()}
    require(files.keys() == expected, "missing or unexpected downloaded assets")
    require(all(p.is_file() and not p.is_symlink() for p in files.values()), "unsafe asset path")

    remote = {}
    for asset in release["assets"]:
        name = asset["name"]
        require(name in expected and name not in remote and asset.get("state") == "uploaded",
                "unexpected, duplicate or incomplete remote asset")
        require(type(asset["size"]) is int and asset["size"] > 0
                and isinstance(asset["digest"], str)
                and bool(re.fullmatch(r"sha256:[a-f0-9]{64}", asset["digest"])),
                "missing remote integrity metadata")
        actual = {"bytes": files[name].stat().st_size, "sha256": sha256_file(files[name])}
        require(actual["bytes"] == asset["size"] and "sha256:" + actual["sha256"] == asset["digest"],
                f"downloaded bytes differ from remote integrity metadata: {name}")
        remote[name] = actual
    require(remote.keys() == expected, "incomplete remote asset inventory")

    evidence = []
    for platform, name in package_names.items():
        digest = verify_checksum(files[name], files[name + ".sha256"])
        require(digest == remote[name]["sha256"], "checksum mismatch")
        evidence.append({"platform": platform, "file": name, **remote[name]})

    # Check the real version in the embedded Fabric metadata, not its filename alone.
    with zipfile.ZipFile(files[package_names["fabric"]]) as jar:
        mods = [info for info in jar.infolist() if info.filename == "fabric.mod.json"]
        require(len(mods) == 1 and mods[0].file_size <= 65536, "invalid Fabric metadata entry")
        mod = strict_json(jar.read(mods[0]).decode("utf-8"))
        require(mod.get("id") == "swarmcraft" and mod.get("version") == version
                and mod.get("environment") == "server", "Fabric version or identity mismatch")

    if not legacy:
        require(files["release-provenance.json"].stat().st_size <= 65536, "oversized release provenance")
        manifest = strict_json(files["release-provenance.json"].read_text(encoding="utf-8"))
        require(set(manifest) == {"commit", "version", "channel", "scope", "assets"}
                and manifest["commit"] == commit and manifest["version"] == version
                and manifest["channel"] == "preview"
                and manifest["scope"] == "current-workflow artifact checksums, not a signing attestation",
                "manifest source/version/channel/scope mismatch")
        groups = expected_groups("preview", commit)
        records = {r["platform"]: r for r in manifest["assets"]}
        require(len(manifest["assets"]) == len(records) == len(package_names), "duplicate or missing platform")
        for record in evidence:
            actual = records.get(record["platform"])
            require(actual == {**record, "source_artifact": groups[record["platform"]]},
                    "manifest disagrees with verified package bytes or exact source group")

    return {"tag": tag, "tag_target": commit, "version": version,
            "source_manifest_verified": not legacy,
            "legacy_provenance_limitation": "No commit-bound manifest; tag and published bytes only" if legacy else None,
            "asset_count": len(remote), "asset_bytes": sum(r["bytes"] for r in remote.values()),
            "assets": remote, "packages": evidence,
            "scope": "Downloaded published bytes, GitHub metadata, checksums and Fabric version; not platform signing, installation or downgrade certification"}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--directory", required=True, type=Path)
    p.add_argument("--release-metadata", required=True, type=Path)
    p.add_argument("--commit", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--legacy", action="store_true")
    p.add_argument("--output", required=True, type=Path)
    args = p.parse_args()
    result = verify(args.directory, strict_json(args.release_metadata.read_text(encoding="utf-8")),
                    args.commit, args.version, legacy=args.legacy)
    with args.output.open("x", encoding="utf-8") as file:
        json.dump(result, file, indent=2)
        file.write("\n")
    print(f"PASS: {result['tag']} {result['asset_count']} assets, {result['asset_bytes']} bytes")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError, zipfile.BadZipFile) as error:
        raise SystemExit(str(error)) from None
