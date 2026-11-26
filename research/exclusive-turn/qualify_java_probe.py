#!/usr/bin/env python3
"""Run the isolated Java process fixture under disposable kernel delegation.

No live save is opened. Copies trusted retained test artifacts into a
private package; the Rust test never starts Minecraft. Production admission
still requires full game/client/OS headroom, regardless of fixture results.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import uuid

from verify_core_runtime import digest, require


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("library-test-binary", "server-fixture", "toolchain-fixture"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--confirm-disposable-test-artifacts", action="store_true", required=True)
    args = parser.parse_args()
    os.umask(0o077)
    binary = args.library_test_binary.resolve(strict=True)
    server = args.server_fixture.resolve(strict=True)
    toolchain = args.toolchain_fixture.resolve(strict=True)
    repo = Path(__file__).resolve().parents[2]
    with tempfile.TemporaryDirectory(prefix="sc8-java-probe-") as tmp:
        package = Path(tmp) / "package"
        package.mkdir()
        for original, dest in ((toolchain / "jdk", "java"), (toolchain / "loader-libs", "fabric-libraries"),
                               (server / "versions", "versions"), (server / "libraries", "libraries")):
            shutil.copytree(original, package / dest, symlinks=True)
        for original, dest in ((server / "server.jar", "server.jar"),
                               (toolchain / "fabric-server-profile.json", "fabric-profile.json"),
                               (server / "mods/swarmcraft-fabric-0.5.1.jar", "mods/swarmcraft-fabric.jar")):
            target = package / dest
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(original, target)
            require(digest(original) == digest(target), "package copy differs")
        for item in [package, *package.rglob("*")]:
            if item.is_symlink():
                continue
            item.chmod(0o700 if item.is_dir() else 0o600 | (item.stat().st_mode & 0o111))
        unit = f"swarmcraft-memory-cap-test-probe-{uuid.uuid4().hex}"
        command = ["systemd-run", "--user", "--scope", "--quiet", f"--unit={unit}", "-p", "Delegate=yes",
                   "env", "SWARMCRAFT_TEST_CPU_DELEGATION=1", f"SWARMCRAFT_TEST_PROBE_PACKAGE={package}",
                   "bash", str(repo / "scripts/e2e/linux_cgroup_memory_gate.sh"), str(binary),
                   "--exact", "exclusive_turn_process_probe::tests::real_java_probe_owns_kernel_resources_and_retires_exact_records",
                   "--ignored", "--nocapture", "--test-threads=1"]
        # Scope belongs solely to this fixture; systemd removes it after exit.
        # On failure, never kill any account-wide process or remove its record.
        try:
            subprocess.run(command, cwd=repo, check=True, timeout=420)
        except subprocess.TimeoutExpired:
            # Stop only the randomly named disposable scope created above.
            # Its durable account record, if any, is not reset here.
            subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], check=False, timeout=30)
            raise


if __name__ == "__main__":
    main()
