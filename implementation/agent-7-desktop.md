# Agent 7 — Desktop Player Journey

## Status

STATUS: NOT STARTED

BRANCH: `fix/agent-7-desktop`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT HEAD SHA: pending

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

- [ ] Fix `installModsUi()` insertion so launcher module graph initializes without uncaught exception.
- [ ] Add a real DOM/browser initialization smoke test loading current `index.html` and ES modules.
- [ ] Assert provider Mods UI is installed.
- [ ] Assert public discovery UI is installed.
- [ ] Assert canonical Create interception is installed and legacy fallback does not own the intended path.
- [ ] Fix Import `minecraft_versions` payload with `includeSnapshots` and `refresh`.
- [ ] Fix Import `fabric_loader_versions` payload with `refresh`.
- [ ] Keep Import controls usable with visible loading/error/retry state if catalog hydration fails.
- [ ] Separate canonical world creation success from later local mod installation failure; retain world ID and enter repair state rather than blind recreate.
- [ ] Update provider UI to any server-owned staging contract introduced by Agent 5.
- [ ] Route exact World ID resolution feedback to the local Join status region.
- [ ] Bring injected provider/discovery controls into current component/style system.
- [ ] Correct Stop-world copy to match durable save/checkpoint/sleep behavior.
- [ ] Render exact current module graph at `980x760`, `720x560`, and wider size.
- [ ] Test keyboard/focus behavior for Create, Join, runtime/EULA, Invite, Transfer, Stop, Leave.
- [ ] Add source/contract coverage for supported runtime tuples from Agent 6.

## Work completed

None yet.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| None yet | - | - | - |

## Required validation before handoff

- [ ] frontend unit tests
- [ ] real browser module initialization smoke
- [ ] zero uncaught startup exceptions
- [ ] canonical Create invocation observed
- [ ] provider UI/discovery UI present
- [ ] Import contract fixture
- [ ] post-create local-mod failure partial-success UX test
- [ ] exact-size screenshots/render checks
- [ ] keyboard/focus pass
- [ ] Desktop package build on supported platforms via CI

## Blockers

None for initial work. Recovery/wake UX coordination waits on Agent 9.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: `launcher-controller.js`, provider Tauri adapters, runtime/recovery UI.

## Agent final statement

NOT COMPLETE
