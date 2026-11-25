#!/usr/bin/env python3
"""Real packaged Linux friend controls and signed loopback presence."""
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
        "scope": "extracted Linux Debian packages, real WebKitGTK, real bundled sidecars and signed loopback friend presence",
        "source_base": q.checked(["git", "-C", q.REPO, "rev-parse", "HEAD"]).decode().strip(),
        "variants": {},
    }
    with tempfile.TemporaryDirectory(prefix="swarmcraft-native-friends-") as folder:
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

        def contact(profile, *argv):
            return json.loads(command([cli, "--data-dir", profile / "data", "contact", *argv], env=env))

        remote = contact(owner, "settings")
        profiles = {}
        for variant in binaries:
            profile = root / (variant + "-profile")
            profile.mkdir(mode=0o700)
            local = contact(profile, "settings")
            profiles[variant] = (profile, local)
            contact(owner, "add", local["own_peer_id"], "--public-key", local["own_public_key_hex"], "--label", variant)
            contact(owner, "share-presence", local["own_peer_id"], "--enabled=true")
        contact(owner, "presence-mode", "--enabled=true")
        xvfb = subprocess.Popen([str(args.xvfb.resolve()), f":{display}", "-screen", "0", "1440x1000x24",
                                 "-nolisten", "tcp", "-auth", str(auth)], stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, start_new_session=True)
        publisher = locator = None
        try:
            q.wait("private Xvfb", lambda: subprocess.run(
                ["xdpyinfo"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0, 12)
            locator = Service([args.infra.resolve(), "--state-dir", root / "locator", "locator",
                               "--listen", "/ip4/127.0.0.1/tcp/0"], root / "locator.log", env)
            locator_address = locator.address("Locator address: ")
            publisher = Service([runtime, "--data-dir", owner / "data", "discovery-serve", "--presence-only",
                                 "--listen", "/ip4/127.0.0.1/tcp/0", "--bootstrap", locator_address],
                                root / "publisher.log", env)
            address = publisher.address("Discovery provider address: ")

            def client_env(profile):
                value = original_profile_env(profile)
                value.update(SWARMCRAFT_DISCOVERY_BOOTSTRAP=locator_address + "," + address,
                             SWARMCRAFT_PRESENCE_LISTEN="/ip4/127.0.0.1/tcp/0", SWARMCRAFT_DISCOVERY_MDNS="0")
                return value

            q.profile_env = client_env
            for variant, binary in binaries.items():
                checks = evidence["variants"][variant]["checks"]
                profile, local = profiles[variant]
                try:
                    # The pre-created private fixture is only for verifying
                    # the existing signed invitation flow, not fake IPC.
                    command([cli, "--data-dir", profile / "data", "world", "create", "--name", "Friends QA private",
                             "--minecraft", "26.1.2", "--fabric-loader", "0.19.3", "--visibility", "private"], env=env)
                    before = q.cli(binary, profile, "world", "list")
                    with q.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
                        q.ready(ui, empty=False)
                        checks["boot"] = True
                        q.click(ui, button("openFriends"))
                        q.wait("hidden contact settings", lambda: read(ui, """
                            return document.getElementById('friendsModeText').textContent.includes('Hidden')
                                && document.getElementById('friendsServiceToggle').disabled
                                && !document.getElementById('friendAdd').disabled;
                            """), 20)
                        checks["hidden_default"] = True
                        q.click(ui, q.Target(".friends-add summary", "Add a contact"))
                        ui.type(q.field("friendLabel", "Local label"), "Native friend")
                        ui.type(q.field("friendPeer", "Peer ID"), remote["own_peer_id"])
                        ui.type(q.field("friendKey", "Public key"), remote["own_public_key_hex"])
                        q.click(ui, button("friendAdd"))
                        q.wait("contact saved without sharing", lambda: read(ui, """
                            return document.getElementById('friendsContacts').textContent.includes('Presence not shared')
                                && !document.getElementById('friendAdd').disabled;
                            """), 20)
                        checks["add_without_consent"] = True
                        q.click(ui, q.Target(".friends-add summary", "Add a contact"))
                        q.click(ui, q.Target("#friendsContacts [data-action=probe]", "Check presence"))
                        q.wait("signed online observation", lambda: read(ui, """
                            return document.getElementById('friendsContacts').textContent.includes('Online observed at');
                            """), 24)
                        checks["actual_signed_probe"] = True
                        q.click(ui, button("friendsModeToggle"))
                        q.wait("global enabled only", lambda: read(ui, """
                            return document.getElementById('friendsModeText').textContent.includes('Sharing enabled')
                                && document.getElementById('friendsServiceToggle').disabled
                                && !document.getElementById('friendsModeToggle').disabled;
                            """), 15)
                        checks["global_without_contact_grant_refused"] = True
                        q.click(ui, q.Target("#friendsContacts [data-action=share]", "Allow sharing"))
                        q.wait("contact share allowed", lambda: read(ui, """
                            return document.getElementById('friendsContacts').textContent.includes('Allowed to see your presence')
                                && !document.getElementById('friendsServiceToggle').disabled;
                            """), 15)
                        q.click(ui, button("friendsServiceToggle"))
                        q.wait("owned presence process", lambda: read(ui, """
                            return document.getElementById('friendsServiceStatus').textContent.includes('process started')
                                && !document.getElementById('friendsServiceToggle').disabled;
                            """), 15)
                        # Independently verify an actual signed response from
                        # the Desktop-started process via a known locator.
                        def probe():
                            result = contact(owner, "presence", local["own_peer_id"], "--bootstrap", locator_address)
                            return result["state"] == "online"
                        q.wait("remote signed observation of Desktop service", probe, 40)
                        checks["desktop_publisher_remote_proof"] = True
                        for width, height in [(980, 760), (720, 560)]:
                            ui.call("/window/rect", "POST", {"width": width, "height": height, "x": 0, "y": 0})
                            q.wait("native viewport", lambda: read(ui, "return [innerWidth,innerHeight]") == [width, height], 8)
                            q.require(read(ui, """
                                const panel=document.getElementById('friendsPanel');
                                return panel.scrollWidth<=panel.clientWidth+2 &&
                                    document.getElementById('friendsContacts').children.length===1;
                                """), "contact layout overflow")
                            ui.screenshot(artifacts / f"{variant}-{width}x{height}-friends.png")
                            checks[f"layout_{width}x{height}"] = True
                        q.click(ui, button("friendsModeToggle"))
                        q.wait("hidden and owned service stopped", lambda: read(ui, """
                            return document.getElementById('friendsModeText').textContent.includes('Hidden')
                                && document.getElementById('friendsServiceStatus').textContent.includes('stopped');
                            """), 15)
                        state = contact(profile, "settings")
                        q.require(not state["presence_enabled"] and state["contacts"][0]["share_presence"],
                                  "privacy mode not persisted")
                        checks["hide_persisted_and_service_stopped"] = True
                        q.click(ui, q.Target("#friendsContacts [data-action=remove]", "Remove"))
                        q.click(ui, q.Target("#friendsContacts [data-action=cancel]", "Cancel"))
                        q.require(len(contact(profile, "settings")["contacts"]) == 1, "cancel removed contact")
                        q.click(ui, q.Target("#friendsContacts [data-action=remove]", "Remove"))
                        q.click(ui, q.Target("#friendsContacts [data-action=confirm]", "Confirm removal"))
                        q.wait("contact removed", lambda: read(ui, "return document.getElementById('friendsContacts').textContent.includes('No contacts yet')"), 15)
                        q.require(not contact(profile, "settings")["contacts"], "contact removal not persisted")
                        checks["remove_confirmation_and_persistence"] = True
                        q.click(ui, button("friendsInvite"))
                        q.wait("existing signed invite dialog", lambda: read(ui, """
                            return document.getElementById('inviteDialog').open
                                && document.getElementById('inviteWorldName').textContent==='Friends QA private'
                                && !document.getElementById('inviteResult').value;
                            """), 10)
                        q.click(ui, button("cancelInvite"))
                        checks["signed_invite_workflow_no_automatic_token"] = True
                    q.require(q.cli(binary, profile, "world", "list") == before, "contact controls changed local worlds")
                    checks["world_membership_unchanged"] = True
                    print(variant + ": PASS", flush=True)
                except Exception as error:
                    evidence["variants"][variant]["failure"] = type(error).__name__
                    print(variant + ": " + str(error), flush=True)
        finally:
            q.profile_env = original_profile_env
            if publisher:
                publisher.close()
            if locator:
                locator.close()
            q.stop_owned_tree(xvfb)
        (artifacts / "native-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return all("failure" not in variant for variant in evidence["variants"].values())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("packages", "artifacts", "xvfb", "driver", "native-driver", "infra"):
        parser.add_argument("--" + argument, type=Path, required=True)
    sys.exit(0 if run(parser.parse_args()) else 1)
