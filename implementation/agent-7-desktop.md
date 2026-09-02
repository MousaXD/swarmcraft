# Agent 7 — Desktop Player Journey

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-7-desktop`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH BASE SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (`integration/audit-remediation-v1`; implementation-ledger-only commit on top of the production starting SHA)

CURRENT IMPLEMENTATION HEAD SHA: `ee38d2159610bf19cffc494abf77fca5dad44310`

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

Latest dependency reconciliation:

- Agent 5 branch `fix/agent-5-supply-chain` is still `IN PROGRESS`, but its committed patch script now publishes the intended native provider contract: `provider_staging_dir` returns an opaque `desktop-*` session; Modrinth download takes `{ locator, stagingSession, maxBytes }`; CurseForge download takes `{ fileId, stagingSession }`.
- Agent 7 now consumes that published contract through a backward-compatible frontend bridge. Current path-shaped staging values retain the old payload; opaque `desktop-*` sessions are converted to the Agent 5 payload without exposing a destination path.
- Agent 6 branch `fix/agent-6-runtime` remains `IN PROGRESS`. Its authoritative runtime contract is derived from the shipped Fabric adapter: Minecraft `~26.1.2`, Fabric Loader `>=0.19.3`, Java `>=25`.
- Agent 7 now locks Create/Import compatibility defaults to the shipped `fabric.mod.json` contract and uses the supported tuple in browser acceptance fixtures.
- Agent 9 still has no implementation branch. Recovery/wake UX for FINAL-023/FINAL-024 remains blocked on its backend semantics.

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
- [x] Update provider UI for Agent 5's server-owned opaque staging contract while remaining compatible with the current path-based backend.
- [x] Route exact World ID resolution feedback to the local Join status region.
- [x] Bring injected provider/discovery controls into current component/style system.
- [x] Correct Stop-world copy to match durable save/checkpoint/sleep behavior.
- [x] Author exact-module render coverage at `980x760`, `720x560`, and `1280x900`; exact-head green proof pending.
- [x] Author keyboard/focus coverage for Create, Join, runtime/EULA, Invite, Transfer, Stop, Leave; exact-head green proof pending.
- [x] Add source/contract coverage for Agent 6's supported runtime tuple by binding Desktop defaults to shipped Fabric adapter metadata.

## Work completed

### Campaign start / audit intake

- Verified the requested `implementation/agent-7-consensus.md` does not exist; the campaign assigns Agent 7 to this ledger and `fix/agent-7-desktop`.
- Read `implementation/README.md`, the final audit, Auditor 8 Desktop UX report, dependency ledgers, repository agent rules, and Desktop UI quality/design guidance.
- Created Agent 7 branch from exact campaign plan head without rewinding newer work.
- Campaign-start ledger milestone: `7c48d08a7b52fa663f3013ac3cf0e5c071c455a2`.

### Desktop player-journey remediation

- `2bb74bcb5c86e0f080c6e8a4816a856246165415` repairs launcher initialization and Desktop player contracts.
- Mods UI insertion now targets the direct Create-form action group instead of calling `insertBefore()` with a nested button reference.
- Injected provider/discovery surfaces use existing component primitives.
- Import catalog calls now send the complete Tauri payload and retain usable exact-version controls with visible retry/error fallback.
- Canonical Create preserves successful world creation when later local mod setup fails, records the returned world ID, prevents duplicate recreation, and exposes same-world repair.
- Exact World ID resolution feedback is local to the Join flow.
- `9793758079207687e4a1ee730cae992453469ad4`, `89e7bd7e75342e61b7d523e56862198eb6a564d6`, and `c3368dc8be69f58a8a686cf288c3bfdaa0b714af` correct and lock durable Stop-world copy.

### Browser/render acceptance gate

- `c88fb3823df298719254ed496dd6dbc0ded14761` authored the first real-browser current-module gate.
- CI proved three transport/lifecycle variants unsuitable on hosted Chrome: CLI dump/screenshot termination, fixed DevTools TCP, and DevTools pipe. Each failure was recorded rather than weakening product assertions.
- `f54d72ae340d8c8aaeeb2a337223521e916706c4` replaces those harnesses with Chrome's dynamically advertised `DevToolsActivePort` WebSocket endpoint. The mechanism was reproduced successfully against local Chromium before push.
- The current gate serves the unmodified production `index.html`/ES-module graph, injects only a deterministic Tauri bridge with `Page.addScriptToEvaluateOnNewDocument`, executes the journey through the real DOM, captures startup errors/rejections, verifies provider/discovery initialization, validates catalog payloads, observes exactly one canonical Create and zero legacy `create_world` calls, verifies post-create repair state, exercises focus targets, checks horizontal overflow, and captures PNGs at `980x760`, `720x560`, and `1280x900`.
- Browser fixtures now use Agent 6's shipped supported tuple `26.1.2` / `0.19.3`.

### Agent 5 provider coordination

- Agent 5 patch script `scripts/agent5_milestone1.py` publishes the exact intended opaque staging payload.
- `74d9435c4aed455be11716acc6e6637f7a8e2259`, `7cc8c7e088fe77648f004cbc3a20bee9c06d3fc5`, and `6dd621737a3539b585ed6fac002a53ae67720ca8` add and install a provider contract bridge plus unit coverage.
- The bridge rewrites only destination strings beginning with a valid opaque `desktop-*` staging session. Current Unix/Windows path-shaped destinations are passed through unchanged, so Agent 7 remains runnable before Agent 5 backend integration.
- New opaque-session Modrinth payload: `{ locator, stagingSession, maxBytes }`.
- New opaque-session CurseForge payload: `{ fileId, stagingSession }`.

### Agent 6 runtime coordination

- `ee38d2159610bf19cffc494abf77fca5dad44310` adds a Desktop runtime contract test against `minecraft/fabric/src/main/resources/fabric.mod.json`.
- The test locks Create and Import defaults to Minecraft `26.1.2` and Fabric Loader `0.19.3`, and asserts the shipped adapter continues to declare Minecraft `~26.1.2`, Loader `>=0.19.3`, and Java `>=25`.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Repository/audit state verification | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Campaign base and ownership verified. |
| Source/audit contract review | PASS | `b4bab08562cf0eb53763674407375b023e1d0858` | Reconfirmed Agent 7 audit findings against source. |
| Existing frontend suite before browser gate | PASS | multiple Agent 7 heads | 58+ non-browser frontend tests remained green through browser-harness iterations. |
| Browser CLI harness | FAIL | `c88fb3823df298719254ed496dd6dbc0ded14761` | Hosted Chromium process-lifecycle hang; harness replaced. |
| Fixed-port CDP harness | FAIL | `c3368dc8be69f58a8a686cf288c3bfdaa0b714af` lineage | Hosted Chrome did not expose the requested TCP endpoint; harness replaced. |
| Pipe CDP harness | FAIL | `0caef92afbc5a308f0ee02b200510d7e71e9a81f` | `Target.getTargets` did not answer over pipe on hosted Chrome; harness replaced. |
| HTTP callback CLI harness | FAIL | `9a216008f82e5eb05bcf645a6e8b0f11f7c4bded` | Hosted screenshot process never returned the page callback; harness replaced. |
| Dynamic `DevToolsActivePort` mechanism | PASS locally | pre-push reproduction | Local Chromium advertised a dynamic port/path and answered `Target.getTargets` over Node WebSocket. |
| Release version guards | PASS | latest completed Agent 7 heads | No release-version regression observed. |
| Exact-head CI | PENDING | `ee38d2159610bf19cffc494abf77fca5dad44310` production-equivalent plus this ledger commit | Final browser and supported-platform package proof still required. |

## Required validation before handoff

- [ ] frontend unit tests green at exact final head
- [ ] real browser module initialization smoke green
- [ ] zero uncaught startup exceptions
- [ ] canonical Create invocation observed
- [ ] provider UI/discovery UI present
- [ ] Import contract fixture
- [ ] post-create local-mod failure partial-success UX test
- [ ] exact-size screenshots/render checks
- [ ] keyboard/focus pass
- [ ] provider opaque-session bridge unit coverage
- [ ] runtime support contract coverage
- [ ] Desktop package build on supported platforms via CI

## Blockers

- Agent 7-owned source implementation and cross-agent compatibility work are now complete; exact-head validation is still outstanding.
- Agent 9 has no implementation branch, so FINAL-023/FINAL-024 recovery/wake UX cannot be coordinated until its backend semantics exist.
- Agent 5 and Agent 6 remain in progress. Agent 7 has implemented compatibility against their currently published contracts, but final integration must still prove those contracts did not change before merge.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending exact-head green validation and dependency recheck.

Draft integration PR: #60 (`fix/agent-7-desktop` -> `integration/audit-remediation-v1`).

Known conflict areas: `launcher-controller.js`, provider Tauri adapters, runtime/recovery UI.

## Agent final statement

NOT COMPLETE
