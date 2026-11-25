#!/usr/bin/env python3
"""Real packaged Linux CPU/heap preference controls; no runtime launch.

Uses a private Xvfb display, new profiles, actual extracted sidecars and
native WebKitGTK input. No configured CPU controller: a saved positive
preference must remain ineligible rather than silently run uncapped.
The independent delegated-cgroup test verifies actual kernel enforcement.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import native_desktop_qa as q
from native_frontend_rollback import require_closed


def read(ui, script, args=None):
    return ui.call("/execute/sync", "POST", {"script": script, "args": args or []})


def button(id_, label):
    return q.button("hostingContribution" + id_, label)


def field(id_, label):
    return q.field("hostingContribution" + id_, label)


def status(binary, profile, world):
    return q.consent(binary, profile, world)


def exercise(binary, profile, args, variant, evidence):
    checks = evidence["checks"]
    q.checked([q.sidecar(binary, "swarmcraft"), "--data-dir", profile / "data",
               "world", "create", "--name", q.WORLD_NAME, "--minecraft", "26.1.2",
               "--fabric-loader", "0.19.3", "--visibility", "private"], env=q.profile_env(profile))
    world = q.world_from_list(binary, profile)
    identity = q.cli(binary, profile, "identity")
    checkpoint = q.verified_checkpoint(binary, profile, world)
    canonical_status = q.cli(binary, profile, "world", "status", world)
    before = status(binary, profile, world)
    q.require(not before["consented"] and before["selected_heap_mib"] == 0
              and before["selected_cpu_millicores"] is None
              and before["consent_revision"] is None and not before["may_host_now"],
              "world creation unexpectedly granted resources")
    with q.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
        q.ready(ui, empty=False)
        q.select_world(ui, world)
        q.open_hosting_contribution(ui)
        q.wait("player-only visible", lambda: q.read_is(ui, q.PLAYER_ONLY, q.PLAYER_ONLY.label))
        confirm = q.Target("#hostingContributionConfirm",
                           "I explicitly allow this device to offer the selected Java heap and CPU limit when eligible.",
                           q.CHECK)
        opt = button("OptIn", "Save explicit opt-in")
        heap = field("Heap", "Java server heap budget (MiB)")
        cpu = field("Cpu", "CPU limit (millicores)")
        q.require(not ui.read(confirm)["checked"] and not ui.read(opt)["enabled"],
                  "preference could be saved without explicit confirmation")
        q.require(status(binary, profile, world)["consent_revision"] is None,
                  "viewing preference wrote a consent")
        checks["default_player_only_no_implicit_consent"] = True

        ui.type(heap, "4096")
        ui.type(cpu, "500")
        q.require(not ui.read(opt)["enabled"], "budget entry implicitly confirmed consent")
        q.click(ui, confirm)
        q.click(ui, opt)
        q.wait("CPU and heap saved", lambda: q.read_is(ui, q.OPTED_IN, q.OPTED_IN.label))
        first = status(binary, profile, world)
        q.require(first["consented"] and first["selected_heap_mib"] == 4096
                  and first["selected_cpu_millicores"] == 500 and not first["may_host_now"]
                  and first["process_cpu"]["mode"] == "unavailable"
                  and first["cpu_capacity_millicores"] is not None,
                  "explicit requested budget not persisted or unsupported cap advertised eligibility")
        q.wait("native CPU state", lambda: read(ui, """
            const text=document.getElementById('hostingContributionCpuStatus').textContent;
            return text.includes('0.5 CPU equivalents') && text.includes('Measured capacity')
                && text.includes('Requested CPU cap unavailable');
            """), 15)
        checks["four_gib_and_half_cpu_saved_but_not_admitted"] = True
        evidence["sampled_cpu_capacity_millicores"] = first["cpu_capacity_millicores"]
        for width, height in ((980, 760), (720, 560)):
            ui.call("/window/rect", "POST", {"width": width, "height": height, "x": 0, "y": 0})
            q.wait("native viewport", lambda: read(ui, "return [innerWidth,innerHeight]") == [width, height], 8)
            ui.scroll_to_control(heap)
            q.require(read(ui, """
                const p=document.getElementById('hostingContributionPanel');
                return p.scrollWidth <= p.clientWidth+2 && document.body.scrollWidth<=innerWidth+2
                    && document.querySelector('label[for=hostingContributionCpu]')!==null;
                """), "resource controls overflow or lack label")
            ui.screenshot(args.artifacts / f"{variant}-{width}x{height}-resources.png")
            checks[f"layout_{width}x{height}"] = True
        ui.type(cpu, "750")
        q.require(not ui.read(confirm)["checked"] and not ui.read(opt)["enabled"],
                  "changed budget retained confirmation")
        q.click(ui, confirm)
        q.click(ui, opt)
        q.wait("changed CPU visible", lambda: read(ui, """
            return document.getElementById('hostingContributionCpuStatus').textContent.includes('0.75 CPU equivalents')
                && !document.getElementById('hostingContributionWithdraw').disabled;
            """), 15)
        revised = status(binary, profile, world)
        q.require(revised["selected_cpu_millicores"] == 750 and revised["selected_heap_mib"] == 4096
                  and int(revised["consent_revision"]) > int(first["consent_revision"])
                  and not revised["may_host_now"], "CPU-only revision not persisted")
        checks["cpu_change_requires_confirmation_and_new_revision"] = True
        q.click(ui, button("Withdraw", "Player only — withdraw"))
        q.wait("withdrawal visible", lambda: q.read_is(ui, q.PLAYER_ONLY, q.PLAYER_ONLY.label))
        withdrawn = status(binary, profile, world)
        q.require(not withdrawn["consented"] and withdrawn["selected_heap_mib"] == 0
                  and withdrawn["selected_cpu_millicores"] is None
                  and int(withdrawn["consent_revision"]) > int(revised["consent_revision"])
                  and not withdrawn["may_host_now"], "zero contribution did not withdraw")
        q.require(not ui.read(button("Withdraw", "Player only — withdraw"))["enabled"],
                  "already withdrawn action still enabled")
        checks["explicit_zero_withdrawal"] = True
    require_closed(ui, [binary])
    with q.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
        q.ready(ui, empty=False)
        q.select_world(ui, world)
        q.open_hosting_contribution(ui)
        q.wait("withdrawal persisted across restart", lambda: q.read_is(ui, q.PLAYER_ONLY, q.PLAYER_ONLY.label))
        q.require(status(binary, profile, world)["consent_revision"] == withdrawn["consent_revision"],
                  "restart rewrote consent")
        checks["withdrawal_persisted_after_restart"] = True
    require_closed(ui, [binary])
    q.require(q.cli(binary, profile, "identity") == identity
              and q.verified_checkpoint(binary, profile, world) == checkpoint
              and q.world_from_list(binary, profile) == world,
              "resource controls changed world, checkpoint or identity")
    after_status = q.cli(binary, profile, "world", "status", world)
    for line in canonical_status.splitlines():
        if line.startswith(("Authority:", "Authorized peers:", "Latest snapshot:", "Visibility:", "Membership epoch:")):
            q.require(line in after_status.splitlines(), "canonical world metadata changed")
    q.require(not (profile / "data/runtime" / world.removeprefix("scworld:") / "eula.txt").exists(),
              "resource preference started/configured runtime")
    checks["canonical_world_and_identity_unchanged_no_runtime"] = True


def run(args):
    os.umask(0o077)
    args.artifacts = args.artifacts.resolve()
    args.artifacts.mkdir(mode=0o700, exist_ok=False)
    evidence = {"scope": "actual local Debian packages and native WebKitGTK, no delegated CPU cap or Minecraft runtime",
                "variants": {}}
    with tempfile.TemporaryDirectory(prefix="swarmcraft-native-resource-") as directory:
        root = Path(directory)
        for name in ("home", "runtime", "config", "cache", "local-share"):
            (root / name).mkdir(mode=0o700)
        display = next(i for i in range(111,160) if not Path(f"/tmp/.X{i}-lock").exists()
                       and not Path(f"/tmp/.X11-unix/X{i}").exists())
        auth = root / "Xauthority"
        auth.touch(mode=0o600)
        q.checked(["xauth", "-f", auth, "add", f":{display}", "MIT-MAGIC-COOKIE-1", os.urandom(16).hex()])
        os.environ.update(DISPLAY=f":{display}", XAUTHORITY=str(auth), HOME=str(root/"home"),
                          XDG_RUNTIME_DIR=str(root/"runtime"), XDG_CONFIG_HOME=str(root/"config"),
                          XDG_CACHE_HOME=str(root/"cache"), XDG_DATA_HOME=str(root/"local-share"),
                          GDK_BACKEND="x11", GSETTINGS_BACKEND="memory", WEBKIT_DISABLE_DMABUF_RENDERER="1",
                          DBUS_SESSION_BUS_ADDRESS="unix:path="+str(root/"no-session-bus"), GIO_USE_VFS="local",
                          GTK_USE_PORTAL="0", NO_AT_BRIDGE="1")
        os.environ.pop("WAYLAND_DISPLAY", None)
        binaries = {}
        for variant in ("shipping", "candidate"):
            package = args.packages.resolve() / (variant + ".deb")
            extracted = root / variant
            extracted.mkdir()
            q.checked(["dpkg-deb", "-x", package, extracted], seconds=90)
            binary = extracted / "usr/bin/swarmcraft-desktop"
            binaries[variant] = binary
            evidence["variants"][variant] = {
                "package_sha256": q.sha(package),
                "files": {name: q.sha(binary if name=="desktop" else q.sidecar(binary,name))
                          for name in ("desktop", *q.SIDECARS)},
                "checks": {},
            }
        q.require(all(evidence["variants"]["shipping"]["files"][name] ==
                      evidence["variants"]["candidate"]["files"][name] for name in q.SIDECARS),
                  "package sidecars differ")
        xvfb = subprocess.Popen([str(args.xvfb.resolve()), f":{display}", "-screen", "0", "1440x1000x24",
                                 "-nolisten", "tcp", "-auth", str(auth)],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            q.wait("private Xvfb", lambda: subprocess.run(
                ["xdpyinfo"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=2).returncode == 0, 15)
            for variant, binary in binaries.items():
                profile = root / (variant+"-profile")
                profile.mkdir(mode=0o700)
                try:
                    exercise(binary, profile, args, variant, evidence["variants"][variant])
                    print(variant + ": PASS", flush=True)
                except Exception as error:
                    evidence["variants"][variant]["failure"] = type(error).__name__
                    print(variant + ": " + str(error), flush=True)
        finally:
            q.stop_owned_tree(xvfb)
            (args.artifacts / "native-evidence.json").write_text(json.dumps(evidence, indent=2)+"\n")
    return all("failure" not in variant for variant in evidence["variants"].values())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("packages", "artifacts", "xvfb", "driver", "native-driver"):
        parser.add_argument("--"+arg, type=Path, required=True)
    sys.exit(0 if run(parser.parse_args()) else 1)
