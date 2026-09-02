# Agent 4 — Network Authentication and Privacy

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-4-network`

CAMPAIGN PRODUCTION BASE SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH SEED SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (campaign ledger commit only; production tree matches the campaign base)

CURRENT HEAD SHA: `af48ed80f78beec7a2866a90ce6de35e4cfc86a8` validated production milestone; this ledger commit follows it

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
- [x] Add per-peer and global connection/request admission limits, separate for unauthenticated/authenticated traffic.
- [x] Specify/enforce friend presence privacy policy: only locally accepted friends receive requester-specific presence rendezvous and responses.
- [x] Specify invite replay/reuse semantics and test them.
- [x] Reclassify DNS invite targets after resolution according to scope policy.
- [x] Add captured-proof replay test proving a different transport cannot reuse a valid application proof.
- [x] Add stranger/removed/banned/current-member authorization matrix tests.
- [ ] Add malicious discovery provider claiming another world ID test.
- [x] Add hostile-load/admission regression tests beyond deterministic budget/controller units.
- [x] Harden proactive world pushes against removed/banned/key-mismatched members.

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
- Added bounded network admission shared by primary and discovery nodes: 64 active application connections, separate per-peer authenticated/pre-auth request windows, and separate global authenticated/pre-auth request budgets. Replacement connections for an already known transport are not blocked by the application cap.
- Added fail-closed `RATE_LIMITED` responses before application dispatch when a request budget is exceeded, while retaining wire-size validation and connection-bound authentication.
- Changed friend presence discovery from a global `peer` rendezvous key to requester-specific `peer + accepted-friend` keys. The discovery service only advertises presence to locally accepted friends, withdraws removed-friend rendezvous entries, and returns no presence to authenticated non-friends.
- Hardened proactive `push_known_worlds` so removed, banned, and application-ID/public-key-mismatched members cannot receive world state merely because a stale descriptor entry exists.
- Specified invitations as reusable bearer capabilities rather than hidden single-use tokens. Token uniqueness comes from the signed nonce; reuse remains valid until expiry while the signer remains current authority. Creation/decoding/join validation now enforce a maximum 24-hour lifetime with checked timestamp arithmetic instead of saturating effectively-unbounded expiry.
- Added immediate pre-dial DNS re-resolution for invite DNS/DNS4/DNS6 hints and apply the same public-scope policy to every resolved answer. DNS names cannot silently rebind an Internet-looking invite hint to loopback/private/link-local scope; explicit LAN invites remain represented by literal private-IP multiaddresses.
- Added direct authorization-matrix tests for current member, stranger/removed member, banned member, and peer/public-key mismatch, sharing the same descriptor authorization helper used by proactive synchronization.
- Added a live pre-auth request flood regression that exhausts the unauthenticated request budget, proves bounded rejection, waits for budget recovery, then confirms a valid client can authenticate and complete useful traffic.

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
| `cargo fmt --all -- --check` | PASS | `8993583acefc728754292c47e5337b4fd19d03a2` | Privacy/admission milestone, Actions run `33582401255`. |
| `cargo check -p swarm-network -p swarm-cli --all-targets --locked` | PASS | `8993583acefc728754292c47e5337b4fd19d03a2` | Admission controller, presence privacy, and proactive-push hardening compile together. |
| `cargo test -p swarm-network --locked` | PASS | `8993583acefc728754292c47e5337b4fd19d03a2` | 39 network unit tests plus handshake/input/reconnect suites green; impaired/multi-GiB soak tests remain intentionally ignored in normal crate test. |
| `cargo test -p swarm-cli --bin swarmcraft --locked` | PASS | `8993583acefc728754292c47e5337b4fd19d03a2` | 7 CLI unit tests green after accepted-friend presence changes. |
| `cargo fmt --all -- --check` | PASS | `af48ed80f78beec7a2866a90ce6de35e4cfc86a8` | Invite/DNS hardening, Actions run `33583517471`. |
| `cargo check -p swarm-network -p swarm-cli --all-targets --locked` | PASS | `af48ed80f78beec7a2866a90ce6de35e4cfc86a8` | DNS resolution policy, bounded invite lifetime, authorization matrix and flood regression compile together. |
| `cargo clippy -p swarm-network -p swarm-cli --all-targets --locked -- -D warnings` | PASS | `af48ed80f78beec7a2866a90ce6de35e4cfc86a8` | Strict affected-crate lint gate. |
| `cargo test -p swarm-network --locked` | PASS | `af48ed80f78beec7a2866a90ce6de35e4cfc86a8` | Full network suite including live pre-auth flood/budget-recovery regression. |
| `cargo test -p swarm-cli --bin swarmcraft --locked` | PASS | `af48ed80f78beec7a2866a90ce6de35e4cfc86a8` | Invite reuse/lifetime and daemon authorization-matrix units green. |

## Required validation before handoff

- [x] format for authorization milestone
- [x] format for handshake milestone
- [x] format/check/tests for privacy/admission milestone
- [x] clippy/lint for affected crates through invite/DNS milestone
- [x] network unit/integration tests for authorization milestone
- [x] captured hello/proof replay rejection
- [x] world request authorization matrix regression test
- [x] private-world confidentiality authorization path regression
- [ ] discovery unauthorized-signer regression
- [x] hostile-load admission test
- [x] ordinary hard reconnect test remains green after admission/privacy changes
- [ ] explicitly run impaired reconnect/transfer test and scheduled-scale soak equivalent where practical
- [ ] exact-head CI/dedicated validation after final independent code change

## Blockers

- Local workspace terminal connector is unavailable because it cannot establish this chat's extension identity. GitHub repository read/write access is available; executable validation is being performed with branch-scoped, self-cleaning GitHub Actions runners.
- First-contact discovery authority proof requires a cryptographically verifiable canonical membership/authority chain. Agent 1 has introduced joint membership certificate primitives and daemon activation work on `fix/agent-1-consensus`, but Agent 1 remains `IN PROGRESS`; Agent 2 is explicitly blocked on Agent 1 and has not established the final current-authority/history validator. Agent 4 will not replace that missing chain with creator-only or TOFU discovery semantics that would fail after legitimate authority transfer.

## Remaining work

Strengthen transport-level connection shedding before establishment, explicitly run impaired reconnect/transfer evidence, complete exact-head validation for all independent Agent 4 work, and either consume the finalized Agent 1/2 authority proof contract for FINAL-028 or declare that item dependency-blocked with exact evidence.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: network event authentication, daemon request handling, discovery records and authority-proof integration.

## Agent final statement

NOT COMPLETE
