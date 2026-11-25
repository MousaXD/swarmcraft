# Desktop accessibility and everyday usability (#43)

## Implementation

Shared keyboard focus/navigation, skip control, modal accessible names and descriptions, polite status and pending-operation messages, visible/forced-color focus treatment, wrapping and scroll behavior, and reduced-motion rules are shipped through both the legacy and candidate UI. Stop and Leave open on Cancel. Dialog close restores the originating focus where available, without stealing focus from a newer task. Full canonical IDs remain copyable.

Native operations are not canceled by a UI dismissal. The UI refuses duplicate Create, Invite, and runtime setup submissions during outstanding requests, prevents cancellation of non-cancellable pending modal operations, preserves partial canonical Create success, and invalidates outdated Play inspection responses on navigation or world changes. The Rust backend remains responsible for membership, consent, authority, EULA and runtime readiness.

## Verified locally

| Scope | Result |
| --- | --- |
| Frontend Node suite | 156 passed; final targeted runtime-wizard regression 7 passed |
| Svelte | Unit tests, 30 source-parity tests, check and production build passed |
| Chromium (shipping and compiled candidate; mocked native IPC) | Shared journeys and interrupted-operation scenarios at 720x560, 980x760, 1280x900; keyboard-only Invite/Stop at both supported sizes; effective CSS viewports 576x448 and 360x280 |
| Native Linux (shipping and candidate extracted Debian packages, real WebKitGTK and bundled Rust sidecars) | Exact 980x760 and 720x560, Tab/Enter/Escape navigation, long world name/full ID, named/described invite dialog and live-state markup, no horizontal dialog overflow, focus restoration, safe Stop cancellation; verified signed checkpoint and player-only consent unchanged |
| Workflow policy | Five workflows accepted; no hosted CI dispatched |

The browser fixture initially leaked a test state flag across viewport runs; reset it per run and reran both successfully. Screenshot inspection found a close control that shrank beside a long title; reserved its width and rebuilt both packages. The final native evidence is [here](evidence/linux-native-20261125.json). Screenshots include synthetic private world identifiers and remain in local private test artifacts, not committed.

## Reproduce native gate

Build the shipping Debian and the opt-in Svelte candidate from the same checkout, with identical bundled Rust sidecars; save as `shipping.deb` and `candidate.deb`. From the checkout:

```sh
python3 scripts/acceptance/native_desktop_accessibility.py \
  --packages /path/to/debs \
  --artifacts /path/to/private-output \
  --xvfb /path/to/Xvfb \
  --driver /path/to/tauri-driver \
  --native-driver /path/to/WebKitWebDriver
```

The harness creates a temporary private Xvfb display, extracts both packages, creates an isolated synthetic private world via the bundled CLI, interacts through native WebDriver keyboard input and checks the checkpoint/consent independently. It records actual package and executable hashes and per-viewport assertions. No system install or operator profile changes are required.

## Acceptance boundary

This is scoped local verification, not full accessibility certification. Native names/descriptions and live-state markup were inspected in the rendered DOM, not spoken by a real screen reader. Reduced CSS viewport tests model space at 125% and 200%; they are not OS text-size or native scaling measurements. Interrupted native responses are controlled browser fixtures, not a real interrupted Minecraft installation. System-installed cross-platform, assistive technology, broader main-action and real runtime interruption checks remain required. No field Fabric qualification is credited. #44 is deferred until the roadmap's proven Fabric baseline prerequisite is met.
