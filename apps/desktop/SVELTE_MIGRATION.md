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
| First boot, device setup, identity | `app.js startup, initialize, showIdentity`; `#nodeBadge #sidebarInit #identity` | PR #88 startup watchdog; native first-boot and identity GUI smoke | DOM retained, controller delegated |
| Honest starting/ready/degraded/failed + retry | `bootstrap.js`, `#status #statusDot #retryStartup` | 60s watchdog, missing sidecar negative test, pure phase tests | DOM/watchdog retained; Svelte model staged |
| Network daemon and structured connectivity | `app.js ensureNetworkingService, refreshConnectivityDiagnostics`; `#daemonListen #connectivitySection` | Native daemon and IPC status, offline/degraded acceptance | DOM retained, controller delegated |
| List, select, refresh, empty/error worlds | `app.js refreshWorlds, selectWorld`; `#worldCards #worldCount #worldLoadError` | Native list/persist test; stale-request guards | DOM retained; typed list parser staged |
| Create canonical world / compatibility | `#createForm #createMinecraft #createLoader`; `catalog-selectors.js` | Strict Fabric adapter support; canonical world creation GUI smoke | DOM retained, controller delegated |
| Import pre-existing world and required mods | `#importForm #importSource #importServerMods`; `import-flow.js` | Rust import; preserve source/untrusted paths; adapter guards | DOM retained, controller delegated |
| Signed invite issuance and acceptance | `#inviteDialog #inviteMinutes #joinForm #joinInvite` | Signed membership remains Rust-only | DOM retained, controller delegated |
| Public discovery and world-ID resolution | `launcher-controller.js`; `#joinWorldId` and dynamic discovery controls | Discovery is not membership; fail closed on invite policy | DOM retained, controller delegated |
| Leave membership with confirmation | `#leaveWorld #leaveDialog #confirmLeave` | Rust signed leave; focus/confirmation regression | DOM retained, controller delegated |
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
| Activity, raw diagnostics, service errors | `#activityView #output #diagnosticsView` | Error details do not leak signed invite/secrets | DOM retained, controller delegated |
| Desktop sizing, focus, accessibility | `style.css`; dialogs, nav, 980x760 and 720x560 | Screenreader/keyboard/viewport screenshot review | NOT YET CERTIFIED in Svelte |
| Native Tauri bridge, CSP, four sidecars | `src-tauri/tauri.conf.json`, `provider-contract-bridge.js` | No native invoke monkey-patch; real Linux installed GUI smoke | Native configuration untouched |
| Voluntary hosting budgets (#84) | Agent 02/03 contracts not yet finalized | No simulated memory pooling or host-consent enforcement | Deferred / no fake controls |

## Backend boundary audit

Current `createBackendAdapter` wraps `initialize_node`, `node_identity`, `list_worlds`, `create_world`, `import_world`, `join_world`, `leave_world`, `create_invite`, `world_status`, `host_readiness`, `world_compatibility`, `world_conflicts`, seeding, peers, verification, export/recovery, networking daemon, runtime configuration/start/stop, connectivity, local mod readiness/supply/removal, runtime status/plan/install/repair/verify/launch, and capability-gated migration/transfer/wake. Provider and discovery modules add catalog, Modrinth, CurseForge, local artifact inspection, staging and public discovery commands. These commands and payload translations must not be renamed as part of framework migration.

## Required integration steps (all remain blockers)

1. Resolve and commit a **real** dependency lockfile, run `npm ci` with locked toolchains, record audit/license output and verify build/type-check/component tests. The pinned package manifest is not a substitute for a transitive lockfile.
2. Replace the delegated imperative controllers with Svelte components, reactive typed stores, and tested event coordination **without removing any feature**, then re-run 171-ID and command parity audits (relax the ID rule only with explicitly equivalent tested selectors). Remove duplicate markup only once no longer needed.
3. Obtain Agent 00 approval to point Tauri `build.frontendDist` to `../frontend/dist` and update every relevant cross-platform build/CI workflow to install locked frontend dependencies and build before packaging. Retain original CSP, sidecars and window bounds.
4. Update the Chrome/mocked frontend harness to test the Svelte bundle rather than its legacy `app.js` assumptions, while keeping separate native WebKitGTK GUI acceptance with **unmocked** IPC. Add break-bootstrap negative control.
5. Test first launch, identity, real create, restart, persistence, damaged install and EULA; review visual screenshots at 980x760 / 720x560, keyboard/focus/accessibility, offline operation, performance, memory and bundle bytes vs baseline. Never claim Windows/macOS GUI testing from packaging alone.
6. Verify all Required Validation and main-installer CI jobs **at the PR's exact SHA**, record screenshots/run links, and deliver for independent Agent 00 review.

**Merge verdict:** BLOCKED. The staged Svelte compatibility shell and typed source are real implementation groundwork, but are not the shipping Svelte application, the complete reactive migration, or release evidence. No issue closure or main-branch changes are authorized.

## Component decomposition progress

A concrete Svelte component split now mounts `Sidebar`, `Topbar`, `WorldsView`, `CreateWorldView`, `ImportWorldView`, `JoinWorldView`, `ActivityView`, `DiagnosticsView`, and `Dialogs` through `Workspace`/`App`. All 171 legacy DOM IDs are preserved and tested. This improves structure only; imperative event handling is still delegated until individual journeys are migrated with native verification.
