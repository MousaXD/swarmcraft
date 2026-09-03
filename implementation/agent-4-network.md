# Agent 4 — Network Authentication and Privacy

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-4-network`

CAMPAIGN PRODUCTION BASE SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH SEED SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (campaign ledger commit only; production tree matches the campaign base)

CURRENT HEAD SHA: `1a5708bf70119d9da86d963cf0e9941abf76bdba` validated independent Agent 4 production head; ledger-only commits follow it

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

Required before starting: none for independent network/privacy hardening.

FINAL-028 dependency gate is now satisfied. Agent 4 consumed the authoritative integration head `f02bb0d54cb44df67e730f01be4c903e25d670ff`, which contains:

- Agent 1 validated production SHA `67493374544d91ad7bbb36be17e9312adb5654f6` and integration merge `a0e0dec659d0b1eb21f9be34c44730edc6ff3984`.
- Agent 2 validated production SHA `dde75ca4e9f2268bb97f42a716864c3e51f266cb`, exact-head run `33693100794` SUCCESS, and integration merge `6e70a0774d7e021cc57681705ccef4620265ce3d`.
- Composed Agent 4 dependency merge: `6a6bd207c8ae4622ec84b9a28efb8c9e8d7045aa`.

The consumed contracts are canonical committed-membership/joint-consensus safety from Agent 1 plus Agent 2 current-authority, exact direct membership history, canonical semantics, and protocol-version fail-closed validation. No parallel authority model is introduced.

Audit sources read before and during closure:

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
- [ ] Anchor discovery announcements to verifiable canonical world authority/authorization, not merely self-signed announcer identity. Dependency gate satisfied; FINAL-028 implementation in progress.
- [x] Add per-peer and global connection/request admission limits, separate for unauthenticated/authenticated traffic.
- [x] Specify/enforce friend presence privacy policy: only locally accepted friends receive requester-specific presence rendezvous and responses.
- [x] Specify invite replay/reuse semantics and test them.
- [x] Reclassify DNS invite targets after resolution according to scope policy.
- [x] Add captured-proof replay test proving a different transport cannot reuse a valid application proof.
- [x] Add stranger/removed/banned/current-member authorization matrix tests.
- [ ] Add malicious discovery provider claiming another world ID test. Dependency gate satisfied; regression implementation in progress.
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
- Composed libp2p `connection_limits::Behaviour` into both primary and discovery swarms so pending and established connection floods are rejected before long-lived application state is allocated. Primary transport limits cap pending incoming/outgoing work at 32 each, established incoming at 72, total established at 96, and per-transport-peer connections at 2; discovery uses tighter 24/48/64 bounds with the same per-peer cap.
- Added a 10-second receiver-challenge lifetime to both network stacks. Silent peers that occupy an authentication slot without proving the application key are disconnected and their pending/authentication state is cleared.
- Added fail-closed `RATE_LIMITED` responses before application dispatch when a request budget is exceeded, while retaining wire-size validation and connection-bound authentication.
- Changed friend presence discovery from a global `peer` rendezvous key to requester-specific `peer + accepted-friend` keys. The discovery service only advertises presence to locally accepted friends, withdraws removed-friend rendezvous entries, and returns no presence to authenticated non-friends.
- Hardened proactive `push_known_worlds` so removed, banned, and application-ID/public-key-mismatched members cannot receive world state merely because a stale descriptor entry exists.
- Specified invitations as reusable bearer capabilities rather than hidden single-use tokens. Token uniqueness comes from the signed nonce; reuse remains valid until expiry while the signer remains current authority. Creation/decoding/join validation now enforce a maximum 24-hour lifetime with checked timestamp arithmetic instead of saturating effectively-unbounded expiry.
- Added immediate pre-dial DNS re-resolution for invite DNS/DNS4/DNS6 hints and apply the same public-scope policy to every resolved answer. DNS names cannot silently rebind an Internet-looking invite hint to loopback/private/link-local scope; explicit LAN invites remain represented by literal private-IP multiaddresses.
- Added direct authorization-matrix tests for current member, stranger/removed member, banned member, and peer/public-key mismatch, sharing the same descriptor authorization helper used by proactive synchronization.
- Added a live pre-auth request flood regression that exhausts the unauthenticated request budget, proves bounded rejection, waits for budget recovery, then confirms a valid client can authenticate and complete useful traffic.
- Ran a real impaired QUIC restart/resume regression after all independent hardening, using 15ms ± 3ms delay, 0.5% packet loss, 100mbit rate shaping, forced sender restart and lost-ACK recovery. It passed.
- Deliberately did not invent a creator-only, self-signed, or TOFU discovery authority shortcut. Such a shortcut would fail the audited invariant after legitimate authority transfer and would make FINAL-028 look closed without a valid trust chain.

### Dependency composition — Agent 1/2 canonical contracts

Composition merge: `6a6bd207c8ae4622ec84b9a28efb8c9e8d7045aa`

- Merged authoritative integration head `f02bb0d54cb44df67e730f01be4c903e25d670ff` into Agent 4 without discarding the independently validated network/privacy work.
- Resolved exactly three conflict files: `crates/swarm-cli/src/daemon.rs`, `crates/swarm-network/src/lib.rs`, and `crates/swarm-network/src/wire.rs`.
- Preserved Agent 1/2 membership proposal/commit wire ordering and committed-membership delivery before appending Agent 4 connection-bound handshake variants, avoiding a protocol-discriminant rollback.
- Preserved Agent 2 committed-membership delivery to a newly admitted proposal member before Agent 4's current-member/public-key fencing of ordinary canonical payload pushes.
- Preserved Agent 4 authorization-matrix tests and Agent 2 daemon protocol-acceptance tests together.
- FINAL-028 is no longer dependency-blocked; production implementation and fresh composed validation remain.

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
| `cargo fmt --all -- --check` | PASS | `1a5708bf70119d9da86d963cf0e9941abf76bdba` | Transport-level admission/handshake-lifetime milestone. |
| `cargo check -p swarm-network -p swarm-cli --all-targets --locked` | PASS | `1a5708bf70119d9da86d963cf0e9941abf76bdba` | Primary/discovery connection-limit behaviours and challenge expiry compile together. |
| `cargo clippy -p swarm-network -p swarm-cli --all-targets --locked -- -D warnings` | PASS | `1a5708bf70119d9da86d963cf0e9941abf76bdba` | Strict affected-crate lint gate after transport hardening. |
| `cargo test -p swarm-network --locked` | PASS | `1a5708bf70119d9da86d963cf0e9941abf76bdba` | Full network suite remains green with transport limits and challenge expiry. |
| `cargo test -p swarm-cli --bin swarmcraft --locked` | PASS | `1a5708bf70119d9da86d963cf0e9941abf76bdba` | CLI/network-facing authorization units remain green. |
| impaired `interrupted_quic_transfer_resumes_after_lost_ack` | PASS | `1a5708bf70119d9da86d963cf0e9941abf76bdba` | 64 MiB forced-restart/lost-ACK transfer under 15ms±3ms delay, 0.5% loss and 100mbit loopback shaping. Actions run `33614422441`. |

## Required validation before handoff

- [x] format for authorization milestone
- [x] format for handshake milestone
- [x] format/check/tests for privacy/admission milestone
- [x] clippy/lint for affected crates through final independent Agent 4 head
- [x] network unit/integration tests for authorization milestone
- [x] captured hello/proof replay rejection
- [x] world request authorization matrix regression test
- [x] private-world confidentiality authorization path regression
- [ ] discovery unauthorized-signer regression — pending FINAL-028 implementation on composed Agent 1/2 contracts
- [x] hostile-load admission test
- [x] ordinary hard reconnect test remains green after admission/privacy changes
- [x] explicitly run impaired reconnect/transfer regression with delay/loss/rate shaping; scheduled multi-GiB soak remains a post-integration acceptance gate
- [x] exact-head dedicated Agent 4 validation for all independent network/privacy code

## Blockers

No upstream dependency blocker remains. Agents 1 and 2 are integrated and their canonical contracts have been consumed.

Current state is `IN PROGRESS` only because FINAL-028 production proof binding, malicious-discovery regressions, and fresh exact-head validation still need completion.

## Remaining work

1. implement authenticated discovery authority using the integrated canonical membership/authority/history contracts;
2. add malicious self-signed provider, stale authority, removed/banned member, malformed proof, wrong-world/history and replay-after-transition regressions for browse and exact resolve;
3. run the full fresh Agent 4 exact-head validation matrix;
4. clean temporary composition workflow machinery and close the ledger only after green validation.

## Handoff

READY FOR INTEGRATION: NO

Exact validated independent production head: `1a5708bf70119d9da86d963cf0e9941abf76bdba`

Blocker: FINAL-028 requires finalized and integrated Agent 1 + Agent 2 canonical authority/history proof semantics.

Known conflict areas after dependency integration: discovery announcement/proof records, discovery acceptance paths, and any shared canonical authority validator used by `daemon.rs`/protocol core.

Post-dependency validation required: unauthorized-world discovery signer regression, public browse/exact-resolve authority-proof matrix, then full Agent 4 network/privacy gate.

## Agent final statement

BLOCKED
