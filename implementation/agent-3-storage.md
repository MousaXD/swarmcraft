# Agent 3 — Storage Transactional Integrity

## Status

STATUS: NOT STARTED

BRANCH: `fix/agent-3-storage`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT HEAD SHA: pending

INTEGRATED SHA: pending

## Mission

Make canonical storage history immutable, crash-detectable, generation-fenced, portable across supported filesystems, and safe under multiple local processes.

## Findings owned

- FINAL-008 — missing newest manifest can silently roll head backward
- FINAL-009 — committed snapshot number can be replaced
- FINAL-010 — portable path aliases can collapse on case-insensitive filesystems
- FINAL-011 — control/recovery read-check-write transitions are not cross-process serialized
- FINAL-027 — fenced/transactional restore and storage durability issues assigned by final audit
- FINAL-041 — platform durability/control persistence hardening assigned by final audit

Read `audits/FINAL-AUDIT.md` and Auditor 3 Storage/Recovery before editing.

## Dependencies

Required before starting: none.

Coordinate semantics with Agent 1 for authority generation and Agent 2 for direct-parent history rules.

## Ownership boundaries

Primary ownership:

- `crates/swarm-storage`
- storage-facing migration/checkpoint call sites as needed
- filesystem/crash/recovery tests

Do not redesign authority election policy.

## Implementation checklist

- [ ] Introduce a durable per-world canonical head record with exact snapshot number and manifest hash.
- [ ] Publish blobs, immutable manifest, then atomically update/sync canonical head in a documented order.
- [ ] Detect missing/mismatched head manifest and fail closed instead of silently rolling back.
- [ ] Ensure snapshot numbers cannot be reused after head loss.
- [ ] Make committed numbered manifests create-only/immutable; allow only exact idempotent duplicate if desired.
- [ ] Add expected-parent/head comparison to commit path.
- [ ] Add an OS-backed per-world control lock around recovery/control read-validate-write transactions.
- [ ] Ensure duplicate daemons sharing a data root cannot both emit conflicting accepted promises.
- [ ] Add a fenced snapshot commit primitive binding expected epoch/fencing generation and expected current head.
- [ ] Define portable path collision identity for case-folding, trailing dot/space, reserved names, and Unicode policy as required by supported platforms.
- [ ] Reject path aliases before publication/restore.
- [ ] Make general restore directory-transactional or persist an explicit incomplete marker/recovery protocol.
- [ ] Sync durable metadata deletions consistently.
- [ ] Document/implement Windows namespace durability guarantees or fail-closed detection.
- [ ] Validate `load_snapshot(world, number)` embedded namespace/number.
- [ ] Add conservative stale-temp cleanup or diagnostics.
- [ ] Remove/deprecate unbounded legacy blob decompression helper or make it bounded.

## Work completed

None yet.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| None yet | - | - | - |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint
- [ ] storage unit/integration tests
- [ ] missing-head rollback regression
- [ ] immutable snapshot-number regression
- [ ] concurrent two-process recovery promise test
- [ ] fenced commit race test
- [ ] Windows/macOS/Linux portable-path collision tests where feasible
- [ ] restore crash injection tests
- [ ] exact-head CI/dedicated validation

## Blockers

None at campaign start.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: snapshot commit APIs, control locks, `daemon.rs`/migration call sites consumed by Agents 1/2/6.

## Agent final statement

NOT COMPLETE
