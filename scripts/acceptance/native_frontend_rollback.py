#!/usr/bin/env python3
"""Local Linux packaging rehearsal: legacy -> Svelte -> legacy, same data.

Uses two real, separately extracted same-product Debian installers, not a
running source server. Does not install over a user's existing application.
This proves frontend replacement/data compatibility, not OS update rollback
or the Windows/macOS release gates.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import os
from pathlib import Path
import tempfile
import sys

import native_desktop_qa as qa


# Any source change in these paths invalidates the operator's build attribution.
# Test-only/documentation changes are permitted after the two builds.
BUILD_INPUTS = (
    "Cargo.toml", "Cargo.lock", "crates", "apps/desktop/src",
    "apps/desktop/src-tauri", "apps/desktop/frontend/src",
    "apps/desktop/frontend/public", "apps/desktop/frontend/index.html",
    "apps/desktop/frontend/package.json", "apps/desktop/frontend/package-lock.json",
    "apps/desktop/frontend/vite.config.ts", "apps/desktop/frontend/tauri.candidate.conf.json",
)


def source_record(commit):
    git = ["git", "-C", qa.REPO]
    exact = qa.checked([*git, "rev-parse", "--verify", commit + "^{commit}"]).decode().strip()
    qa.checked([*git, "diff", "--quiet", exact, "--", *BUILD_INPUTS])
    untracked = qa.checked([*git, "ls-files", "--others", "--exclude-standard", "--", *BUILD_INPUTS])
    qa.require(not untracked.strip(), "untracked build inputs present")
    return {"operator_build_commit": exact,
            "build_inputs_unchanged": True,
            "runner_commit": qa.checked([*git, "rev-parse", "HEAD"]).decode().strip()}


def extract(package, directory):
    qa.require(package.is_file() and package.suffix == ".deb", "real Debian installer required")
    metadata = qa.checked(["dpkg-deb", "-f", package, "Package", "Version", "Architecture"]).decode()
    directory.mkdir()
    qa.checked(["dpkg-deb", "-x", package, directory], seconds=90)
    binary = directory / "usr/bin/swarmcraft-desktop"
    _, arch = qa.target_platform()
    files = {}
    for name, file in {"desktop": binary, **{name: qa.sidecar(binary, name) for name in qa.SIDECARS}}.items():
        qa.require(file.is_file() and not file.is_symlink() and os.access(file, os.X_OK),
                   "missing native packaged executable")
        qa.binary_arch(file, "linux", arch)
        files[name] = qa.sha(file)
    return binary, {"package_sha256": qa.sha(package), "metadata": metadata, "files": files}


def validate_pair(shipping, candidate):
    qa.require(shipping["metadata"] == candidate["metadata"], "same product/version/architecture required")
    qa.require(shipping["package_sha256"] != candidate["package_sha256"]
               and shipping["files"]["desktop"] != candidate["files"]["desktop"],
               "two distinct packaged frontends required")
    qa.require(all(shipping["files"][name] == candidate["files"][name] for name in qa.SIDECARS),
               "frontends must use byte-identical real sidecars")


def packaged_processes_exited(binaries, process_group):
    """Prove no member of the disposable session or either package is live.

    Never inspect process environments or kill by process name. The native
    driver owns a fresh process group; an escaped sidecar is detected by
    its exact executable in one of the uniquely extracted packages.
    """
    executables = {
        str(path.resolve())
        for binary in binaries
        for path in (binary, *(qa.sidecar(binary, name) for name in qa.SIDECARS))
    }
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
            if fields[0] in ("Z", "X"):
                continue
            if int(fields[2]) == process_group:
                return False
            if os.readlink(entry / "exe") in executables:
                return False
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            # A process can exit while /proc is enumerated; other users'
            # executables need not be readable for our isolated test.
            continue
    return True


def require_closed(ui, binaries):
    qa.require(ui.session is None and ui.proc.poll() is not None,
               "native session did not close")
    qa.wait("disposable desktop and sidecars exited",
            lambda: packaged_processes_exited(binaries, ui.proc.pid), seconds=15)


@contextmanager
def isolated_home(root):
    keys = {"HOME": root / "home", "XDG_DATA_HOME": root / "xdg-data",
            "XDG_CONFIG_HOME": root / "xdg-config", "XDG_CACHE_HOME": root / "xdg-cache",
            "XDG_STATE_HOME": root / "xdg-state"}
    original = {key: os.environ.get(key) for key in keys}
    try:
        for key, directory in keys.items():
            directory.mkdir(mode=0o700)
            os.environ[key] = str(directory)
        yield
    finally:
        for key, value in original.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


def frontend_owned(ui, candidate):
    state = ui.call("/execute/sync", "POST", {"script": """
        return { owner: window.__swarmcraftSvelteCreateOwned === true,
            form: document.getElementById('createForm')?.dataset.journeyOwner === 'svelte',
            bridge: typeof window.__TAURI__?.core?.invoke === 'function',
            startup: document.documentElement.dataset.swarmcraftStartup };
    """, "args": []})
    qa.require(state == {"owner": candidate, "form": candidate, "bridge": True, "startup": "ready"},
               "wrong installed frontend ownership or startup incomplete")


def check_saved(binary, profile, identity, world, checkpoint, heap):
    qa.require(qa.cli(binary, profile, "identity") == identity, "identity changed during frontend replacement")
    qa.require(qa.world_from_list(binary, profile) == world, "world disappeared or changed during frontend replacement")
    qa.require(qa.verified_checkpoint(binary, profile, world) == checkpoint,
               "signed checkpoint changed during frontend replacement")
    qa.require("Visibility: Private" in qa.cli(binary, profile, "world", "status", world),
               "private visibility changed during frontend replacement")
    saved = qa.consent(binary, profile, world)
    qa.require(saved["selected_heap_mib"] == heap and saved["consented"] == (heap != 0),
               "persisted local consent changed during frontend replacement")
    return saved


def set_consent(ui, binary, profile, world, heap):
    qa.open_hosting_contribution(ui)
    if heap:
        qa.wait("default player-only preference", lambda: qa.read_is(ui, qa.PLAYER_ONLY, "Player only — 0 MiB"))
        opt_in = qa.button("hostingContributionOptIn", "Save explicit opt-in")
        qa.require(not ui.read(opt_in)["enabled"], "opt-in was enabled without confirmation")
        ui.type(qa.field("hostingContributionHeap", "Java server heap budget (MiB)"), str(heap))
        confirm = qa.Target("#hostingContributionConfirm",
                            "I explicitly allow this device to offer the selected Java heap when eligible.", qa.CHECK)
        qa.require(not ui.read(confirm)["checked"], "confirmation was preselected")
        qa.click(ui, confirm)
        qa.require(ui.read(confirm)["checked"], "explicit confirmation did not register")
        qa.click(ui, opt_in)
        expected = qa.OPTED_IN
    else:
        qa.wait("opted-in preference after rollback", lambda: qa.read_is(ui, qa.OPTED_IN, "Hosting memory contribution selected"))
        qa.click(ui, qa.button("hostingContributionWithdraw", "Player only — withdraw"))
        expected = qa.PLAYER_ONLY
    qa.wait("native consent persisted", lambda: qa.consent(binary, profile, world)["selected_heap_mib"] == heap)
    qa.wait("native consent UI updated", lambda: qa.read_is(ui, expected, expected.label))


def rehearsal(factory, shipping, candidate, root, evidence, report):
    profile = qa.fresh_profile(root, "shared-profile")
    with factory(shipping, profile) as ui:
        qa.ready(ui, empty=True)
        frontend_owned(ui, False)
        world = qa.create_world(ui, shipping, profile)
        identity = qa.cli(shipping, profile, "identity")
        checkpoint = qa.verified_checkpoint(shipping, profile, world)
        check_saved(shipping, profile, identity, world, checkpoint, 0)
        ui.screenshot(evidence / "01-shipping-created.png")
    # This rehearsal closes an idle test application, not an active host.
    # The separate #37 native networking-control journey remains a gate.
    require_closed(ui, (shipping, candidate))
    check_saved(shipping, profile, identity, world, checkpoint, 0)
    report["stages"]["shipping-created"] = True
    print("PASS: shipping-created", flush=True)

    # The only state-changing actions after creation are explicit candidate
    # consent opt-in/withdrawal. Never launch Java or accept its EULA.
    for name, binary, is_candidate, previous, next_heap in (
        ("candidate-opt-in", candidate, True, 0, 512),
        ("shipping-rollback", shipping, False, 512, 512),
        ("candidate-withdraw", candidate, True, 512, 0),
        ("shipping-final-rollback", shipping, False, 0, 0),
    ):
        report["active_stage"] = name
        with factory(binary, profile) as ui:
            qa.ready(ui, empty=False)
            frontend_owned(ui, is_candidate)
            check_saved(binary, profile, identity, world, checkpoint, previous)
            qa.select_world(ui, world)
            if is_candidate:
                set_consent(ui, binary, profile, world, next_heap)
            check_saved(binary, profile, identity, world, checkpoint, next_heap)
            ui.screenshot(evidence / (name + ".png"))
        require_closed(ui, (shipping, candidate))
        check_saved(binary, profile, identity, world, checkpoint, next_heap)
        report["stages"][name] = True
        print("PASS:", name, flush=True)


def run(args):
    qa.require(qa.target_platform()[0] == "linux", "this rollback rehearsal requires native Linux")
    qa.require(args.disposable_display and os.environ.get("DISPLAY"), "explicit isolated GUI display required")
    evidence = args.artifacts.resolve()
    evidence.mkdir(mode=0o700)
    report = {"schema": 1, "complete": False, "release_certified": False,
              "scope": "extracted-deb-frontend-replacement-shared-profile", "stages": {},
              "active_stage": "source-and-packages"}
    try:
        report["source"] = source_record(args.build_commit)
        with tempfile.TemporaryDirectory(prefix="swarmcraft-frontend-rollback-") as temp:
            root = Path(temp)
            shipping, ship_record = extract(args.shipping.resolve(strict=True), root / "shipping")
            candidate, candidate_record = extract(args.candidate.resolve(strict=True), root / "candidate")
            validate_pair(ship_record, candidate_record)
            report["packages"] = {"shipping": ship_record, "candidate": candidate_record}
            driver, native = args.driver.resolve(strict=True), args.native_driver.resolve(strict=True)
            qa.require(driver.stem == "tauri-driver" and native.name == "WebKitWebDriver",
                       "native WebKitGTK driver required")
            with isolated_home(root):
                report["active_stage"] = "shipping-created"
                factory = lambda binary, profile: qa.WebDriver(binary, profile, driver, native)
                rehearsal(factory, shipping, candidate, root, evidence, report)
        report["complete"] = True
        report.pop("active_stage", None)
    except BaseException as error:
        report["failure"] = {"classification": type(error).__name__}
        if isinstance(error, qa.Failure):
            report["failure"]["reason"] = str(error)[:240]
        raise
    finally:
        qa.write_json(evidence / "result.json", report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("shipping", "candidate", "driver", "native-driver", "artifacts"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--build-commit", required=True)
    parser.add_argument("--disposable-display", action="store_true")
    try:
        run(parser.parse_args())
    except Exception as error:
        print("FAIL: " + type(error).__name__ + (": " + str(error) if isinstance(error, qa.Failure) else ""),
              file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
