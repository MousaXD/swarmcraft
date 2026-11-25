#!/usr/bin/env python3
"""Real packaged Linux directory moderation and publication controls."""
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
        "scope": "extracted Linux Debian packages, real WebKitGTK, real bundled sidecars and signed moderation fixtures",
        "source_base": q.checked(["git", "-C", q.REPO, "rev-parse", "HEAD"]).decode().strip(),
        "variants": {},
    }
    with tempfile.TemporaryDirectory(prefix="swarmcraft-native-moderation-") as folder:
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
        original_profile_env = q.profile_env
        owner = root / "publisher"
        owner.mkdir(mode=0o700)
        env = original_profile_env(owner)
        env["SWARMCRAFT_DISCOVERY_MDNS"] = "0"
        cli = q.sidecar(binaries["shipping"], "swarmcraft")
        runtime = q.sidecar(binaries["shipping"], "swarmcraft-runtime")

        def create(profile, name, visibility, region, language):
            args = [cli, "--data-dir", profile / "data", "world", "create", "--name", name,
                    "--minecraft", "26.1.2", "--fabric-loader", "0.19.3", "--visibility", visibility]
            if region:
                args += ["--region", region, "--language", language]
            return WORLD.search(command(args, env=env)).group(0)

        arabic = create(owner, "Native Moderation Arabic", "public", "middle-east", "ar")
        create(owner, "Native Moderation English", "public", "europe", "en")
        create(owner, "Native Moderation Unlisted", "unlisted", "middle-east", "ar")
        create(owner, "Native Moderation Private", "private", None, None)
        owner_before = canonical_bytes(owner / "data")
        xvfb = subprocess.Popen([str(args.xvfb.resolve()), f":{display}", "-screen", "0", "1440x1000x24",
                                 "-nolisten", "tcp", "-auth", str(auth)], stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, start_new_session=True)
        publisher = None
        try:
            q.wait("private Xvfb", lambda: subprocess.run(
                ["xdpyinfo"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0, 12)
            publisher = Service([runtime, "--data-dir", owner / "data", "discovery-serve",
                                 "--listen", "/ip4/127.0.0.1/tcp/0"], root / "publisher.log", env)
            address = publisher.address("Discovery provider address: ")

            def client_env(profile):
                value = original_profile_env(profile)
                value.update(SWARMCRAFT_DISCOVERY_BOOTSTRAP=address, SWARMCRAFT_DISCOVERY_MDNS="0")
                return value

            def policy(binary, profile):
                return json.loads(command([q.sidecar(binary, "swarmcraft-runtime"), "--data-dir", profile / "data",
                                           "moderation", "state"], env=client_env(profile)))

            def listing_preference(binary, profile, world):
                return json.loads(command([q.sidecar(binary, "swarmcraft"), "--data-dir", profile / "data",
                                           "world", "discovery-listing", world], env=client_env(profile)))

            def choose(ui, field, value):
                # W3C native option selection: no DOM state mutation.
                target = q.Target("#" + field, field)
                ui.scroll_to_control(target)
                option = q.Target(f'#{field} option[value="{value}"]', value)
                ui.call(ui.element(option) + "/click", "POST", {})
                q.wait("native select changed", lambda: read(ui, "return document.getElementById(arguments[0]).value", [field]) == value, 8)

            q.profile_env = client_env
            for variant, binary in binaries.items():
                checks = evidence["variants"][variant]["checks"]
                profile = root / (variant + "-profile")
                profile.mkdir(mode=0o700)
                local_world = create(profile, "Native local publication", "public", "north-america", "en")
                before = canonical_bytes(profile / "data")
                try:
                    with q.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
                        q.ready(ui, empty=False)
                        checks["boot"] = True
                        q.click(ui, q.Target("#worldListingDetails > summary", "Discovery visibility and local publication"))
                        q.wait("verified local signed visibility", lambda: read(ui, """
                            return document.getElementById('worldListingState').textContent.includes('signed visibility: public')
                                && !document.getElementById('worldListingToggle').disabled;
                            """), 15)
                        q.click(ui, button("worldListingToggle"))
                        q.wait("paused local listing", lambda: read(ui, """
                            return document.getElementById('worldListingState').textContent.includes('publication is paused')
                                && !document.getElementById('worldListingToggle').disabled;
                            """), 15)
                        paused = listing_preference(binary, profile, local_world)
                        q.require(paused["locally_paused"] and paused["signed_visibility"] == "public"
                                  and not paused["signed_visibility_changed"] and not paused["world_membership_changed"],
                                  "local publication pause receipt mismatch")
                        q.click(ui, button("worldListingToggle"))
                        q.wait("resumed local listing", lambda: read(ui, """
                            return document.getElementById('worldListingState').textContent.includes('publication is not paused')
                                && !document.getElementById('worldListingToggle').disabled;
                            """), 15)
                        q.require(not listing_preference(binary, profile, local_world)["locally_paused"],
                                  "local resume not persisted")
                        checks["local_publication_pause_resume"] = True
                        q.click(ui, button("openDiscover"))
                        q.click(ui, q.Target(".listing-filters > summary", "Filter by region or language"))
                        choose(ui, "publicWorldRegion", "middle-east")
                        choose(ui, "publicWorldLanguage", "ar")
                        ui.type(q.field("publicWorldQuery", "Search public worlds"), "Native Moderation")
                        q.click(ui, button("publicWorldSearch"))
                        q.wait("verified filtered listing", lambda: read(ui, """
                            const rows=[...document.querySelectorAll('#publicWorldResults li')];
                            return !document.getElementById('publicWorldSearch').disabled && rows.length===1
                                && rows[0].dataset.worldId===arguments[0]
                                && rows[0].textContent.includes('Region: middle-east')
                                && rows[0].textContent.includes('Language: ar');
                            """, [arabic]), 40)
                        checks["actual_signed_metadata_filter"] = True
                        for width, height in [(980, 760), (720, 560)]:
                            ui.call("/window/rect", "POST", {"width": width, "height": height, "x": 0, "y": 0})
                            q.wait("native viewport", lambda: read(ui, "return [innerWidth,innerHeight]") == [width,height], 8)
                            q.require(read(ui, """
                                const panel=document.getElementById('discoverPanel');
                                return panel.scrollWidth<=panel.clientWidth+2;
                                """), "discovery panel overflows")
                            ui.screenshot(artifacts / f"{variant}-{width}x{height}-filters.png")
                            checks[f"layout_{width}x{height}"] = True
                        q.click(ui, q.Target("#publicWorldResults [data-action=report]", "Report listing"))
                        q.wait("visible report draft", lambda: read(ui, """
                            return !document.getElementById('directoryReportDraft').hidden
                                && document.getElementById('directoryReportTarget').textContent.includes(arguments[0]);
                            """, [arabic]), 8)
                        choose(ui, "directoryReportReason", "misleading_metadata")
                        ui.screenshot(artifacts / f"{variant}-report-draft.png")
                        q.click(ui, button("directoryReportSave"))
                        q.wait("saved local report", lambda: read(ui, """
                            return document.getElementById('directoryPolicyStatus').textContent.includes('Report saved locally')
                                && document.getElementById('directoryReportDraft').hidden;
                            """), 40)
                        state = policy(binary, profile)
                        q.require(len(state["reports"]) == 1 and not state["hidden_worlds"] and not state["directory_suppressions"],
                                  "report implied a hide or moderation decision")
                        entry = state["reports"][0]
                        q.require(entry["world_id"] == arabic and entry["reason"] == "misleading_metadata"
                                  and set(entry) == {"report_id","world_id","config_hash","reason","verified_unix_ms","recorded_unix_ms"},
                                  "unexpected report evidence")
                        checks["actual_report_saved_without_auto_moderation"] = True
                        q.click(ui, q.Target("#directoryReports [data-action=view]", "View report"))
                        exported = json.loads(read(ui, "return document.getElementById('directoryExport').value"))
                        q.require(exported == {"schema":"swarmcraft-local-listing-report-v1","allegation":True,**entry},
                                  "report export includes unexpected content")
                        checks["minimal_report_export"] = True
                        ui.screenshot(artifacts / f"{variant}-saved-report.png")
                        q.click(ui, q.Target("#publicWorldResults [data-action=hide]", "Hide listing"))
                        q.wait("listing hidden locally", lambda: read(ui, """
                            return document.getElementById('publicWorldResults').children.length===0
                                && document.getElementById('publicWorldStatus').textContent.includes('hidden on this device');
                            """), 15)
                        q.require(policy(binary, profile)["hidden_worlds"] == [arabic], "local hide not persisted")
                        # A fresh native search must also omit this listing,
                        # not just the previous renderer snapshot.
                        q.click(ui, button("publicWorldSearch"))
                        q.wait("native hidden result", lambda: read(ui, """
                            return !document.getElementById('publicWorldSearch').disabled
                                && document.getElementById('publicWorldResults').children.length===0
                                && document.getElementById('publicWorldStatus').textContent.includes('hidden on this device');
                            """), 40)
                        checks["persisted_hide_and_fresh_search"] = True
                        q.click(ui, q.Target("#directoryHidden [data-action=restore]", "Restore"))
                        q.wait("restored local listing", lambda: read(ui, """
                            return document.getElementById('directoryHidden').textContent.includes('No hidden listings');
                            """), 15)
                        q.click(ui, button("publicWorldSearch"))
                        q.wait("restored signed result", lambda: read(ui, """
                            return !document.getElementById('publicWorldSearch').disabled
                                && document.querySelector('#publicWorldResults li')?.dataset.worldId===arguments[0];
                            """, [arabic]), 40)
                        q.require(not policy(binary, profile)["hidden_worlds"], "restore not persisted")
                        checks["local_restore"] = True
                        q.click(ui, q.Target("#directoryReports [data-action=delete]", "Delete record"))
                        q.wait("report deleted", lambda: read(ui, """
                            return document.getElementById('directoryReports').textContent.includes('No saved reports')
                                && document.getElementById('directoryExportWrap').hidden;
                            """), 15)
                        q.require(not policy(binary, profile)["reports"], "report deletion not persisted")
                        checks["local_report_deletion"] = True
                    q.require(canonical_bytes(profile / "data") == before, "local world canonical files changed")
                    q.require(canonical_bytes(owner / "data") == owner_before, "owner canonical files changed")
                    q.require(not policy(binary, owner)["reports"], "report was sent to publisher")
                    checks["canonical_files_and_remote_policy_unchanged"] = True
                    print(variant + ": PASS", flush=True)
                except Exception as error:
                    evidence["variants"][variant]["failure"] = type(error).__name__
                    print(variant + ": " + str(error), flush=True)
        finally:
            q.profile_env = original_profile_env
            if publisher:
                publisher.close()
            q.stop_owned_tree(xvfb)
        (artifacts / "native-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return all("failure" not in variant for variant in evidence["variants"].values())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("packages", "artifacts", "xvfb", "driver", "native-driver"):
        parser.add_argument("--" + argument, type=Path, required=True)
    sys.exit(0 if run(parser.parse_args()) else 1)
