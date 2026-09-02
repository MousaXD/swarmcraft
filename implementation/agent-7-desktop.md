# Agent 7 — Desktop Player Journey

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-7-desktop`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH BASE SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (`integration/audit-remediation-v1`; implementation-ledger-only commit on top of the production starting SHA)

CURRENT HEAD SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` at work start; milestone commits are recorded below as they are pushed

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

Dependency state verified at campaign start:

- Agent 5 (`implementation/agent-5-supply-chain.md`): `NOT STARTED`; provider staging/Tauri contract may change later, so Agent 7 must not guess its future payload.
- Agent 6 (`implementation/agent-6-runtime.md`): `NOT STARTED`; authoritative runtime support-matrix UX contract does not exist yet.
- Agent 9 (`implementation/agent-9-recovery-wake.md`): `BLOCKED ON AGENTS 1 + 6`; recovery/wake UX coordination cannot be finalized yet.

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

### Campaign start / audit intake

- Verified the requested `implementation/agent-7-consensus.md` does not exist; the master ledger assigns Agent 7 to `implementation/agent-7-desktop.md` and branch `fix/agent-7-desktop`.
- Read `implementation/README.md`, `audits/FINAL-AUDIT.md`, Auditor 8 `audits/08-desktop-ux.md`, all dependency ledgers named above, repository `AGENTS.md`, and the required Desktop UI quality/design skills.
- Verified the production campaign base and the implementation integration head differ only by the implementation ledger commit.
- Verified no pre-existing `fix/agent-7-desktop` remote branch existed before this session; created it from exact integration head `a9736b159d9e9618a3ed8515c20e93f92c1453cb` without rebasing or discarding work.
- Confirmed the audited defects remain present in the production baseline: invalid `insertBefore` launcher initialization, missing Import catalog Tauri booleans, post-create local-mod failure conflation, wrong exact-World-ID status target, stale injected component classes, and inaccurate Stop-world copy.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Repository/audit state verification | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Production tree is campaign base plus ledger-only integration commit; Agent 7 branch did not previously exist. |
| Source/audit contract review | PASS | `b4bab08562cf0eb53763674407375b023e1d0858` | Reconfirmed A8-001 through A8-007 against launcher/index source. |

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

- No blocker for Agent 7-owned initialization, Import-contract, partial-success, status/copy/component, browser-smoke, render, and keyboard work.
- Agent 5 must define and land the server-owned provider staging contract before the corresponding frontend payload can be finalized.
- Agent 6 must define and land the authoritative runtime support matrix before Agent 7 can add final tuple-source contract coverage.
- Agent 9 remains blocked on Agents 1 + 6, so FINAL-023/FINAL-024 recovery/wake UX coordination is not currently implementable.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: `launcher-controller.js`, provider Tauri adapters, runtime/recovery UI.

## Agent final statement

NOT COMPLETE
