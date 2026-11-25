#!/usr/bin/env python3
"""Extract two verified published Debian packages and rehearse package rollback.

Never downgrade live data. Reopen each release's separately preserved,
quiescent profile after running the other release. Use no Java server,
external account, production profile or system-wide installation.
"""
import argparse
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile

import native_desktop_qa as qa
from native_frontend_rollback import extract, require_closed, isolated_home


def verified_historical_snapshot(binary, profile, world):
    # Both published historical CLI versions verify snapshot contents and
    # signatures before printing the state root (not the newer checkpoint label).
    output = qa.cli(binary, profile, "world", "verify", world)
    roots = re.findall(r"^OK snapshot #([0-9]+) ([a-f0-9]{64})$", output, re.M)
    qa.require(len(roots) == 1 and roots[0][0] == "1", "historical signed genesis snapshot not verified")
    root = profile / "data" / "worlds"
    files = {str(path.relative_to(root)): qa.sha(path)
             for path in sorted(root.rglob("*.postcard")) if path.is_file() and not path.is_symlink()}
    qa.require(bool(files), "no retained canonical protocol files")
    return (roots[0][1], files)


def create_world(ui, binary, profile):
    qa.click(ui, qa.button("openCreate", "New world"))
    name = qa.field("createName", "World name")
    qa.wait("native world-name field", lambda: ui.read(name)["enabled"])
    ui.type(name, qa.WORLD_NAME)
    qa.click(ui, qa.button("createWorld", "Create world"))
    world = qa.wait("GUI-created persisted world", lambda: qa.world_from_list(binary, profile))
    qa.select_world(ui, world)
    qa.require("Visibility: Private" in qa.cli(binary, profile, "world", "status", world),
               "GUI creation did not retain private visibility")
    return world


def main(args):
    os.umask(0o077)
    args.artifacts.mkdir(mode=0o700, parents=True, exist_ok=True)
    qa.require(args.artifacts.stat().st_mode & 0o077 == 0, "private evidence directory required")
    result = {"scope": "actual downloaded Debian packages, real WebKitGTK, separate quiescent data profiles; not live-data downgrade", "packages": {}, "stages": {}}
    try:
        with tempfile.TemporaryDirectory(prefix="swarmcraft-published-rollback-") as tmp:
            root = Path(tmp)
            binaries = {}
            profiles = {}
            for label, version in (("legacy", "0.5.0"), ("pinned", "0.5.1")):
                record = json.loads((args.verified / f"{label}-verified.json").read_text())
                qa.require(record["version"] == version and record["source_manifest_verified"] == (label == "pinned"),
                           "verified download evidence/version missing")
                package_record, = [p for p in record["packages"] if p["platform"] == "linux"]
                package = args.verified / label / package_record["file"]
                qa.require(qa.sha(package) == package_record["sha256"], "Debian package changed after verification")
                binary, native = extract(package, root / label)
                qa.require(f"Version: {version}" in native["metadata"] and "Architecture: amd64" in native["metadata"],
                           "Debian metadata differs from verified version/platform")
                binaries[label] = binary
                profiles[label] = qa.fresh_profile(root, label + "-profile")
                result["packages"][label] = {**native, "tag": record["tag"], "tag_target": record["tag_target"],
                                              "source_manifest_verified": record["source_manifest_verified"]}
            display = next(i for i in range(111, 160) if not Path(f"/tmp/.X{i}-lock").exists()
                           and not Path(f"/tmp/.X11-unix/X{i}").exists())
            auth = root / "Xauthority"
            auth.touch(mode=0o600)
            qa.checked(["xauth", "-f", auth, "add", f":{display}", "MIT-MAGIC-COOKIE-1", os.urandom(16).hex()])
            runtime = root / "runtime"
            runtime.mkdir(mode=0o700)
            os.environ.update(DISPLAY=f":{display}", XAUTHORITY=str(auth), XDG_RUNTIME_DIR=str(runtime),
                              GDK_BACKEND="x11", GSETTINGS_BACKEND="memory", WEBKIT_DISABLE_DMABUF_RENDERER="1")
            xvfb = subprocess.Popen([str(args.xvfb.resolve()), f":{display}", "-screen", "0", "1440x1000x24",
                                     "-nolisten", "tcp", "-auth", str(auth)], stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL, start_new_session=True)
            saved = {}
            try:
                qa.wait("private Xvfb", lambda: subprocess.run(["xdpyinfo"], stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL).returncode == 0, 12)
                with isolated_home(root):
                    for label, stage in (("legacy", "legacy_baseline"), ("pinned", "pinned_current"),
                                         ("legacy", "rollback_previous"), ("pinned", "return_current")):
                        result["active_stage"] = stage
                        binary, profile = binaries[label], profiles[label]
                        with qa.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
                            qa.ready(ui, empty=label not in saved)
                            if label not in saved:
                                world = create_world(ui, binary, profile)
                                saved[label] = (qa.cli(binary, profile, "identity"), world,
                                                verified_historical_snapshot(binary, profile, world))
                            identity, world, checkpoint = saved[label]
                            qa.require(qa.cli(binary, profile, "identity") == identity
                                       and qa.world_from_list(binary, profile) == world
                                       and verified_historical_snapshot(binary, profile, world) == checkpoint,
                                       "selected package did not reopen its preserved verified profile")
                            ui.screenshot(args.artifacts / f"{stage}.png")
                        require_closed(ui, tuple(binaries.values()))
                        # Check both profiles read-only after each GUI stage.
                        for prior, (identity, world, checkpoint) in saved.items():
                            qa.require(qa.cli(binaries[prior], profiles[prior], "identity") == identity
                                       and verified_historical_snapshot(binaries[prior], profiles[prior], world) == checkpoint,
                                       "other package's preserved profile was altered")
                        result["stages"][stage] = True
                        print("PASS:", stage, flush=True)
            finally:
                qa.stop_owned_tree(xvfb)
        result["complete"] = True
        result.pop("active_stage", None)
    except Exception as error:
        result["failure"] = {"category": type(error).__name__, "reason": str(error)[:240] if isinstance(error, qa.Failure) else "see local harness logs"}
        raise
    finally:
        qa.write_json(args.artifacts / "result.json", result)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("verified", "artifacts", "xvfb", "driver", "native-driver"):
        parser.add_argument("--" + name, type=Path, required=True)
    main(parser.parse_args())
