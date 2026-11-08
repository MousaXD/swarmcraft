#!/usr/bin/env python3
"""Visual acceptance of the *installed* Svelte candidate at the real Tauri window bounds.

Uses the actual packaged executable and unmodified #83 WebKitGTK driver.
No fake IPC, fixture worlds, screenshot generation, or browser source server.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

ROOT = Path(__file__).resolve().parents[4]
NATIVE = ROOT / "scripts" / "acceptance" / "packaged-desktop-webdriver.py"
spec = importlib.util.spec_from_file_location("native_swarmcraft_e2e", NATIVE)
if spec is None or spec.loader is None:
    raise RuntimeError("Cannot locate installed-GUI acceptance module")
native = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = native
spec.loader.exec_module(native)


def inspect_layout(app, width, height):
    request = native.request
    request(f"/session/{app.session}/window/rect", "POST", {"width": width, "height": height})
    time.sleep(0.5)
    return app.execute(r"""
      const pane = document.querySelector('.world-detail');
      const play = document.getElementById('playWorld');
      const invite = document.getElementById('inviteWorld');
      const panel = document.getElementById('worldsView');
      const details = document.getElementById('selectionContent');
      const visible = (element) => {
        if (!element) return false;
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && rect.right <= window.innerWidth + 2;
      };
      return {
        width: window.innerWidth,
        height: window.innerHeight,
        rootScrollWidth: document.documentElement.scrollWidth,
        paneScrollWidth: pane?.scrollWidth ?? 0,
        paneClientWidth: pane?.clientWidth ?? 0,
        panelScrollWidth: panel?.scrollWidth ?? 0,
        panelClientWidth: panel?.clientWidth ?? 0,
        detailsVisible: !!details && !details.hidden,
        playVisible: visible(play),
        inviteVisible: visible(invite),
        startup: document.documentElement.dataset.swarmcraftStartup || 'not-run',
      };
    """)


def run(binary: Path, artifacts: Path):
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise AssertionError("Candidate binary is missing or non-executable")
    if not os.environ.get("SWARMCRAFT_DATA_DIR"):
        raise AssertionError("CI must supply the existing isolated acceptance profile")
    artifacts.mkdir(parents=True, exist_ok=True)
    with native.Application(binary, artifacts) as app:
        ready = native.await_ready(app, artifacts)
        assert ready["startup"] in ("ready", "degraded"), "Native candidate startup not complete"
        # The preceding real packaged GUI acceptance creates a world. Ensure
        # that Create World was actually owned by the Svelte handler rather
        # than coincidentally passing through the legacy fallback.
        create_owner = app.execute("""
          return {
            form: document.getElementById('createForm')?.dataset.journeyOwner,
            active: window.__swarmcraftSvelteCreateOwned === true,
            bridge: typeof window.__swarmcraftAfterSvelteCreate === 'function',
          };
        """)
        assert (create_owner["form"] == "svelte" and create_owner["active"]
                and create_owner["bridge"]), (
            "Svelte native Create World ownership/handoff not active in installed app"
        )
        app.click("#navWorlds")
        app.click("#worldCards .world-row")
        for width, height in ((980, 760), (720, 560)):
            result = inspect_layout(app, width, height)
            (artifacts / f"layout-{width}x{height}.json").write_text(
                json.dumps(result, indent=2)
            )
            app.screenshot(f"world-details-{width}x{height}.png")
            assert result["startup"] in ("ready", "degraded"), f"Bad GUI state: {result}"
            assert result["detailsVisible"], f"Selected world is hidden: {result}"
            assert result["playVisible"] and result["inviteVisible"], (
                f"Primary controls clipped at {width}x{height}: {result}"
            )
            assert result["rootScrollWidth"] <= result["width"] + 2, (
                f"Horizontal application scroll: {result}"
            )
            assert result["paneScrollWidth"] <= result["paneClientWidth"] + 2, (
                f"Horizontal selected-world scroll: {result}"
            )
            assert result["panelScrollWidth"] <= result["panelClientWidth"] + 2, (
                f"Horizontal world workspace scroll: {result}"
            )
            print(f"PASS: installed native Svelte GUI layout at {width}x{height}")
        # Exercise the new Svelte signed-invite form in an installed app.
        # No invite is sent to Rust here; an empty form must fail locally,
        # stay on Join, and display an actionable validation message.
        app.click("#openJoin")
        join_owner = app.execute("""
          return {
            form: document.getElementById('joinForm')?.dataset.journeyOwner,
            active: window.__swarmcraftSvelteJoinOwned === true,
            bridge: typeof window.__swarmcraftAfterSvelteJoin === 'function'
          };
        """)
        assert (join_owner["form"] == "svelte" and join_owner["active"]
                and join_owner["bridge"]), (
            "Installed Join form is not owned by Svelte"
        )
        app.click("#joinWorld")
        observed = None
        for _ in range(15):
            observed = app.execute("""
              const banner = document.getElementById('joinError');
              const panel = document.getElementById('joinPanel');
              return {
                visible: !!banner && !banner.hidden,
                message: banner?.textContent || '',
                viewVisible: !!panel && !panel.hidden,
                inviteEmpty: !document.getElementById('joinInvite')?.value,
              };
            """)
            if observed["visible"]:
                break
            time.sleep(0.1)
        app.screenshot("join-empty-invite-validation.png")
        assert (observed["visible"] and observed["inviteEmpty"]
                and observed["viewVisible"]
                and "signed invite is required" in observed["message"].lower()), (
            "Empty invite did not fail closed through Svelte: " + repr(observed)
        )
        # Checking a World ID may display discovery guidance but must not
        # impersonate joining, create membership or request native IPC.
        # The World ID lookup is inside a collapsed native <details> panel.
        # Open it as a user would before typing into the field.
        app.click("#joinPanel details.task-secondary summary")
        app.type("#joinWorldId", "scworld:discovery-only")
        app.click("#joinWorldIdButton")
        discovery = None
        for _ in range(70):
            discovery = app.execute("""
              const note = document.getElementById('joinWorldIdNotice');
              const button = document.getElementById('joinWorldIdButton');
              return {
                visible: !!note && !note.hidden,
                finished: !!button && !button.disabled,
                explainsMembership: !!note && note.textContent.includes('does not grant membership'),
                signedInviteStillEmpty: !document.getElementById('joinInvite')?.value,
              };
            """)
            if discovery["finished"] and discovery["visible"] and discovery["explainsMembership"]:
                break
            time.sleep(0.2)
        assert all(discovery.values()), "Native signed discovery must not grant membership"
        app.click("#navWorlds")
        # Create a real signed invite on the previously created world, but
        # record only boolean observations, never the token or addresses.
        app.click("#inviteWorld")
        dialog_owner = app.execute("""
          return {
            active: window.__swarmcraftSvelteInviteOwned === true,
            createEnabled: !document.getElementById('createInvite')?.disabled
          };
        """)
        assert dialog_owner["active"] and dialog_owner["createEnabled"], (
            "Native invite dialog is not owned by Svelte"
        )
        app.click("#createInvite")
        invite_observation = None
        for _ in range(60):
            invite_observation = app.execute("""
              const result = document.getElementById('inviteResult');
              const wrapper = document.getElementById('inviteResultWrap');
              return {
                visible: !!wrapper && !wrapper.hidden,
                hasSignedValue: !!result && result.value.trim().length > 0,
              };
            """)
            if invite_observation["visible"] and invite_observation["hasSignedValue"]:
                break
            time.sleep(0.2)
        assert (invite_observation["visible"] and invite_observation["hasSignedValue"]), (
            "Signed invite did not appear after actual native Rust creation"
        )
        # Do not screenshot the signed token; close the dialog before other
        # evidence/screenshots are captured.
        app.click("#closeInvite")
        # The imported world must originate in a real filesystem path. An
        # empty import is rejected in Svelte before any Rust sidecar command;
        # import never synthesizes Minecraft EULA acceptance.
        app.click("#openImport")
        import_owner = app.execute("""
          return {
            form: document.getElementById('importForm')?.dataset.journeyOwner,
            active: window.__swarmcraftSvelteImportOwned === true,
            bridge: typeof window.__swarmcraftAfterSvelteImport === 'function'
          };
        """)
        assert (import_owner["form"] == "svelte" and import_owner["active"]
                and import_owner["bridge"]), "Installed Import form is not owned by Svelte"
        app.click("#importWorld")
        import_validation = None
        for _ in range(15):
            import_validation = app.execute("""
              const banner = document.getElementById('importError');
              const panel = document.getElementById('importPanel');
              return {
                visible: !!banner && !banner.hidden,
                message: banner?.textContent || '',
                formVisible: !!panel && !panel.hidden,
              };
            """)
            if import_validation["visible"]:
                break
            time.sleep(0.1)
        app.screenshot("import-missing-source-validation.png")
        assert (import_validation["visible"] and import_validation["formVisible"]
                and "world folder is required" in import_validation["message"].lower()), (
            "Empty import did not fail closed through Svelte: " + repr(import_validation)
        )
        app.click("#navWorlds")
        # Restore configured default for follow-up screenshots.
        native.request(f"/session/{app.session}/window/rect", "POST", {
            "width": 980, "height": 760
        })


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    try:
        run(args.binary.resolve(), args.artifacts.resolve())
    except Exception as error:
        args.artifacts.mkdir(parents=True, exist_ok=True)
        (args.artifacts / "failure.txt").write_text(
            type(error).__name__ + ": layout verification failed\n"
        )
        print(f"FAIL: installed candidate layout: {type(error).__name__}", file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
