# Agent 4 — Network Authentication and Privacy

## Status

STATUS: BLOCKED

BRANCH: `fix/agent-4-network`

CAMPAIGN PRODUCTION BASE SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH SEED SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb`

INDEPENDENT AGENT 4 VALIDATED SHA: `1a5708bf70119d9da86d963cf0e9941abf76bdba`

AGENT 1/2 COMPOSITION MERGE: `6a6bd207c8ae4622ec84b9a28efb8c9e8d7045aa`

EXACT VALIDATED COMPOSED PRODUCTION SHA: `7f151439418833d89fe0e4fd3c961878c0b51093`

EXACT VALIDATION RUN: `33760654684` — SUCCESS

POST-VALIDATION CLEANUP COMMIT: `4367e5fcd38f71b9f78a6f8fe009c188c62f9dee` removed the temporary composition workflow. Documentation-only closure commits may follow the validated production SHA.

INTEGRATED SHA: pending — Agent 4 is not ready to merge because FINAL-028 remains unresolved.

## Mission

Repair the network trust boundary so a peer proves current possession of its application key on the live transport connection and world-scoped data is disclosed only to authorized peers.

## Findings owned

- FINAL-012 — replayable application peer hello/identity impersonation
- FINAL-013 — unauthorised private-world metadata access
- FINAL-028 — discovery authority authenticity gap
- FINAL-029 — request/connection admission and rate limiting gap
- FINAL-030 — friend presence privacy semantics
- FINAL-040 — invite/DNS/address/privacy hardening assigned by final audit

## Dependencies consumed

Agent 4 consumed authoritative integration head `f02bb0d54cb44df67e730f01be4c903e25d670ff`, containing:

- Agent 1 validated production SHA `67493374544d91ad7bbb36be17e9312adb5654f6` and integration merge `a0e0dec659d0b1eb21f9be34c44730edc6ff3984`.
- Agent 2 validated production SHA `dde75ca4e9f2268bb97f42a716864c3e51f266cb`, exact-head run `33693100794` SUCCESS, and integration merge `6e70a0774d7e021cc57681705ccef4620265ce3d`.
- Composed Agent 4 dependency merge `6a6bd207c8ae4622ec84b9a28efb8c9e8d7045aa`.

The consumed contracts are Agent 1 committed-membership/joint-consensus safety plus Agent 2 current-authority, direct membership history validation, canonical semantics, and protocol-version fail-closed behavior. No parallel discovery-only authority model was introduced.

The authoritative integration branch remained unchanged at `f02bb0d54cb44df67e730f01be4c903e25d670ff` throughout Agent 4 composition and validation.

## Ownership boundaries

Primary ownership:

- `crates/swarm-network`
- discovery protocol/networking
- network-facing daemon authorization helpers and request matrix
- invite connectivity validation
- hostile-peer tests

Agent 4 must not redesign canonical membership election semantics.

## Implementation checklist

- [x] Replace reusable one-message application hello authentication with connection-bound proof of possession.
- [x] Include a fresh receiver challenge and bind proof to both sides of live transport context.
- [x] Reject captured proof replay on a different transport identity/connection.
- [x] Do not reuse authenticated application identity across replacement connections without fresh proof.
- [x] Build an exhaustive authorization classification for world-scoped `WireRequest` variants.
- [x] Require current, non-banned membership for ordinary canonical metadata/data requests.
- [x] Preserve specialized Agent 1 membership proposal/commit authorization so a pending joiner can receive the joint-consensus transition without being incorrectly subjected to the ordinary current-member gate.
- [x] Ensure removed/banned/key-mismatched members lose canonical metadata/data access.
- [ ] Anchor discovery announcements to a verifiable canonical world authority/current-head proof rather than merely the announcer's self-signature. **BLOCKED on missing canonical non-omittable freshness/current-head proof.**
- [x] Add separate per-peer/global unauthenticated and authenticated admission limits.
- [x] Enforce friend-presence privacy using requester-specific accepted-friend rendezvous.
- [x] Specify reusable invite semantics and bounded lifetime.
- [x] Re-resolve DNS invite targets and enforce address-scope policy on resolved answers.
- [x] Add captured-proof replay regression.
- [x] Add stranger/removed/banned/current-member authorization regressions.
- [ ] Add malicious discovery provider / stale-authority / malformed-proof / wrong-history / replay-after-transition acceptance regressions. These cannot truthfully pass until the canonical proof primitive exists.
- [x] Add hostile-load/admission regression coverage.
- [x] Harden proactive world pushes against removed/banned/key-mismatched members.

## Work completed

### Authentication and admission

- Replaced reusable application hello authentication with receiver-generated challenges and fresh Ed25519 application-key proof bound to the challenge, local transport peer, remote transport peer, and exact live libp2p connection.
- Authentication state is connection-specific and cleared on disconnect/replacement.
- Added a three-transport replay regression proving a proof captured on B→A cannot authenticate C→A.
- Added bounded application and transport admission to primary and discovery swarms, including pre-auth challenge expiry and separate request budgets.
- Added fail-closed rate-limit responses before application dispatch while preserving wire-size validation and connection-bound authentication.

### World confidentiality and privacy

- Added exhaustive `WireRequest::membership_world_id()` classification for ordinary canonical requests.
- Added one fail-closed daemon current-membership gate for ordinary canonical world traffic.
- Preserved `JoinRequest`, Agent 1 `MembershipProposal`, and Agent 1 `MembershipCommit` as intentionally specialized transition paths with their own cryptographic/consensus validation.
- Hardened current-member checks against banned identities and peer/public-key mismatch.
- Hardened proactive `push_known_worlds` against stale removed/banned/key-mismatched descriptor entries.
- Changed friend presence from a global peer rendezvous key to requester-specific accepted-friend rendezvous keys and withdraws removed-friend entries.

### Invite and connectivity hardening

- Invitations are explicit reusable bearer capabilities until expiry, not hidden single-use tokens.
- Invite creation/decoding/join validation enforce a maximum 24-hour lifetime with checked arithmetic.
- DNS/DNS4/DNS6 hints are immediately re-resolved before dialing and every resolved address is reclassified by scope policy.
- Public-looking DNS cannot silently rebind to loopback/private/link-local scope.
- The composed Agent 1 `live_join_replication` fixture was corrected from `u64::MAX` expiry to a valid one-hour future expiry. This was a test-fixture compatibility correction; the 24-hour production security policy was not weakened.

### Agent 1/2 composition repairs

Composition merge: `6a6bd207c8ae4622ec84b9a28efb8c9e8d7045aa`

Resolved three initial conflict files:

- `crates/swarm-cli/src/daemon.rs`
- `crates/swarm-network/src/lib.rs`
- `crates/swarm-network/src/wire.rs`

Preserved:

- Agent 1 membership proposal/commit postcard ordering and joint-consensus transition behavior.
- Agent 2 committed-membership delivery to newly admitted proposal members.
- Agent 4 connection-bound handshake variants after the integrated membership wire variants.
- Agent 4 authorization matrix plus Agent 2 protocol-acceptance tests.

Fresh composed validation exposed and repaired three merge/fixture defects without changing canonical consensus semantics:

1. rustfmt drift in `daemon.rs`.
2. non-exhaustive world-authorization classification after Agent 1 added `MembershipProposal` and `MembershipCommit`; these transition messages are now explicitly outside the ordinary current-member gate and continue through their stricter dedicated validators.
3. stale `live_join_replication` fixture using an unbounded invite lifetime, corrected to a valid bounded expiry.

## FINAL-028 blocker analysis

FINAL-028 is no longer blocked because Agent 1 or Agent 2 are unfinished. Their authoritative contracts are composed and validated on this branch.

The remaining blocker is architectural and specific:

1. Public discovery announcements are currently self-signed by the announcer.
2. Canonical world authority evolves through Agent 1/2 membership, epoch, transfer, recovery, and related signed/certified state.
3. The repository retains the current control heads and selected latest certificates, but it does not expose a complete, independently verifiable, non-omittable authority-transition proof that a first-contact discovery verifier can use to prove that the presented authority generation is the **current** canonical head.
4. A stale former authority can possess valid historical signatures/certificates. If the verifier accepts a valid history prefix without a non-omittable freshness/current-head commitment, that stale authority can omit a later legitimate transition and present a cryptographically valid stale prefix.
5. The existing discovery replay guard helps after a verifier has already observed a newer generation, but it cannot establish freshness for first contact.
6. Creator pinning, announcer self-signature, or first-observed-key/TOFU would only hide the gap and would fail after legitimate authority transfer.

Therefore Agent 4 cannot truthfully implement the required invariant — including stale-authority rejection after transition on first contact — without a canonical primitive owned by the consensus/protocol/storage trust model.

### Canonical primitive required to unblock FINAL-028

A follow-up must provide a canonical, bounded and verifiable authority proof with all of the following properties:

- anchored to world genesis / `WorldId`;
- proves the authority and membership transition chain using the existing Agent 1/2 rules rather than a parallel election model;
- commits to the current accepted authority generation so later transitions cannot be omitted by a stale signer;
- retains or derives the transition evidence needed by a first-contact verifier;
- supports authority transfer, recovery, membership churn, removal/banning, and counter/fencing semantics;
- has explicit bounded wire/storage limits and fail-closed versioning;
- can be reused by discovery browse and exact resolve.

Only after that primitive lands should Agent 4 bind `WorldAnnouncementV1` to it and add malicious-provider, stale-authority, removed/banned signer, malformed proof, wrong-world/history, and replay-after-transition regressions.

## Exact composed validation

Validated production SHA: `7f151439418833d89fe0e4fd3c961878c0b51093`

GitHub Actions run: `33760654684` — SUCCESS

The run passed:

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --all-targets --locked` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test -p swarm-network --locked` | PASS |
| `cargo test -p swarm-protocol --locked` | PASS |
| `cargo test -p swarm-core --locked` | PASS |
| `cargo test -p swarm-storage --locked` | PASS |
| `cargo test -p swarm-consensus --locked` | PASS |
| `cargo test -p swarm-cli --lib --bins --locked` | PASS |
| Agent 1 `consensus_partition_safety` | PASS |
| Agent 1 `live_join_replication` | PASS |
| Agent 1 `automatic_invite_join` | PASS |
| Agent 1 `three_daemon_recovery` | PASS |
| Agent 1 `recovery_successor_dies` | PASS |
| Agent 2 `migration_core` | PASS |
| Compile all affected integration targets with `--no-run` | PASS |
| Impaired QUIC `interrupted_quic_transfer_resumes_after_lost_ack` under 15ms ± 3ms delay, 0.5% loss, 100mbit rate shaping | PASS |
| Exact validated production head / clean status assertion | PASS |

The ordinary network suite also includes connection-bound authentication, captured-proof replay rejection, hostile pre-auth flood recovery, and hard reconnect coverage.

## Required validation before handoff

- [x] format
- [x] workspace check
- [x] strict workspace clippy with `-D warnings`
- [x] network tests
- [x] protocol tests
- [x] core tests
- [x] storage tests
- [x] consensus tests
- [x] CLI library/bin tests
- [x] Agent 1 partition-safety regressions
- [x] Agent 1 live membership regressions
- [x] Agent 1 recovery regressions
- [x] Agent 2 migration/history regression
- [x] all affected integration targets compile
- [x] impaired reconnect/lost-ACK regression
- [x] exact composed production SHA validated
- [ ] discovery unauthorized-signer/current-authority proof regressions — blocked on the canonical current-head proof primitive described above

## Cleanup

- Temporary composition/validation workflow removed after successful validation in commit `4367e5fcd38f71b9f78a6f8fe009c188c62f9dee`.
- No merge into `integration/audit-remediation-v1` or `main` was performed.
- Integration head remained `f02bb0d54cb44df67e730f01be4c903e25d670ff`.

## Remaining work

1. Add the canonical non-omittable current-authority/current-head proof primitive in the consensus/protocol/storage trust model.
2. Re-consume that primitive on Agent 4.
3. Bind public discovery announcements to that proof.
4. Add malicious self-signed provider, stale former authority, removed/banned member, malformed proof, wrong-world/history, and replay-after-transition tests for public browse and exact resolve.
5. Re-run Agent 4 exact-head validation and only then mark READY FOR INTEGRATION.

## Handoff

READY FOR INTEGRATION: NO

Validated composed production SHA: `7f151439418833d89fe0e4fd3c961878c0b51093`

Exact validation run: `33760654684` SUCCESS

Blocker: FINAL-028 cannot be closed safely until the canonical trust model provides a first-contact-verifiable, non-omittable current authority/current-head proof across legitimate membership/authority transitions.

## Agent final statement

BLOCKED


## FINAL-028 closure composition (2026-09-04)

- Starting Agent 4 remote head verified before closure: `992e9c05d690eb2832476a9e2b2e074a8d0c97e2`.
- Authoritative Agent 1+2+3 integration ancestor consumed: `c9252820a560e6ed4d30bb77227e3a494c6ce869`.
- Composition conflict: `crates/swarm-cli/src/daemon.rs` only. Resolution preserves Agent 4 connection/auth/privacy hardening and reapplies Agent 3 `commit_snapshot_fenced` recovery promotion using the durable canonical expected head, epoch, and fencing token.
- FINAL-028 design: first-contact discovery now requires a verifier-generated random nonce. The current authority supplies genesis-anchored membership transition material; every accepted record then needs a live canonical quorum to sign a challenge binding the exact announcement hash, current membership hash/sequence, pending joint-transition identity, current authority, epoch, fencing token, WorldConfig hash/sequence, and Agent 3 canonical snapshot head.
- A DHT provider remains an untrusted locator. Current active public/unlisted members publish the exact-world provider key solely so the verifier can reach a live quorum; only the current authority publishes the announcement/public-directory record.
- A signer reloads durable membership, pending membership promise, epoch/fence, WorldConfig, and Agent 3 canonical head before signing. Any mismatch fails closed. Reused `(verifier, nonce)` challenges are refused by signers and rejected by the verifier replay guard.
- Joint transitions use Agent 1's old+new quorum rule. The proof cannot be certified by one voter universe alone.
- Membership-changing transitions are anchored from genesis through Agent 1 membership certificates; certificate history is retained immutably for future discovery proofs. Same-voter authority/recovery refreshes are made current by the live quorum challenge, not by trusting an old authority signature.
- Security argument: truncating a historical prefix no longer proves freshness. After a committed membership/authority transition, quorum intersection guarantees at least one member of any would-be old majority has durable newer state and refuses to sign the stale challenge. A stale former authority therefore cannot answer a new verifier nonce with a valid current quorum.
- Temporary exact-head validation workflow: `.github/workflows/agent4-final028.yml`; temporary patch vehicle: `.github/agent4_final028_patch.py`. Both are removed only after the exact production SHA and cross-platform proof serialization checks succeed.
