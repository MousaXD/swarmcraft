# Agent 9 — Recovery / Wake Product Completion

## Status

STATUS: BLOCKED ON AGENTS 1 + 6

BRANCH: `fix/agent-9-recovery-wake`

STARTING SHA: newest `integration/audit-remediation-v1` head after Agents 1 and 6 are integrated

CURRENT HEAD SHA: pending

INTEGRATED SHA: pending

## Mission

Complete the supported multiplayer recovery and wake product contract without weakening the canonical-history safety model established by Agent 1 or the runtime lifecycle guarantees established by Agent 6.

## Findings owned

- FINAL-007 — automatic recovery candidacy/host capability problem
- FINAL-023 — required multiplayer crash/recovery product gap
- FINAL-024 — multi-member sleeping-world wake gap

Read `audits/FINAL-AUDIT.md`, Auditor 2 Authority/Consensus, Auditor 5 Runtime, Auditor 10 Adversarial E2E, Agent 1 ledger, and Agent 6 ledger before editing.

## Dependencies

Required before starting:

- Agent 1 integrated exact head
- Agent 6 integrated exact head

Start from the newest integration head containing both.

## Ownership boundaries

Primary ownership:

- recovery candidate eligibility integration
- multi-member sleep/wake transition protocol
- recovery/wake supervisor behavior
- product topology policy and process acceptance

Do not weaken quorum to one-of-two merely to make a two-player demo green.

## Implementation checklist

- [ ] Explicitly specify supported voter topology for automatic crash recovery.
- [ ] Preserve fail-closed behavior for unsafe two-voter crash topology unless a genuinely safe new proof mechanism is implemented.
- [ ] Distinguish storage voters from host candidates.
- [ ] Require fresh authenticated host capability for automatic authority candidacy, including runtime/mod compatibility and conflict-free readiness.
- [ ] Prevent a storage-only or runtime-incompatible deterministic winner from wedging the world as accepted authority.
- [ ] Define a sleep-record-bound quorum wake proof anchored to exact durable sleep record/canonical snapshot.
- [ ] Advance authority generation/fencing safely on wake.
- [ ] Reject stale/competing wake attempts.
- [ ] Define behavior for all hosts stopped then restarted.
- [ ] Ensure wake restores exact canonical snapshot then launches one authority runtime.
- [ ] Prove stale peers remain fenced after wake/recovery.
- [ ] Add successor failure tests at meaningful persistence/runtime boundaries.
- [ ] Add 3-peer or larger real process wake/recovery acceptance.
- [ ] Make two-voter UX/status semantics explicit for Agent 7.

## Work completed

None yet.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| None yet | - | - | - |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint
- [ ] host-ready candidate election test
- [ ] storage-only lowest-ID candidate does not become unusable authority
- [ ] safe multi-member sleep/wake process test
- [ ] stale peer fencing after wake
- [ ] successor crash/recovery tests
- [ ] documented two-voter behavior tests
- [ ] exact-head CI/dedicated validation

## Blockers

Agent 1 and Agent 6 must be integrated first.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Required integration order: after Agents 1 and 6.

Known conflict areas: daemon recovery loop, migration supervisor, host readiness, sleep record handling.

## Agent final statement

BLOCKED
