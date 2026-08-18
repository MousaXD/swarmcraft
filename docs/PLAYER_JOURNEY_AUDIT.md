# SwarmCraft Post-Integration Player Journey Audit

**Audited integration SHA:** `UNAVAILABLE` — GitHub does not currently contain the requested `integration/runtime-player-journey` ref, so no exact SHA can be resolved.  
**Audit date:** 2026-08-18  
**Desktop build/version:** `0.3.0` on the available component branches; the exact integration build cannot be identified or launched because the requested ref is absent.  
**Platforms tested:** No platform could be executed against the exact integration ref. Available branch/CI evidence was reviewed for Ubuntu, Windows, macOS ARM64 and macOS x86_64. The Runtime Installer branch packages Desktop successfully on all four targets, but its Windows Rust test job is red.  
**Overall release recommendation:** **NOT READY**

This is an independent player-journey audit. It does not inherit the previous classifications and it does not treat a backend or feature-agent claim as proof that the Desktop journey works.

A critical audit constraint must be explicit: the requested release-candidate branch does not exist in the GitHub repository at audit time. Repeated branch resolution returned no `integration/runtime-player-journey` ref, and a ref-specific file lookup returned `No commit found for the ref integration/runtime-player-journey`. I did **not** substitute `main` as the audited branch.

Because the exact integrated tree is unavailable, this document does two things:

1. records the release-candidate/ref failure as a P0 release gate; and
2. audits the currently available feature heads and their cross-branch contracts to identify defects that an integration branch must demonstrably close before it can be called ready.

The available feature evidence includes:

- Runtime Installer: `agent/runtime-installer`, head `5eb1310bd3d32628a20dd8c6c1445af324adabd8`;
- Runtime Wizard: `agent/runtime-wizard-ui`;
- Server Mods: `agent/server-mod-management`, head `8e63e436fc29071d260277420b364f092cdacc91`;
- Host Readiness: `agent/host-readiness`, head `d0a82c57a6908f85e4819a6aa66a99ed54d4a0ef`;
- Runtime Setup Hardening: `agent/runtime-setup-hardening`, head `4873a4ffe06aea493e7139ac35a942a8cf14fb43`.

## Classification

- **GREEN** — works and is obvious.
- **YELLOW** — works in a meaningful path, but is confusing, incomplete, weakly integrated, or insufficiently proven.
- **RED** — cannot be completed as the normal Desktop journey.
- **FALSE GREEN** — Desktop claims or strongly implies behavior that the inspected backend path does not guarantee.

Severity:

- **P0** — release blocker.
- **P1** — major.
- **P2** — moderate.
- **P3** — polish.

## Release-gate findings

| Finding | Status | Severity | Evidence | Release impact |
| --- | --- | --- | --- | --- |
| Requested integration candidate is not present | **RED** | **P0** | `integration/runtime-player-journey` resolves to no GitHub branch and cannot be used as a file ref. | There is no auditable integration SHA or exact build. Release certification cannot be issued. |
| Stop world still maps to process kill in the available composition while Desktop says “stopped gracefully” | **FALSE GREEN** | **P0** | `apps/desktop/src/app.js::sleepWorld()` calls `backend.stopHost()` and reports “World runtime stopped gracefully.” `apps/desktop/src-tauri/src/runtime.rs::stop()` calls `CommandChild.kill()`. None of the inspected Runtime Installer, Wizard, Mods, Host Readiness, or Hardening changes replace this lifecycle. | Safety claim outruns implementation. Final checkpoint/sleep durability is not proven before success is shown. |
| Runtime Wizard and Runtime Installer JSON contracts do not match on their feature heads | **RED** | **P0** | Installer `RuntimeStatus.components` is an array of `{kind,state,...}`, while Wizard normalization expects a keyed component object. Installer status has `eula_accepted` plus an EULA component, while Wizard expects `eula_required`. Installer `install` returns `{status, completed_phases, launch_config_saved}`, while Wizard normalizes that wrapper as though it were a status. Wizard contract tests use a synthetic schema that the installer does not emit. | Clean-machine automatic setup cannot be considered integrated. EULA can fail to appear and successful install can be interpreted as not-ready. |
| Runtime sidecar/Tauri launch seam is not present in the available feature heads | **RED** | **P0** | Runtime Installer adds `swarmcraft-runtime status|plan|install|repair|verify`; Runtime Wizard expects Tauri `runtime_status`, `runtime_plan`, `runtime_install`, `runtime_repair`, `runtime_verify`, and `runtime_launch`. Installer branch does not modify Desktop/Tauri; Wizard branch does not add the Tauri commands or sidecar packaging. | Normal Play cannot be proven end-to-end. |
| Host-readiness backend exists, but the broad player-facing shutdown surface and producer hooks are not integrated | **RED** | **P0** | Host Readiness adds a backend contract and `backend.hostReadiness(world)` seam, but its branch does not wire the report into the primary selected-world UI. Runtime Installer does not call `record_runtime_verified`; Server Mods does not call `record_server_mod_readiness`. | “Can I shut down?” remains an incomplete product journey even though the safety calculator is strong. |
| Runtime Installer Windows test is failing | **RED** | **P1** | CI run `32084112192`, Windows Rust job `95553000177`: `runtime_installer::tests::atomic_local_install_is_retry_safe` fails because the Windows hashing path invokes `Get-FileHash` and the command is unavailable in the runner environment. | Cross-platform clean-machine setup is not green on Windows. |

## Clean-machine journey

Desired journey:

```text
Install / launch SwarmCraft
        ↓
Create world
        ↓
Play
        ↓
Automatic setup
        ↓
EULA
        ↓
Minecraft starts
```

### Manual prerequisites observed

The Runtime Installer backend is designed to eliminate manual Java, Minecraft server JAR, Fabric server JAR, Fabric API, and SwarmCraft Fabric JAR selection. That is the correct direction. However, the available Desktop integration does not establish the necessary Tauri/sidecar contract, and the Wizard/Installer schemas disagree.

Therefore the desired “zero manual runtime downloads/paths” journey is **not yet proven from Desktop**. The existing Advanced/manual runtime controls remain the only demonstrated fallback path.

| Journey | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| First launch / device setup | **YELLOW** | P2 | Current Desktop is launcher-shaped and auto-initializes networking, but exact integration build cannot be retested. | Launch into Worlds/Create/Join without daemon or identity concepts. | Existing Desktop startup plus no feature-head evidence of intentional regression. Exact candidate unavailable. | Desktop | Re-run on the real integration build and keep technical node setup out of the normal path. |
| Create world | **YELLOW** | P2 | Current Create form is player-oriented, with compatibility settings behind details. Server-mod branch adds CLI `--server-mod` but no Desktop creation integration. | Create a normal world without runtime/JAR knowledge; modded creation should offer a clear Mods step when needed. | `apps/desktop/src/index.html`; `agent/server-mod-management` changes CLI/core only. | Desktop + Server Mods | Integrate mod requirements into Create without exposing hashes or Fabric internals. |
| Create → Play on a clean machine | **RED** | **P0** | Automatic setup pieces exist separately but do not share a proven Desktop contract. | Press Play, install/repair everything automatically, ask EULA once, verify, launch. | Wizard/Installer schema mismatch and missing Tauri sidecar seam. | Runtime Installer + Runtime Wizard + Desktop/Tauri | Add one integration adapter against the actual installer JSON, package the sidecar, and exercise a clean-machine acceptance test. |
| No manual Java download | **YELLOW** | P1 | Installer can reuse compatible system Java or resolve Adoptium Java, but the Desktop route is not integrated and Windows installer tests are red. | Java is invisible unless setup fails. | `runtime_installer.rs::resolve_managed_java`; Windows CI failure. | Runtime Installer | Make hashing/extraction self-contained and cross-platform, then prove through Desktop. |
| No manual Minecraft server JAR | **YELLOW** | P1 | Installer resolves Mojang server artifacts, but Desktop cannot currently prove invocation of the installer sidecar. | Server artifact managed automatically. | `RuntimeInstaller::install_inner`; no Desktop/Tauri changes on installer head. | Runtime Installer + Desktop/Tauri | Wire and package installer. |
| No manual Fabric server JAR | **YELLOW** | P1 | Installer resolves Fabric launcher automatically; Desktop integration remains unproven. | Fabric launcher managed automatically. | `resolve_fabric_installer` and Fabric server resolution. | Runtime Installer + Desktop/Tauri | Wire and acceptance-test. |
| No manual Fabric API download | **YELLOW** | P1 | Installer resolves/verifies Fabric API and stages it, fixing the previous backend gap. End-to-end Desktop setup remains unproven. | Fabric API is managed automatically. | `resolve_fabric_api` and managed staging in Runtime Installer. | Runtime Installer + Desktop/Tauri | Preserve this backend behavior in integration and validate packaged release assets. |
| No manual SwarmCraft Fabric mod selection | **YELLOW** | P1 | Installer resolves the release asset or a controlled local build artifact, but Desktop integration is missing. | Matching SwarmCraft integration is automatic. | `resolve_swarmcraft_fabric`; no Tauri sidecar glue on feature heads. | Runtime Installer + Desktop/Tauri | Package/invoke the sidecar and validate release asset availability. |
| EULA before launch | **RED** | **P0** | Installer correctly treats EULA as explicit, but Wizard does not derive `eulaRequired` from the installer’s EULA component and can miss the EULA step entirely. | Explicit checkbox before launch; never implicit. | Installer emits `eula_accepted` and an EULA component with state `required`; Wizard checks `eula_required`/overall state. | Runtime Wizard integration | Normalize the actual installer contract and add a real cross-component test, not a synthetic fixture. |
| Minecraft starts after verification | **RED** | **P0** | Wizard expects `runtime_launch`; Runtime Installer sidecar has no launch subcommand and feature heads do not add the required Tauri launch bridge. | Launch only after backend verify returns ready. | `runtime_main.rs` only exposes status/plan/install/repair/verify; Wizard calls `backend.runtime.launch`. | Desktop/Tauri + Migration Runtime | Add one safe launch command that consumes durable managed runtime config and retain backend verification as the gate. |

## Runtime Wizard

| Check | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Why wizard appeared | **GREEN** in isolated UI / **RED** end-to-end | P1 | Copy says “Minecraft setup” and explains SwarmCraft will prepare the server runtime for the selected Minecraft version. | Player understands Play needs one-time setup. | `runtime-wizard.js` dialog copy. | Runtime Wizard | Keep the copy; integrate the real backend. |
| Component states understandable | **YELLOW** | P2 | UI labels are good, but actual installer component array/kinds are not normalized, so real states become unknown. | Java, Minecraft, Fabric, API, SwarmCraft, directories and mods show accurate plain-language states. | Wizard alias map vs Installer `Vec<RuntimeComponentStatus>`. | Runtime Wizard adapter | Map actual `kind` values such as `swarmcraft_fabric`, `server_directories`, `eula`, and `server_mods`. |
| Progress reflects backend progress | **YELLOW** | P2 | Wizard polls backend status every ~650 ms rather than using a fake timer, but installer progress is emitted on stderr and its status object has no phase field. | Progress reflects actual backend phase. | Wizard polling implementation; installer `RuntimeProgress` stderr contract. | Runtime Wizard + Tauri bridge | Forward installer progress events or persist current phase in the status contract. |
| EULA explicit | **RED** | **P0** | UI control is explicit, but actual installer status does not activate it through the current normalization. | Explicit EULA checkbox appears exactly when required. | Contract mismatch described above. | Runtime Wizard | Fix normalization and integration test. |
| Safe retry | **RED** | P1 | Wizard exposes Retry only when `retrySafe===true`, but Runtime Installer errors are plain process errors and RuntimeStatus/InstallReport do not provide `retry_safe`. Retry therefore tends to be hidden instead of actionable. | Failure says whether retry is safe and offers it when safe. | Wizard failure model vs installer schema. | Runtime Installer contract + Wizard | Add structured failure/safety result or a deterministic error-to-safety adapter owned by backend integration. |
| Failures understandable | **YELLOW** | P1 | Wizard can show a message and Advanced details, but backend errors are mostly technical strings and world/retry safety says “Not reported by backend.” | What happened, world safety, retry safety, and next action are explicit. | `renderFailure()` plus installer `anyhow` errors. | Runtime Installer + Wizard | Return structured failure code, player message, world-data safety and retry safety. |
| Advanced remains available | **GREEN** | P3 | Advanced setup button routes to Diagnostics and preserves manual paths. | Power users can inspect/override without contaminating normal setup. | `openAdvancedSetup()`. | Desktop | Keep it. |
| Accidental verification bypass | **YELLOW** | P1 | Wizard blocks launch until its normalized status says ready and captures Play before legacy handler, which is good. But the actual runtime launch bridge is missing and integration is not testable. | Normal Play cannot launch an unverified runtime; Advanced remains explicit. | `launchPreparedWorld()` checks `status.ready`; click capture uses `stopImmediatePropagation`. | Runtime Wizard + Desktop/Tauri | Preserve the guard in the real integrated launch path and add an end-to-end negative test. |

## Play semantics

| State when player presses Play | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Already configured runtime | **RED** | P1 | Wizard intercepts Play and requires `runtime_status`/`runtime_launch`. Those commands are not added on the available feature heads, so even a manually configured legacy runtime can be diverted into “automatic setup unavailable.” | Ready runtime starts immediately. | Wizard click interception plus missing Tauri runtime commands. | Desktop/Tauri integration | Make runtime status/launch available in packaged Desktop and preserve legacy Advanced config as an explicit compatibility path. |
| Fresh runtime | **RED** | **P0** | Setup cannot complete reliably because of contract mismatch and missing integration bridge. | Wizard prepares everything, EULA, verify, launch. | Runtime contract mismatch. | Runtime Installer + Wizard | Integrate actual schema and sidecar. |
| Partially configured runtime | **RED** | P1 | Backend can inspect component state, but Wizard cannot accurately consume the array and does not call repair automatically. | Show what is present/missing and complete only missing work. | Installer status model; Wizard normalization. | Runtime Wizard | Normalize and choose install vs repair based on backend plan/state. |
| Broken/corrupt runtime | **RED** | P1 | Installer can mark `corrupt` and has `repair`, but Wizard’s Retry re-runs install and its component parser misses actual states. | Explain damage, repair safely, reverify. | Installer repair API vs Wizard Retry path. | Runtime Wizard | Call the backend repair contract when repair is required and surface verification result. |
| Incompatible runtime | **RED** | P1 | Installer can mark incompatible; Wizard cannot reliably render the real status. | Explain required version and repair automatically when safe. | Installer component state vs Wizard adapter. | Runtime Wizard | Map actual status and provide player remediation. |
| Missing EULA | **RED** | **P0** | Real installer status does not trigger Wizard’s EULA requirement detection. | EULA step is unavoidable and obvious. | Schema mismatch. | Runtime Wizard | Derive EULA requirement from actual EULA component or add canonical `eula_required`. |

## Server Mods

The Server Mods backend is substantially stronger than the old audit: it inspects Fabric metadata without executing JARs, hashes exact bytes, detects missing/version/hash/client-only/duplicate/conflicting/unexpected mods, stores verified JARs outside the ephemeral runtime, and stages only the canonical required set before host launch.

The player journey is still missing because that branch does not add the required Desktop surface.

| Journey | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Mods panel | **RED** | P1 | No Desktop Mods panel is added by the Server Mods branch. | Selected world has a clear Mods panel. | `agent/server-mod-management` changes CLI/backend/tests/docs, not Desktop. | Desktop + Server Mods | Add player-facing Mods panel backed by structured readiness. |
| Add required mod | **RED** | P1 | Backend `mods-add` accepts only an exact canonical ID/version/hash, but Desktop has no picker/action. | Pick JAR, explain whether it satisfies a requirement. | `server_mods::add_local_mod`. | Desktop | Add file picker/command and render structured result. |
| Remove mod | **RED** | P2 | Backend `mods-remove` exists; no Desktop action. | Remove local mod with warning if it becomes required/missing. | `remove_local_mod`. | Desktop | Add removal UI and refresh readiness. |
| Open mods folder | **RED** | P2 | Backend `mods-path` exists; no Desktop action. | “Open folder” opens persistent per-world mod store. | CLI command exists only. | Desktop/Tauri | Add shell-open command scoped to returned canonical path. |
| Missing required mod | **RED** | P1 | Backend detects and names it; Desktop has no remediation surface. | “Missing: Lithium 0.x — choose the required JAR.” | `ModIssueKind::MissingRequired`. | Desktop + Server Mods | Render structured issue and Add action. |
| Wrong version | **RED** | P1 | Backend rejects exact version mismatch with a precise message; Desktop does not expose it as a player workflow. | Explain required vs selected version. | `VersionMismatch` and `mods-add` version check. | Desktop | Translate structured issue. |
| Wrong hash | **RED** | P1 | Backend rejects hash mismatch; Desktop no workflow. | Explain that bytes do not match the world requirement without exposing hash jargon by default. | `HashMismatch` and exact artifact validation. | Desktop | Player copy first, digest in Advanced. |
| Client-only mod | **RED** | P1 | Backend rejects client-only JARs; Desktop no workflow. | “This mod is for clients, not the server.” | `ModIssueKind::ClientOnly`. | Desktop | Translate issue in Mods panel. |
| Duplicate/conflicting mod | **RED** | P1 | Backend detects duplicate IDs and conflicting versions. | Identify conflicting files and give safe removal path. | `DuplicateModId`, `ConflictingVersion`. | Desktop | Surface conflicts and open-folder/removal actions. |
| Canonical mod requirements after world creation | **YELLOW** | P2 | Backend correctly refuses adding a new canonical requirement after creation and explains that protocol v1 profiles cannot change in place. Desktop has no player explanation because there is no Mods surface. | Product clearly says required mods are fixed for this world version; Add only supplies a missing required artifact. | `SERVER_MOD_RUNTIME_PROFILES.md`; `add_local_mod` refusal. | Desktop + Server Mods | Preserve the safety refusal and explain it in player language. Do not weaken the protocol. |

## Two-player journey: Alice and Bob

| Step | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Alice creates world | **YELLOW** | P2 | Existing Create is understandable; exact integration not available. | Create with defaults. | Desktop Create flow. | Desktop | Re-run on integration build. |
| Alice starts world | **RED** | **P0** | Managed Play path is not integrated. | Automatic runtime setup then launch. | Runtime seam blockers. | Runtime Installer + Wizard + Desktop/Tauri | Close clean-machine blockers. |
| Alice invites Bob | **YELLOW** | P2 | Existing invite UI is simple and signed-token mechanics are mostly hidden. | Create/copy invite. | Current Desktop invite dialog. | Desktop/Core | Re-test exact build; keep advanced bootstrap hidden. |
| Bob joins | **YELLOW** | P2 | Join UI accepts complete invite; backend join stages membership request and daemon completes network flow. Exact integration cannot be executed. | Paste invite, membership completes, world appears. | Existing Join UI; process-level live-join acceptance coverage on available CI. | Desktop + Networking/Core | Add clear “joining/syncing/accepted” states and re-run on integration. |
| Bob receives/syncs world | **YELLOW** | P1 | Replication backend has process-level coverage, but player-facing synchronization progress remains indirect. | Bob sees “Syncing world…” then ready. | Replication acceptance jobs; current world status surfaces are generic. | Replication + Desktop | Show canonical sync progress in world view. |
| Bob prepares runtime | **RED** | **P0** | Same managed-runtime integration failure as Alice. | Bob prepares runtime before takeover without manual paths. | Runtime seams. | Runtime Installer + Desktop | Integrate and test as non-authority successor. |
| Bob satisfies required mods | **RED** | P1 | Backend can verify local JARs, but Desktop has no Mods workflow. | Bob sees exact required missing mods and can supply them. | Server Mods branch has no Desktop surface. | Server Mods + Desktop | Add Mods panel. |
| Alice can understand whether Bob can take over | **RED** | **P0** | Host Readiness backend exists, but primary UI does not consume it; runtime/mod producer hooks are not integrated. | One player-facing shutdown/host-readiness answer. | Host Readiness seam only. | Host Readiness + Desktop + Runtime/Mods producers | Wire backend report directly and record authoritative readiness proofs. |

## Can I shut down this PC?

This is release-critical. The Host Readiness backend is intentionally conservative and is a strong safety foundation. It correctly separates replica presence from authority eligibility, runtime readiness, mods, reachability, conflict state, and surviving quorum. The Desktop must render its `safe_to_shutdown` decision directly.

Important safety clarification: in a **two-member** world, Bob can have a perfect replica and runtime and still fail automatic takeover because Bob alone does not form the required recovery quorum after Alice disappears. The backend correctly returns `blocked_by_quorum`; this is not a defect. A safe explicit host transfer is the intended route. The current Desktop, however, deliberately disables manual transfer, so the player can be left with a safe refusal but no completing action.

| Scenario | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Ready successor with surviving quorum | **RED** | **P0** | Backend can produce `safe`, but no broad primary shutdown surface consumes it. | “Safe to shut down this PC. Bob can take over.” | Host Readiness backend/adapter only; no corresponding primary app wiring on branch. | Desktop + Host Readiness | Render backend report in selected-world/close/shutdown flows. |
| Bob missing runtime | **RED** | **P0** | Backend state `blocked_by_runtime` exists, but Runtime Installer does not record readiness proof and UI does not surface host-readiness report. | “Bob has the world but needs Minecraft setup before he can host.” | Host Readiness producer contract; Runtime Installer lacks producer hook. | Runtime Installer + Host Readiness + Desktop | Record verified runtime proof after authoritative verify and surface state. |
| Bob missing required mod | **RED** | P1 | Backend state `blocked_by_mods` exists, but Server Mods does not record readiness into Host Readiness and UI lacks surface. | “Bob is missing required server mods.” | Host Readiness producer contract; no producer call in Server Mods branch. | Server Mods + Host Readiness + Desktop | Record exact-fingerprint mod readiness and render it. |
| Bob still syncing | **RED** | P1 | Backend state `syncing` exists; no player-facing shutdown card. | “Wait — Bob is still syncing the latest world state.” | Host Readiness contract. | Desktop | Render exact backend state, do not infer from peer counts. |
| Bob offline | **RED** | P1 | Reachability participates in backend green calculation; Desktop does not expose shutdown decision. | “Bob is offline; no other ready host is reachable.” | Host Readiness contract. | Desktop + Networking | Render state and offer retry/peer status. |
| Quorum would disappear | **RED** | P1 | Backend correctly returns `blocked_by_quorum` and may name a handoff candidate. Desktop has no complete transfer action. | Explain why automatic takeover is unsafe and offer safe Transfer Host when available. | Host Readiness docs plus Desktop transfer capability forced off. | Migration integration + Desktop | Keep refusal; add one safe transfer orchestration command and player action. |
| Conflicting history | **YELLOW** | P1 | Current world safety UI already warns about divergent history, and backend Host Readiness has `conflict`, but shutdown-specific state is not integrated. | Clear conflict warning and no safe-shutdown green. | Existing safety panel + Host Readiness `conflict`. | Desktop + Host Readiness | Tie conflict state into shutdown/close banner. |
| Alice is only member | **RED** | P1 | Backend can return `world_will_stop`; player-facing shutdown decision is not wired. | “You can shut down, but this world will be offline until this device returns/wakes it.” Do not imply another host exists. | Host Readiness `world_will_stop`. | Desktop | Surface accurate solo consequence. |
| Already durably sleeping | **RED** | P1 | Backend can report `sleeping` and `safe_to_shutdown=true`; Stop path does not currently prove that durable sleep was reached before its own success message. | “World is safely stopped; shutting down is safe.” | Host Readiness contract vs current Stop kill path. | Runtime lifecycle + Desktop | Fix graceful Stop first, then surface sleeping state. |

## Stop world

**Status: FALSE GREEN**  
**Severity: P0 release blocker**

**Observed behavior:** the Desktop `sleepWorld()` action calls `backend.stopHost()`. The Tauri process layer takes the owned host child and calls `CommandChild.kill()`. Immediately after that command succeeds, Desktop prints **“World runtime stopped gracefully. Replica storage can continue separately.”**

**Expected behavior:** request the real Minecraft/Fabric save barrier, durably commit the final accepted checkpoint/sleep state, wait for the backend lifecycle result, and only then report that the world stopped safely. A force-kill action may exist under Advanced/Diagnostics, but it cannot be presented as graceful.

**Evidence:**

- `apps/desktop/src/app.js::sleepWorld()` reports graceful success after `backend.stopHost()`;
- `apps/desktop/src-tauri/src/runtime_commands.rs::stop_host()` delegates to `RuntimeProcesses::stop_host()`;
- `apps/desktop/src-tauri/src/runtime.rs::stop()` uses `child.kill()`;
- none of the inspected feature heads replace this path.

**Responsible subsystem:** Desktop runtime process control + migration/runtime lifecycle.

**Recommended fix:** expose one backend graceful-stop/sleep command that owns Fabric save/checkpoint/shutdown semantics and returns only after durability is established. Keep force kill separate and visibly unsafe.

## Close application / turn off PC / stop seeding / leave world

| Action | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Stop world | **FALSE GREEN** | **P0** | Claims graceful after process kill. | Durable save/sleep lifecycle. | See Stop world section. | Runtime lifecycle | Replace with safe backend stop. |
| Close SwarmCraft | **RED** | P1 | No normal close interception/policy explains whether Minecraft, replication, or migration remains active. `RuntimeProcesses::Drop` explicitly stops the daemon but does not establish a player-facing shutdown contract. | Close action explains/guards consequences based on backend readiness and active runtime. | Current Tauri process ownership and no close UI. | Desktop lifecycle | Add close guard/policy using authoritative backend state. |
| Turn off this PC | **RED** | **P0** | Strong Host Readiness backend exists but is not surfaced end-to-end. | One clear safe/blocked/wait/unavailable answer. | Host Readiness seam only. | Desktop + Host Readiness | Wire report directly; never recompute in JS. |
| Stop replica/seeding | **GREEN** | P3 | Separate “Stop seeding” control exists and wording distinguishes background replica availability from Minecraft runtime. | Player understands this stops serving the replica, not membership/world ownership. | Existing “Keep the world available” section. | Desktop | Keep wording and add consequence tooltip if needed. |
| Leave world | **YELLOW** | P2 | Separate Leave action explains membership removal after signed acceptance. Current authority is correctly blocked from leaving until transfer, but Desktop transfer is unavailable. | Non-authority can leave; current host is guided to safe transfer first. | Existing Leave flow + backend authority-leave refusal. | Desktop + Migration | Integrate manual transfer so the safe refusal has a completing path. |

## Invite journey

| Check | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Invite generation | **YELLOW** | P2 | Existing dialog creates a signed invite and keeps bootstrap addresses Advanced. Exact integration cannot be re-executed. | Click Invite and get a shareable token. | Existing Desktop flow, unaffected by inspected feature heads. | Desktop/Core | Re-run on integration candidate. |
| Copy | **YELLOW** | P3 | Copy Invite action exists. | One-click copy with success feedback. | Existing Desktop. | Desktop | Re-test exact build. |
| Expiration communication | **YELLOW** | P2 | Alice chooses expiry while creating the invite, but recipient-side expired/invalid errors rely heavily on backend text. | Both sender and recipient get player-language expiration result. | Current invite/join flow. | Desktop/Core | Map expired/invalid invite errors to concise player copy. |
| Paste/join | **YELLOW** | P2 | Bob can paste the complete signed invite without understanding membership internals. | Paste and join. | Existing Join form. | Desktop/Core | Keep cryptographic details out of normal copy. |
| Invalid invite | **YELLOW** | P2 | Empty input is handled clearly; malformed/expired tokens bubble backend error text. | “This invite is invalid/expired” with retry guidance. | Join error handling. | Desktop/Core | Add structured invite error codes. |
| Membership accepted | **YELLOW** | P1 | Backend stages a signed join and daemon completes authority-mediated membership; UI does not have a rich accepted/pending state machine. | “Request sent → accepted → syncing.” | CLI join output and live-join process coverage. | Core/Networking + Desktop | Expose join progression. |
| World discovery/sync after acceptance | **YELLOW** | P1 | Replication is covered in backend acceptance tests; player receives generic world/status refresh rather than an explicit sync journey. | World appears, then shows sync progress until playable/host-ready. | Existing refresh + replication tests. | Replication + Desktop | Add sync progress/state. |

## Internet connectivity

**Status: YELLOW**  
**Severity: P2**

**Observed behavior:** the main world panel already translates structured connectivity into player labels such as Direct connection, Connected through relay, Discovery unavailable, and Could not reach other peers. Technical diagnostics remain under Diagnostics. This is materially better than exposing AutoNAT/DCUtR/multiaddresses.

**Expected behavior:** the primary question should be answered directly: **“Can my friends reach this world?”** with one next action if not.

**Evidence:** `backend-adapter.js` connectivity normalization and selected-world Connectivity cell. Project roadmap still explicitly says representative home NAT/CGNAT/mobile/IPv6 certification is incomplete.

**Responsible subsystem:** NAT/network + Desktop.

**Recommended fix:** keep technical detail Advanced, but make the primary label friend-reachability-oriented and keep field validation as a release limitation until representative networks are certified.

## Existing world import

**Status: RED**  
**Severity: P1**

**Observed behavior:** no inspected Desktop/Tauri or feature branch provides an “Import existing Minecraft world” journey. Export and recovery are not import. The current roadmap does not explicitly exclude existing-world import from the requested MVP player journey.

**Expected behavior:** choose an existing world folder, validate compatibility, explain copy/move behavior, create the initial canonical snapshot/world record, then continue into normal runtime setup.

**Evidence:** no `import_world`/equivalent Desktop action was found; Runtime Installer/Mods/Readiness/Wizard feature work does not add it.

**Responsible subsystem:** Core/storage + Desktop.

**Recommended fix:** add explicit import flow or explicitly remove it from the release target/documented MVP scope. Under the requested audit criteria, the current absence is RED.

## Wake world

| Scenario | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Solo/single-member sleeping world | **YELLOW** | P1 | Backend has a safe wake path, and Desktop exposes Wake when capability probing succeeds. Managed runtime setup/launch integration remains broken, so “play tomorrow” is not yet a clean normal-player loop. | Press Play/Wake; backend validates sleeping state, restores, repairs runtime if needed, starts Minecraft. | Existing migration wake capability + runtime integration blockers. | Migration + Runtime + Desktop | Connect wake to managed runtime setup and test from a cold app restart. |
| Multi-member sleeping world | **RED** | P1 | Backend intentionally refuses multi-member wake until a quorum-backed transition exists. Desktop only gives generic “safe wake when backend allows it” messaging, so the limitation is not taught well. | Either complete safe multi-member wake or clearly state that this release cannot wake a multi-member sleeping world and why at player level. | Migration capability/documented safety refusal. | Migration Core + Desktop communication | Do not weaken the backend. Improve player explanation; implement quorum-backed wake when ready. |

The multi-member refusal is a deliberate safety boundary, not a backend bug. The RED grade is for the player journey being unavailable and insufficiently communicated.

## Manual host transfer

**Status: RED**  
**Severity: P1**

**Observed behavior:** the backend contains staged signed transfer primitives, but the Desktop adapter deliberately forces `migrationCapabilities.transfer=false` because there is no single Desktop-safe orchestration command for the complete flow. The UI accurately says manual host transfer is unavailable.

**Expected behavior:** Alice chooses **Make Bob the host**, Bob’s readiness is proven by backend, final checkpoint/authority transition/runtime launch complete, then the UI reports the new host.

**Evidence:** `backend-adapter.js` explicitly disables transfer; Host Readiness can expose a `handoff_candidate_peer_id` when automatic recovery would lose quorum.

**Responsible subsystem:** Migration orchestration + Desktop.

**Recommended fix:** expose one safe backend transfer operation that owns the entire signed handoff. Desktop should select only candidates returned by backend readiness. The current refusal should remain until that exists.

## Advanced mode

**Status: GREEN for availability, YELLOW for release integration**  
**Severity: P2**

**Observed behavior:** Runtime Wizard has an **Advanced setup** action that returns to Diagnostics with explicit Java/server/SwarmCraft runtime paths. Low-level networking controls also live under Diagnostics. Normal Create keeps compatibility settings collapsed.

**Expected behavior:** Advanced remains available without becoming the default escape hatch for missing normal functionality.

**Evidence:** `runtime-wizard.js::openAdvancedSetup()` and existing Diagnostics view.

**Responsible subsystem:** Desktop.

**Recommended fix:** keep Advanced as-is, but do not route ordinary players there because the automatic sidecar/integration is missing. Normal Play must own setup.

## Failure journeys

The hardening branch adds useful runtime safety tests for EULA rejection, missing Java, retry after partial setup, incompatible Fabric handshake, and corrupt runtime metadata. Those tests primarily exercise the existing migration runtime path on Unix. They do not close the managed Runtime Installer ↔ Wizard contract or provide a complete player-facing failure contract.

| Failure | Status | Severity | Observed behavior | Expected behavior | Evidence | Responsible subsystem | Recommended fix |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Download interrupted | **YELLOW** backend / **RED** Desktop | P1 | Installer uses temporary artifacts and removes failed partial downloads, which is good. Wizard cannot reliably say retry is safe because installer errors do not provide `retry_safe`. | “Download was interrupted. World is safe. Retry is safe.” | `install_artifact` temp cleanup; Wizard failure model. | Runtime Installer + Wizard | Return structured failure/safety result and enable retry. |
| Bad runtime artifact | **YELLOW** backend / **RED** Desktop | P1 | Hash mismatch is rejected and temp artifact removed. Player receives technical error and no structured retry/safety answer. | Explain verification failure, keep world safe, offer redownload/repair. | Installer hash verification. | Runtime Installer + Wizard | Structured artifact-verification failure + repair action. |
| Permission problem | **RED** | P1 | File/directory errors bubble through `anyhow`/process stderr; no player-specific remediation contract. | “SwarmCraft cannot write to its runtime folder. World data was not changed. Fix permission / choose supported location.” | Installer filesystem error paths. | Runtime Installer + Wizard | Add classified permission/storage failures and safe retry guidance. |
| Java missing | **RED** | **P0** for clean-machine guarantee | Backend can download Java, but Desktop integration is absent and Windows installer test is red. | Automatically install compatible Java or provide one actionable error. | Runtime Installer + Windows CI failure. | Runtime Installer + Desktop/Tauri | Fix cross-platform hashing/tool dependencies and integrate. |
| Required mod missing | **RED** | P1 | Backend detects exactly; no Desktop Mods remediation. | Name missing mod and Add action. | Server Mods readiness. | Server Mods + Desktop | Mods panel. |
| Wrong mod/version/hash | **RED** | P1 | Backend rejects accurately; no Desktop workflow. | Explain mismatch in player language and let user choose correct JAR. | Server Mods issue kinds. | Server Mods + Desktop | Mods panel + structured copy. |
| Network offline during runtime setup | **YELLOW** | P2 | Runtime metadata/download commands fail with technical request text; global connectivity UI separately explains offline state. The two are not composed. | “Setup needs internet; your world is safe. Reconnect and Retry.” | `curl_text`/`curl_download` errors plus connectivity UI. | Runtime Installer + Desktop | Map network errors and reuse connectivity state. |
| Successor disappears | **RED** | P1 | Host Readiness is designed to fail closed on stale/unreachable peers, but shutdown surface is not wired. | Green immediately disappears and UI says no ready successor is reachable. | Host Readiness freshness/reachability rules. | Host Readiness + Desktop | Live refresh authoritative report in shutdown/close UI. |
| Runtime verification fails | **RED** | P1 | Installer can classify components corrupt/incompatible, but Wizard parser does not consume actual status and its repair/retry path is not connected. | Explain failed component, repair if safe, never launch. | Installer status + Wizard mismatch. | Runtime Wizard + Installer | Actual schema integration and repair action. |
| Incompatible Fabric handshake | **YELLOW** backend / **RED** normal Desktop | P1 | Hardening test proves legacy migration runtime does not publish ready or change canonical world on incompatible handshake. Normal managed-runtime player flow is still unintegrated. | World remains safe; player gets clear compatibility fix. | `runtime_setup_hardening.rs::incompatible_fabric_handshake_is_rejected_before_ready_and_world_stays_canonical`. | Migration Runtime + Wizard | Preserve backend safety and surface remediation. |
| Partial setup retry | **YELLOW** backend / **RED** normal Desktop | P1 | Hardening test proves a legacy partial setup can be retried safely. Wizard cannot currently obtain retry safety from managed installer. | Retry button when backend says safe. | Hardening test + Wizard/Installer contract. | Runtime Installer + Wizard | Add managed-path failure contract and acceptance test. |

## CI and platform evidence

The available Runtime Installer PR CI provides useful but insufficient evidence:

- Desktop package: Linux — success;
- Desktop package: Windows — success;
- Desktop package: macOS ARM64 — success;
- Desktop package: macOS x86_64 — success;
- Linux frontend backend-contract tests — success;
- Ubuntu Rust — success;
- macOS Rust — success;
- Fabric server mod build — success;
- dependency audit — success;
- process-level acceptance — success;
- Windows Rust — **failure** in `runtime_installer::tests::atomic_local_install_is_retry_safe` because the runtime hashing implementation depends on unavailable `Get-FileHash` in that runner environment.

A package successfully building is not proof that the clean-machine runtime journey works inside that package. In particular, the Runtime Installer branch itself notes that Desktop still needs to package/invoke the runtime sidecar, and the Wizard branch was built against a synthetic contract rather than the actual installer schema.

## Responsible subsystem summary

The remaining blockers are integration problems more than missing algorithms:

- **Runtime Installer:** managed artifacts are largely implemented, but Windows portability, structured failures, host-readiness producer hook, and actual Desktop sidecar contract are incomplete.
- **Runtime Wizard:** player copy and guardrails are promising, but it consumes the wrong JSON shape and assumes commands the available Desktop does not provide.
- **Server Mods:** backend verification is strong; Desktop management and host-readiness producer integration are missing.
- **Host Readiness:** backend safety model is strong and correctly fail-closed; broad Desktop rendering and producer integration are missing.
- **Runtime lifecycle:** Stop world remains a safety false green because process kill is presented as graceful.
- **Desktop lifecycle:** Close/turn-off behavior is not unified around authoritative readiness.
- **Migration:** manual transfer and multi-member wake remain intentionally unavailable until safe orchestration exists; Desktop must communicate these limitations rather than weaken them.
- **Release integration:** the named integration branch itself must exist and be pinned before any final audit can be certified.

## Required release blockers to close

1. Publish `integration/runtime-player-journey` and provide its exact immutable SHA.
2. Replace the Stop-world process-kill success path with a backend-owned durable save/checkpoint/sleep lifecycle. Do not retain the current “graceful” claim around `kill()`.
3. Integrate Runtime Wizard against the **actual** Runtime Installer schema, including EULA, component kinds, install-report wrapper, progress, failure safety, and repair.
4. Package/invoke `swarmcraft-runtime` from Desktop and provide safe Tauri status/plan/install/repair/verify/launch commands.
5. Fix the Windows runtime hashing/test failure and rerun cross-platform runtime setup tests.
6. Wire Runtime Installer and Server Mods readiness producers into Host Readiness.
7. Render Host Readiness directly in the primary world/close/shutdown journey, including safe, syncing, missing runtime, missing mods, offline, quorum-blocked, conflict, solo, and sleeping states.
8. Add the Desktop Server Mods surface for add/remove/open-folder/remediation without permitting unsafe canonical profile mutation.
9. Either add existing-world import or explicitly remove it from the MVP release scope in project docs; under the current requested scope it remains RED.
10. Re-run the complete Alice/Bob player journey and failure matrix on the exact integration SHA, not on separate feature branches.

## Release decision

SwarmCraft has meaningful backend progress: automatic runtime acquisition exists, Fabric API is managed, server-mod verification is deterministic, and host-readiness safety is well designed. Those pieces have not yet become one coherent, auditable player journey. The requested integration candidate is absent, the Runtime Wizard and Installer contracts disagree on their current heads, Host Readiness is not wired end-to-end, Windows runtime setup has a failing test, and Stop world still presents a process kill as a graceful durable shutdown.

These are not polish issues. The Stop-world safety claim and clean-machine/runtime/shutdown integration gaps are release blockers.

**NOT READY**
