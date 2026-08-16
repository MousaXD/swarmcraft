#!/usr/bin/env python3
"""Verify SwarmCraft application/package version metadata stays coherent."""

from pathlib import Path
import json
import re
import tomllib

EXPECTED_APP = "0.2.1"
EXPECTED_PROTOCOL = "1"
ROOT = Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    raise SystemExit(f"release version check failed: {message}")


with (ROOT / "Cargo.toml").open("rb") as handle:
    cargo = tomllib.load(handle)
if cargo.get("workspace", {}).get("package", {}).get("version") != EXPECTED_APP:
    fail("workspace.package.version is not 0.2.1")

with (ROOT / "apps/desktop/src-tauri/Cargo.toml").open("rb") as handle:
    desktop = tomllib.load(handle)
if desktop.get("package", {}).get("version") != EXPECTED_APP:
    fail("desktop Cargo.toml package.version is not 0.2.1")

with (ROOT / "apps/desktop/src-tauri/tauri.conf.json").open(encoding="utf-8") as handle:
    tauri = json.load(handle)
if tauri.get("version") != EXPECTED_APP:
    fail("Tauri application version is not 0.2.1")

gradle = (ROOT / "minecraft/fabric/gradle.properties").read_text(encoding="utf-8")
if not re.search(r"^mod_version=0\.2\.1$", gradle, re.MULTILINE):
    fail("Fabric mod_version is not 0.2.1")

fabric_mod = (ROOT / "minecraft/fabric/src/main/resources/fabric.mod.json").read_text(encoding="utf-8")
if '"version": "${version}"' not in fabric_mod:
    fail("fabric.mod.json no longer inherits the Gradle mod version")

lock = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
owned = {
    "swarm-cli",
    "swarm-consensus",
    "swarm-core",
    "swarm-ipc",
    "swarm-network",
    "swarm-protocol",
    "swarm-storage",
}
seen = set()
for block in lock.split("[[package]]")[1:]:
    name = re.search(r'\nname = "([^"]+)"', block)
    version = re.search(r'\nversion = "([^"]+)"', block)
    if name and name.group(1) in owned:
        seen.add(name.group(1))
        if not version or version.group(1) != EXPECTED_APP:
            actual = version.group(1) if version else "missing"
            fail(f"Cargo.lock package {name.group(1)} is {actual}, expected {EXPECTED_APP}")
missing = owned - seen
if missing:
    fail(f"Cargo.lock is missing workspace packages: {', '.join(sorted(missing))}")

protocol = (ROOT / "crates/swarm-protocol/src/lib.rs").read_text(encoding="utf-8")
if not re.search(rf"pub const PROTOCOL_VERSION:\s*u16\s*=\s*{EXPECTED_PROTOCOL}\s*;", protocol):
    fail("wire protocol version changed unexpectedly; application version must not drive it")

print(f"SwarmCraft application metadata is {EXPECTED_APP}; wire protocol remains {EXPECTED_PROTOCOL}.")
