# Agent 2 — Protocol Authorization and History

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-2-protocol`

STARTING SHA: `c69cb0a75c82688a91692bcd2ca47efa6827b958`

CURRENT IMPLEMENTATION SHA: `3ece4c8602a8e7a97a3584bd9cb1a00124c02f0d`

INTEGRATED SHA: pending

## Mission

Make canonical signed records semantically safe, not merely cryptographically valid. Enforce current-authority binding, strict history extension, canonical representations, and version fail-closed behavior.

## Findings owned

- FINAL-004 — stale/previous-epoch membership authority acceptance
- FINAL-005 — non-authority WorldConfig writes
- FINAL-025 — protocol-version/canonical semantic validation gaps
- FINAL-026 — canonical representation/history validation gaps

## Audit inputs read

- `implementation/README.md`
- this ledger
- `implementation/agent-1-consensus.md`
- `audits/FINAL-AUDIT.md` from `audit/final-integration-report`
- `audits/01-protocol-core.md` from `audit/protocol-core`
- live Agent 3 ledger at `fix/agent-3-storage`
- Agent 3 direct-history implementation in `crates/swarm-storage/src/integrity.rs`

Auditor 1 maps the owned findings to APC-001 through APC-006: stale-authority membership, non-authority WorldConfig, non-direct/conflicting snapshot history, unsupported protocol versions, provider-hint fingerprint ambiguity, and noncanonical set-like signed collections.

## Dependencies

Dependency gate satisfied:

- Agent 1 validated production SHA: `67493374544d91ad7bbb36be17e9312adb5654f6`
- Agent 1 exact-head validation: run `33619420045` — SUCCESS
- Agent 1 integration merge: `a0e0dec659d0b1eb21f9be34c44730edc6ff3984`
- Agent 2 branch/start integration SHA: `c69cb0a75c82688a91692bcd2ca47efa6827b958`

Agent 1 quorum, committed-membership, fencing, recovery value-locking, and counter-exhaustion semantics are integration invariants and must not be redesigned here.

Agent 3 coordination state observed during implementation:

- live branch `fix/agent-3-storage` head: `03948a37d72112f0c17ba4dced89d92d75ca07f1`
- Agent 3 ledger reports feature-complete storage implementation SHA: `e27a3278dbd8884d1900a05aae21e7a8c4161968`, validation still pending
- Agent 3 storage contract for canonical snapshots is exactly: `snapshot_number + 1`, `sequence + 1`, exact previous manifest hash, with atomic head/generation recheck owned by Agent 3

Agent 2 will mirror that direct-extension rule at protocol/daemon acceptance but does not claim Agent 3's durable head, immutable-slot, cross-process lock, or atomic final-commit work.

## Ownership boundaries

Primary ownership:

- `crates/swarm-protocol`
- `crates/swarm-core`
- semantic acceptance paths in `crates/swarm-cli/src/daemon.rs`
- history-aware protocol tests

Coordinate with Agent 3 when storage APIs enforce the same history invariants. Agent 2 may enforce snapshot direct-parent/exact-sequence semantics at the protocol/daemon acceptance boundary, but does not claim Agent 3-owned atomic final-commit/head-CAS or immutable-slot work as complete.

Do not change quorum membership design established by Agent 1 without documenting and coordinating it.

## Implementation checklist

- [ ] Reject membership records not valid for the currently accepted membership/authority generation.
- [ ] Require direct membership sequence extension and exact previous-membership hash where the protocol requires it.
- [ ] Preserve exact duplicate idempotency while rejecting same-generation conflicts.
- [ ] Bind WorldConfig signer/sender/key to the accepted current authority, not merely any member.
- [ ] Define WorldConfig authority behavior across authority transitions explicitly.
- [ ] Require supported `protocol_version` on every state-bearing signed record before interpretation/persistence.
- [x] Add one semantic validator per record family rather than scattered ad hoc checks.
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

### Start-state reconciliation

- Verified remote `fix/agent-2-protocol` head was still exactly `c69cb0a75c82688a91692bcd2ca47efa6827b958`; no newer legitimate Agent 2 commits existed to preserve.
- Confirmed the branch starts from the post-Agent-1 integration state requested by the campaign coordinator.
- Read the authoritative Agent 1 handoff and the Protocol/Core audit evidence for APC-001 through APC-006.

### Milestone 1 — centralized semantic validation

Implementation head: `3ece4c8602a8e7a97a3584bd9cb1a00124c02f0d`

Changed contracts/files:

- `crates/swarm-protocol/src/semantics.rs`
  - added centralized semantic validators for genesis, snapshot manifests, epoch, descriptors, membership, invites, transfer/lease, join/leave/sleep, runtime compatibility, WorldConfig, membership votes, recovery ballot/vote, and solo branch
  - unsupported `protocol_version` is a semantic error
  - signed membership members require strict peer-id order/uniqueness
  - snapshot entries require strict path order/uniqueness and exact state root
  - runtime compatibility rejects duplicate artifact identity even when `provider_hint` differs, so provider metadata cannot nondeterministically choose a duplicate survivor
- `crates/swarm-protocol/src/root.rs`
  - wired the semantic validator module into the protocol crate
- `crates/swarm-core/src/lib.rs`
  - snapshot/membership/invite/transfer/lease signing and verification now invoke semantic validation before cryptographic acceptance
- `crates/swarm-core/src/protocol_v2.rs`
  - recovery ballot/vote, WorldConfig, and solo-branch signing/verification now invoke semantic validation
  - WorldConfig signing normalizes compatibility/presentation before signing
- `crates/swarm-core/src/lifecycle.rs`
  - join/leave/sleep signing and verification now invoke semantic validation

This milestone intentionally does not modify Agent 1 consensus/quorum code.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Remote branch ancestry/state verification | PASS | `c69cb0a75c82688a91692bcd2ca47efa6827b958` | Exact branch head matched required start; no newer work present. |
| Source/audit semantic contract review | PASS | `3ece4c8602a8e7a97a3584bd9cb1a00124c02f0d` | Static review only; executable CI still pending. |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint with warnings denied where applicable
- [ ] protocol/core unit tests
- [ ] daemon semantic acceptance integration tests
- [ ] stale membership authority regression
- [ ] WorldConfig authorization regression
- [ ] protocol-version negative matrix
- [ ] snapshot history conflict matrix
- [ ] canonicalization determinism/permutation/duplicate tests
- [ ] exact-head CI/dedicated validation

## Blockers

None. The local desktop tunnel is unavailable in this chat, so executable validation will use GitHub Actions; that is not yet a product blocker.

## Remaining work

1. Bind membership and WorldConfig persistence to the current accepted authority/history.
2. Complete protocol-version enforcement for storage-side control records, including epoch persistence.
3. Enforce snapshot direct-parent/exact-sequence history before replication negotiation and align finalization with Agent 3's contract.
4. Finish provider-hint/canonical collection contract and regression matrix.
5. Run exact-head validation and fix all failures.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Required integration order: after Agent 1.

Known conflict areas: `daemon.rs`, protocol record definitions/validators, snapshot acceptance boundary. Preserve Agent 1 consensus safety semantics and defer Agent 3-owned storage atomicity claims.

## Agent final statement

NOT COMPLETE
