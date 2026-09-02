# Agent 1 — Consensus Configuration Safety

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-1-consensus`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH CREATION SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (campaign planning head; production tree remains the declared campaign baseline plus implementation ledgers)

CURRENT HEAD SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` before this ledger-start commit

INTEGRATED SHA: pending

## Mission

Repair the canonical voter-set and authority-safety model so supported partitions and membership transitions cannot create two independently writable canonical histories.

## Findings owned

- FINAL-001 — divergent membership sets can form independent quorums
- FINAL-002 — automatic Solo fallback can race majority recovery
- FINAL-006 — higher recovery rounds are not value preserving
- FINAL-039 — strict monotonic generation/counter exhaustion handling
- FINAL-045 — legacy consensus/test models diverge from production semantics

## Audit inputs read

- `implementation/README.md` from `integration/audit-remediation-v1`
- this ledger
- `audits/FINAL-AUDIT.md` from `audit/final-integration-report`
- `audits/02-authority-consensus.md` from `audit/authority-consensus`

The final audit maps Agent 1 to FINAL-001, FINAL-002, FINAL-006, FINAL-039, and FINAL-045. The authority audit provides the concrete AC-01/02/03/06/07 failure scenarios and required regression classes. AC-05 duplicate-process non-equivocation is coordinated with Agent 3 storage and is not silently claimed as fully owned here.

## Dependencies

Required before starting: none.

This is a Wave 1 agent and starts from campaign base.

Dependency heads consumed: none.

Downstream dependencies:

- Agent 2 Protocol waits for Agent 1 integration.
- Agent 9 Recovery/Wake waits for Agent 1 and Agent 6 integration.

## Ownership boundaries

Primary ownership:

- `crates/swarm-consensus`
- authority/recovery/membership quorum logic in `crates/swarm-cli`
- consensus-linked protocol/storage call sites only as needed
- process/integration tests for elections, leases, recovery, partitions, membership transitions

Do not redesign package providers, Desktop UX, or runtime artifact installation.

## Implementation checklist

- [ ] Define a canonical committed membership generation used by authority quorum calculations.
- [ ] Prevent an uncommitted membership update from immediately redefining the active voter set.
- [ ] Implement safe membership transitions, preferably joint-consensus/old+new quorum semantics or another proven equivalent.
- [ ] Ensure removed/banned stale members cannot continue voting from an obsolete local membership universe.
- [ ] Remove unsafe automatic writable Solo fallback after unclean multi-member quorum loss, or redesign it so it cannot race canonical recovery.
- [ ] Preserve explicitly safe single-member/clean relinquishment semantics.
- [ ] Make higher recovery rounds preserve any previously accepted/certified value for the target generation.
- [ ] Ensure recovery promises/votes cannot equivocate under the production execution model in coordination with Agent 3 storage locking if needed.
- [ ] Replace security-significant saturating next-generation arithmetic with checked fail-closed exhaustion behavior.
- [ ] Add 3-peer and 5-peer divergent-membership partition regression tests.
- [ ] Add minority-old-authority Solo versus majority recovery regression test.
- [ ] Add recovery candidate crash/resume after certificate persistence regression test.
- [ ] Add generation MAX-1/MAX boundary tests.

## Work completed

- Verified there was no pre-existing `fix/agent-1-consensus` branch to preserve.
- Created `fix/agent-1-consensus` from the campaign planning head without changing the declared production baseline.
- Read the required audit sources and extracted the exact safety invariants and reproduction scenarios before production edits.

## Current exact state

What works: the existing code has local epoch/fencing validation, signed recovery ballots/certificates, deterministic election ordering, lease expiry, and same-round promise checks.

Confirmed incomplete at start:

- membership records become locally active without a committed/joint configuration transition;
- quorum calculations use the local latest membership universe;
- multi-member automatic Solo can race majority recovery;
- higher recovery rounds may choose a different candidate/value after an earlier certificate exists;
- security-significant generation increments use saturating arithmetic in production paths;
- central simulator/test helpers do not model distributed membership/authority divergence.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Audit/source review only | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | No production edits yet. Local terminal connector is unavailable due caller-identity rejection, so validation will use repository/CI evidence unless that connector recovers. |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint for affected Rust crates
- [ ] unit tests
- [ ] consensus tests
- [ ] process-level 3-peer recovery tests
- [ ] process-level 5-peer divergent-membership partition tests
- [ ] Solo/recovery race test
- [ ] recovery higher-round value-preservation test
- [ ] exact-head CI or dedicated validation

## Blockers

No implementation blocker at start. Local checkout execution is currently unavailable because the desktop/local repository connector rejects this chat with `CALLER_IDENTITY_REQUIRED`; GitHub read/write access is available.

## Remaining work

All production implementation and regression-test checklist items remain.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Required integration order: before Agent 2; before Agent 9 together with Agent 6.

Known conflict areas: `crates/swarm-cli/src/daemon.rs`, consensus/recovery protocol state, membership persistence semantics.

Post-merge validation required: full authority/recovery process suite.

## Agent final statement

NOT COMPLETE
