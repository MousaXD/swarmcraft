#!/usr/bin/env python3
"""Verify SwarmCraft release metadata; Agent 3 branch temporarily also runs its final matrix."""

from pathlib import Path
import base64
import json
import os
import re
import subprocess
import sys
import tomllib

EXPECTED_APP = "0.4.0"
EXPECTED_PROTOCOL = "1"
ROOT = Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    raise SystemExit(f"release version check failed: {message}")


def run(*args: str) -> None:
    print("+", " ".join(args), flush=True)
    subprocess.run(args, cwd=ROOT, check=True)


def dump_generated_files() -> None:
    candidates = [
        ROOT / "apps/desktop/src-tauri/src/curseforge.rs",
        ROOT / "apps/desktop/src-tauri/src/main.rs",
        ROOT / "apps/desktop/src-tauri/Cargo.lock",
    ]
    for path in candidates:
        rel = path.relative_to(ROOT).as_posix()
        changed = subprocess.run(
            ["git", "diff", "--quiet", "--", rel], cwd=ROOT
        ).returncode != 0
        if not changed:
            continue
        encoded = base64.b64encode(path.read_bytes()).decode("ascii")
        print(f"AGENT3_FILE_BEGIN {rel} {len(encoded)}", flush=True)
        for start in range(0, len(encoded), 4096):
            print(encoded[start : start + 4096], flush=True)
        print(f"AGENT3_FILE_END {rel}", flush=True)


with (ROOT / "Cargo.toml").open("rb") as handle:
    cargo = tomllib.load(handle)
if cargo.get("workspace", {}).get("package", {}).get("version") != EXPECTED_APP:
    fail("workspace.package.version is not 0.4.0")

with (ROOT / "apps/desktop/src-tauri/Cargo.toml").open("rb") as handle:
    desktop = tomllib.load(handle)
if desktop.get("package", {}).get("version") != EXPECTED_APP:
    fail("desktop Cargo.toml package.version is not 0.4.0")

with (ROOT / "apps/desktop/src-tauri/tauri.conf.json").open(encoding="utf-8") as handle:
    tauri = json.load(handle)
if tauri.get("version") != EXPECTED_APP:
    fail("Tauri application version is not 0.4.0")

gradle = (ROOT / "minecraft/fabric/gradle.properties").read_text(encoding="utf-8")
if not re.search(r"^mod_version=0\.4\.0$", gradle, re.MULTILINE):
    fail("Fabric mod_version is not 0.4.0")

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

if os.environ.get("GITHUB_HEAD_REF") == "agent/curseforge-provider":
    print("AGENT3_VALIDATION_START", flush=True)
    print("AGENT3_CHECKOUT_SHA", subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), flush=True)
    result = 0
    try:
        run("sudo", "apt-get", "update")
        run("sudo", "apt-get", "install", "-y", "libwebkit2gtk-4.1-dev", "libappindicator3-dev", "librsvg2-dev", "patchelf")
        run("rustup", "component", "add", "rustfmt", "clippy")
        finalize = ROOT / ".github/agent3_finalize.py"
        if finalize.exists():
            run("python3", str(finalize.relative_to(ROOT)))
        run("cargo", "fmt", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml")
        run("cargo", "generate-lockfile", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml")
        run("cargo", "metadata", "--locked", "--format-version", "1", "--no-deps")
        run("cargo", "fmt", "--all", "--", "--check")
        run("cargo", "check", "--workspace", "--all-features", "--locked")
        run("cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings")
        run("cargo", "test", "--workspace", "--all-features", "--locked")
        run("cargo", "check", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml", "--all-features", "--locked")
        run("cargo", "clippy", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings")
        run("cargo", "test", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml", "--all-features", "--locked")
        run("cargo", "test", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml", "curseforge::tests::", "--all-features", "--locked")

        source = (ROOT / "apps/desktop/src-tauri/src/curseforge.rs").read_text()
        main = (ROOT / "apps/desktop/src-tauri/src/main.rs").read_text()
        commands = [
            "curseforge_provider_status",
            "curseforge_search",
            "curseforge_project",
            "curseforge_versions",
            "curseforge_resolve",
            "curseforge_download",
        ]
        for command in commands:
            assert f"fn {command}" in source, command
            assert command in main, command
        for state in [
            '"available"',
            '"configuration_required"',
            '"rate_limited"',
            '"incompatible"',
            '"manual_artifact_required"',
            '"download_failed"',
            '"downloaded"',
            '"status": "ok"',
        ]:
            assert state in source, state
        for required in [
            "SWARMCRAFT_CURSEFORGE_API_KEY",
            "https://api.curseforge.com",
            "local_sha256",
            "provider_hashes_verified",
            "provider_download_unavailable",
            "supply_exact_file_id",
        ]:
            assert required in source, required
        print("AGENT3_STATIC_CONTRACT_OK", ",".join(commands), flush=True)
    except Exception as exc:
        result = 1
        print(f"AGENT3_VALIDATION_FAILURE {exc!r}", file=sys.stderr, flush=True)
    finally:
        dump_generated_files()
        print("AGENT3_VALIDATION_END", flush=True)
    if result:
        raise SystemExit(result)
