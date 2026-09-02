# Agent 6 — Minecraft / Runtime Lifecycle

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-6-runtime`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT VALIDATED IMPLEMENTATION SHA: `e323d7c51225d93e7862a1d2cfc98f652c2849d2`

CURRENT BRANCH SHA AT THIS LEDGER UPDATE: `81dbb3feac5a06f631cff7e11ca62760f9090abb`

INTEGRATED SHA: pending

## Mission

Make Minecraft lifecycle safe under import, compatibility selection, supervisor failure, controller disconnect, and relaunch. Preserve the strong existing save/checkpoint ordering while closing the hard-death gaps.

## Findings owned

- FINAL-014 — import can snapshot a live/torn Minecraft save
- FINAL-015 — catalog-valid runtime tuple can violate shipped Fabric bridge contract
- FINAL-016 — runtime supervisor/controller death can orphan writable Java process
- FINAL-032 — runtime diagnostics/supportability gap

Read `audits/FINAL-AUDIT.md` and Auditor 5 Minecraft Runtime before editing.

## Dependencies

Required before starting: none.

Agent 9 depends on Agent 6 integration.

Coordinate canonical support-matrix UI exposure with Agent 7.

The user-supplied filename `implementation/agent-6-consensus.md` does not exist in the repository. The campaign index names this Agent 6 runtime ledger, `implementation/agent-6-runtime.md`, as the implementation contract; repository search also found no Agent 6 consensus file. No missing file was invented.

## Ownership boundaries

Primary ownership:

- `crates/swarm-cli` runtime installer/layout/migration/launch guard/world import/authority permit/server mods
- Fabric bridge lifecycle
- Desktop native runtime process manager where required
- runtime process/chaos tests

Do not weaken authority quorum semantics to keep Java running.

## Implementation checklist

- [x] Prove external Minecraft source quiescence for the full import snapshot operation in the import implementation by holding the source lock guard through snapshot commit.
- [x] Acquire/hold Minecraft-compatible session lock or another authoritative save/quiescence proof. Linux/Android use POSIX record locking compatible with Java NIO; macOS uses Darwin `fcntl`; Windows uses the OS lock through `fs2`.
- [ ] Reject import while a real Minecraft process owns/mutates the source. Cross-process lock regression is authored; real-server acceptance remains required.
- [x] Define one authoritative runtime adapter support matrix derived from shipped bridge artifacts.
- [x] Enforce supported Minecraft version/range, Fabric loader range, and Java constraints before canonical create/import and again at live Fabric handshake. The shipped adapter contract is one source of truth.
- [x] Prevent unsupported tuples from becoming canonical worlds through Desktop Create and CLI import.
- [x] Make runtime adapter/artifact selection explicit if multiple Minecraft lines are supported. Not applicable in this build: one shipped adapter contract (`~26.1.2`, Fabric `>=0.19.3`, Java `>=25`) is authoritative.
- [x] Make Fabric/controller liveness part of authority runtime safety.
- [x] Treat authenticated controller IPC startup failure, EOF, socket failure, or lease expiry as fail-closed save/stop from the Fabric side.
- [x] Add supervisor heartbeat/lease distinct from daemon authority permit. The authenticated IPC session emits controller heartbeats and the Fabric bridge expires them independently of the authority permit.
- [x] Before resetting runtime directory, prove previous recorded Java runtime is gone. A persistent per-world runtime-process record survives Rust supervisor death and blocks reset while the recorded Java PID is live.
- [x] Add practical process containment without replacing save-first shutdown semantics. Controller-IPC loss drives Fabric save/stop first, while the persistent Java PID fence prevents a new controller from deleting/restoring the runtime until the old process is proven gone. Kill-on-parent-death OS containment is intentionally not used because it could preempt the save-first bridge path.
- [ ] Persist bounded per-world runtime stdout/stderr diagnostics and surface usable references without secrets. Source and redaction/path-safety tests are authored; Desktop compile/test/clippy validation is in progress.
- [ ] Add real-server import-while-running rejection test.
- [x] Add unsupported runtime tuple negative create/import coverage. Protocol-contract and import negative tests are authored and validated in the runtime fence lane.
- [ ] Add runtime-supervisor hard-kill chaos test while daemon remains alive. Production fencing is implemented; a process-level orphan-child regression is the next test milestone.

## Work completed

- Campaign contract read from `implementation/README.md` and this ledger.
- Audit evidence read from `audits/FINAL-AUDIT.md` on `audit/final-integration-report` and `audits/05-runtime-minecraft.md` on `audit/runtime-minecraft`.
- Verified no Agent 6 implementation branch existed remotely before start.
- Created `fix/agent-6-runtime` from `a9736b159d9e9618a3ed8515c20e93f92c1453cb`; that commit contains only implementation ledgers and has required production baseline `b4bab08562cf0eb53763674407375b023e1d0858` as its parent.
- Milestone 1 implementation: `3079775346d5d2109a21106898f21df9bbb588e4` (`fix(runtime): fence imports and canonical runtime compatibility`).
- Added shared `swarm_protocol::RuntimeAdapterSupport` contract synchronized by test with shipped `fabric.mod.json`: Minecraft `~26.1.2`, Fabric Loader `>=0.19.3`, Java `>=25`.
- Desktop canonical world creation now rejects bridge-incompatible provider-valid tuples before canonicalization.
- Existing-world import now rejects bridge-incompatible tuples before publication.
- Existing-world import now requires and holds Minecraft `session.lock` for the full snapshot operation; failure is closed with an explicit stop-Minecraft error.
- Milestone 1 test hardening: `b4a86868f403c76faa732519166a7953409c1486` (`test(runtime): exercise import lock contention cross-process`). The contention regression uses a separate helper process because POSIX record locks are process-scoped.
- Milestone 2 initial implementation: `02e5ef09c7c8e453e20045f966b7bf8b7de7f836`. Fabric now treats authenticated controller startup failure, EOF, socket failure, and controller-heartbeat expiry as ownership loss and invokes save/stop fail-closed. IPC world-info also carries the live Java major for support-contract validation.
- Milestone 2 validated implementation: `e323d7c51225d93e7862a1d2cfc98f652c2849d2` (`fix(runtime): fence Java ownership across supervisor death`). Migration now validates the canonical runtime tuple before reset/launch, persists Java ownership immediately after spawn, and refuses runtime reset while a previous Java PID remains alive.
- The one-shot lifecycle-fence workflow was deleted after successful validation in `9e17b317ae2873c5faaef4fc4277866ad99f0426`; no permanent CI-write mechanism was retained.
- Diagnostics source milestones: `858ddf1de34ecc8adad771ee93c411014871fd9e` and `32d57bbe154b2f0f84470c54404f0186492e0587`. Desktop authority-host stdout/stderr is written to bounded per-world current/rotated files, secret-bearing lines are redacted, world keys are filename-sanitized, and a native command exposes references rather than log contents.
- Initial diagnostics validation run `33581907532` reached sidecar compilation and failed only because the Desktop lockfile required regeneration while the workflow used `--locked`; no Rust compiler diagnostic was produced. Validation lane `33582264761` now refreshes and prints the exact Desktop lock delta before re-running check/test/clippy under `--locked`.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Pre-implementation audit review | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Scope and failure mechanisms confirmed from final and domain audits. |
| Static source/contract review | PASS | `b4a86868f403c76faa732519166a7953409c1486` | Bridge metadata and shared runtime contract are aligned by source and an authored synchronization test. |
| Runtime fence format | PASS | `e323d7c51225d93e7862a1d2cfc98f652c2849d2` / run `33581638299` | `cargo fmt` completed cleanly before the validated implementation commit. |
| Runtime support contract tests | PASS | `e323d7c51225d93e7862a1d2cfc98f652c2849d2` / run `33581638299` | Shared adapter matrix tests passed. |
| IPC controller lease tests | PASS | `e323d7c51225d93e7862a1d2cfc98f652c2849d2` / run `33581638299` | Authenticated bridge controller-liveness tests passed. |
| Runtime process fence tests | PASS | `e323d7c51225d93e7862a1d2cfc98f652c2849d2` / run `33581638299` | Persistent Java process guard unit tests passed. |
| Existing-world import tests | PASS | `e323d7c51225d93e7862a1d2cfc98f652c2849d2` / run `33581638299` | Quiescence and compatibility import tests passed. |
| Clippy touched runtime crates | PASS | `e323d7c51225d93e7862a1d2cfc98f652c2849d2` / run `33581638299` | `swarm-protocol`, `swarm-ipc`, and `swarm-cli` all passed `-D warnings`. |
| Initial Desktop diagnostics validation | INFRA/LOCKFILE FAIL | `3fd0e9af2a797300d8509e2d8d8fb53745b5e549` / run `33581907532` | Sidecars built; Desktop `cargo check --locked` stopped because the checked-in Desktop lockfile wanted regeneration. No compiler error occurred. |
| Local cargo/process validation | NOT RUN | current | Local desktop/terminal connector cannot establish worker identity; GitHub Actions is the executable validation environment for this agent. |

## Required validation before handoff

- [x] format for validated runtime-fence implementation
- [x] clippy/lint for validated runtime-fence implementation
- [x] runtime unit/process fence tests at `e323d7c51225d93e7862a1d2cfc98f652c2849d2`
- [ ] live source import rejection + stopped source success with a real Minecraft server
- [x] supported tuple contract tests
- [ ] real live Minecraft/Fabric acceptance for supported tuple(s)
- [ ] supervisor-death/orphan-Java process-level chaos test
- [ ] diagnostic retention/no-secret Desktop validation
- [ ] exact-head CI/dedicated validation after remaining milestones

## Blockers

- Local terminal bridge is unavailable to this chat because the desktop connector cannot establish worker identity. GitHub repository read/write and GitHub Actions remain available, so source implementation and executable CI validation are continuing remotely.
- Real Minecraft/Fabric acceptance is not yet proven; this is a validation requirement, not yet a declared external blocker.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Downstream: Agent 9 must consume the integrated Agent 6 head.

Known conflict areas: migration/runtime supervisor, Desktop runtime process manager, Fabric bridge.

## Agent final statement

NOT COMPLETE
