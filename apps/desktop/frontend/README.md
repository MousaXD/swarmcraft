# Svelte + TypeScript migration workspace (NOT SHIPPING)

This is the dedicated frontend migration workspace for SwarmCraft issue #87. The installed app **continues to load** `apps/desktop/src` from Tauri 2 until all parity, locked-build and installed-GUI gates pass. Do not point production at this unfinished workspace.

## What exists

- Svelte 5, TypeScript and Vite source/configuration in an isolated directory.
- Ten extracted Svelte components retain all 171 existing DOM IDs. `App.svelte` mounts them before loading the original controller for still-unported flows. Create, Join, Import, signed invite creation/leave confirmation and signed World-ID lookup use typed Rust IPC with Svelte submit/busy/error handling. Activity is a reactive Svelte store. The Diagnostics identity refresh is Svelte-owned, with genuine `node_identity` reads, stale-request protection, sanitized errors, and setup-vs-verified-identity distinction; legacy startup still handles initialization. Rich world listing, runtime, provider, safety, recovery, wider diagnostics and navigation still delegate to legacy modules. Do not mislabel this transitional architecture as complete.
- Classic PR #88 startup watchdog copied byte-for-byte to `public/bootstrap.js`, with a test preventing drift.
- A typed, non-patching native invoke wrapper for incremental controller extraction; backend remains authoritative.
- Typed native World actions, fail-closed world/safety modeling, a reactive startup/session coordinator staged for atomic cutover, and component/native-contract tests. The separate session coordinator is **not** currently instantiated alongside the legacy one.
- Feature inventory and blockers in `../SVELTE_MIGRATION.md`.

## Development and gates

Use Node 22.12+. `package-lock.json` was generated on a clean GitHub Actions Node 22.16 runner and committed after matching the exact Git blob SHA-1 with the downloaded candidate artifact. Run `npm ci` (never an unlocked install) to reproduce the dependencies. This is candidate-build evidence, not a certification of the native Tauri installer.

```sh
cd apps/desktop/frontend
npm ci                      # use the committed lockfile
npm run check
npm run test
npm run build
node --test tests/source-parity.test.mjs
```

## Isolated native candidate acceptance

Production still serves `../src`, but the candidate can be packaged **without modifying production Tauri configuration** using `frontend/tauri.candidate.conf.json`, whose only override is `build.frontendDist`. See:

- `.github/workflows/svelte-migration-validation.yml` — locked Svelte, TypeScript, Vitest and source parity.
- `.github/workflows/svelte-candidate-native-gui.yml` — actual installed Linux Debian/WebKitGTK first boot, create, restart, persistence, missing-sidecar negative control, and visual/error checks at 980×760 / 720×560.
- `.github/workflows/svelte-candidate-platform-builds.yml` — Windows NSIS and macOS arm64/Intel DMG **packaging only**. These are not proof the UI was opened on those platforms.

The new Svelte Create, Join and Import forms use exact Rust command names and a legacy **completion-only** world-list refresh bridge. Duplicate native submissions from the legacy form handlers are suppressed only while the Svelte owner flags are active. Tests and native screenshot evidence are mandatory before qualifying these new paths as stable.

A production cutover additionally needs Agent 00 approval for `src-tauri/tauri.conf.json` (`build.frontendDist`), tightly coordinated workflow changes so Vite runs before every package build, unchanged CSP/sidecars, real installed WebKitGTK acceptance, missing-sidecar negative control, and documented Windows/macOS package builds. These changes are **not** made here.

Never replace a missing action with a placeholder or claim UI completeness based on compilation. The majority of rich desktop behaviors remain delegated to old JavaScript modules. Migrate each with contract tests and installed-GUI evidence; do not replace a working action with a placeholder.

### Failure before Svelte can render

A classic candidate startup guard and minimal static fallback DOM/CSS ensure a broken or missing Vite module still produces `Startup failed` and a clickable retry, rather than a blank WebKitGTK window. PR #88's original `bootstrap.js` is copied verbatim and continues to own the failure state. Do not remove the fallback until the native deliberately-damaged installer test remains green.

Native visual acceptance writes `failure.json` with a safe stage label, coarse error category and allowlisted boolean/dimension/status observations. No raw native exception text, signed invite data, or filesystem paths may enter that diagnostic. `test_visual_failure_evidence.py` runs as a required candidate-native preflight. The installed Linux GUI must still independently prove the normal, missing-sidecar and damaged-Svelte-module cases.

### Verified identity vs device setup

The candidate's `Refresh identity` control reads the existing Rust Tauri `node_identity` command; it never generates peer IDs in JavaScript. Legacy startup and device setup can publish their actual results to the Svelte identity display without adding a second startup coordinator. An `initialize_node` response is displayed as setup progress, **not proof that a peer identity was verified**. If the native read fails, details are redacted from the renderer. The legacy production frontend remains unchanged in ownership and is preserved until Agent 00 authorizes the final cutover.


### Candidate-only Diagnostics visibility

The shipping legacy runtime wizard stylesheet hides `#navDiagnostics`.
The Svelte candidate keeps legacy styles for parity but imports
`src/candidate-navigation.css` last to expose its actual Svelte-owned
Diagnostics action. The source-parity test checks this ordering, and the
installed WebKitGTK acceptance verifies the real navigation is visible and
clickable before invoking Rust-backed `node_identity`. A successful Vite
build or the earlier legacy GUI smoke does not certify this native interaction.


The Diagnostics acceptance also requires `data-native-verified-refresh-count`
to increase after its native click. That count changes only when the real
`node_identity` call succeeds with a nonempty current-generation result;
a pre-existing identity from first boot, a failed native call, a stale reply,
or an inert button cannot satisfy the installed-GUI gate. No identity bytes
are written to evidence artifacts.


### 2026-11-20 Agent 04 native-recovery and acceptance follow-up

- The Svelte Diagnostics `node_identity` refresh now has a 15-second UI-bound read timeout and can retry after an unresolved native IPC request. A stale callback cannot overwrite newer identity state or falsely increment the verified native-read counter. The Rust command and the shipping startup controller are unchanged.
- Real Linux installed-app evidence from run `42917948627` identified a candidate test navigation blocker: the Leave World membership disclosure is nested inside collapsed **Advanced world controls**. The candidate WebDriver test now opens both real summaries in order and still requires the actual Cancel action to preserve the selected canonical world.
- Run source/TypeScript/Vitest, real installed Linux/WebKitGTK positive and negative controls, and Windows/macOS packaging on the **exact final feature-branch SHA**. There is no claim of successful native certification for this follow-up until those Actions finish.

The migration is still a hybrid compatibility candidate with delegated world-list, managed Minecraft, provider/mod, replication, and migration/recovery flows. Keep `frontendDist` on the legacy shipping UI until Agent 00 and independent installed-app QA approve complete parity.


### 2026-11-20 Native Svelte world browsing candidate (Agent 04)

- The Svelte `WorldsView` now invokes existing registered Rust `list_worlds`, `world_status`, and `world_compatibility` commands directly. Native parser validation prevents invented or duplicated worlds; failed/missing per-world metadata is presented as unknown, never permission to host.
- The list, keyboard-operable world rows, native Refresh, selection styling, initial skeleton, empty state actions and visible error/retry are Svelte-owned. The rich selected-world detail panel still delegates to the original Rust-backed `app.js` controller through a narrow candidate-only handoff. The shipping `frontendDist` is unchanged.
- At most four worlds' metadata are loaded concurrently. Individual native reads are bounded to 18 seconds and the whole refresh to 36 seconds with late-response fencing. A confirmed or uncertain network stop cannot be repainted as green Ready by the new list.
- Vitest validates native line parsing, malformed/duplicate identifiers, partial offline worlds, dangerous inherited properties, coalescing, a hung sidecar, whole-refresh timeout and no false readiness. The installed Linux Tauri/WebKitGTK test must perform a real native world read, press Refresh to increase a native-only successful-read counter, and click a Svelte row to select the same actual world. The original four native sidecars and negative bootstrap/sidecar cases remain mandatory.
- **Certification and shipping are blocked** until candidate source checks, genuine installed Linux evidence, Windows/macOS packages/interaction, full feature parity, and Agent 00 acceptance at the exact candidate SHA are independently confirmed. PR #115 does not automatically include changes to the Agent 04 source branch.


### 2026-11-21: Read-only native selection, safety and host-readiness slice

- `WorldsView.svelte` now displays the selected world's **read-only** name, canonical ID, reported authority/members, Minecraft version, checkpoint, compatibility, and source-reported safety from its current Rust-backed Svelte world-list state. Legacy `selectWorld` remains the authoritative controller for all buttons; its selected-header DOM writes are guarded in the candidate, but preserved for the original shipping frontend.
- `host-readiness-state.ts` owns only the selected world's shutdown-safety presentation. The candidate invokes the existing typed Rust `host_readiness({world})` command, bounds reads to 12 seconds, fences stale results when selection changes, coalesces requests and exposes fixed non-secret copy. Positive green "Safe to shut down" requires **both** Rust `state=safe|sleeping` and an explicit `safe_to_shutdown=true`; missing, contradictory, failed or unsupported responses never grant shutdown/hosting permission.
- Added Vitest/source-parity regressions for contradictory native readiness, stale-world results, timed-out IPC/retry, unsafe native diagnostic fields and preservation of the legacy host-readiness action path. Real installed Linux acceptance now compares Svelte selected header/safety with the actual selected Rust world row and requires a fresh native shutdown-readiness result without logging any world ID or peer identity.
- The previous 171-ID equality gate rejected three *new* Svelte diagnostics IDs while all original IDs survived. It now requires every original ID, prohibits duplication and explicitly audits the additions `refreshNativeConnectivity`, `networkServiceNotice`, `networkServiceError`. The first follow-up CI at `9569e2e60ecdc1855c1b190547560109bf5ff40a` proved source parity and Svelte typechecking **PASS**, then identified a Vitest timing/rejection-handler flaw in tests, corrected later on the branch. Latest exact-head native checks, package build and Rust audits remain unverified; no shipping switch.

- Native CI attribution: source-parity and Svelte checks passed at `9569e2e60ecdc1855c1b190547560109bf5ff40a`; the remaining Vitest regression was a **test-only microtask-order expectation** and delayed fake-timer rejection observer. The fixes now synchronize on the actual native invocation and register expected rejections before advancing timers. Re-run the latest candidate SHA to verify.
- Candidate read-only safety badges now fail closed on negated/unsupported source statuses such as `noncanonical`, `not canonical`, `canonical but unverified`, or `quorum unavailable`; no source status authorizes a host.
