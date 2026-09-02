# Agent 3 — Storage Transactional Integrity

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-3-storage`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT HEAD SHA: `b4bab08562cf0eb53763674407375b023e1d0858` (campaign base; ledger-only start commit advances branch head)

INTEGRATED SHA: pending

## Mission

Make canonical storage history immutable, crash-detectable, generation-fenced, portable across supported filesystems, and safe under multiple local processes.

## Findings owned

- FINAL-008 — duplicate local daemons can equivocate recovery/control state
- FINAL-009 — missing newest manifest can silently roll head backward
- FINAL-010 — committed snapshot number can be replaced and commit is not atomically fenced
- FINAL-011 — portable path aliases can collapse on case-insensitive filesystems
- FINAL-027 — general restore/durability/namespace integrity gaps
- FINAL-041 — crash debris and legacy unbounded blob read

Read before editing:

- `audits/FINAL-AUDIT.md` from `audit/final-integration-report`
- `audits/03-storage-recovery.md` from `audit/storage-recovery`
- `implementation/agent-1-consensus.md` and `implementation/agent-2-protocol.md` for the shared authority-generation/direct-parent boundary

## Dependencies

Required before starting: none.

Coordinate semantics with Agent 1 for authority generation and Agent 2 for direct-parent history rules. Agent 3 owns storage-side atomicity/non-equivocation, not quorum policy or protocol authorization design.

## Ownership boundaries

Primary ownership:

- `crates/swarm-storage`
- storage-facing migration/checkpoint call sites as needed
- filesystem/crash/recovery tests

Do not redesign authority election policy.

## Implementation checklist

- [ ] Introduce durable per-world canonical head record with exact snapshot number and manifest hash.
- [ ] Publish blobs, immutable manifest, then atomically update/sync canonical head in documented order.
- [ ] Detect missing/mismatched head manifest and fail closed instead of silently rolling back.
- [ ] Ensure snapshot numbers cannot be reused after head loss.
- [ ] Make committed numbered manifests create-only/immutable; allow only exact idempotent duplicate if desired.
- [ ] Add expected-parent/head comparison to commit path.
- [ ] Add OS-backed per-world control lock around recovery/control read-validate-write transactions.
- [ ] Ensure duplicate daemons sharing a data root cannot both emit conflicting accepted promises.
- [ ] Add a fenced snapshot commit primitive binding expected epoch/fencing generation and expected current head.
- [ ] Define portable path collision identity for case-folding, trailing dot/space, reserved names, and Unicode policy.
- [ ] Reject path aliases before publication/restore.
- [ ] Make general restore directory-transactional or persist an explicit incomplete marker/recovery protocol.
- [ ] Sync durable metadata deletions consistently.
- [ ] Document/implement Windows namespace durability guarantees or fail-closed detection.
- [ ] Validate `load_snapshot(world, number)` embedded namespace/number.
- [ ] Add conservative stale-temp cleanup or diagnostics.
- [ ] Remove/deprecate unbounded legacy blob decompression helper or make it bounded.

## Work completed

- Read campaign README, Agent 3 ledger, Agent 1/2 coordination ledgers, final audit, and full Storage/Recovery audit.
- Verified branch is created from the exact campaign base and no prerequisite integration blocks Agent 3.
- Mapped affected storage modules and current positive controls so bounded streaming verification, publication pins, and GC ordering remain intact.

## Current exact state

What works: existing blob integrity, bounded streaming verification/restore, durable publication pin ownership, mark-before-sweep retention, corrupt-manifest fail-closed behavior, and import staging controls remain the baseline to preserve.

Incomplete: all implementation checklist items remain open at this first ledger milestone.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Source/audit prerequisite review | PASS | `b4bab08562cf0eb53763674407375b023e1d0858` | No code changed yet. |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint
- [ ] storage unit/integration tests
- [ ] missing-head rollback regression
- [ ] immutable snapshot-number regression
- [ ] concurrent two-process recovery promise test
- [ ] fenced commit race test
- [ ] portable-path collision tests
- [ ] restore crash/incomplete-marker tests
- [ ] exact-head CI/dedicated validation

## Blockers

None at implementation start. Local workstation execution is unavailable in this chat; GitHub Actions will be used for executable exact-head validation after pushed milestones.

## Remaining work

All implementation checklist items.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: snapshot commit APIs, control locks, `daemon.rs`/migration call sites consumed by Agents 1/2/6.

## Agent final statement

NOT COMPLETE
