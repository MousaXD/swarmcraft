#!/usr/bin/env python3
"""Verify SwarmCraft release identity and shipped package metadata stay coherent."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import tomllib

EXPECTED_PROTOCOL = "1"
ROOT = Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    raise SystemExit(f"release version check failed: {message}")


def read_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def package_versions(lock_path: Path) -> dict[str, set[str]]:
    lock = read_toml(lock_path)
    versions: dict[str, set[str]] = {}
    for package in lock.get("package", []):
        name = package.get("name")
        version = package.get("version")
        if isinstance(name, str) and isinstance(version, str):
            versions.setdefault(name, set()).add(version)
    return versions


def require_lock_versions(
    lock_path: Path,
    expected: str,
    package_names: set[str],
) -> None:
    versions = package_versions(lock_path)
    for name in sorted(package_names):
        actual = versions.get(name)
        if actual is None:
            fail(f"{lock_path.relative_to(ROOT)} is missing shipped package {name}")
        if actual != {expected}:
            fail(
                f"{lock_path.relative_to(ROOT)} package {name} has versions "
                f"{', '.join(sorted(actual))}; expected exactly {expected}"
            )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--tag",
        help="Release tag to bind to application metadata, for example v0.5.0.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()

    cargo = read_toml(ROOT / "Cargo.toml")
    expected_app = cargo.get("workspace", {}).get("package", {}).get("version")
    if not isinstance(expected_app, str) or not re.fullmatch(r"\d+\.\d+\.\d+", expected_app):
        fail("workspace.package.version must be an explicit X.Y.Z application version")

    desktop = read_toml(ROOT / "apps/desktop/src-tauri/Cargo.toml")
    if desktop.get("package", {}).get("version") != expected_app:
        fail("desktop Cargo.toml package.version does not match workspace.package.version")

    provider = read_toml(ROOT / "crates/swarm-provider/Cargo.toml")
    if provider.get("package", {}).get("version") != expected_app:
        fail("excluded swarm-provider package.version does not match workspace.package.version")

    with (ROOT / "apps/desktop/src-tauri/tauri.conf.json").open(encoding="utf-8") as handle:
        tauri = json.load(handle)
    if tauri.get("version") != expected_app:
        fail("Tauri application version does not match workspace.package.version")

    gradle = (ROOT / "minecraft/fabric/gradle.properties").read_text(encoding="utf-8")
    if not re.search(rf"^mod_version={re.escape(expected_app)}$", gradle, re.MULTILINE):
        fail("Fabric mod_version does not match workspace.package.version")
    loom_match = re.search(r"^loom_version=(.+)$", gradle, re.MULTILINE)
    if not loom_match:
        fail("Fabric loom_version is missing")
    if "SNAPSHOT" in loom_match.group(1).upper():
        fail("Fabric loom_version must be an immutable released coordinate, not a snapshot")

    fabric_mod = (ROOT / "minecraft/fabric/src/main/resources/fabric.mod.json").read_text(
        encoding="utf-8"
    )
    if '"version": "${version}"' not in fabric_mod:
        fail("fabric.mod.json no longer inherits the Gradle mod version")

    root_owned = {
        "swarm-catalog",
        "swarm-cli",
        "swarm-consensus",
        "swarm-core",
        "swarm-ipc",
        "swarm-network",
        "swarm-protocol",
        "swarm-storage",
    }
    require_lock_versions(ROOT / "Cargo.lock", expected_app, root_owned)

    desktop_shipped = {
        "swarm-catalog",
        "swarm-core",
        "swarm-protocol",
        "swarm-provider",
        "swarm-storage",
        "swarmcraft-desktop",
    }
    require_lock_versions(
        ROOT / "apps/desktop/src-tauri/Cargo.lock", expected_app, desktop_shipped
    )

    protocol = (ROOT / "crates/swarm-protocol/src/lib.rs").read_text(encoding="utf-8")
    if not re.search(
        rf"pub const PROTOCOL_VERSION:\s*u16\s*=\s*{EXPECTED_PROTOCOL}\s*;", protocol
    ):
        fail("wire protocol version changed unexpectedly; application version must not drive it")

    if args.tag is not None:
        expected_tag = f"v{expected_app}"
        if args.tag != expected_tag:
            fail(f"release tag {args.tag!r} does not match application version {expected_tag!r}")

    suffix = f" and release tag {args.tag}" if args.tag else ""
    print(
        f"SwarmCraft application metadata is {expected_app}{suffix}; "
        f"wire protocol remains {EXPECTED_PROTOCOL}."
    )


if __name__ == "__main__":
    main()
