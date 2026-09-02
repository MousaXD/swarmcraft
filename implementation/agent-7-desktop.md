# Agent 7 — Desktop Player Journey

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-7-desktop`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH BASE SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (`integration/audit-remediation-v1`; implementation-ledger-only commit on top of the production starting SHA)

CURRENT IMPLEMENTATION HEAD SHA: `c3368dc8be69f58a8a686cf288c3bfdaa0b714af`

INTEGRATED SHA: pending

## Mission

Make the shipped Desktop launcher/player journey actually initialize and use the canonical/provider/discovery backends correctly, with browser-level proof rather than source-string assumptions.

## Findings owned

- FINAL-020 — launcher enhancement initialization crash
- FINAL-031 — Import catalog Tauri contract mismatch and related Desktop integration failures
- FINAL-042 — player UX/visual/contract hardening assigned by final audit

Coordinate UX for FINAL-023 and FINAL-024 after Agent 9 backend semantics exist.

Read `audits/FINAL-AUDIT.md` and Auditor 8 Desktop UX before editing.

## Dependencies

Required before starting: none for the initialization and contract work.

Dependency state at latest reconciliation:

- Agent 5 (`fix/agent-5-supply-chain`): `IN PROGRESS`. Its server-owned provider staging/Tauri contract is not implemented yet, so Agent 7 cannot truthfully finalize that frontend payload.
- Agent 6 (`fix/agent-6-runtime`): `IN PROGRESS`, current implementation milestone `b4a86868f403c76faa732519166a7953409c1486`. It has authored an authoritative shipped runtime adapter contract (`~26.1.2`, Fabric `>=0.19.3`, Java `>=25`) but has not completed or integrated its runtime work. Agent 7 must consume the integrated contract rather than duplicate an in-flight backend rule.
- Agent 9: no implementation branch exists at latest reconciliation; its campaign ledger remains blocked on Agents 1 + 6. Recovery/wake UX cannot be finalized yet.

Coordinate with:

- Agent 5 for provider Tauri payload/staging contract changes
- Agent 6 for runtime support-matrix UX
- Agent 9 for recovery/wake UX once backend semantics are integrated

## Ownership boundaries

Primary ownership:

- `apps/desktop/src/*`
- Desktop frontend tests
- Tauri command adapter contracts where the fix is purely Desktop integration
- exact-head browser/render/keyboard evidence

Do not duplicate backend security checks in frontend as the sole defense.

## Implementation checklist

- [x] Fix `installModsUi()` insertion so launcher module graph initializes without uncaught exception.
- [x] Add a real DOM/browser initialization smoke test loading current `index.html` and ES modules.
- [x] Assert provider Mods UI is installed.
- [x] Assert public discovery UI is installed.
- [x] Assert canonical Create interception is installed and legacy fallback does not own the intended path.
- [x] Fix Import `minecraft_versions` payload with `includeSnapshots` and `refresh`.
- [x] Fix Import `fabric_loader_versions` payload with `refresh`.
- [x] Keep Import controls usable with visible loading/error/retry state if catalog hydration fails.
- [x] Separate canonical world creation success from later local mod installation failure; retain world ID and enter repair state rather than blind recreate.
- [ ] Update provider UI to the server-owned staging contract introduced by Agent 5. Blocked until Agent 5 publishes that contract.
- [x] Route exact World ID resolution feedback to the local Join status region.
- [x] Bring injected provider/discovery controls into current component/style system.
- [x] Correct Stop-world copy to match durable save/checkpoint/sleep behavior.
- [x] Author exact-module render coverage at `980x760`, `720x560`, and `1280x900`; exact-head CI proof pending.
- [x] Author keyboard/focus coverage for Create, Join, runtime/EULA, Invite, Transfer, Stop, Leave; exact-head CI proof pending.
- [ ] Add source/contract coverage for supported runtime tuples from Agent 6. Blocked until Agent 6 is integrated.

## Work completed

### Campaign start / audit intake

- Verified the requested `implementation/agent-7-consensus.md` does not exist; the master ledger assigns Agent 7 to `implementation/agent-7-desktop.md` and branch `fix/agent-7-desktop`.
- Read `implementation/README.md`, `audits/FINAL-AUDIT.md`, Auditor 8 `audits/08-desktop-ux.md`, all dependency ledgers, repository `AGENTS.md`, and the required Desktop UI quality/design skills.
- Verified the production campaign base and the initial implementation integration head differ only by the implementation-ledger commit.
- Verified no pre-existing `fix/agent-7-desktop` remote branch existed before this session; created it from exact integration head `a9736b159d9e9618a3ed8515c20e93f92c1453cb` without rebasing or discarding work.
- Confirmed the audited defects remained present in the production baseline.
- Milestone ledger start: `7c48d08a7b52fa663f3013ac3cf0e5c071c455a2` (`docs(progress): start agent 7 desktop remediation`).

### Desktop player-journey remediation

- Milestone `2bb74bcb5c86e0f080c6e8a4816a856246165415` (`fix(desktop): repair launcher initialization and player contracts`).
- Repaired Mods UI insertion by inserting the new section before the direct-child Create form action group instead of calling `form.insertBefore()` with a nested submit button reference.
- Migrated injected provider/discovery surfaces onto existing `player-section`, `section-heading`, `field`, `field-grid`, `compact-actions`, `button`, `field-help`, `inline-notice`, `details-grid`, and `detail-row` primitives.
- Added correct Import catalog command payloads: `minecraft_versions` receives explicit `includeSnapshots` and `refresh` booleans; `fabric_loader_versions` receives `minecraftVersion` and `refresh`.
- Import hydration now preserves the exact-version inputs unless both initial official catalog calls succeed. Provider failure leaves usable inputs plus a visible status/error/retry path instead of replacing them with empty disabled controls.
- A Minecraft change that cannot hydrate compatible Fabric loaders reverts to the previous compatible Minecraft selection and retains usable loader state.
- Canonical Create now distinguishes durable world creation from later local mod installation. After `create_canonical_world` succeeds, a later `world_mods_add` failure records the returned world ID, keeps Create disabled, reports partial success, and offers `Retry local mod setup` against only the remaining artifacts for the existing world.
- Exact World ID discovery feedback now targets `joinWorldIdNotice` rather than the unrelated Public Worlds status area.

### Browser/render gate

- Initial milestone `c88fb3823df298719254ed496dd6dbc0ded14761` authored a real current-module browser smoke/render test.
- PR #60 CI run `33581009161`, Linux Desktop job `100095104417`, proved all 57 pre-existing frontend tests still passed but the new Chromium CLI harness timed out at `980x760`. The failure was harness process-lifecycle behavior, not a weakened or bypassed assertion.
- Replaced the hanging `--dump-dom`/`--screenshot` CLI harness with explicit Chrome DevTools Protocol lifecycle control while preserving the same product assertions. Replacement milestone sequence: `9b5bbc716e7fd30fbd323fe31e12217349df3419` then removal of the obsolete harness at `3d55c262915a27d2b2788b8f5569946cbce8e2d7`.
- The CDP test serves the actual current `index.html` and ES-module graph, injects only a deterministic Tauri test bridge, captures window errors/unhandled rejections, verifies Mods and discovery initialization, validates every catalog payload, observes canonical Create exactly once with zero legacy `create_world` calls, verifies the post-create repair state, exercises keyboard focus targets, checks horizontal overflow, and captures screenshots at `980x760`, `720x560`, and `1280x900`.

### Stop-world semantics

- Milestones `9793758079207687e4a1ee730cae992453469ad4`, `89e7bd7e75342e61b7d523e56862198eb6a564d6`, and `c3368dc8be69f58a8a686cf288c3bfdaa0b714af` correct and lock the Stop-world copy.
- The shipped module graph now states that Stop requests a Minecraft save barrier, publishes the canonical checkpoint, and waits for durable sleeping state before success, while background replica storage may continue separately.

### Dependency reconciliation

- The integration branch advanced to `49554075a2a46c8bd14630474afa0f19147c4f59` during Agent 7 work only through an Agent 10 ledger commit, with no production conflict.
- Agent 5 has started but has not yet published the server-owned provider staging contract required by Agent 7.
- Agent 6 has authored its shared runtime support matrix on its own in-progress branch, but its work is not ready or integrated. Agent 7 recorded the current contract for coordination and will not fork the backend rule into a separate frontend authority.
- Agent 9 still has no implementation branch and remains unavailable for recovery/wake UX coordination.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Repository/audit state verification | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Production tree is campaign base plus ledger-only integration commit; Agent 7 branch did not previously exist. |
| Source/audit contract review | PASS | `b4bab08562cf0eb53763674407375b023e1d0858` | Reconfirmed A8-001 through A8-007 against launcher/index source. |
| PR #60 frontend suite, first browser harness | FAIL | `c88fb3823df298719254ed496dd6dbc0ded14761` | CI run `33581009161`, Linux Desktop job `100095104417`: 57 existing tests passed; new browser smoke alone timed out because Chromium CLI did not terminate. Harness replaced with explicit CDP control. |
| Release version guard | PASS | `c3368dc8be69f58a8a686cf288c3bfdaa0b714af` | Run `33581316525`. |
| Exact-head CI | RUNNING | `c3368dc8be69f58a8a686cf288c3bfdaa0b714af` | Run `33581316562`; Linux Desktop frontend/CDP gate is currently executing. |

## Required validation before handoff

- [ ] frontend unit tests green at exact final implementation head
- [ ] real browser module initialization smoke green
- [ ] zero uncaught startup exceptions
- [ ] canonical Create invocation observed
- [ ] provider UI/discovery UI present
- [ ] Import contract fixture
- [ ] post-create local-mod failure partial-success UX test
- [ ] exact-size screenshots/render checks
- [ ] keyboard/focus pass
- [ ] Desktop package build on supported platforms via CI

## Blockers

- Agent 7-owned FINAL-020/031/042 implementation is complete enough for exact-head validation, but the full Agent 7 checklist cannot be finished yet because two coordinated contracts remain in flight.
- Agent 5 is `IN PROGRESS` and has not published the server-owned provider staging contract. Until it does, Agent 7 cannot safely replace the current frontend-computed provider destinations without guessing an API owned by Agent 5.
- Agent 6 is `IN PROGRESS`. Its runtime support matrix exists on its branch but is not integrated; Agent 7 cannot add final integrated source/contract coverage or UI exposure against a moving backend head.
- Agent 9 still has no implementation branch and recovery/wake UX coordination for FINAL-023/024 cannot be completed.
- Exact-head CI for `c3368dc8be69f58a8a686cf288c3bfdaa0b714af` is still running.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending dependency integration and exact-head green validation.

Draft integration PR: #60 (`fix/agent-7-desktop` -> `integration/audit-remediation-v1`).

Known conflict areas: `launcher-controller.js`, provider Tauri adapters, runtime/recovery UI.

## Agent final statement

NOT COMPLETE
