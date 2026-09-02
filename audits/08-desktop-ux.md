# Auditor 8: Desktop UX, Tauri Contracts, and Player Usability

## Audit identity

- Repository: `MousaXD/swarmcraft`
- Audit branch: `audit/desktop-ux`
- Audited baseline: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Baseline source: live `origin/main`, verified before review
- Production code changed: **none**
- Audit date: 2026-09-02

## Executive verdict

**VERDICT: FAIL**

The desktop contains strong safety semantics, a well-structured managed runtime wizard, explicit EULA handling, authoritative Minecraft/Fabric selectors for the normal Create form, safe stop/checkpoint behavior, and generally careful authority-versus-replica messaging.

However, the intended launcher enhancement module deterministically throws during installation on the audited tree. `installModsUi()` finds the nested Create submit button and then calls `createForm.insertBefore(section, submit)`. The submit button is inside `.form-actions`, not a direct child of `createForm`, so the browser raises `NotFoundError`. The exception aborts the rest of `launcher-controller` setup.

That one failure prevents the intended normal-path Modrinth/CurseForge mod picker, public discovery UI, import catalog hydration, and canonical Create submit interception from being installed. The application therefore falls back to older handlers for important player journeys even though the corresponding backend commands exist.

A second confirmed Tauri contract defect is masked by that crash: import catalog hydration calls `minecraft_versions` and `fabric_loader_versions` without required boolean arguments. Fixing the first failure without fixing this contract will expose an Import regression immediately.

Visual correctness of the audited tree is **not proven**. The available local desktop/render connector could not execute in this audit environment. The repository's existing screenshot workflow is not valid evidence for this SHA because it renders a historical redesign branch and only copies `index.html`, `style.css`, and `app.js`, omitting the current module graph.

## Method and evidence

Reviewed the exact audited SHA across:

- `apps/desktop/src/index.html`
- `apps/desktop/src/style.css`
- `apps/desktop/src/app.js`
- `apps/desktop/src/backend-adapter.js`
- `apps/desktop/src/catalog-selectors.js`
- `apps/desktop/src/import-flow.js`
- `apps/desktop/src/launcher-controller.js`
- `apps/desktop/src/player-experience.js`
- `apps/desktop/src/runtime-wizard.js`
- `apps/desktop/src/transfer-wizard.js`
- `apps/desktop/src-tauri/src/main.rs`
- `apps/desktop/src-tauri/src/catalog_commands.rs`
- `apps/desktop/src-tauri/src/canonical_world_commands.rs`
- `apps/desktop/src-tauri/src/launcher_commands.rs`
- `apps/desktop/src-tauri/src/runtime_commands.rs`
- `apps/desktop/src-tauri/src/transfer_commands.rs`
- desktop frontend tests
- `apps/desktop/src-tauri/tauri.conf.json`
- `AGENTS.md`
- `.agents/skills/swarmcraft-ui-design/SKILL.md`
- `.agents/skills/frontend-quality-gate/SKILL.md`
- `.agents/skills/desktop-app-ux/SKILL.md`
- `.agents/skills/visual-review/SKILL.md`

Observed exact-head workflow results included green repository CI, green Player journey live acceptance, green Network Soak, green Agent 1 Catalog Validation, and green Release version guard runs. These are useful evidence for covered behavior, but they do not browser-mount the current desktop module graph and therefore do not disprove the deterministic DOM installation failure below.

## Findings

### A8-001 HIGH: Launcher enhancement installation aborts on an invalid DOM `insertBefore`

**Files / functions**

- `apps/desktop/src/launcher-controller.js`
  - `installModsUi()`
  - `install()`
- `apps/desktop/src/index.html`
  - `#createForm`
  - `.form-actions`
  - `#createWorld`

**Invariant**

The normal desktop module graph must initialize without uncaught exceptions, and the player-facing provider/discovery/canonical-create enhancements must actually be attached to the existing form structure.

**Evidence**

`installModsUi()` does the following:

```js
const form = byId('createForm');
const submit = form?.querySelector('button[type="submit"]');
...
form.insertBefore(section, submit);
```

In `index.html`, the submit button is nested under:

```html
<div class="form-actions field-wide">
  ...
  <button id="createWorld" class="button button-primary" type="submit">Create world</button>
</div>
```

`Node.insertBefore(newNode, referenceNode)` requires `referenceNode` to be a direct child of the receiver. Here the receiver is `form`, but the reference node is a child of `.form-actions`. A browser therefore throws `NotFoundError`.

The error is not caught inside `install()`.

`install()` calls, in order:

1. `hideInternalInputs()`
2. `installModsUi()`
3. `installDiscoveryUi()`
4. `hydrateImportCatalogs()`
5. provider and canonical Create event wiring
6. discovery event wiring

The exception at step 2 prevents steps 3 onward.

**Player impact**

On the audited tree, the intended launcher enhancement path cannot complete installation. Consequences include:

- no normal-path Modrinth/CurseForge mod-selection UI from `launcher-controller.js`
- no injected public-world discovery browser
- no import Minecraft/Fabric catalog hydration from this module
- no `create_canonical_world` submit interception from this module
- no `discovery_search` / `discovery_resolve` handlers from this module
- the older `app.js` Create handler remains the operative fallback and calls `create_world`
- the static Join World ID fallback only explains discovery conceptually; it does not perform the intended exact discovery resolution

This is not a styling-only issue. It disconnects several capabilities that the desktop presents as part of the player launcher journey.

**Existing coverage**

`apps/desktop/tests/launcher-controller.test.mjs` imports and tests exported pure helpers such as `canonicalPackageFromDownloaded()` and `errorText()`. It does not create the real DOM or execute `installModsUi()`.

Other frontend tests primarily inspect source strings or adapter functions. They do not mount `index.html` with the complete current ES module graph and fail on uncaught browser exceptions.

**Missing test**

A browser-level desktop smoke test should load the audited frontend module graph and assert:

- zero uncaught initialization exceptions
- `#launcherMods` exists
- public discovery UI exists
- Create submission with provider selections invokes `create_canonical_world`, not the legacy fallback
- Import selectors are hydrated
- exact World ID resolution invokes `discovery_resolve`

**Recommended remediation**

Insert the Mods section relative to a direct child of the form, for example before the `.form-actions` container, or use `submit.closest('.form-actions').before(section)`. Then add a browser DOM integration test that uses the real markup.

**Confidence:** HIGH

---

### A8-002 MEDIUM: Import catalog hydration has a confirmed Tauri argument mismatch

**Files / functions**

- `apps/desktop/src/launcher-controller.js`
  - `hydrateImportCatalogs()`
- `apps/desktop/src-tauri/src/catalog_commands.rs`
  - `minecraft_versions()`
  - `fabric_loader_versions()`

**Invariant**

Frontend Tauri calls must provide every required Rust command argument using the correct camelCase key names.

**Evidence**

Import hydration calls:

```js
const catalog = await call('minecraft_versions');
```

Rust requires:

```rust
pub async fn minecraft_versions(
    include_snapshots: bool,
    refresh: bool,
)
```

with `#[tauri::command(rename_all = "camelCase")]`.

Import hydration then calls:

```js
call('fabric_loader_versions', { minecraftVersion: minecraftSelect.value })
```

Rust requires both `minecraft_version: String` and `refresh: bool`.

The normal Create catalog selector uses the correct contract:

```js
minecraft_versions({ includeSnapshots, refresh })
fabric_loader_versions({ minecraftVersion, refresh })
```

**Current masking condition**

A8-001 prevents `hydrateImportCatalogs()` from being reached today. This finding is still a confirmed contract mismatch and will become immediately player-visible if A8-001 is fixed without also correcting these payloads.

**Failure handling defect**

The outer call is:

```js
hydrateImportCatalogs(call).catch((error) => console.warn('Import catalog unavailable', error));
```

The function replaces the original editable inputs with select elements before awaiting catalog results. If the first call fails, the UI can be left with empty replacement selectors and no player-facing error.

**Existing coverage**

`apps/desktop/tests/import-flow.test.mjs` validates request construction, the `import_world` adapter, and success/failure flow. It does not execute `hydrateImportCatalogs()` against the actual catalog command contract.

**Missing test**

Mount the Import form with a fake Tauri invoke that validates required payload keys and verify successful Mojang/Fabric hydration plus visible failure/retry behavior.

**Recommended remediation**

Call:

```js
call('minecraft_versions', { includeSnapshots: false, refresh: false })
call('fabric_loader_versions', { minecraftVersion: ..., refresh: false })
```

Do not destructively replace the original controls until hydration can enter a usable loading/error state, and report provider failures in the Import form rather than only `console.warn`.

**Confidence:** HIGH

---

### A8-003 MEDIUM: Canonical Create can report failure after the world was already created

**Files / functions**

- `apps/desktop/src/launcher-controller.js`
  - Create form submit handler
- `apps/desktop/src-tauri/src/canonical_world_commands.rs`
  - `create_canonical_world()`

**Invariant**

A player should never be told “world creation failed” after durable canonical world creation has already succeeded unless the UI explicitly distinguishes world creation from local post-create setup.

**Evidence**

The launcher flow performs:

1. resolve/download selected provider artifacts
2. call `create_canonical_world`
3. for every package, call `world_mods_add`
4. only after all local JAR additions succeed, report success and reload

The Rust `create_canonical_world()` command persists world storage, descriptor, membership record, signed world config, and initial snapshot before returning its `worldId`.

If any later `world_mods_add` fails, the catch block reports an error and re-enables the Create submit button. It does not tell the player that the canonical world already exists, select that world, or offer local-mod repair. Retrying can create another world with a new identity.

**Current masking condition**

A8-001 prevents this handler from being attached on the audited tree. The defect is nevertheless present in the intended canonical launcher path.

**Player impact**

- duplicate worlds after retry
- confusing “creation failed” messaging for an already-created world
- local mod setup failure conflated with canonical world creation failure

**Recommended remediation**

Treat canonical creation and local artifact installation as separate committed phases. Once `create_canonical_world` returns, retain the world ID and transition to a repair/setup state instead of allowing a blind second creation. Prefer a backend transaction/orchestration command if atomicity across these operations is required.

**Missing test**

Force `world_mods_add` to fail after a successful `create_canonical_world`, then assert the UI reports “world created, local setup needs attention,” retains the returned world ID, and does not offer an unqualified retry that creates another identity.

**Confidence:** HIGH

---

### A8-004 LOW: Injected provider/discovery controls bypass the current desktop component classes

**Files / functions**

- `apps/desktop/src/launcher-controller.js`
  - `installModsUi()`
  - `installDiscoveryUi()`
  - dynamically rendered result rows
- `apps/desktop/src/style.css`

**Evidence**

The injected normal-path UI uses classes such as:

- `form-section`
- `muted`
- `actions`
- `secondary`
- `stack`
- `card-row`

The current shell and forms use classes such as:

- `player-section`
- `button`
- `button-secondary`
- `compact-actions`
- `field-help`
- existing tokenized layout classes

Repository code search on the audited tree does not show matching current CSS definitions for several injected classes, including `form-section` and `card-row`.

**Impact**

When A8-001 is fixed and these controls actually render, provider/discovery sections may fall outside the established button, spacing, status, and responsive component system. This matters most at the required `720x560` minimum size.

**Visual proof limitation**

I could not render the audited current tree in this environment, so I am not claiming a specific pixel defect. The source-level component mismatch is confirmed; exact visual severity remains unrendered.

**Recommended remediation**

Use the same semantic component classes already used by `index.html` or add explicit tokenized styles for the injected components, then render all required window sizes.

**Confidence:** MEDIUM-HIGH for component mismatch, MEDIUM for visual impact

---

### A8-005 LOW: Exact World ID discovery feedback is routed to the wrong status region in the enhanced path

**Files / functions**

- `apps/desktop/src/launcher-controller.js`
  - `#joinWorldIdButton` capture handler
- `apps/desktop/src/app.js`
  - `joinWorldId()`
- `apps/desktop/src/index.html`
  - `#joinWorldIdNotice`

**Evidence**

The static Join panel provides a local status region `#joinWorldIdNotice` directly beside the exact World ID workflow.

`app.js` uses that region.

The enhanced capture handler instead chooses:

```js
const status = byId('publicWorldStatus') || byId('joinError');
```

Because the enhanced installer also creates `#publicWorldStatus`, exact-resolution feedback appears in the separate Public Worlds search section. The capture handler stops immediate propagation, preventing the older local handler from updating `#joinWorldIdNotice`.

**Impact**

The action can succeed but place its result away from the control that triggered it, weakening local feedback and keyboard/screen-reader task continuity.

**Current masking condition**

A8-001 prevents the enhanced discovery handler from being installed today.

**Recommended remediation**

Route exact World ID status to `#joinWorldIdNotice`; reserve `#publicWorldStatus` for browse/search results.

**Confidence:** HIGH

---

### A8-006 LOW: Stop-world copy contradicts the actual durable safe-stop backend behavior

**Files / functions**

- `apps/desktop/src/index.html`
  - `#sleepDialog`
- `apps/desktop/src/app.js`
  - `sleepWorld()` / `stopHost()`
- `apps/desktop/src-tauri/src/runtime_commands.rs`
  - `stop_host()`

**Evidence**

The Stop world confirmation says:

> This does not itself create a durable sleeping migration state.

The Tauri `stop_host()` command actually invokes `world stop` and waits until migration status reports `phase == "sleeping"`. On success it reports that the world stopped safely after the save barrier and canonical checkpoint. If sleeping is not observed, it returns an error and explicitly does not force-kill Minecraft.

**Impact**

The implementation is safer than the dialog copy suggests, which is good operationally, but the UI fails the requested distinction between a safe checkpointed stop and an arbitrary shutdown. Users are told the action is less durable than what the backend requires.

**Recommended remediation**

Update the confirmation to say that SwarmCraft requests a save barrier, canonical checkpoint, and durable sleeping state before reporting success, while replica storage is separate.

**Confidence:** HIGH

---

### A8-007 LOW: Current visual-size correctness lacks trustworthy exact-SHA evidence

**Files**

- `.github/workflows/final-ui-screenshots.yml`
- current desktop module graph under `apps/desktop/src/`

**Evidence**

The required configuration is correct in `tauri.conf.json`:

- default `980x760`
- minimum `720x560`
- resizable

The CSS contains explicit compact-window rules and deliberate wrapping/truncation for machine IDs and status text.

However, the repository screenshot workflow named `final-ui-screenshots.yml`:

- runs only on `artifacts/pr-10-ui-audit`
- fetches `redesign/desktop-frontend-overhaul`
- copies only `index.html`, `style.css`, and `app.js`
- injects a mock Tauri bridge
- does not load current modules such as `import-flow.js`, `catalog-selectors.js`, `launcher-controller.js`, `runtime-wizard.js`, `transfer-wizard.js`, or `player-experience.js`

Therefore its screenshots do not prove layout, focus, overflow, or interaction correctness for the audited SHA.

The audit environment's local desktop/render connector was unavailable due a connector identity error, so I did not substitute source inspection for screenshot claims.

**Recommended remediation**

Add an exact-head browser/Tauri screenshot smoke test that loads the same frontend module graph shipped by the app, at minimum for:

- `980x760`
- `720x560`
- a wider desktop size
- empty state
- healthy selected world
- solo/degraded state
- conflict state
- Create with provider UI
- Join with discovery UI
- runtime setup/EULA dialog
- migration/transfer dialog

**Confidence:** HIGH

## Positive controls confirmed

The audit found several desktop controls that are materially good and should be preserved while fixing the findings.

### Authoritative Create version selection

`catalog-selectors.js` upgrades the static Create Minecraft/Fabric fields to select controls, fetches Mojang and Fabric catalogs with the correct Tauri payloads, rejects malformed responses, supports cached provider data messaging, prevents stale async responses from overriding new selections, disables Create until both exact choices are ready, and provides Retry/Refresh behavior.

The static text inputs in `index.html` are therefore not, by themselves, evidence that normal Create remains free text.

### Authority, replica, conflict, and degraded state are distinguished

`app.js` maps:

- conflict to danger and disables Play
- solo/degraded to warning
- canonical/quorum to safe
- replica-only/not-authority-eligible compatibility to a disabled Play path with explicit replica wording
- unknown compatibility to fail-closed Play disablement

Host readiness separately explains safe shutdown, syncing, runtime/mod blockers, quorum blockers, conflict, degraded safety, world-offline consequences, and not-current-host state.

### Discovery is not presented as membership or authority

Static Join copy states that discovery does not grant membership. Enhanced discovery copy, when the installer is fixed, likewise says authenticated announcements do not confer membership and that authority decides membership.

### Managed Play path is safety-oriented

`runtime-wizard.js`:

- checks backend runtime status before launch
- exposes backend-managed component states
- requires explicit EULA acceptance when needed
- installs, then verifies, before launch
- refuses launch unless backend reports ready
- disables/relabels busy actions
- supports Escape/close for the setup dialog
- restores focus to the invoking control
- keeps advanced manual runtime paths separate from the normal Play flow

The Rust `runtime_launch` path also waits for shared migration/runtime readiness rather than assuming process spawn equals success.

### Safe Stop is fail-closed

Desktop stop uses the backend world-stop path and waits for durable `sleeping`. It does not report success on a raw process kill or force-kill after timeout.

### Signed host-transfer contract is well bounded at the Tauri boundary

`transfer-wizard.js` and `transfer_commands.rs` agree on `manual_transfer_step { world, action, value }`. The backend bounds pasted transfer tokens, validates encoded shape before CLI dispatch, and keeps prepare/accept/commit/activate/observe as explicit stages. The UI filters obvious banned/non-authority-eligible candidates before selection, while the Rust protocol remains authoritative.

### Long machine data receives source-level overflow handling

The current CSS uses `overflow-wrap`, ellipsis, min-width constraints, and monospace only for machine-oriented values. This is positive source evidence, but exact window rendering remains required to prove the final geometry.

## Tauri invoke contract matrix

| Frontend call | Rust command | Contract assessment |
|---|---|---|
| `initialize_node` | `initialize_node` | MATCH |
| `node_identity` | `node_identity` | MATCH |
| `list_worlds` | `list_worlds` | MATCH |
| `minecraft_versions { includeSnapshots, refresh }` from Create selector | `minecraft_versions(include_snapshots, refresh)` camelCase | MATCH |
| `fabric_loader_versions { minecraftVersion, refresh }` from Create selector | `fabric_loader_versions(minecraft_version, refresh)` camelCase | MATCH |
| `minecraft_versions` from Import hydration | same command, both booleans required | **MISMATCH** |
| `fabric_loader_versions { minecraftVersion }` from Import hydration | same command, `refresh` required | **MISMATCH** |
| `import_world` via adapter | `import_world` camelCase fields | MATCH |
| `join_world { invite }` | `join_world(invite)` | MATCH |
| `create_invite` camelCase payload | `create_invite(world, expires_minutes, bootstrap_addrs)` camelCase | MATCH |
| world status/compatibility/conflicts/peers/verify | matching commands | MATCH |
| background seeding | `set_background_seeding(world, enabled)` | MATCH |
| runtime status/plan/install/repair/verify/launch | matching commands | MATCH |
| configure direct runtime | `configure_world_runtime` camelCase | MATCH |
| stop host | `stop_host(world)` | MATCH |
| migration status/wake | matching commands | MATCH |
| manual transfer step | `manual_transfer_step(world, action, value)` camelCase | MATCH |
| provider staging / artifact inspection | matching launcher commands | MATCH |
| Modrinth search/resolve/download | registered corresponding commands | MATCH at command/payload boundary reviewed |
| CurseForge search/resolve/download | registered corresponding commands | MATCH at command/payload boundary reviewed |
| `discovery_search { query }` | `discovery_search(query: Option<String>)` | MATCH |
| `discovery_resolve { world }` | `discovery_resolve(world)` | MATCH |
| `create_canonical_world { request }` | camelCase-deserialized request | MATCH |

## Player-journey table

| Step | Expected | Actual code path | Failure handling | Verdict |
|---|---|---|---|---|
| First launch | initialize identity/network, then list worlds | `startup()` initializes node, ensures daemon, reads identity, refreshes worlds | setup/network failures are surfaced without hiding local worlds | PASS |
| No worlds | clear task entry points for create/import/join | explicit empty state with all three actions | world-load errors have alert/status path | PASS |
| Create: Minecraft selection | authoritative Mojang choices | `catalog-selectors.js` upgrades to select and calls `minecraft_versions` correctly | visible retry/error, Create disabled | PASS |
| Create: Fabric selection | compatible Fabric choices for selected Minecraft | correct `fabric_loader_versions { minecraftVersion, refresh }` | visible retry/error, stale-request protection | PASS |
| Create: mod selection | Modrinth/CurseForge normal-path picker | intended injection in `installModsUi()` | deterministic `NotFoundError` prevents installation | **BROKEN** |
| Create: canonical world | provider-resolved exact modpack and `create_canonical_world` | intended capture submit handler in `launcher-controller.js`; older `app.js` fallback remains | intended handler never attaches because installer aborts | **BROKEN** |
| Provider error | actionable local error, no false success | intended launcher controller has structured error mapping | provider UI does not install because A8-001 | **BROKEN for intended normal path** |
| Runtime setup | managed backend-owned setup | runtime wizard status/install/verify flow | detailed failure state, retry safety, advanced fallback | PASS by source/contract |
| EULA | explicit acceptance before install/host | managed dialog checkbox and direct advanced checkbox | launch blocked until accepted | PASS |
| Play / launch | only authority-eligible non-conflicted world can host | `hostingEligibility` plus managed runtime wizard | fail-closed disablement and actionable status | PASS by source/contract |
| Stop / checkpoint | safe barrier/checkpoint/sleep before success | `stop_host` invokes world stop and waits for `sleeping` | timeout/failure does not force-kill | PASS behavior; copy needs fix |
| Invite | signed invite, no mandatory manual bootstrap | normal invite dialog with advanced addresses hidden by launcher setup before A8-001 occurs | local invite error/result UI | PASS by source/contract |
| Join by invite | membership requires signed invite | `join_world { invite }` | inline join error | PASS |
| Public discovery | browse authenticated public announcements without granting membership | intended injected `discovery_search` section | never installed because A8-001 | **BROKEN** |
| Exact World ID lookup | resolve public/unlisted exact ID without implying membership | intended `discovery_resolve` enhanced handler; static fallback is explanatory only | intended handler never installs; enhanced feedback target also wrong | **BROKEN** |
| Import | normal import flow, exact version metadata, local mod declaration | static import works through adapter; intended catalog hydration never reached | form validation is good, but authoritative hydration is absent due A8-001 and itself has bad Tauri args | PARTIAL / FAIL |
| Replica availability | storage-only role separate from hosting | seeding/verify controls and compatibility copy | Play disabled for replica-only peers | PASS |
| Migration status | clear progress/block/failure distinctions | backend adapter normalizes known phases and app renders state | stale selection guarded by generation counters | PASS by source/contract |
| Manual transfer | explicit signed staged handoff | transfer wizard + `manual_transfer_step` | stage-level errors and busy states | PASS by source/contract |
| Recovery/export | advanced, world-scoped, explicit destination | Diagnostics calls verify/export/recover commands | destination validation; backend errors go to Activity/status | PASS by source/contract |
| Incompatible world | keep replica useful, block hosting | authority eligibility mapping in `hostingEligibility` | clear reason under Play | PASS |
| Offline/provider outage | preserve local use and actionable degradation | network warning and Create catalog cached/error states | local worlds remain; Create can disable when authoritative catalog unavailable | PARTIAL, launcher provider UI itself broken |
| Failed backend | do not fake success | common `run()` logs error and status | action-specific inline paths exist for major forms | PASS with some raw-error leakage |
| Loading/duplicate submit | prevent accidental double action | `bindAction` busy flags plus runtime/transfer busy controls | relevant controls disabled and `aria-busy` used | PASS on wired handlers |
| Long IDs | no layout break | CSS ellipsis/overflow-wrap and machine-value styles | source only, not rendered on audited SHA | PARTIALLY PROVEN |
| Keyboard/focus | native controls, visible focus, dialog focus behavior | global `:focus-visible`; runtime dialog restores focus; native buttons/forms/details used | full keyboard traversal not rendered/executed | PARTIALLY PROVEN |
| `980x760` default | fully usable | Tauri config correct; CSS targets desktop shell | no exact-SHA render available | UNPROVEN visually |
| `720x560` minimum | no horizontal overflow/clipped primary actions | Tauri config and compact media rules exist | no exact-SHA render available | UNPROVEN visually |

## Safety-semantics assessment

| Distinction | Assessment |
|---|---|
| Authority vs replica | Clear. Host eligibility is separate from ability to keep a replica. |
| Public vs unlisted vs private | Create exposes all three. Join copy distinguishes discovery from membership. Public browse implementation is currently unreachable due A8-001. |
| Canonical vs solo/degraded vs conflict | Clear labels and different semantic treatments. Conflict blocks Play; solo/degraded is warning, not safe. |
| Membership vs discovery | Clear in static and enhanced copy. Discovery does not claim membership. |
| Compatible vs storage-only peer | Clear. Replica-only/not-authority-eligible disables Play with explicit reason. |
| Safe checkpoint vs arbitrary shutdown | Backend is fail-closed and safe; Stop-world dialog text is stale and understates the durable sleeping behavior. |
| Relay vs authority | Connectivity copy explicitly says relay transport does not make a peer canonical host. |

## Accessibility and desktop usability

### Positive source evidence

- native `button`, `input`, `select`, `textarea`, `details`, and `dialog` are used extensively
- visible labels are associated with major form controls
- `role="alert"`, `role="status"`, and `aria-live` regions exist for important asynchronous states
- focus-visible has a strong explicit ring
- long identifiers are intentionally wrapped/truncated
- loading world list uses `aria-busy`
- async action wrappers set `aria-busy` and disable controls
- runtime dialog intercepts Escape safely and restores prior focus
- destructive membership leave has a world-specific confirmation
- host Stop has a confirmation dialog

### Not fully proven

Without rendering and keyboard execution on the exact SHA, I cannot certify:

- complete Tab order after `player-experience.js` moves DOM nodes
- focus restoration for every dialog/workflow
- minimum-window reachability of every primary action
- absence of horizontal overflow once all dynamic modules are installed
- actual contrast of all muted/helper text on target platforms
- visual hierarchy of injected provider/discovery controls

## Required remediation order

1. **Fix A8-001 first.** Add a full browser/module initialization smoke test so the launcher cannot silently fall back again.
2. Fix A8-002 in the same change, because the Import Tauri mismatch is currently masked by A8-001.
3. Add end-to-end frontend contract assertions that Create uses `create_canonical_world`, public browse uses `discovery_search`, exact lookup uses `discovery_resolve`, and the provider UI is present.
4. Fix A8-003 so a post-create local mod failure cannot be presented as total world-creation failure.
5. Bring injected provider/discovery controls into the current component system and route exact lookup feedback locally.
6. Correct Stop-world copy to match the safe sleeping/checkpoint contract.
7. Render the exact post-fix module graph at `980x760`, `720x560`, and a wider size across empty/healthy/degraded/conflict/create/join/runtime/migration states.

## Re-audit requirements

Auditor 8 should be rerun after fixes with all of the following evidence:

- browser/Tauri initialization with no uncaught exceptions
- provider Mods UI visible and operable
- canonical Create command observed from the normal Create flow
- correct Import catalog Tauri payloads and usable provider-error state
- public discovery and exact resolution observed through their actual commands
- forced post-create local-mod failure showing a repairable partial-success state rather than a second-create path
- keyboard pass for Create, Join, runtime EULA dialog, Invite, Transfer, Stop, and Leave
- screenshots at `980x760`, `720x560`, and one wider size on the exact audited fix SHA
- no horizontal page overflow, clipped primary actions, or inaccessible dialogs

## Final verdict

The backend-facing desktop has several good safety controls, but the actual player launcher enhancement layer does not initialize correctly on the audited `main` SHA. The affected features are central to Create-with-mods and discovery, so this is not acceptable as a passing desktop/player UX baseline.

**VERDICT: FAIL**
