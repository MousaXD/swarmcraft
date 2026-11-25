#!/usr/bin/env python3
"""Real packaged Linux redacted support history and export."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import native_desktop_qa as q

sys.path.insert(0, str(q.REPO / "scripts/e2e"))
from private_infrastructure_smoke import Service, WORLD, command


def read(ui, script, args=None):
    return ui.call("/execute/sync", "POST", {"script": script, "args": args or []})


def button(id_):
    return q.Target("#" + id_, id_)


def run(args):
    os.umask(0o077)
    packages = args.packages.resolve()
    artifacts = args.artifacts.resolve()
    artifacts.mkdir(mode=0o700, parents=True, exist_ok=True)
    q.require(artifacts.stat().st_mode & 0o077 == 0, "private artifact directory required")
    evidence = {
        "scope": "extracted Linux Debian packages, real WebKitGTK, real bundled sidecars and isolated private world",
        "source_base": q.checked(["git", "-C", q.REPO, "rev-parse", "HEAD"]).decode().strip(),
        "variants": {},
    }
    with tempfile.TemporaryDirectory(prefix="swarmcraft-native-support-") as folder:
        root = Path(folder)
        for name in ("home", "runtime", "config", "cache", "local-share"):
            (root / name).mkdir(mode=0o700)
        display = next(i for i in range(111, 160) if not Path(f"/tmp/.X{i}-lock").exists()
                       and not Path(f"/tmp/.X11-unix/X{i}").exists())
        auth = root / "Xauthority"
        auth.touch(mode=0o600)
        q.checked(["xauth", "-f", auth, "add", f":{display}", "MIT-MAGIC-COOKIE-1", os.urandom(16).hex()])
        os.environ.update({
            "DISPLAY": f":{display}", "XAUTHORITY": str(auth), "HOME": str(root / "home"),
            "XDG_RUNTIME_DIR": str(root / "runtime"), "XDG_CONFIG_HOME": str(root / "config"),
            "XDG_CACHE_HOME": str(root / "cache"), "XDG_DATA_HOME": str(root / "local-share"),
            "GDK_BACKEND": "x11", "GSETTINGS_BACKEND": "memory", "WEBKIT_DISABLE_DMABUF_RENDERER": "1",
        })
        binaries = {}
        for variant in ("shipping", "candidate"):
            package = packages / (variant + ".deb")
            extracted = root / variant
            extracted.mkdir()
            q.checked(["dpkg-deb", "-x", package, extracted], seconds=60)
            binary = extracted / "usr/bin/swarmcraft-desktop"
            binaries[variant] = binary
            evidence["variants"][variant] = {
                "package_sha256": q.sha(package),
                "files": {name: q.sha(binary if name == "desktop" else q.sidecar(binary, name))
                          for name in ("desktop", *q.SIDECARS)},
                "checks": {},
            }
        q.require(all(evidence["variants"]["shipping"]["files"][name] ==
                      evidence["variants"]["candidate"]["files"][name] for name in q.SIDECARS),
                  "both builds must contain the same four native sidecars")

        from directory_moderation_smoke import canonical_bytes
        import threading
        import time

        def choose_dialog(ui, folder=None):
            # Native GTK folder chooser; input is restricted to this harness's
            # private X display and newly created temporary destination.
            result = []
            def interact():
                try:
                    deadline = time.monotonic() + 12
                    while time.monotonic() < deadline:
                        search = subprocess.run(["xdotool", "search", "--onlyvisible", "--name",
                                                 "Choose folder for redacted support report"],
                                                capture_output=True, text=True)
                        if search.returncode == 0 and search.stdout.strip():
                            window = search.stdout.splitlines()[-1]
                            break
                        time.sleep(.1)
                    else:
                        raise q.Failure("support export folder chooser did not appear")
                    q.checked(["xdotool", "windowfocus", "--sync", window])
                    if folder is None:
                        q.checked(["xdotool", "key", "Escape"])
                    else:
                        # Click the isolated Home location, then select its sole
                        # non-hidden test folder. No path injection or app IPC.
                        q.checked(["xdotool", "mousemove", "--window", window, "55", "60", "click", "1"])
                        time.sleep(.6)
                        q.checked(["import", "-window", window, str(artifacts / f"{folder.name}-chooser-home.png")])
                        q.checked(["xdotool", "mousemove", "--window", window, "370", "85", "click", "1"])
                        time.sleep(.3)
                        q.checked(["import", "-window", window, str(artifacts / f"{folder.name}-chooser-selected.png")])
                        geometry = q.checked(["xdotool", "getwindowgeometry", "--shell", window]).decode()
                        values = dict(line.split("=", 1) for line in geometry.splitlines() if "=" in line)
                        q.checked(["xdotool", "mousemove", "--window", window,
                                   str(int(values["WIDTH"])-60), str(int(values["HEIGHT"])-23), "click", "1"])
                    result.append(True)
                except Exception as error:
                    result.append(error)
            thread = threading.Thread(target=interact, daemon=True)
            thread.start()
            q.click(ui, button("supportSave"))
            thread.join(timeout=16)
            q.require(result == [True], "native folder chooser input did not complete")

        def history_file(profile):
            return profile / "data" / "support-history-v1.json"

        def history(profile):
            return json.loads(history_file(profile).read_text())

        def open_activity(ui, candidate):
            if not q.visible(ui, button("openSupportHistory")):
                q.open_diagnostics(ui, candidate)
            q.click(ui, button("openSupportHistory"))
            q.wait("support history receipt", lambda: read(ui, """
                return document.getElementById('supportHistoryStatus').textContent.includes('Recording')
                    && !document.getElementById('supportRefresh').disabled;
                """), 15)

        def preview(ui):
            q.click(ui, button("supportPreview"))
            q.wait("reviewed redacted JSON", lambda: read(ui, """
                return !document.getElementById('supportPreviewPanel').hidden
                    && !document.getElementById('supportSave').disabled;
                """), 15)
            return read(ui, "return document.getElementById('supportPreviewText').value")

        def verify(ui, profile, expected):
            q.click(ui, button("diagnosticVerify"))
            if expected:
                q.wait("durable native action receipt", lambda: len(history(profile)["events"]) >= expected, 20)
            else:
                q.wait("native replica verification", lambda: read(ui, """
                    const output=document.getElementById('output');
                    return (output.value || output.textContent || '').includes('Verified checkpoint:');
                    """), 20)

        xvfb = subprocess.Popen([str(args.xvfb.resolve()), f":{display}", "-screen", "0", "1440x1000x24",
                                 "-nolisten", "tcp", "-auth", str(auth)], stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            q.wait("private Xvfb", lambda: subprocess.run(
                ["xdpyinfo"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0, 12)
            for variant, binary in binaries.items():
                checks = evidence["variants"][variant]["checks"]
                candidate = variant == "candidate"
                profile = root / (variant + "-profile")
                profile.mkdir(mode=0o700)
                export_folder = root / "home" / "support-export"
                export_folder.mkdir(mode=0o700)
                secret_name = "Synthetic private world name"
                invite = "scinvite:" + "ab" * 80
                try:
                    # Prepare one legitimate private world using the actual CLI.
                    cli = q.sidecar(binary, "swarmcraft")
                    world = WORLD.search(command([cli, "--data-dir", profile / "data", "world", "create",
                        "--name", secret_name, "--minecraft", "26.1.2", "--fabric-loader", "0.19.3"],
                        env=q.profile_env(profile))).group(0)
                    before = canonical_bytes(profile / "data")
                    with q.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
                        q.ready(ui, empty=False)
                        q.open_diagnostics(ui, candidate)
                        verify(ui, profile, 0)
                        q.require(not history_file(profile).exists(), "history created before opt-in")
                        open_activity(ui, candidate)
                        q.require("Recording off" in ui.read(button("supportHistoryStatus"))["value"],
                                  "history was enabled by default")
                        checks["default_off_no_implicit_collection"] = True
                        q.click(ui, button("supportRecording"))
                        q.wait("durable recording opt-in", lambda: history_file(profile).exists()
                               and history(profile)["enabled"], 10)
                        q.require(not history(profile)["events"], "opt-in backfilled prior actions")
                        checks["explicit_opt_in_no_backfill"] = True
                        q.click(ui, button("navWorlds"))
                        q.open_diagnostics(ui, candidate)
                        verify(ui, profile, 1)
                        recorded = history(profile)["events"]
                        q.require(len(recorded) == 1 and recorded[0]["event"]["action"] == "verify_replica"
                                  and recorded[0]["event"]["outcome"] == "returned_ok", "native success not recorded")
                        checks["native_success_recorded"] = True
                        q.click(ui, button("openJoin"))
                        ui.type(q.field("joinInvite", "Invite"), invite)
                        q.click(ui, button("joinWorld"))
                        q.wait("native failed join outcome", lambda: len(history(profile)["events"]) >= 2, 20)
                        recorded = history(profile)["events"]
                        q.require(len(recorded) == 2 and recorded[1]["event"]["action"] == "join_request"
                                  and recorded[1]["event"]["outcome"] == "returned_error", "native failure not recorded")
                        checks["native_failure_recorded"] = True
                        q.click(ui, button("navWorlds"))
                        open_activity(ui, candidate)
                        q.wait("actual local action list", lambda: read(ui, """
                            return document.getElementById('supportEvents').children.length===2;
                            """), 15)
                        for width, height in [(980, 760), (720, 560)]:
                            ui.call("/window/rect", "POST", {"width": width, "height": height, "x": 0, "y": 0})
                            q.wait("native viewport", lambda: read(ui, "return [innerWidth,innerHeight]") == [width,height], 8)
                            q.require(read(ui, """
                                const p=document.getElementById('activityView');
                                return p.scrollWidth<=p.clientWidth+2;
                                """), "support panel overflow")
                            ui.screenshot(artifacts / f"{variant}-{width}x{height}-history.png")
                            checks[f"layout_{width}x{height}"] = True
                        # Native selected-world scope uses the locally typed ID,
                        # but the safe report does not carry it.
                        ui.scroll_to_control(q.Target("#supportScope", "History and export scope"))
                        ui.call(ui.element(q.Target('#supportScope option[value="selected"]', "Selected world only"))+"/click","POST",{})
                        q.wait("selected-world history scope", lambda: read(ui, """
                            return document.getElementById('supportScope').value==='selected'
                                && document.getElementById('supportEvents').children.length===1
                                && !document.getElementById('supportPreview').disabled;
                            """), 15)
                        selected = json.loads(preview(ui))
                        q.require(selected["scope"] == "selected_world" and len(selected["history"]["events"]) == 1
                                  and selected["history"]["events"][0]["action"] == "verify_replica",
                                  "selected scope is not bound to selected local world")
                        checks["selected_scope_without_identifiers"] = True
                        q.click(ui, button("supportDiscard"))
                        ui.scroll_to_control(q.Target("#supportScope", "History and export scope"))
                        ui.call(ui.element(q.Target('#supportScope option[value="all"]', "All world actions"))+"/click","POST",{})
                        q.wait("all-world history", lambda: read(ui, """
                            return document.getElementById('supportEvents').children.length===2
                                && !document.getElementById('supportPreview').disabled;
                            """), 15)
                        raw = preview(ui)
                        report = json.loads(raw)
                        q.require(report["schema"] == "swarmcraft-redacted-support-v1"
                                  and report["scope"] == "all_worlds"
                                  and len(report["history"]["events"]) == 2
                                  and report["desktop_build"]["commit"] == evidence["source_base"]
                                  and report["desktop_build"]["tree"] == "modified"
                                  and report["last_reported_node_path"] in ("unknown","unavailable","limited","direct","relay","direct_and_relay"),
                                  "redacted report missing provenance or scope")
                        for secret in (world, secret_name, invite, "scinvite:", str(profile), "127.0.0.1", "/ip4/", "private_key"):
                            q.require(secret not in raw, "private value in report")
                        checks["actual_redaction_and_build_provenance"] = True
                        ui.screenshot(artifacts / f"{variant}-preview.png")
                        choose_dialog(ui)
                        q.wait("cancel receipt", lambda: read(ui, """
                            return document.getElementById('supportExportStatus').textContent.includes('Save cancelled')
                                && !document.getElementById('supportPreview').disabled;
                            """), 15)
                        q.require(not list(export_folder.iterdir()), "cancel unexpectedly wrote a report")
                        checks["native_cancel_writes_nothing"] = True
                        try:
                            raw = preview(ui)
                            choose_dialog(ui, export_folder)
                            q.wait("native save receipt", lambda: read(ui, """
                                return !document.getElementById('supportPreview').disabled;
                                """), 15)
                            save_status = ui.read(button("supportExportStatus"))["value"]
                            if "chosen folder" not in save_status:
                                raise q.Failure("native save status: " + save_status)
                            q.require(len(list(export_folder.glob("swarmcraft-support-*.json"))) == 1,
                                      "native save did not write to the selected folder")
                            saved = next(export_folder.glob("swarmcraft-support-*.json"))
                            q.require(saved.read_text() == raw and saved.stat().st_mode & 0o077 == 0,
                                      "saved bytes differ from exact reviewed JSON or permissions are not private")
                            checks["native_save_exact_reviewed_bytes"] = True
                        except Exception:
                            checks["native_save_exact_reviewed_bytes"] = False
                            evidence["variants"][variant]["save_blocker"] = "native folder chooser did not complete exact file save"
                            subprocess.run(["xdotool", "key", "Escape"], check=False)
                            q.wait("picker closed", lambda: read(ui, "return !document.getElementById('supportPreview').disabled"), 15)
                        q.click(ui, button("supportRecording"))
                        q.wait("paused collection", lambda: not history(profile)["enabled"], 10)
                        q.require(len(history(profile)["events"]) == 2, "pause silently erased history")
                        checks["pause_keeps_existing_history"] = True
                    # Actual app restart with the same private profile.
                    with q.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
                        q.ready(ui, empty=False)
                        open_activity(ui, candidate)
                        q.require("Recording off" in ui.read(button("supportHistoryStatus"))["value"]
                                  and read(ui, "return document.getElementById('supportEvents').children.length") == 2,
                                  "history/consent did not persist across restart")
                        checks["history_persisted_across_restart"] = True
                        q.click(ui, button("supportClear"))
                        q.click(ui, button("supportClearCancel"))
                        q.require(len(history(profile)["events"]) == 2, "cancelled clear deleted history")
                        q.click(ui, button("supportClear"))
                        q.click(ui, button("supportClearConfirm"))
                        q.wait("all local history cleared", lambda: not history(profile)["events"], 10)
                        checks["explicit_clear_and_cancel"] = True
                    q.require(canonical_bytes(profile / "data") == before, "history altered canonical world protocol files")
                    checks["canonical_world_unchanged"] = True
                    print(variant + ": PASS", flush=True)
                except Exception as error:
                    evidence["variants"][variant]["failure"] = type(error).__name__
                    print(variant + ": " + str(error), flush=True)
                finally:
                    for report_path in export_folder.glob("swarmcraft-support-*.json"):
                        report_path.unlink()
                    export_folder.rmdir()
        finally:
            q.stop_owned_tree(xvfb)
        (artifacts / "native-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return all("failure" not in variant and not variant.get("save_blocker") for variant in evidence["variants"].values())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("packages", "artifacts", "xvfb", "driver", "native-driver"):
        parser.add_argument("--" + argument, type=Path, required=True)
    sys.exit(0 if run(parser.parse_args()) else 1)
