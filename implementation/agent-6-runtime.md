# Agent 6 — Minecraft / Runtime Lifecycle

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-6-runtime`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT IMPLEMENTATION SHA: `b4a86868f403c76faa732519166a7953409c1486`

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
- [x] Enforce supported Minecraft version/range and Fabric loader range before canonical create/import. Java constraint is encoded in the same contract and still needs runtime-installer enforcement.
- [x] Prevent unsupported tuples from becoming canonical worlds through Desktop Create and CLI import.
- [x] Make runtime adapter/artifact selection explicit if multiple Minecraft lines are supported. Not applicable in this build: one shipped adapter contract (`~26.1.2`, Fabric `>=0.19.3`, Java `>=25`) is authoritative.
- [ ] Make Fabric/controller liveness part of authority runtime safety.
- [ ] Treat authenticated controller IPC loss as bounded fail-closed save/stop or a securely re-established controller session.
- [ ] Add supervisor heartbeat/lease distinct from daemon authority permit.
- [ ] Before resetting runtime directory, prove previous Java runtime is gone.
- [ ] Add platform process containment where practical without replacing save-first shutdown semantics.
- [ ] Persist bounded per-world runtime stdout/stderr diagnostics and surface usable references without secrets.
- [ ] Add real-server import-while-running rejection test.
- [x] Add unsupported runtime tuple negative create/import coverage. Protocol-contract and import negative tests are authored; Desktop exact-head validation remains pending.
- [ ] Add runtime-supervisor hard-kill chaos test while daemon remains alive.

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

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Pre-implementation audit review | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Scope and failure mechanisms confirmed from final and domain audits. |
| Static source/contract review | PASS | `b4a86868f403c76faa732519166a7953409c1486` | Bridge metadata and shared runtime contract are aligned by source and an authored synchronization test. |
| Local cargo/process validation | NOT RUN | `b4a86868f403c76faa732519166a7953409c1486` | Local desktop/terminal connector cannot establish worker identity; validation must run through available CI until that recovers. |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint
- [ ] runtime unit/process tests
- [ ] live source import rejection + stopped source success
- [ ] supported tuple contract tests
- [ ] real live Minecraft/Fabric acceptance for supported tuple(s)
- [ ] supervisor-death/orphan-Java chaos test
- [ ] diagnostic retention/no-secret test
- [ ] exact-head CI/dedicated validation

## Blockers

- Local terminal bridge is currently unavailable to this chat because the desktop connector cannot establish worker identity. GitHub repository read/write remains available. This does not yet block source implementation, but local build/process validation may require CI unless the terminal bridge recovers.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Downstream: Agent 9 must consume the integrated Agent 6 head.

Known conflict areas: migration/runtime supervisor, Desktop runtime process manager, Fabric bridge.

## Agent final statement

NOT COMPLETE
