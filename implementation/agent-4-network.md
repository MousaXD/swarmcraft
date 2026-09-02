# Agent 4 — Network Authentication and Privacy

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-4-network`

CAMPAIGN PRODUCTION BASE SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH SEED SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (campaign ledger commit only; production tree matches the campaign base)

CURRENT HEAD SHA: `c2ac38c0a94156285fa841fdfdd382a269940a03` production milestone; this ledger commit follows it

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

Dependency heads consumed: none.

Audit sources read before production edits:

- `audits/FINAL-AUDIT.md` from `audit/final-integration-report`
- `audits/04-network-discovery.md` from `audit/network-discovery`
- `audits/07-security.md` from `audit/security`

## Ownership boundaries

Primary ownership:

- `crates/swarm-network`
- discovery protocol/networking
- network-facing daemon authorization helpers and request matrix
- invite connectivity validation
- hostile-peer tests

Do not change canonical membership election semantics.

## Implementation checklist

- [x] Replace reusable one-message application hello authentication with connection-bound proof of possession.
- [x] Include fresh receiver challenge/nonce and bind proof to both sides of live transport context.
- [x] Ensure replay of a captured valid hello over a different transport identity/connection fails.
- [x] Do not reuse authenticated application identity across replacement connections without fresh proof.
- [x] Build an exhaustive authorization matrix for every world-scoped `WireRequest`.
- [x] Require current, non-banned membership for WorldDescriptor, WorldStatus, HostCapability and other canonical metadata unless a narrowly scoped pre-membership protocol explicitly applies.
- [x] Ensure removed/banned members lose metadata access.
- [ ] Anchor discovery announcements to verifiable canonical world authority/authorization, not merely self-signed announcer identity.
- [ ] Add per-peer and global connection/request admission limits, separate for unauthenticated/authenticated traffic.
- [ ] Specify/enforce friend presence privacy policy.
- [ ] Specify invite replay/reuse semantics and test them.
- [ ] Reclassify DNS invite targets after resolution according to scope policy.
- [x] Add three-peer captured-hello replay cryptographic regression proving a captured B→A connection proof cannot authenticate C's transport as B.
- [ ] Add end-to-end private snapshot/metadata non-disclosure regression around the replay case.
- [ ] Add stranger/removed/banned/current-member authorization matrix tests.
- [ ] Add malicious discovery provider claiming another world ID test.
- [ ] Add hostile-load/admission regression tests.

## Work completed

- Verified the authoritative Agent 4 ledger is `implementation/agent-4-network.md`; the requested `implementation/agent-4-consensus.md` does not exist in the campaign plan.
- Verified the branch production baseline is `b4bab08562cf0eb53763674407375b023e1d0858`; branch seed `a9736b159d9e9618a3ed8515c20e93f92c1453cb` adds only implementation ledgers.
- Read all required audit sources and extracted the concrete trust-boundary and regression requirements for FINAL-012/013/028/029/030/040.
- Added exhaustive `WireRequest::membership_world_id()` classification. Adding a new wire variant now requires an explicit membership decision at compile time.
- Added one fail-closed daemon membership gate before canonical request dispatch. `JoinRequest` remains the intentional pre-membership path; discovery and Ping remain outside canonical-world authorization.
- Hardened membership authorization so banned members are rejected and descriptor public keys must derive the authenticated peer ID.
- Closed FINAL-013 production handler path for `WorldDescriptor`, `WorldStatus`, `HostCapability`, and all other canonical world requests.
- Replaced reusable `PeerHelloV1` authentication with receiver-challenged `PeerHelloProofV1`: the application key signs the static hello, a fresh receiver challenge, claimant transport identity, and receiver transport identity under a separate domain.
- Bound authenticated application identities to the exact libp2p request-response `ConnectionId`; proof challenges are single-use and replacement connections clear authentication and require a fresh challenge/proof exchange.
- Legacy `WireRequest::Hello` is no longer accepted as authentication. Both replication and discovery network nodes reject it with a connection-proof-required response.
- `SwarmNode` and `DiscoveryNode` now receive an in-process application signing key clone and self-test it against their advertised hello before networking starts; the signing key is not serialized by networking.
- Added bounded wire encoding for transport peer identifiers used in proofs.
- Added a three-transport replay regression: a valid B→A captured proof verifies for B/A but is rejected when replayed as C→A because the transport binding differs.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Branch/baseline compare | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Integration seed is exactly one documentation-only commit ahead of campaign production base. |
| `cargo fmt --all -- --check` | PASS | `8dd7d685e2295a561bc5c1958786bd77a6829815` | Authorization milestone GitHub Actions validation runner. |
| `cargo check -p swarm-network -p swarm-cli --all-targets --locked` | PASS | `8dd7d685e2295a561bc5c1958786bd77a6829815` | Authorization milestone affected crates compile. |
| `cargo test -p swarm-network --locked` | PASS | `8dd7d685e2295a561bc5c1958786bd77a6829815` | Authorization milestone full network crate suite green. |
| `cargo fmt --all -- --check` | PASS | `c2ac38c0a94156285fa841fdfdd382a269940a03` | Connection-bound authentication milestone. |
| `cargo check -p swarm-network -p swarm-cli --all-targets --locked` | PASS | `c2ac38c0a94156285fa841fdfdd382a269940a03` | Protocol extensions, both network nodes, daemon/CLI constructors, examples and all affected test targets compile. |
| `cargo test -p swarm-network --locked` | PASS | `c2ac38c0a94156285fa841fdfdd382a269940a03` | Full network suite includes QUIC mutual authentication and captured-proof transport-replay rejection. |

## Required validation before handoff

- [x] format for completed milestones
- [ ] clippy/lint
- [x] network unit/integration tests for completed milestones
- [x] captured connection-proof replay rejection
- [ ] world request authorization matrix regression test
- [ ] private-world confidentiality regression
- [ ] discovery unauthorized-signer regression
- [ ] hostile-load admission test
- [ ] network soak/reconnect tests remain green on final head
- [ ] exact-head CI/dedicated validation

## Blockers

- Local workspace terminal connector is currently unavailable because it cannot establish this chat's extension identity. GitHub repository read/write access is available; executable validation is being performed with branch-scoped, self-cleaning GitHub Actions runners.

## Remaining work

Dedicated authorization/privacy regressions, discovery authority proof, admission/rate limiting, friend-presence privacy, invite replay/reuse semantics, DNS post-resolution scope hardening, soak/reconnect validation, clippy, and exact-head CI remain.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: network event authentication, daemon request handling, discovery records.

## Agent final statement

NOT COMPLETE
