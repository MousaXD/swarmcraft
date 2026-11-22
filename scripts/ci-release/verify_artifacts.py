#!/usr/bin/env python3
"""Fail-closed verification of exact-run, platform-labelled GitHub Actions artifacts.

This is a publication boundary, not a replacement for package, signing, or
installed-GUI acceptance. Each source directory is the unmerged download-artifact
folder from the *current workflow run*, named with its expected commit SHA.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import tomllib

ROOT = Path(__file__).resolve().parents[2]
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
DIGEST_LINE = re.compile(r"([0-9a-fA-F]{64}) [ *](.+)\Z")
PLATFORM_EXTENSIONS = {
    "linux": (".deb",),
    "windows": (".exe",),
    "macos-arm64": (".dmg",),
    "macos-x86_64": (".dmg",),
    "fabric": (".jar",),
}
ARCH_IDENTIFIERS = {
    "macos-arm64": ("arm64", "aarch64"),
    "macos-x86_64": ("x86_64", "x64", "amd64"),
}


def reject(reason: str) -> None:
    raise ValueError(f"release artifact verification failed: {reason}")


def expected_groups(channel: str, commit: str) -> dict[str, str]:
    if channel == "preview":
        return {
            "linux": f"swarmcraft-linux-deb-{commit}",
            "windows": f"swarmcraft-windows-exe-{commit}",
            "macos-arm64": f"swarmcraft-macos-arm64-dmg-{commit}",
            "macos-x86_64": f"swarmcraft-macos-x86_64-dmg-{commit}",
            "fabric": f"swarmcraft-fabric-{commit}",
        }
    if channel == "production":
        return {
            "linux": f"linux-release-{commit}",
            "windows": f"windows-release-{commit}",
            "macos-arm64": f"macos-arm64-release-{commit}",
            "macos-x86_64": f"macos-x86_64-release-{commit}",
            "fabric": f"fabric-release-{commit}",
        }
    reject("unknown release channel")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_checksum(package: Path, checksum: Path) -> str:
    if not checksum.is_file() or checksum.is_symlink():
        reject(f"missing/unsafe checksum for {package.name}")
    line = checksum.read_text(encoding="utf-8").strip()
    match = DIGEST_LINE.fullmatch(line)
    if not match:
        reject(f"malformed SHA-256 sidecar for {package.name}")
    declared, target = match.groups()
    # Existing Linux/macOS jobs run sha256sum on dist/<filename>, while
    # Windows and Fabric jobs record <filename>. Reject arbitrary paths.
    if target not in (package.name, f"dist/{package.name}"):
        reject(f"checksum sidecar names the wrong asset: {package.name}")
    actual = sha256_file(package)
    if declared.lower() != actual:
        reject(f"SHA-256 mismatch: {package.name}")
    return actual


def expected_status_file(platform: str) -> str | None:
    return {
        "linux": "SIGNING_STATUS_LINUX.txt",
        "windows": "SIGNING_STATUS_WINDOWS.txt",
        "macos-arm64": "SIGNING_STATUS_MACOS_arm64.txt",
        "macos-x86_64": "SIGNING_STATUS_MACOS_x86_64.txt",
    }.get(platform)


def verify(source: Path, output: Path, channel: str, commit: str, version: str) -> list[dict]:
    if not COMMIT.fullmatch(commit):
        reject("expected commit must be a full lower-case SHA-1")
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        reject("invalid application version")
    if source.resolve() == output.resolve():
        reject("source and publication directories must be different")
    if not source.is_dir() or source.is_symlink():
        reject("artifact download directory does not exist or is a symlink")

    groups = expected_groups(channel, commit)
    found_dirs = list(source.iterdir())
    actual_groups = {entry.name for entry in found_dirs}
    if any(not entry.is_dir() or entry.is_symlink() for entry in found_dirs):
        reject("artifact download contains unexpected files/symlinked directories")
    if actual_groups != set(groups.values()):
        reject(f"missing/unexpected exact-run artifact groups: {sorted(set(groups.values()) - actual_groups)}; extra: {sorted(actual_groups - set(groups.values()))}")

    evidence: list[dict] = []
    to_copy: list[Path] = []
    staged_names: set[str] = set()

    for platform, folder in groups.items():
        directory = source / folder
        files = list(directory.iterdir())
        if any(not item.is_file() or item.is_symlink() for item in files):
            reject(f"{folder} contains subdirectories or symlinks")

        expected_extensions = list(PLATFORM_EXTENSIONS[platform])
        if channel == "production" and platform == "linux":
            expected_extensions.append(".AppImage")
        packages: list[Path] = []
        for ext in expected_extensions:
            matching = [item for item in files if item.name.endswith(ext)]
            if len(matching) != 1:
                reject(f"{folder} requires exactly one {ext} package, got {len(matching)}")
            package = matching[0]
            if package.stat().st_size == 0 or "swarmcraft" not in package.name.lower():
                reject(f"empty or unexpected package: {package.name}")
            if version not in package.name:
                reject(f"package version differs from Cargo workspace: {package.name}")
            if platform in ARCH_IDENTIFIERS and not any(
                arch in package.name.lower() for arch in ARCH_IDENTIFIERS[platform]
            ):
                reject(f"macOS package architecture unrecognizable: {package.name}")
            packages.append(package)

        expected_filenames = {item.name for item in packages}
        expected_filenames.update(f"{item.name}.sha256" for item in packages)
        if channel == "production":
            status = expected_status_file(platform)
            if status:
                expected_filenames.add(status)
        if {item.name for item in files} != expected_filenames:
            reject(f"{folder} has missing/unknown package or signing-status files")

        for package in packages:
            digest = verify_checksum(package, directory / f"{package.name}.sha256")
            evidence.append({
                "platform": platform,
                "source_artifact": folder,
                "file": package.name,
                "sha256": digest,
                "bytes": package.stat().st_size,
            })
        for item in files:
            if item.name in staged_names:
                reject(f"colliding asset names between platform groups: {item.name}")
            staged_names.add(item.name)
            to_copy.append(item)

    if output.exists() and (output.is_symlink() or not output.is_dir() or any(output.iterdir())):
        reject("publication staging directory must be empty")
    output.mkdir(parents=True, exist_ok=True)
    for item in to_copy:
        shutil.copy2(item, output / item.name)
    (output / "release-provenance.json").write_text(
        json.dumps({
            "commit": commit,
            "version": version,
            "channel": channel,
            "scope": "current-workflow artifact checksums, not a signing attestation",
            "assets": evidence,
        }, indent=2) + "\n", encoding="utf-8"
    )
    return evidence


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--channel", choices=("preview", "production"), required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    with (ROOT / "Cargo.toml").open("rb") as stream:
        version = tomllib.load(stream)["workspace"]["package"]["version"]
    results = verify(args.source, args.output, args.channel, args.commit, version)
    for entry in results:
        print(f"VERIFIED {entry['platform']} {entry['file']} sha256={entry['sha256']} bytes={entry['bytes']}")
    print(f"PASS: {len(results)} checked packages from exact-run groups at {args.commit}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError) as error:
        raise SystemExit(str(error)) from None
