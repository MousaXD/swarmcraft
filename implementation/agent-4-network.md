# Agent 4 — Network Authentication and Privacy

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-4-network`

CAMPAIGN PRODUCTION BASE SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH SEED SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (campaign ledger commit only; production tree matches the campaign base)

CURRENT HEAD SHA: continuing from remote head; authorization and connection-bound handshake milestones are committed and validated

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
- [x] Ensure removed/banned members lose metadata access through inbound canonical request handling.
- [ ] Anchor discovery announcements to verifiable canonical world authority/authorization, not merely self-signed announcer identity.
- [ ] Add per-peer and global connection/request admission limits, separate for unauthenticated/authenticated traffic.
- [ ] Specify/enforce friend presence privacy policy.
- [ ] Specify invite replay/reuse semantics and test them.
- [ ] Reclassify DNS invite targets after resolution according to scope policy.
- [x] Add captured-proof replay test proving a different transport cannot reuse a valid application proof.
- [ ] Add stranger/removed/banned/current-member authorization matrix tests.
- [ ] Add malicious discovery provider claiming another world ID test.
- [ ] Add hostile-load/admission regression tests.
- [ ] Harden proactive world pushes against removed/banned/key-mismatched members.

## Work completed

- Verified the authoritative Agent 4 ledger is `implementation/agent-4-network.md`; the requested `implementation/agent-4-consensus.md` does not exist in the campaign plan.
- Verified the branch production baseline is `b4bab08562cf0eb53763674407375b023e1d0858`; branch seed `a9736b159d9e9618a3ed8515c20e93f92c1453cb` adds only implementation ledgers.
- Read all required audit sources and extracted the concrete trust-boundary and regression requirements for FINAL-012/013/028/029/030/040.
- Added exhaustive `WireRequest::membership_world_id()` classification. Adding a new wire variant now requires an explicit membership decision at compile time.
- Added one fail-closed daemon membership gate before canonical request dispatch. `JoinRequest` remains the intentional pre-membership path; discovery and Ping remain outside canonical-world authorization.
- Hardened membership authorization so banned members are rejected and descriptor public keys must derive the authenticated peer ID.
- Closed FINAL-013 inbound production path for `WorldDescriptor`, `WorldStatus`, `HostCapability`, and all other canonical world requests.
- Replaced reusable application hello authentication with receiver-generated connection challenges and a fresh Ed25519 application-key proof bound to the challenge, local transport peer, remote transport peer, and the exact live libp2p `ConnectionId`.
- Authentication state is now connection-specific and is cleared on disconnect/replacement. Legacy standalone `Hello` requests no longer establish authentication.
- Added a three-transport replay regression: a proof captured on B→A is rejected when replayed from C→A.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Branch/baseline compare | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Integration seed is exactly one documentation-only commit ahead of campaign production base. |
| `cargo fmt --all -- --check` | PASS | `8dd7d685e2295a561bc5c1958786bd77a6829815` | Authorization milestone GitHub Actions validation runner. |
| `cargo check -p swarm-network -p swarm-cli --all-targets --locked` | PASS | `8dd7d685e2295a561bc5c1958786bd77a6829815` | Affected crates compile after fail-closed request gate. |
| `cargo test -p swarm-network --locked` | PASS | `8dd7d685e2295a561bc5c1958786bd77a6829815` | Full network crate suite green. |
| `cargo fmt --all -- --check` | PASS | `c2ac38c0a94156285fa841fdfdd382a269940a03` | Connection-bound handshake milestone. |
| `cargo check -p swarm-network -p swarm-cli --all-targets --locked` | PASS | `c2ac38c0a94156285fa841fdfdd382a269940a03` | Handshake protocol, network node, daemon, discovery constructors and tests compile together. |
| `cargo test -p swarm-network --locked` | PASS | `c2ac38c0a94156285fa841fdfdd382a269940a03` | Full network suite including captured-proof replay regression. |

## Required validation before handoff

- [x] format for authorization milestone
- [x] format for handshake milestone
- [ ] clippy/lint
- [x] network unit/integration tests for authorization milestone
- [x] captured hello/proof replay rejection
- [ ] world request authorization matrix regression test
- [ ] private-world confidentiality regression
- [ ] discovery unauthorized-signer regression
- [ ] hostile-load admission test
- [ ] network soak/reconnect tests remain green after all changes
- [ ] exact-head CI/dedicated validation

## Blockers

- Local workspace terminal connector is unavailable because it cannot establish this chat's extension identity. GitHub repository read/write access is available; executable validation is being performed with branch-scoped, self-cleaning GitHub Actions runners.

## Remaining work

Discovery authority proof, admission/rate limiting, friend-presence privacy, invite/DNS hardening, proactive world-push hardening, dedicated authorization/privacy regressions, soak/reconnect validation, clippy, and exact-head CI remain.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: network event authentication, daemon request handling, discovery records.

## Agent final statement

NOT COMPLETE
