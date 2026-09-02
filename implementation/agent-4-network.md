# Agent 4 — Network Authentication and Privacy

## Status

STATUS: NOT STARTED

BRANCH: `fix/agent-4-network`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT HEAD SHA: pending

INTEGRATED SHA: pending

## Mission

Repair the network trust boundary so a peer proves current possession of its application key on the live transport connection and world-scoped data is disclosed only to authorized peers.

## Findings owned

- FINAL-012 — replayable application peer hello/identity impersonation
- FINAL-013 — unauthorised private-world metadata access
- FINAL-028 — discovery authority authenticity gap
- FINAL-029 — request/connection admission and rate limiting gap
- FINAL-030 — friend presence privacy semantics
- FINAL-040 — invite/DNS/address/privacy hardening assigned by final audit

Read `audits/FINAL-AUDIT.md`, Auditor 4 Network/Discovery, and Auditor 7 Security before editing.

## Dependencies

Required before starting: none.

Coordinate any membership-generation authorization lookup changes with Agent 1/2 after integration.

## Ownership boundaries

Primary ownership:

- `crates/swarm-network`
- discovery protocol/networking
- network-facing daemon authorization helpers and request matrix
- invite connectivity validation
- hostile-peer tests

Do not change canonical membership election semantics.

## Implementation checklist

- [ ] Replace reusable one-message application hello authentication with connection-bound proof of possession.
- [ ] Include fresh receiver challenge/nonce and bind proof to both sides of live transport context.
- [ ] Ensure replay of a captured valid hello over a different transport identity/connection fails.
- [ ] Do not reuse authenticated application identity across replacement connections without fresh proof.
- [ ] Build an exhaustive authorization matrix for every world-scoped `WireRequest`.
- [ ] Require current, non-banned membership for WorldDescriptor, WorldStatus, HostCapability and other canonical metadata unless a narrowly scoped pre-membership protocol explicitly applies.
- [ ] Ensure removed/banned members lose metadata access.
- [ ] Anchor discovery announcements to verifiable canonical world authority/authorization, not merely self-signed announcer identity.
- [ ] Add per-peer and global connection/request admission limits, separate for unauthenticated/authenticated traffic.
- [ ] Specify/enforce friend presence privacy policy.
- [ ] Specify invite replay/reuse semantics and test them.
- [ ] Reclassify DNS invite targets after resolution according to scope policy.
- [ ] Add three-peer captured-hello replay test proving no private snapshot/metadata disclosure.
- [ ] Add stranger/removed/banned/current-member authorization matrix tests.
- [ ] Add malicious discovery provider claiming another world ID test.
- [ ] Add hostile-load/admission regression tests.

## Work completed

None yet.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| None yet | - | - | - |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint
- [ ] network unit/integration tests
- [ ] captured hello replay rejection
- [ ] world request authorization matrix
- [ ] private-world confidentiality regression
- [ ] discovery unauthorized-signer regression
- [ ] hostile-load admission test
- [ ] network soak/reconnect tests remain green
- [ ] exact-head CI/dedicated validation

## Blockers

None at campaign start.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: network event authentication, daemon request handling, discovery records.

## Agent final statement

NOT COMPLETE
