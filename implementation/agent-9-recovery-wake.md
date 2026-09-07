# Agent 9 — Recovery / Wake Product Completion

## Status

STATUS: READY FOR INTEGRATION

BRANCH: `fix/agent-9-recovery-wake`

STARTING SHA: `5911dce8eb13d71ad52d670129c4ddb955137e20`

VALIDATED IMPLEMENTATION SHA: `4ba278223a752fa2eb3de60cb714d6fd5687d886`

CURRENT HEAD SHA: this ledger commit

INTEGRATED SHA: pending

## Mission

Complete the supported multiplayer recovery and wake product contract without weakening the canonical-history safety model established by Agent 1 or the runtime lifecycle guarantees established by Agent 6.

## Findings owned

- FINAL-007 — automatic recovery candidacy/host capability problem
- FINAL-023 — required multiplayer crash/recovery product gap
- FINAL-024 — multi-member sleeping-world wake gap

## Dependencies

Satisfied before implementation:

- Agent 1 consensus remediation was integrated.
- Agent 6 runtime lifecycle remediation was integrated.

## Implementation checklist

- [x] Explicitly specify supported voter topology for automatic crash recovery.
- [x] Preserve fail-closed behavior for unsafe two-voter crash topology.
- [x] Distinguish storage voters from host candidates.
- [x] Require fresh authenticated host capability for automatic authority candidacy, including runtime/mod compatibility and conflict-free readiness.
- [x] Prevent a storage-only or runtime-incompatible deterministic winner from wedging the world as accepted authority.
- [x] Bind quorum wake to the exact signed durable sleep record and canonical snapshot.
- [x] Advance authority epoch and fencing token safely on wake.
- [x] Reject stale, mismatched, corrupt, or competing wake state.
- [x] Define all-hosts-stopped/restarted behavior.
- [x] Restore the exact canonical snapshot and launch exactly one authority runtime.
- [x] Keep stale peers fenced after recovery and wake.
- [x] Cover successor failure after recovery certificate formation.
- [x] Add real three-daemon process recovery and wake acceptance.
- [x] Document the fail-closed two-voter topology and storage-voter/host-candidate distinction.

## Work completed

- Recovery election now separates membership voters from eligible runtime hosts.
- A recovery candidate must present a fresh, authenticated, exact-world host-capability observation with a matching runtime fingerprint, runtime readiness, server-mod readiness, and no conflict.
- A voter requires both fresh candidate world status and host capability before signing.
- Sleep is fail closed: a missing record is awake; an unreadable, corrupt, stale, or mismatched record cannot authorize recovery.
- A local wake request can start recovery only from the exact signed sleep generation, epoch, fence, authority key, and latest canonical snapshot.
- Certified wake uses a distinct epoch reason, advances epoch/fencing, clears the sleep record and wake intent only after durable certification, and maps back to the `WorldWake` migration trigger.
- Sleeping voters continue polling and retain the authenticated observations needed to validate a ready wake candidate without becoming writable.
- Partition fixtures now provision truthful host readiness instead of weakening candidate validation.
- `recovery_successor_dies` now builds canonical bootstrap state in the storage-required order and proves recovery-value locking.
- `three_daemon_recovery` proves a storage-only lower-ID voter does not win, a host-ready successor does, stale authority is fenced and resynchronized, all hosts can sleep, and a surviving majority can perform an exact-bound quorum wake.
- `docs/HOST_READINESS.md` specifies supported three-or-more-voter recovery and explicitly retains fail-closed behavior for a two-voter crash.

## Validation

| Validation | Result | Exact SHA | Evidence |
|---|---|---|---|
| Release version guard | PASS | `4ba278223a752fa2eb3de60cb714d6fd5687d886` | GitHub Actions run `34168755680` |
| Full CI matrix | PASS | `4ba278223a752fa2eb3de60cb714d6fd5687d886` | GitHub Actions run `34168755678` |
| Rust tests — Ubuntu/macOS/Windows | PASS | same | Jobs `101884947415`, `101884947433`, `101884947491` |
| Process-level acceptance | PASS | same | Job `101884947495`; includes real three-daemon crash recovery and quorum wake |
| Desktop packages — Linux/Windows/macOS x86_64/macOS arm64 | PASS | same | Jobs `101884947550`, `101884947393`, `101884947502`, `101884947395` |
| QUIC impairment, fuzz, dependency audit, Fabric server mod | PASS | same | Run `34168755678` |

The ledger-only commit is required to receive one more exact-head CI and release-guard pass before merge.

## Remaining work

- Run the exact-head workflows for this ledger commit.
- Merge this exact head into `integration/audit-remediation-v1` after both workflows pass.
- Record the resulting integration merge SHA in the campaign integration ledger.

## Handoff

READY FOR INTEGRATION: YES

Validated implementation head: `4ba278223a752fa2eb3de60cb714d6fd5687d886`

Required integration order: after Agents 1 and 6 (satisfied).

Known conflict areas: daemon recovery loop, migration supervisor, host readiness, sleep-record handling.

## Agent final statement

READY FOR INTEGRATION
