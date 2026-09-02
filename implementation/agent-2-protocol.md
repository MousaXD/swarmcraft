# Agent 2 — Protocol Authorization and History

## Status

STATUS: BLOCKED ON AGENT 1

BRANCH: `fix/agent-2-protocol`

STARTING SHA: must be newest `integration/audit-remediation-v1` head after Agent 1 integration

CURRENT HEAD SHA: pending

INTEGRATED SHA: pending

## Mission

Make canonical signed records semantically safe, not merely cryptographically valid. Enforce current-authority binding, strict history extension, canonical representations, and version fail-closed behavior.

## Findings owned

- FINAL-004 — stale/previous-epoch membership authority acceptance
- FINAL-005 — non-authority WorldConfig writes
- FINAL-025 — protocol-version/canonical semantic validation gaps
- FINAL-026 — canonical representation/history validation gaps

Read `audits/FINAL-AUDIT.md`, Auditor 1 Protocol/Core, and Agent 1 ledger before editing.

## Dependencies

Required before starting:

- Agent 1 status: READY FOR INTEGRATION and integrated
- Start from the exact integration head containing Agent 1

## Ownership boundaries

Primary ownership:

- `crates/swarm-protocol`
- `crates/swarm-core`
- semantic acceptance paths in `crates/swarm-cli/src/daemon.rs`
- history-aware protocol tests

Coordinate with Agent 3 when storage APIs enforce the same history invariants.

Do not change quorum membership design established by Agent 1 without documenting and coordinating it.

## Implementation checklist

- [ ] Reject membership records not valid for the currently accepted membership/authority generation.
- [ ] Require direct membership sequence extension and exact previous-membership hash where the protocol requires it.
- [ ] Preserve exact duplicate idempotency while rejecting same-generation conflicts.
- [ ] Bind WorldConfig signer/sender/key to the accepted current authority, not merely any member.
- [ ] Define WorldConfig authority behavior across authority transitions explicitly.
- [ ] Require supported `protocol_version` on every state-bearing signed record before interpretation/persistence.
- [ ] Add one semantic validator per record family rather than scattered ad hoc checks.
- [ ] Enforce direct-parent/exact-sequence rules for replicated snapshot acceptance in coordination with Agent 3.
- [ ] Reject same-sequence conflicting manifests and skipped parents.
- [ ] Resolve provider-hint canonicality contract and make normalization deterministic.
- [ ] Define canonical order/uniqueness for set-like signed collections or reject noncanonical forms.
- [ ] Add stale-authority membership replay test.
- [ ] Add non-authority WorldConfig test.
- [ ] Add unsupported-version matrix for all canonical/control record families.
- [ ] Add snapshot wrong-parent/jump/same-sequence conflict tests.
- [ ] Add canonical collection permutation/duplicate tests.

## Work completed

None yet.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| None yet | - | - | - |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint
- [ ] protocol/core unit tests
- [ ] daemon semantic acceptance integration tests
- [ ] stale membership authority regression
- [ ] WorldConfig authorization regression
- [ ] protocol-version negative matrix
- [ ] snapshot history conflict matrix
- [ ] canonicalization determinism tests
- [ ] exact-head CI/dedicated validation

## Blockers

Agent 1 must first define/integrate canonical membership generation semantics.

## Remaining work

All implementation checklist items.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Required integration order: after Agent 1.

Known conflict areas: `daemon.rs`, protocol record definitions/validators, snapshot acceptance boundary.

## Agent final statement

BLOCKED
