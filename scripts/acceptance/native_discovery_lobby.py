#!/usr/bin/env python3
"""Real packaged Linux Discover flow using an isolated signed loopback publisher."""
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
        "scope": "extracted Linux Debian packages, real WebKitGTK, real bundled sidecars and signed loopback publisher",
        "source_base": q.checked(["git", "-C", q.REPO, "rev-parse", "HEAD"]).decode().strip(),
        "variants": {},
    }
    with tempfile.TemporaryDirectory(prefix="swarmcraft-native-lobby-") as folder:
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

        original_profile_env = q.profile_env
        owner = root / "publisher"
        owner.mkdir(mode=0o700)
        env = original_profile_env(owner)
        env["SWARMCRAFT_DISCOVERY_MDNS"] = "0"
        cli = q.sidecar(binaries["shipping"], "swarmcraft")
        runtime = q.sidecar(binaries["shipping"], "swarmcraft-runtime")
        worlds = {}
        for name, visibility in [(f"Native village {i:02}", "public") for i in range(9, 0, -1)] + [
            ("Native unlisted", "unlisted"), ("Native private", "private"),
        ]:
            output = command([cli, "--data-dir", owner / "data", "world", "create", "--name", name,
                              "--minecraft", "26.1.2", "--fabric-loader", "0.19.3",
                              "--visibility", visibility], env=env)
            worlds[name] = WORLD.search(output).group(0)
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
            q.require(address.startswith("/ip4/127.0.0.1/"), "loopback provider required")

            def client_env(profile):
                # Actual Tauri configuration. No fixture IPC, DOM mutation,
                # fake membership, or modification of a user's profile.
                value = original_profile_env(profile)
                value.update(SWARMCRAFT_DISCOVERY_BOOTSTRAP=address, SWARMCRAFT_DISCOVERY_MDNS="0")
                return value

            q.profile_env = client_env
            for variant, binary in binaries.items():
                checks = evidence["variants"][variant]["checks"]
                profile = root / (variant + "-profile")
                profile.mkdir(mode=0o700)
                try:
                    with q.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
                        q.ready(ui, empty=True)
                        checks["boot"] = True
                        q.click(ui, button("openDiscover"))
                        # Use a real filtered search. Record only the safe
                        # summary and row count, even on failure.
                        ui.type(q.field("publicWorldQuery", "Search public worlds"), "Native")
                        for attempt in range(2):
                            q.click(ui, button("publicWorldSearch"))
                            q.wait("native public search settled", lambda: read(ui, """
                                return !document.getElementById('publicWorldSearch').disabled
                                    && document.getElementById('publicWorldResults').getAttribute('aria-busy')==='false';
                                """), 40)
                            status = read(ui, """
                                return {summary:document.getElementById('publicWorldStatus').textContent,
                                    rows:document.getElementById('publicWorldResults').children.length};
                                """)
                            checks["search_observation"] = status
                            print(variant + ": search " + json.dumps(status), flush=True)
                            if "9 verified public listings" in status["summary"]:
                                break
                        q.require("9 verified public listings" in status["summary"],
                                  "nine verified native listings not returned")
                        checks["signed_public_search"] = True
                        for width, height in [(980, 760), (720, 560)]:
                            ui.call("/window/rect", "POST", {"width": width, "height": height, "x": 0, "y": 0})
                            q.wait("exact native viewport", lambda: read(ui, "return [innerWidth,innerHeight]") ==
                                   [width, height], 8)
                            page = read(ui, """
                                const panel=document.getElementById('discoverPanel');
                                const rows=[...document.querySelectorAll('#publicWorldResults li')];
                                return {count:rows.length, names:rows.map(r=>r.querySelector('strong').textContent),
                                    fresh:rows.every(r=>r.textContent.includes('Verified at')&&r.textContent.includes('Expires')),
                                    invite:rows.every(r=>r.textContent.includes('Signed invite required')),
                                    overflow:panel.scrollWidth>panel.clientWidth+2};
                                """)
                            q.require(page["count"] == 6 and page["names"] ==
                                      [f"Native village {i:02}" for i in range(1, 7)]
                                      and page["fresh"] and page["invite"] and not page["overflow"],
                                      "native listing order, freshness or layout failed")
                            ui.screenshot(artifacts / f"{variant}-{width}x{height}-discover.png")
                            checks[f"layout_{width}x{height}"] = True
                        q.click(ui, button("publicWorldNext"))
                        q.wait("second native page", lambda: read(ui, """
                            return document.getElementById('publicWorldPage').textContent.includes('Page 2 of 2')
                                && document.getElementById('publicWorldResults').children.length===3
                                && document.getElementById('publicWorldNext').disabled;
                            """), 5)
                        q.click(ui, button("publicWorldPrev"))
                        checks["bounded_pagination"] = True
                        q.click(ui, q.Target("#discoverPanel details summary", "Check an exact World ID"))
                        ui.type(q.field("joinWorldId", "World ID"), worlds["Native unlisted"])
                        q.click(ui, button("joinWorldIdButton"))
                        q.wait("real native exact unlisted result", lambda: read(ui, """
                            const s=document.getElementById('joinWorldIdNotice');
                            return s.textContent.includes('Native unlisted is unlisted')
                                && s.textContent.includes('Verified signed announcement')
                                && s.textContent.includes('does not grant membership');
                            """), 40)
                        checks["exact_unlisted"] = True
                        ui.screenshot(artifacts / f"{variant}-exact-unlisted.png")
                        q.click(ui, button("discoverToJoin"))
                        q.wait("separate private join", lambda: read(ui, """
                            return !document.getElementById('joinPanel').hidden
                                && document.getElementById('discoverPanel').hidden
                                && !document.getElementById('joinInvite').value;
                            """), 5)
                        checks["private_join_separate"] = True
                    q.require(q.cli(binary, profile, "world", "list") == "No local worlds.",
                              "discovery must not create membership")
                    checks["no_membership_created"] = True
                    print(variant + ": PASS", flush=True)
                except Exception as error:
                    # Keep screenshots private. No raw data, ID or address in
                    # the retained machine evidence.
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
