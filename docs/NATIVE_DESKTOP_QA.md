# Installed native desktop QA (#141 / Sabathon #37)

**Status:** implemented local runner and external platform adapters; actual
Windows, Apple Silicon macOS and Intel macOS execution remains a release gate.
Do not infer a platform pass from a package build, a headless browser, the
Python contract tests, a different architecture, or a Linux native result.

## Scope and isolation

Run from the exact clean integration commit on a disposable local OS account
and isolated private test LAN. This is an installed-app test, not the
three-machine Minecraft gameplay certification. It uses two independent
SwarmCraft data roots/identities on one test computer, the real bundled CLI
and live authenticated networking. It never calls a mocked renderer bridge,
creates a world with the CLI, accepts the EULA, launches Minecraft, or grants
hosting outside the explicit GUI preference control.

| Host | Native adapter | Required evidence |
| --- | --- | --- |
| Windows x86_64 | Tauri WebDriver + matching Microsoft Edge WebDriver (`msedgedriver.exe`) | Actual installed WebView2 session, matching driver/runtime build |
| Apple Silicon macOS | External Swift Accessibility client against the application's WKWebView | Native arm64 executable, AX web area, real GUI actions |
| Intel macOS | Same external Accessibility client, compiled natively | Native x86_64 on Intel hardware; Rosetta does not count |
| Linux (optional regression) | Tauri WebDriver + WebKitWebDriver, disposable display | Local regression only; never substitutes for the above |

The macOS helper is **not** a Tauri command, plugin, feature, JavaScript
injection, web inspector, mock, or bundled executable. It attaches to the
exact process ID it started, enumerates that process's native AX web area,
presses accessible controls and enters text through native keyboard events.
Ambiguous actionable controls, missing permissions, wrong process, missing
web area and unavailable screenshots fail the run. The helper does not
dump an accessibility tree or type an invite through the clipboard.
Its binary is compiled separately and never included in a shipping package.

## Produce and install three artifacts

At the exact commit, with the normal pinned build prerequisites installed,
run local validation first:

```sh
python3 -m unittest discover -s scripts/acceptance -p 'test_*.py'
python3 -m unittest discover -s apps/desktop/frontend/scripts -p 'test_*.py'
python3 scripts/check_workflow_policy.py
```

Use the existing platform build and sidecar staging procedures in
`.github/workflows/main-installers.yml` as **local instructions**, not
as permission to dispatch a workflow. Build the four Rust sidecars once from
that commit for the native target architecture and stage them unchanged.
Use the normal shipping Tauri configuration for the shipping-path smoke.
Build the Svelte candidate with the existing pinned frontend lockfile
(`npm ci`, `npm run check`, `npm run build` in
`apps/desktop/frontend`). From the repository root, create a separate
negative asset directory:

```sh
python3 scripts/acceptance/native_desktop_qa.py prepare-broken
```

This command refuses an existing output directory; it does not modify
positive assets. It replaces the hashed module URL only in the separate
copy while retaining the real watchdog and startup guard. Use the QA-only
overlays in the next two builds. They change the product identifier/name
and frontend asset directory, not the command handler, security policy,
sidecar runtime, or default shipping configuration.

From `apps/desktop`, build each package separately (replace
`<bundle>` with `nsis` on Windows, `dmg` on macOS, or `deb` for
optional Linux regression):

```sh
cargo tauri build --bundles <bundle> --ci -- --locked
cargo tauri build --config frontend/tauri.native-qa.candidate.conf.json --bundles <bundle> --ci -- --locked
cargo tauri build --config frontend/tauri.native-qa.broken.conf.json --bundles <bundle> --ci -- --locked
```

Preserve each installer before building the next. Install into separate
dedicated test locations using the actual OS installer (or copy the actual
mounted DMG app on macOS). The three variants have distinct product IDs.
Do not replace a daily installation or install into a shared bin directory.
On macOS the executable is in the installed `.app/Contents/MacOS`
directory. On Windows pass the installed desktop executable, not the NSIS
installer. Linux may use disposable extracted Debian packages.

For this exact-commit QA matrix, the four installed sidecar files must
match byte-for-byte across all variants. Use the same staged sidecar
artifacts and consistent local QA signing. If independent production
signing changes binary bytes, retain that release's separate production
signature/provenance evidence; do not relax the runner's hash comparison.
This is not the signed/notarized release gate (#50).

Capture the installer and installed-file hashes for each variant using the
same clean checkout, for example:

```sh
python3 scripts/acceptance/native_desktop_qa.py capture \
  --variant shipping --binary /absolute/installed/desktop \
  --package /absolute/shipping-installer.dmg --manifest /private/evidence/shipping.json
```

Repeat with `candidate` and `broken`. Capture is an operator-attributed
manifest of the installed files and package hash, **not** cryptographic
proof that the OS installed those files from that package. Retain the
actual installer/build records and installation logs; compare installed
files against the captured manifests before the test. Never generate a
manifest from a modified installer and call it the original.

## Platform-specific prerequisites

**Windows:** install the supported Tauri WebDriver release locally and
the matching Microsoft Edge WebDriver for the actual installed WebView2
runtime (the first three version components must agree). Supply absolute
paths to `tauri-driver.exe` and `msedgedriver.exe`. Do not substitute
ChromeDriver or an ordinary Edge browser session. A driver or runtime
mismatch must fail. Run in an interactive, isolated Windows login; don't
run native GUI tests on a service account with no desktop.

**macOS:** on each actual architecture, compile the external helper in a
private QA directory using the local Xcode Command Line Tools:

```sh
xcrun swiftc -O scripts/acceptance/native_desktop_ax.swift \
  -o /private/qa/native-desktop-ax
```

Give the helper/its launching terminal the explicitly required Accessibility
and screen-recording permissions in System Settings. Do not disable
Gatekeeper, SIP, or platform security. Use a native terminal/Python (not
Rosetta). Run with the interactive desktop unlocked. Permission denial or
an unsupported accessibility mapping is an incomplete run, not a reason
to substitute a browser or an injected JavaScript bridge. No
`tauri-driver` is used on macOS.

**All platforms:** Python 3.11+, a fresh private evidence path, adequate
physical RAM for the 512 MiB consent preference, official Mojang/Fabric
catalog network access, and an explicit locally assigned RFC1918 IPv4
interface on the isolated test LAN. Do not expose the test to a public
interface. Literal loopback invites are deliberately prohibited by the
production invite validator; the runner refuses them. The explicit
private listener exists only during the test.

## Execute

The three variant arguments each take **installed executable, installer,
captured manifest**. On macOS run:

```sh
python3 scripts/acceptance/native_desktop_qa.py run \
  --shipping "$SHIP_EXE" "$SHIP_PACKAGE" "$SHIP_MANIFEST" \
  --candidate "$CANDIDATE_EXE" "$CANDIDATE_PACKAGE" "$CANDIDATE_MANIFEST" \
  --broken "$BROKEN_EXE" "$BROKEN_PACKAGE" "$BROKEN_MANIFEST" \
  --ax-helper /private/qa/native-desktop-ax \
  --lan-ip "$TEST_LAN_IP" --disposable-account \
  --artifacts /private/qa/native-result
```

On Windows (PowerShell or Git Bash), use the same three triples, LAN IP,
disposable-account attestation and evidence directory; replace
`--ax-helper` with `--driver C:/qa/tauri-driver.exe --native-driver
C:/qa/msedgedriver.exe`. On Linux use `--driver` and
`--native-driver /usr/bin/WebKitWebDriver` and a dedicated Xvfb display
in a disposable account.

The script checks native architecture and bundle identity, exact source
commit/tree and all installer/installed file hashes. It runs:

1. Shipping boot, fresh UI list, private GUI creation with a signed
   genesis epoch 1 and verified first checkpoint, no implicit hosting
   consent, full process close/restart, unchanged identity and world,
   automatic GUI invitation from the actual private daemon listener (no
   hidden bootstrap override), real player-only join from a second GUI,
   signed membership acceptance and exact verified checkpoint on both
   profiles.
2. Candidate repeats the shipping journey and independently verifies
   default player-only, disabled opt-in before explicit confirmation,
   GUI opt-in to 512 MiB, persisted consent after restart, GUI withdrawal,
   and persisted withdrawal after another restart. It never starts Java
   or treats a preference as live authority.
3. The separately installed broken-module QA package must visibly show
   `Startup failed`, an actionable diagnostic and an enabled
   `Retry startup` action. The action is clicked and failure must remain
   observable afterward. A driver exception or process crash alone fails.
4. The candidate is copied to a new disposable location; only its copied
   CLI sidecar is removed. On macOS only that damaged QA copy is ad-hoc
   re-signed and verified to avoid treating an OS signature refusal as
   the expected application diagnostic. Repeat the negative observation.
   Verify the original installed CLI hash is unchanged.

The read-only packaged CLI verification is a cross-check of actual GUI
side effects, not a substitute for clicking the UI. The app/driver are
closed after each session; explicit networking stop is exercised and the
test listener must be released. The runner terminates its isolated
process group/tree on cleanup, never all SwarmCraft processes. If a
failed session leaves an orphan, inspect the disposable account before
retrying rather than using a daily profile.

## Evidence and decision

`result.json` records the commit/tree, installer and installed binary
hashes, adapter provenance, native architecture, named scenario results
and explicit completion. The directory is new and private. Screenshots
are captured from the native session/window; keep raw screenshots
private, since synthetic world/peer IDs and local paths can be visible.
The runner intentionally does not log invitation tokens, raw identity
strings, full driver response bodies or native stdout/stderr. Report
sanitized failures and archive redacted evidence for independent review.

Pass only if the complete runner finishes on **all three native platform
rows**, with the shipping/candidate positive scenarios and both negative
controls on each exact commit. Review screenshots and installer
provenance independently. A timeout, permission denial, failed native
build, unavailable catalog, driver mismatch, incomplete cleanup or
unsupported AX control is blocked/failed, not a skip counted as pass.

The result still does not certify a Minecraft client/server connection,
WAN reachability, physical host migration, safe JVM shutdown, release
signing or notarization. Continue the separate
[three-machine field runbook](../scripts/e2e/three_machine/SABATHON_DESKTOP_FIELD_RUNBOOK.md)
for those observations. No hosted workflow is added or dispatched here.
