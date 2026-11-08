# Issue #87 — SwarmCraft Svelte/TypeScript migration and feature-parity audit

**Baseline:** `main` at `45174310d1433116e64b9b6978cb4f2aa1c22c15`, including merged PR #88. **Branch:** `agent-04/svelte-typescript-desktop`. **Status:** IN PROGRESS / NOT READY TO MERGE.

## Scoped policy exception

Issue #87 and the owner's Agent 04 instructions expressly authorize **Svelte 5 + TypeScript + Vite** for the Desktop, superseding only `AGENTS.md`'s ordinary rule against introducing a framework or build pipeline for *cosmetic restyling*. All security, IPC, protocol, runtime, accessibility and native acceptance guidance remains in force. This document records the exception without editing shared `AGENTS.md` (Agent 00 owns shared guidance).

## Baseline inventory

The shipping frontend contains one 33 KB HTML document with **171 unique IDs**, six primary views, three `dialog` confirmations/workflows, around 49 KB of controller code in `app.js`, and separate modules for backend adaptation, provider discovery/mod management, catalog selection, import, runtime wizard, transfer wizard, launcher and player experience. Tauri points at `../src`, uses global Tauri core bridge, restricts CSP to local assets/IPC, and packages four Rust sidecars. No cloud-hosted renderer is required.

The original DOM is retained in ten extracted Svelte components under `frontend/src/components/` with a source-parity regression check; old controllers are loaded **after** Svelte mounting to preserve their element bindings. This proves DOM coverage only — it does **not** certify independently ported Svelte logic, reactive stores, production packaging, keyboard usability, real startup or feature parity.

## Feature / acceptance traceability

| Existing player capability | Original source / critical anchors | Contract and regression gate | Migration status |
| --- | --- | --- | --- |
| First boot, device setup, identity | `app.js startup, initialize, showIdentity`; `#nodeBadge #sidebarInit #identity` | PR #88 startup watchdog; native first-boot and identity GUI smoke | Native Svelte identity refresh and reactive display; first-boot/device setup still legacy-controlled |
| Honest starting/ready/degraded/failed + retry | `bootstrap.js`, `#status #statusDot #retryStartup` | 60s watchdog, missing sidecar negative test, pure phase tests | DOM/watchdog retained; Svelte model staged |
| Network daemon and structured connectivity | `app.js ensureNetworkingService, refreshConnectivityDiagnostics`; `#daemonListen #connectivitySection` | Native daemon and IPC status, offline/degraded acceptance | DOM retained, controller delegated |
| List, select, refresh, empty/error worlds | `app.js refreshWorlds, selectWorld`; `#worldCards #worldCount #worldLoadError` | Native list/persist test; stale-request guards | DOM retained; typed list parser staged |
| Create canonical world / compatibility | `#createForm #createMinecraft #createLoader`; `catalog-selectors.js` | Strict Fabric adapter support; canonical world creation GUI smoke | Native Svelte form and typed IPC; legacy world-list/catalog handoff; exact-head native recertification pending |
| Import pre-existing world and required mods | `#importForm #importSource #importServerMods`; `import-flow.js` | Rust import; preserve source/untrusted paths; adapter guards | Native Svelte validation/submit; legacy rich list; full real-world import E2E pending |
| Signed invite issuance and acceptance | `#inviteDialog #inviteMinutes #joinForm #joinInvite` | Signed membership remains Rust-only | Svelte native Join and signed-invite issuance are implemented; successful remote signed acceptance still requires E2E |
| Public discovery and world-ID resolution | `launcher-controller.js`; `#joinWorldId` and dynamic discovery controls | Discovery is not membership; fail closed on invite policy | Svelte World-ID signed discovery resolves through Rust; broader provider/list flows remain delegated |
| Leave membership with confirmation | `#leaveWorld #leaveDialog #confirmLeave` | Rust signed leave; focus/confirmation regression | Svelte confirm/Cancel and native leave request implemented; successful multi-peer reconciliation needs E2E |
| Play and runtime eligibility | `#playWorld #host #stopHost`, `backend-adapter.js` | Rust authority/compatibility and authenticated Fabric readiness | DOM retained, controller delegated |
| Managed Java, Minecraft, Fabric, EULA | `runtime-wizard.js`; `#runtimeSection #eula` | Explicit EULA acceptance; runtime install/repair/verify | DOM retained, controller delegated |
| Canonical and local server mods | `#modsPanel #modJarPath #supplyRequiredMod` | Signed modpack remains backend-only; JAR validation | DOM retained, controller delegated |
| Modrinth/CurseForge search and download | `launcher-controller.js`, `provider-contract-bridge.js` | Exact provider args, staging session, integrity hashes | DOM retained, controller delegated |
| Background replica seeding & snapshots | `#seedOn #seedOff #diagnosticVerify` | Storage-only != authority eligible; Rust verification | DOM retained, controller delegated |
| Safety / quorum / conflict feedback | `#safetyPanel #selectedSafety #worldConflicts` | canonical/solo/conflicted distinct; never fake quorum | DOM retained, controller delegated |
| Host readiness / shutdown safety | `#hostReadinessPanel #hostReadinessDetail` | Rust `host_readiness`; two-voter quorum blocked | DOM retained, controller delegated |
| Signed staged host transfer | `transfer-wizard.js`; `#transferHost` | prepare/accept/commit/activate/observe with Rust checks | DOM retained, controller delegated |
| Migration, wake, sleep / migration state | `#migrationCard #wakeWorld #sleepDialog` | Capability-probed; wake fails closed without quorum support | DOM retained, controller delegated |
| Export, recovery, checked snapshot paths | `#recoverySection #exportWorld #recoverWorld` | Rust recovery verifies snapshot and destination | DOM retained, controller delegated |
| Activity, raw diagnostics, service errors | `#activityView #output #diagnosticsView` | Error details do not leak signed invite/secrets | Reactive Svelte Activity store live with fallback; diagnostics and service outputs still legacy |
| Desktop sizing, focus, accessibility | `style.css`; dialogs, nav, 980x760 and 720x560 | Screenreader/keyboard/viewport screenshot review | NOT YET CERTIFIED in Svelte |
| Native Tauri bridge, CSP, four sidecars | `src-tauri/tauri.conf.json`, `provider-contract-bridge.js` | No native invoke monkey-patch; real Linux installed GUI smoke | Native configuration untouched |
| Voluntary hosting budgets (#84) | Agent 02/03 contracts not yet finalized | No simulated memory pooling or host-consent enforcement | Deferred / no fake controls |

## Backend boundary audit

Current `createBackendAdapter` wraps `initialize_node`, `node_identity`, `list_worlds`, `create_world`, `import_world`, `join_world`, `leave_world`, `create_invite`, `world_status`, `host_readiness`, `world_compatibility`, `world_conflicts`, seeding, peers, verification, export/recovery, networking daemon, runtime configuration/start/stop, connectivity, local mod readiness/supply/removal, runtime status/plan/install/repair/verify/launch, and capability-gated migration/transfer/wake. Provider and discovery modules add catalog, Modrinth, CurseForge, local artifact inspection, staging and public discovery commands. These commands and payload translations must not be renamed as part of framework migration.

## Required integration steps (all remain blockers)

1. **Done (candidate only):** A real transitive `package-lock.json` generated by GitHub Actions is committed and can be installed with `npm ci`. Node 22.16 CI passed Svelte check, Vitest, source-parity and Vite build at `828915811d1b2919483ede70c4ecbf284d464677`; repeat at the final exact PR head, review npm audit/license results, and include native packaging before merge.
2. Replace the delegated imperative controllers with Svelte components, reactive typed stores, and tested event coordination **without removing any feature**, then re-run 171-ID and command parity audits (relax the ID rule only with explicitly equivalent tested selectors). Remove duplicate markup only once no longer needed.
3. Obtain Agent 00 approval to point Tauri `build.frontendDist` to `../frontend/dist` and update every relevant cross-platform build/CI workflow to install locked frontend dependencies and build before packaging. Retain original CSP, sidecars and window bounds.
4. Update the Chrome/mocked frontend harness to test the Svelte bundle rather than its legacy `app.js` assumptions, while keeping separate native WebKitGTK GUI acceptance with **unmocked** IPC. Add break-bootstrap negative control.
5. Test first launch, identity, real create, restart, persistence, damaged install and EULA; review visual screenshots at 980x760 / 720x560, keyboard/focus/accessibility, offline operation, performance, memory and bundle bytes vs baseline. Never claim Windows/macOS GUI testing from packaging alone.
6. Verify all Required Validation and main-installer CI jobs **at the PR's exact SHA**, record screenshots/run links, and deliver for independent Agent 00 review.

**Merge verdict:** BLOCKED. The staged Svelte compatibility shell and typed source are real implementation groundwork, but are not the shipping Svelte application, the complete reactive migration, or release evidence. No issue closure or main-branch changes are authorized.

## Component decomposition progress

A concrete Svelte component split now mounts `Sidebar`, `Topbar`, `WorldsView`, `CreateWorldView`, `ImportWorldView`, `JoinWorldView`, `ActivityView`, `DiagnosticsView`, and `Dialogs` through `Workspace`/`App`. All 171 legacy DOM IDs are preserved and tested. This improves structure only; imperative event handling is still delegated until individual journeys are migrated with native verification.

## Reactive session extraction

`frontend/src/lib/desktop-session.ts` now implements a Svelte-readable, Rust-backed session model for first launch, identity, network degradation, local world listing, selection, retries and stale-response quarantine. Its injected backend interface corresponds to existing adapter functions, rather than inventing new Rust authority. Six Vitest regressions cover ready/failed/degraded, retries and out-of-order responses. **This is not yet wired into the live UI:** simultaneous legacy and new coordinators would duplicate device initialization; cutover must be atomic with controller removal and packaged validation.

## Frontend dependency reproducibility

The dependency-lock artifact from GitHub Actions run `42468369672` was downloaded independently and verified as a 66,118-byte `package-lock.json`, lockfile version 3, 133 package entries. Imported exact blob SHA `4151e059b92108873ff8bb31ed6de6cc4d281f8e` into GitHub without rewriting its bytes. The migration-only workflow now runs `npm ci` directly. Its success does **not** certify the production Tauri application, which still bundles the original `apps/desktop/src`.

## Typed world actions

`frontend/src/lib/world-actions.ts` now stages typed create/import/join/invite actions using the existing Rust IPC request shapes and frontend validation. It preserves explicit third-party mod declarations, carries no EULA flag for imports, rejects missing canonical IDs instead of inventing success, prevents concurrent duplicate actions, and leaves signed membership, authority and compatibility decisions to Rust. Seven Vitest cases cover payload parity, errors and duplicate attempts. Actual Svelte form binding and installed-GUI acceptance are pending.

## Native candidate certification (separate from shipping)

A dedicated candidate-only workflow `.github/workflows/svelte-candidate-native-gui.yml` now builds the Svelte `dist` using the committed npm lockfile and packages an actual Tauri Linux Debian installer with the ephemeral `frontend/tauri.candidate.conf.json` overlay. That overlay changes **only** `build.frontendDist`; the production `src-tauri/tauri.conf.json`, four Rust sidecars, CSP and release gates remain untouched. The workflow reuses #83's unmocked installed WebKitGTK/WebDriver first-launch/create/restart/persistence acceptance and a missing-sidecar negative control. A passing check is required before proposing a production cutover. Do not represent a queued/unrun workflow as passing evidence.

## World details and authority display safety

The reactive model now requests each world's actual `world_status` and `world_compatibility` through bounded batches of at most four worlds. Failed detail calls do not erase the local world list or invent a compatible host. Presentation-only `playAvailability` prevents Play for conflicting, replica-only, unknown and partially unavailable states; Rust remains the final admission authority. Vitest covers failed/unknown compatibility, solo vs canonical distinctions, conflicts and prototype-pollution-resistant status parsing. This module remains staged until the full Svelte controller cutover.

## Live Activity UI migration (hybrid handoff)

`ActivityView.svelte` now owns a reactive `activityText` store and native keyboard-readable read-only console. The original `app.js` and `runtime-wizard.js` log producers forward formatted activity entries to the Svelte activity bridge **only when mounted by the Svelte shell**; otherwise their existing direct-DOM fallback remains unchanged. Clear activity is handled by Svelte without double firing the legacy listener. Separate source and Vitest regressions protect this coexistence path. This completes the Activity presentation slice only, not the other legacy controller migrations.

The staged Play hint now also fails closed if Rust safety is unknown even when compatibility appears `Compatible`. The migration-only native CI uses non-canceling concurrency so a useful in-progress installed-app run is not discarded by another documentation or test commit.

## First native screenshot-driven visual review

The installed Svelte candidate passed #83's Linux/WebKitGTK first-launch, world creation, process relaunch and persistence test, plus a negative control that physically removed the installed CLI sidecar and proved visible failed startup with Retry. Evidence: [workflow 42470386080](https://github.com/MousaXD/swarmcraft/actions/runs/42470386080), commit `2b6d08640fd531f97ddd06f363e0013badd2aa3f`, [artifact 13613582960](https://github.com/MousaXD/swarmcraft/actions/runs/42470386080/artifacts/13613582960). Screenshots were inspected, not merely generated. The world-details screenshot revealed excessive title wrapping and horizontal overflow from a long host-readiness path at the 980x760 default window. A scoped CSS fix now allows flex wrapping and wrapping of long diagnostics. A new native visual acceptance script tests 980x760 and 720x560 on a persisted real world, requiring visible Play/Invite controls and no horizontal content scroll. **The CSS fix and new viewport test need fresh exact-head CI evidence.**

## Native-backed Svelte Create World journey

`CreateWorldView.svelte` now owns Create World submit/busy/error state and sends the real `create_world` Rust command through `native-world-actions.ts`. The original controller skips duplicate `createForm` submissions only when Svelte explicitly claims ownership; its legacy production behavior is preserved otherwise. A narrowly scoped post-create callback transfers the canonical ID to the existing rich world-list controller and refreshes/opens Worlds. The Svelte form preserves the existing catalog selector's chosen Minecraft/Fabric versions and all DOM IDs; the world is never locally simulated. The real installed GUI acceptance already exercises Create World, so that test must pass again at the exact new SHA before this cutover is certified.

## Native-backed Svelte Join World journey

The signed-invite join form now uses a Svelte event handler, explicit busy/validation/error states and the native `join_world` command. The Rust backend alone verifies invite authenticity and membership; the renderer does not store or log signed invites beyond the user-supplied textarea. A legacy compatibility hook refreshes the world list and selects the returned canonical ID without granting authority. `app.js` skips its join handler only when the mounted Svelte candidate owns the form, preventing a duplicate native join; production legacy HTML retains the unchanged fallback. A successful backend call with missing ID triggers a real refresh, never a fabricated world. Native installed-GUI tests must cover the empty-invite path and no-double-submit regression before this is certified.

## Native-backed Svelte Import World journey

`ImportWorldView.svelte` now has reactive busy/error state and a typed call to the exact native `import_world` contract through `world-actions.ts`. Its validation requires the existing local folder, game version, Fabric Loader and explicit third-party mod declaration (JAR paths or positive no-mod confirmation). No EULA flag is constructed; Rust remains responsible for file, JAR, snapshot and profile verification. The original controller conditionally skips duplicate submission, and a narrow compatibility hook refreshes the real world list after a canonical import without replacing any CLI data. The native candidate must pass initial/error interaction QA before the import journey is considered certified.

## Cross-platform candidate packaging

Dedicated, additive workflow `.github/workflows/svelte-candidate-platform-builds.yml` now constructs candidate Svelte Windows NSIS and macOS Intel/Apple Silicon application/DMG packages using locked Node 22, reproducible Rust sidecars, a bounded NSIS tool-download retry, and the isolated frontendDist overlay. No native release configuration or shared installer workflow changes are made. **Packaging success is not GUI launch evidence.** Native Windows/macOS usability still needs testing on actual installed machines. Main release gates also remain blocked by intermittent upstream HTTP 503 artifact downloads (Tauri linuxdeploy/AppRun and official managed Java); Agent 08/Agent 00 own infrastructure remediation. Maintain clear separate failure attribution.

## Native deliberately damaged frontend bootstrap negative control

The candidate-only Linux workflow additionally builds a **second, separately extracted real Tauri Debian installer** whose compiled Svelte module entry is deliberately replaced with a non-existent asset while preserving PR #88's classic startup watchdog. It launches that installed package under WebKitGTK with four valid Rust sidecars and a clean data profile, and requires the normal #83 acceptance to fail with an observable `Startup failed` banner and visible Retry button. This cannot be satisfied by a fake browser IPC bridge, missing CLI sidecar, or an unlaunched bundle. Packaging the damaged module is only test instrumentation: the shipping Tauri config and normal candidate production bundle are never changed. **This additional negative control remains unverified until the exact-head native workflow succeeds.**

## Actual defect from compiled Svelte bootstrap corruption

At `3906b8b7ec04bd0a2de305a622003d7032493cab`, [native GUI run 42485876003](https://github.com/MousaXD/swarmcraft/actions/runs/42485876003) passed real boot/create/restart, both responsive window dimensions, and the missing-installed-sidecar negative control. The **deliberately broken Svelte module** failed the required observable startup failure: Svelte never mounted, so no `#status` DOM existed, and the PR #88 classic watchdog's old `app.js` module error selector did not match Vite's hashed entry. The new `public/candidate-startup-guard.js` is a separate, classic pre-Svelte companion that observes the original watchdog's `failed` transition and handles the Vite module error; `index.html` now includes a static, accessible `#bootstrapFallback` and standalone CSS, removed only after successful Svelte mount. The **legacy PR #88 `bootstrap.js` remains byte-for-byte unchanged**. The broken compiled-module native acceptance must be rerun and explicitly pass before claiming this fix is certified.

## Native-backed Svelte signed invite creation

`Dialogs.svelte` now owns invite creation with reactive busy/error/result state, Rust-backed `create_invite`, safe reset on every dialog opening, no duplicate legacy native invocation and no token content written to Activity logs. Signed invitations remain Rust-authoritative. An actual native WebKitGTK test requests a signed invite for the test-created world and verifies **only boolean presence**, intentionally not saving the token in artifacts/screenshots; it closes the dialog before capturing subsequent screenshots. This must pass at the exact head before certification. Signed invite acceptance and remote membership remain separately unverified in multi-machine E2E.

## Svelte World-ID discovery guidance

`JoinWorldView.svelte` now owns the existing informational World-ID Check action with reactive validation and focus. It explicitly explains that a World ID does **not** grant membership and does not call Rust `join_world`, matching the prior frontend behavior. The duplicate legacy click handler is disabled only when Svelte owns the form, with a native GUI check confirming the signed invite input remains empty. Full public discovery/resolve provider flows are still delegated to the legacy launcher module.

## Real discovery regression caught during Native CI #42488163744 follow-up

The actual installed Tauri package passed first launch, creating a world, persistence, 980×760 and 720×560 layout checks, and empty signed-invite validation. Its visual test then attempted to send keystrokes into the collapsed “Have only a World ID?” disclosure, causing an element-not-interactable WebDriver failure. This was an **incorrect native test sequence**, not a failed world creation. The test now expands the disclosure through its actual summary before typing.

Reviewing that failure uncovered a **real feature regression**: the initial Svelte World ID handler only displayed a static hint, while the legacy launcher had a real signed `discovery_resolve` call registered at capture phase. The Svelte handler now invokes typed Rust `discovery_resolve`, explicitly distinguishes verified public/unlisted announcements from missing, malformed and failed responses, never equates discovery with membership, and preserves a real legacy capture fallback only in the shipping frontend. Both handlers cannot independently invoke Rust when Svelte owns the form. Five Vitest tests and source-parity assertions enforce this contract, while native WebKitGTK acceptance must perform the actual disclosure interaction and verify signed membership is not synthesized. The correction is **not yet certified** at the final exact head.

## Svelte signed Leave World confirmation

The leave confirmation dialog now has reactive busy/error state and explicit Cancel vs. confirmed Leave behavior. Only the confirmation calls typed Rust `leave_world`; the renderer never fabricates membership revocation. The original `app.js` confirmation handler is suppressed only while Svelte owns the dialog, with a completion-only callback clearing the UI selection after a successful Rust result and then refreshing the actual world list. Cancel remains the browser-native dialog action and is checked in a real installed WebKitGTK GUI without removing the test world. Error messages advise refreshing rather than blindly retrying a possibly accepted signed request. Full positive multi-peer signed-leave E2E is still pending; do not claim complete certification.

## Exact-head candidate submission regression — 2026-11-08

At commit `e2f620b8bab3171e4d6bb997c5fef59a0527dc00`, Svelte source validation [42492966676](https://github.com/MousaXD/swarmcraft/actions/runs/42492966676) and native Debian candidate [42492966742](https://github.com/MousaXD/swarmcraft/actions/runs/42492966742) failed **before native GUI acceptance** on a missing signed-invite form `submit` guard. Windows and macOS ARM64 candidate packaging failed the same source-parity check; those failures do not prove the installed packages were defective. The Svelte button-side `busy` check existed, but a DOM form's implicit/programmatic submission could bypass disabled-button protection and close the dialog during a pending Rust call.

On this same migration branch, commit `8678d67dd8a128f987d4854e38fc0d4081a6776c` wires `onsubmit={preventInviteSubmitWhilePending}` to the signed-invite form with `if (busy) event.preventDefault();`. Commit `6a96237834f8a1ea93cc5f92ba13b14a918afe64` strengthens the parity assertion to require the handler **actually be attached to the form**, not merely exist as inert source. The Create action, native Rust `create_invite` validation, fail-closed error handling, activity redaction, and modal cancellation prevention remain unchanged. The exact-head CI, installed Linux positive/negative native controls, and multi-OS packages must be checked after these commits; none is claimed passed merely because the source-level defect was repaired. Production `frontendDist` remains the original legacy UI pending Agent 00/07 approval.

## Candidate Diagnostics identity slice — native IPC, stale protection and safe display

The Svelte candidate now owns the user-triggered **Refresh identity** action through the typed Rust `node_identity` command. Native initialization and first-boot orchestration remain in the established legacy controller, and its genuine startup/setup identity outputs are bridged into Svelte's reactive presentation only when mounted. The shipping legacy fallback remains available and unchanged in ownership. The Svelte identity controller coalesces duplicate refreshes, ignores stale native completions after a newer legacy update, gives a sanitized and actionable error without printing raw native error text, and does not infer an initialized device from an empty result. This is one incremental, independently testable slice; it does not constitute full startup-controller migration.

Acceptance: the source-parity test checks actual Svelte button event wiring, legacy duplicate-handler suppression and typed native invocation; Vitest exercises success, failures with secret-like messages, duplicate requests, stale completions and retry. All checks and installed-WebKitGTK acceptance need **exact-head CI** before this slice can be called certified. No production Tauri configuration, CSP or backend changes accompany this slice.
