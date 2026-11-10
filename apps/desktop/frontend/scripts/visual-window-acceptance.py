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
CURRENT_STAGE = 'boot'
CURRENT_UI_STATE: dict[str, bool | int | str] = {}

# Only these values may enter CI failure artifacts. Native errors, invite
# contents, filesystem paths, and arbitrary DOM text must NEVER be serialized.
SAFE_BOOL_KEYS = frozenset({
    'detailsVisible', 'playVisible', 'inviteVisible', 'active', 'bridge',
    'visible', 'viewVisible', 'inviteEmpty', 'finished',
    'explainsMembership', 'signedInviteStillEmpty', 'hasSignedValue',
    'createEnabled', 'formVisible', 'dialogOpen', 'owned', 'enabled',
    'closed', 'stillSelected', 'worldStillListed',
    'identityOwned', 'identityVerified', 'identityNotEmpty', 'identityRefreshFinished',
    'identityNavigationVisible', 'identityNativeConfirmed',
    'identityNavDisplayVisible', 'identityNavHasGeometry', 'identityNavInsideViewport',
    'leaveSelectionVisible', 'leaveDisclosureOpen', 'leaveButtonVisible', 'leaveSummaryOnScreen',
    'identityToolsPresent', 'identityToolsOpen',
})
SAFE_INT_KEYS = frozenset({
    'width', 'height', 'rootScrollWidth', 'paneScrollWidth',
    'paneClientWidth', 'panelScrollWidth', 'panelClientWidth',
})
SAFE_STARTUP = frozenset({'loading', 'ready', 'degraded', 'failed', 'not-run'})


def mark_stage(stage: str, observed: dict | None = None) -> None:
    global CURRENT_STAGE, CURRENT_UI_STATE
    CURRENT_STAGE = stage
    CURRENT_UI_STATE = {}
    record_ui(observed)


def record_ui(observed: dict | None) -> None:
    if not isinstance(observed, dict):
        return
    for key, value in observed.items():
        if key in SAFE_BOOL_KEYS and type(value) is bool:
            CURRENT_UI_STATE[key] = value
        elif key in SAFE_INT_KEYS and type(value) is int and 0 <= value <= 100_000:
            CURRENT_UI_STATE[key] = value
        elif key == 'startup' and isinstance(value, str) and value in SAFE_STARTUP:
            CURRENT_UI_STATE[key] = value


def failure_category(error: Exception) -> str:
    if isinstance(error, AssertionError):
        return 'assertion'
    if isinstance(error, TimeoutError):
        return 'timeout'
    return 'native-driver-or-runtime'

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
    mark_stage('native-boot')
    with native.Application(binary, artifacts) as app:
        ready = native.await_ready(app, artifacts)
        record_ui({'startup': ready.get('startup')})
        assert ready["startup"] in ("ready", "degraded"), "Native candidate startup not complete"
        # The preceding real packaged GUI acceptance creates a world. Ensure
        # that Create World was actually owned by the Svelte handler rather
        # than coincidentally passing through the legacy fallback.
        mark_stage('svelte-create-ownership')
        create_owner = app.execute("""
          return {
            form: document.getElementById('createForm')?.dataset.journeyOwner,
            active: window.__swarmcraftSvelteCreateOwned === true,
            bridge: typeof window.__swarmcraftAfterSvelteCreate === 'function',
          };
        """)
        record_ui({'active': create_owner.get('active'), 'bridge': create_owner.get('bridge')})
        assert (create_owner["form"] == "svelte" and create_owner["active"]
                and create_owner["bridge"]), (
            "Svelte native Create World ownership/handoff not active in installed app"
        )
        app.click("#navWorlds")
        app.click("#worldCards .world-row")
        for width, height in ((980, 760), (720, 560)):
            mark_stage(f'layout-{width}x{height}')
            result = inspect_layout(app, width, height)
            record_ui(result)
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
        # Native identity refresh must be owned by Svelte and actually produce
        # a verified Rust-backed identity. The evidence records flags only:
        # never return or screenshot the peer's canonical identity bytes.
        mark_stage('native-identity-refresh')
        # The still-delegated player-experience controller moves Activity and
        # Diagnostics inside the collapsed #advancedNavigation <details>
        # ("Tools"). Open the real disclosure through native WebDriver first.
        # A synthetic JavaScript click on the hidden button would bypass the
        # user's actual navigation and leave a release regression undetected.
        tools = app.execute("""
          const menu = document.getElementById('advancedNavigation');
          return {
            identityToolsPresent: menu?.tagName === 'DETAILS',
            identityToolsOpen: menu?.open === true,
          };
        """)
        record_ui(tools)
        assert tools['identityToolsPresent'], (
            "Installed hybrid desktop must expose the real Tools disclosure"
        )
        if not tools['identityToolsOpen']:
            app.click("#advancedNavigation > summary")
        opened = app.execute("""
          return { identityToolsOpen: document.getElementById('advancedNavigation')?.open === true };
        """)
        record_ui(opened)
        assert opened['identityToolsOpen'], "Native Tools navigation did not expand"
        # The legacy runtime wizard also hides #navDiagnostics in its own CSS.
        # The Svelte candidate's specific override must win after expansion.
        # Check computed visibility before the real native WebDriver click.
        navigation = app.execute("""
          const nav = document.getElementById('navDiagnostics');
          const rect = nav?.getBoundingClientRect();
          return {
            identityNavDisplayVisible: !!nav && getComputedStyle(nav).display !== 'none',
            identityNavHasGeometry: !!rect && rect.width > 0 && rect.height > 0,
            identityNavInsideViewport: !!rect && rect.left >= 0 && rect.top >= 0
              && rect.right <= window.innerWidth + 2 && rect.bottom <= window.innerHeight + 2,
            identityNavigationVisible: !!nav && getComputedStyle(nav).display !== 'none'
              && !!rect && rect.width > 0 && rect.height > 0
              && rect.left >= 0 && rect.top >= 0
              && rect.right <= window.innerWidth + 2 && rect.bottom <= window.innerHeight + 2
          };
        """)
        record_ui(navigation)
        assert navigation['identityNavigationVisible'], (
            "Installed Svelte Diagnostics navigation must be visibly reachable"
        )
        app.click("#navDiagnostics")
        identity_owner = app.execute("""
          return window.__swarmcraftSvelteIdentityOwned === true;
        """)
        record_ui({'identityOwned': identity_owner})
        assert identity_owner, "Installed Diagnostics identity action is not Svelte-owned"
        # A prior legacy startup identity may already render Ready. Require a
        # new SUCCESSFUL current-generation native read after this user click,
        # not merely the old Ready text (which could otherwise falsely pass).
        native_before = app.execute("""
          return Number(document.getElementById('identity')?.dataset.nativeVerifiedRefreshCount ?? -1);
        """)
        assert type(native_before) is int and native_before >= 0, (
            "Svelte Diagnostics must expose its native-only refresh completion count"
        )
        app.click("#identityButton")
        identity_check = None
        for _ in range(60):
            identity_check = app.execute("""
              const output = document.getElementById('identity');
              const button = document.getElementById('identityButton');
              return {
                identityVerified: output?.dataset.identityPhase === 'ready',
                identityNotEmpty: !!output?.textContent?.trim(),
                identityRefreshFinished: !!button && !button.disabled,
                identityNativeConfirmed:
                  Number(output?.dataset.nativeVerifiedRefreshCount ?? -1) > arguments[0],
              };
            """, [native_before])
            record_ui(identity_check)
            if all(identity_check.values()):
                break
            time.sleep(0.2)
        assert all(identity_check.values()), "Native Svelte identity refresh did not complete a new verified Rust read"
        app.click("#navWorlds")
        # Exercise the new Svelte signed-invite form in an installed app.
        # No invite is sent to Rust here; an empty form must fail locally,
        # stay on Join, and display an actionable validation message.
        mark_stage('join-form-validation')
        app.click("#openJoin")
        join_owner = app.execute("""
          return {
            form: document.getElementById('joinForm')?.dataset.journeyOwner,
            active: window.__swarmcraftSvelteJoinOwned === true,
            bridge: typeof window.__swarmcraftAfterSvelteJoin === 'function'
          };
        """)
        record_ui({'active': join_owner.get('active'), 'bridge': join_owner.get('bridge')})
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
            record_ui(observed)
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
        mark_stage('signed-discovery-resolution')
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
            record_ui(discovery)
            if discovery["finished"] and discovery["visible"] and discovery["explainsMembership"]:
                break
            time.sleep(0.2)
        assert all(discovery.values()), "Native signed discovery must not grant membership"
        app.click("#navWorlds")
        # Create a real signed invite on the previously created world, but
        # record only boolean observations, never the token or addresses.
        mark_stage('real-signed-invite-issuance')
        app.click("#inviteWorld")
        dialog_owner = app.execute("""
          return {
            active: window.__swarmcraftSvelteInviteOwned === true,
            createEnabled: !document.getElementById('createInvite')?.disabled
          };
        """)
        record_ui(dialog_owner)
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
            record_ui(invite_observation)
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
        mark_stage('import-form-validation')
        app.click("#openImport")
        import_owner = app.execute("""
          return {
            form: document.getElementById('importForm')?.dataset.journeyOwner,
            active: window.__swarmcraftSvelteImportOwned === true,
            bridge: typeof window.__swarmcraftAfterSvelteImport === 'function'
          };
        """)
        record_ui({'active': import_owner.get('active'), 'bridge': import_owner.get('bridge')})
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
            record_ui(import_validation)
            if import_validation["visible"]:
                break
            time.sleep(0.1)
        app.screenshot("import-missing-source-validation.png")
        assert (import_validation["visible"] and import_validation["formVisible"]
                and "world folder is required" in import_validation["message"].lower()), (
            "Empty import did not fail closed through Svelte: " + repr(import_validation)
        )
        app.click("#navWorlds")
        # Cancellation is non-destructive: open Leave, cancel, and verify
        # that the canonical world remains in the actual installed GUI.
        mark_stage('leave-confirmation-cancel')
        selected = app.execute("""
          const content = document.getElementById('selectionContent');
          const world = document.getElementById('world');
          return {
            leaveSelectionVisible: !!content && !content.hidden && !!world?.value,
          };
        """)
        record_ui(selected)
        assert selected['leaveSelectionVisible'], (
            "World selection was lost before the real leave-cancellation journey"
        )
        # Leave world is intentionally inside a collapsed danger-zone <details>.
        # Open its real summary through native WebDriver rather than synthesizing
        # a click on a hidden destructive action or weakening the visibility gate.
        disclosure = app.execute("""
          const section = document.querySelector('#selectionContent details.danger-zone');
          return { leaveDisclosureOpen: section?.open === true };
        """)
        record_ui(disclosure)
        if not disclosure['leaveDisclosureOpen']:
            # The danger-zone is below the initial 980x760 right-pane viewport.
            # Scroll its real summary into view before the native WebDriver
            # click; a hidden/programmatic click would mask a broken GUI.
            summary_on_screen = app.execute("""
              const summary = document.querySelector('#selectionContent details.danger-zone > summary');
              if (summary) summary.scrollIntoView({ behavior: 'auto', block: 'center', inline: 'nearest' });
              const rect = summary?.getBoundingClientRect();
              return {
                leaveSummaryOnScreen: !!rect && rect.width > 0 && rect.height > 0
                  && rect.left >= 0 && rect.top >= 0
                  && rect.right <= window.innerWidth + 2 && rect.bottom <= window.innerHeight + 2,
              };
            """)
            record_ui(summary_on_screen)
            assert summary_on_screen['leaveSummaryOnScreen'], (
                "Membership and recovery disclosure is not reachable after scrolling"
            )
            app.click("#selectionContent details.danger-zone > summary")
        expanded = app.execute("""
          const section = document.querySelector('#selectionContent details.danger-zone');
          const leave = document.getElementById('leaveWorld');
          const rect = leave?.getBoundingClientRect();
          return {
            leaveDisclosureOpen: section?.open === true,
            leaveButtonVisible: !!leave && getComputedStyle(leave).display !== 'none'
              && !!rect && rect.width > 0 && rect.height > 0,
          };
        """)
        record_ui(expanded)
        assert expanded['leaveDisclosureOpen'] and expanded['leaveButtonVisible'], (
            "Installed membership disclosure must visibly expose the real Leave world action"
        )
        app.click("#leaveWorld")
        leave_open = app.execute("""
          return {
            dialogOpen: document.getElementById('leaveDialog')?.open === true,
            owned: window.__swarmcraftSvelteLeaveOwned === true,
            enabled: !document.getElementById('confirmLeave')?.disabled,
          };
        """)
        record_ui(leave_open)
        assert all(leave_open.values()), "Svelte signed leave confirmation is unavailable"
        app.click("#cancelLeave")
        leave_cancelled = app.execute("""
          return {
            closed: document.getElementById('leaveDialog')?.open === false,
            stillSelected: !!document.getElementById('world')?.value,
            worldStillListed: document.querySelectorAll('#worldCards .world-row').length > 0,
          };
        """)
        record_ui(leave_cancelled)
        assert all(leave_cancelled.values()), "Cancelling leave changed canonical membership"
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
        category = failure_category(error)
        (args.artifacts / 'failure.json').write_text(json.dumps({
            'stage': CURRENT_STAGE,
            'category': category,
            'ui': CURRENT_UI_STATE,
        }, indent=2) + '\n')
        (args.artifacts / 'failure.txt').write_text(
            f'{CURRENT_STAGE}: {category}\n'
        )
        print(f'FAIL: installed candidate stage {CURRENT_STAGE}: {category}', file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
