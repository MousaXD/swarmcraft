# Agent 3 — Storage Transactional Integrity

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-3-storage`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT IMPLEMENTATION SHA: `e27a3278dbd8884d1900a05aae21e7a8c4161968`

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

- [x] Introduce durable per-world canonical head record with exact snapshot number and manifest hash.
- [x] Publish blobs, immutable manifest, then atomically update/sync canonical head in documented order.
- [x] Detect missing/mismatched head manifest and fail closed instead of silently rolling back.
- [x] Ensure snapshot numbers cannot be reused after head loss.
- [x] Make committed numbered manifests create-only/immutable; exact duplicates are idempotent.
- [x] Add expected-parent/head comparison to commit path.
- [x] Add OS-backed per-world control lock around recovery/control read-validate-write transactions.
- [x] Ensure duplicate daemons sharing a data root cannot both emit conflicting accepted promises.
- [x] Add a fenced snapshot commit primitive binding expected epoch/fencing generation and expected current head.
- [x] Define portable path collision identity for case-folding, trailing dot/space, reserved names, and Unicode policy.
- [x] Reject path aliases before publication/restore.
- [x] Make general restore persist an explicit incomplete marker/recovery protocol.
- [x] Sync durable metadata deletions consistently in remediated control/restore/replica paths.
- [x] Implement Windows fail-closed namespace detection through durable commit intent + required-head/head-target consistency checks.
- [x] Validate `load_snapshot(world, number)` embedded namespace/number.
- [x] Add conservative stale-temp diagnostics without deleting possibly-live writer data.
- [x] Make the legacy blob decompression helper bounded and streaming.

## Work completed

- Added `transaction.rs` with a stable per-world kernel-backed exclusive lock, unique temporary files, durable atomic writes, durable create-once, durable delete, and parent sync helpers.
- Added `integrity.rs` with `CanonicalSnapshotHeadV1`, exact manifest hashes, a durable required-head marker, a durable commit-intent record, one-time legacy head adoption, fail-closed incomplete-commit recovery, orphan detection, direct-parent/sequence checks, and `SnapshotCommitFence`.
- Snapshot publication now verifies blobs and portable paths, takes the GC lock before the world transaction lock, writes durable intent, publishes a create-only manifest, advances the durable head, then releases publication pins.
- Missing/mismatched head targets, orphan manifests above head, conflicting numbered manifests, stale expected heads, and stale epoch/fencing tokens now return explicit storage errors instead of silently selecting older state.
- Added conservative cross-platform path identity: ASCII canonical path policy, ASCII case folding for collision identity, traversal/drive/backslash rejection, trailing dot/space rejection, Windows reserved device-name rejection, and invalid Windows character rejection.
- Restore verifies source blobs before mutation, writes `.swarmcraft-restore-incomplete`, clears/reconstructs the destination without following symlinks, verifies the exact resulting file set, removes the marker only on success, and exposes `discard_incomplete_restore` for explicit recovery.
- Recovery promises/reservations, epoch/control records, world descriptor/membership/pending membership state, background seeding, solo-branch writes, and recovery certificates now serialize through the per-world OS lock where they mutate durable state.
- Replicated blob finalization now syncs the blob directory after deletion/rename transitions.
- `load_snapshot` validates embedded world/number identity and canonical namespace; `latest_snapshot` and `next_snapshot_number` are driven by the durable canonical head rather than directory sorting.
- Legacy `read_blob` now checks encoded size and streams with a 256 MiB hard maximum instead of decompressing unbounded data into memory.
- Added stale transaction-temp diagnostics through `storage_temp_debris`.
- Added unit/integration regressions for immutable snapshot slots, direct-parent history, portable aliases, incomplete restore markers, missing head targets, fencing-token changes, namespace mismatch, stale-temp diagnostics, and concurrent promise/control races.
- Added an integration test that launches two independent test processes against the same storage root and asserts conflicting equal-round recovery promises cannot both be accepted.

## Current exact state

Implementation is feature-complete against the Agent 3 checklist at `e27a3278dbd8884d1900a05aae21e7a8c4161968`. It has not yet earned READY because executable format/clippy/test/CI validation has not been run on this exact head.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Source/audit prerequisite review | PASS | `b4bab08562cf0eb53763674407375b023e1d0858` | Campaign mapping and full referenced audits/dependencies read. |
| Static source/diff review | PASS | `e27a3278dbd8884d1900a05aae21e7a8c4161968` | Branch is 15 commits ahead of exact base with changes confined to Agent 3 storage scope plus ledger. |

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

Local workstation execution remains unavailable in this chat. GitHub Actions will be used for executable validation on the pushed branch/PR. This is not yet a product blocker because the GitHub connection can create the validation vehicle and inspect exact-head runs.

## Remaining work

1. Run format/check/clippy/tests through GitHub Actions on the exact pushed branch.
2. Fix any compiler/test regressions found by CI and rerun until green or a genuine external blocker is proven.
3. Re-audit the final diff and update this ledger to one terminal state.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending exact-head executable validation

Known conflict areas: snapshot commit APIs, control locks, and authority checkpoint call sites consumed by Agents 1/2/6.

## Agent final statement

NOT COMPLETE
