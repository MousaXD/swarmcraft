# Agent 4 — Network Authentication and Privacy

## Status

STATUS: READY FOR INTEGRATION

BRANCH: `fix/agent-4-network`

CAMPAIGN PRODUCTION BASE SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH SEED SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb`

INDEPENDENT AGENT 4 VALIDATED SHA: `1a5708bf70119d9da86d963cf0e9941abf76bdba`

AGENT 1/2 COMPOSITION MERGE: `6a6bd207c8ae4622ec84b9a28efb8c9e8d7045aa`

EXACT VALIDATED COMPOSED PRODUCTION SHA: `7f151439418833d89fe0e4fd3c961878c0b51093`

EXACT VALIDATION RUN: `33760654684` — SUCCESS

POST-VALIDATION CLEANUP COMMIT: `4367e5fcd38f71b9f78a6f8fe009c188c62f9dee` removed the temporary composition workflow. Documentation-only closure commits may follow the validated production SHA.

INTEGRATED SHA: pending — Agent 4 is validated and ready for integration; Agent 4 did not merge itself.

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
- [x] Anchor discovery announcements to a verifier-interactive freshness proof bound to canonical membership, authority/fence, WorldConfig, and the Agent 3 canonical head, certified by the current Agent 1 quorum (joint old+new quorum while pending).
- [x] Add separate per-peer/global unauthenticated and authenticated admission limits.
- [x] Enforce friend-presence privacy using requester-specific accepted-friend rendezvous.
- [x] Specify reusable invite semantics and bounded lifetime.
- [x] Re-resolve DNS invite targets and enforce address-scope policy on resolved answers.
- [x] Add captured-proof replay regression.
- [x] Add stranger/removed/banned/current-member authorization regressions.
- [x] Add malicious discovery provider / stale-authority / malformed-proof / wrong-history / replay-after-transition acceptance regressions for public browse and exact resolve.
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
- [x] discovery unauthorized-signer/current-authority proof regressions, including live browse/resolve malicious-provider ordering, malformed freshness responses, durable recovery-promise fencing, and joint quorum

## Cleanup

- Temporary composition/validation workflow removed after successful validation in commit `4367e5fcd38f71b9f78a6f8fe009c188c62f9dee`.
- No merge into `integration/audit-remediation-v1` or `main` was performed.
- At that earlier composed-validation stage, integration head remained `f02bb0d54cb44df67e730f01be4c903e25d670ff`; the later FINAL-028 closure consumed authoritative Agent 1+2+3 ancestor `c9252820a560e6ed4d30bb77227e3a494c6ce869`.

## Historical FINAL-028 remaining work — RESOLVED

The items below were open at the earlier composed-validation stage and are retained only as audit lineage. The canonical final closure later in this ledger resolves all of them:

1. Canonical current-authority/current-head freshness primitive — RESOLVED by verifier-interactive FINAL-028 freshness proof.
2. Agent 4 consumption of that primitive — RESOLVED in the final production milestone.
3. Public discovery binding — RESOLVED for browse and exact resolve.
4. Malicious/stale/malformed/wrong-history/replay acceptance regressions — RESOLVED and passing.
5. Exact-head revalidation — RESOLVED by run `33931301852` on Linux, Windows, and macOS.

## Handoff

READY FOR INTEGRATION: YES

Historical composed production SHA: `7f151439418833d89fe0e4fd3c961878c0b51093`

Historical composed validation run: `33760654684` SUCCESS

Historical blocker at that stage: FINAL-028 lacked a first-contact-verifiable current authority/current-head proof. This blocker is RESOLVED by the canonical final closure below.

## Agent final statement

READY FOR INTEGRATION


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


## FINAL-028 validation repair history

- `33865188491` — FAIL at workspace check on composed milestone `3a8d92089133c54ff1588011442cb0c83eb31dc1`; cause was Serde tuple-arity exhaustion after adding all signed announcement freshness fields. No security field was removed. The canonical signing representation was changed to two ordered nested tuples.
- The same repair pass also makes pending new voters advertise the exact-world locator, while keeping announcement publication restricted to current durable authority, so Agent 1 joint old+new freshness quorum is reachable in real discovery.

## FINAL-028 final closure (2026-09-06)

This section is the canonical final Agent 4 handoff and supersedes the earlier historical BLOCKED/FINAL-028 narrative. Earlier failure entries remain only as an audit trail.

### Dependency and validation lineage

- Original independent Agent 4 validated SHA: `1a5708bf70119d9da86d963cf0e9941abf76bdba`.
- Original composed Agent 1/2 validation: merge `6a6bd207c8ae4622ec84b9a28efb8c9e8d7045aa`, exact validated composed production SHA `7f151439418833d89fe0e4fd3c961878c0b51093`, run `33760654684` SUCCESS.
- Authoritative Agent 1+2+3 integration ancestor consumed for FINAL-028: `c9252820a560e6ed4d30bb77227e3a494c6ce869`.
- Normal final Agent 4 production milestone: `77300031751d0e3df0fc5a7c9aac1c2b2625989f` (`fix(agent4): finalize FINAL-028 production tree`).
- Exact validated production SHA: `77300031751d0e3df0fc5a7c9aac1c2b2625989f`.
- Focused closure/materialization run: `33930963545` SUCCESS before the normal production commit.
- Exact-head acceptance run: `33931301852` SUCCESS on Linux, Windows, and macOS against the same immutable production SHA.

### FINAL-028 closure history

- The original FINAL-028 blocker was that self-signed discovery announcements could be authentic yet stale relative to canonical membership/authority/head state. The repaired construction uses verifier-interactive freshness instead of trusting an announcement or locator at face value.
- Freshness proofs bind verifier identity and fresh nonce, world ID, exact announcement hash, committed membership sequence/hash, pending membership transition identity, current authority, authority epoch, fencing token/generation, WorldConfig sequence/hash, and Agent 3 canonical snapshot/manifest/head identity and sequence.
- Steady state requires the current Agent 1 majority. Pending membership requires BOTH old and new majorities. Signer sets remain bounded, unique, canonical, and tied to canonical member keys.
- A newer durable Agent 1 recovery promise fences a stale authority generation from freshness signing. Counter/expiry exhaustion and malformed/noncanonical proof shapes fail closed.
- Explicit bootstraps and Kademlia providers remain untrusted locators only; locator identity grants no world authority.
- The first permanent network helper shape hit the Serde/test-helper arity/clippy boundary in run `33870722585`; the 10-argument helper was restructured into `AnnouncementFixture` without removing any signed/security-bound field.
- Intermediate stale-fixture and brittle patch-anchor failures were harness/materialization defects and were corrected without weakening the verifier contract.
- Run `33876484031` passed formatting, workspace check, strict clippy, protocol/core freshness, joint quorum, FINAL-028 verifier `6/6`, and durable recovery freshness, then failed the live network target with `discovery response channel closed`.
- Response-channel root cause: `DiscoveryNode::next_event()` propagated closed `HelloChallengeAccepted`/`HelloAccepted` acknowledgement channels with `?`, allowing one peer-local request/connection replacement to abort the whole browse/resolve operation. The final repair keeps proof verification unchanged and isolates closed acknowledgement channels to the peer/request.
- Run `33877859315` proved that channel-close death was gone but exposed partial Kademlia visibility. Explicit bootstrap peers were retained as bounded untrusted locator candidates while all returned announcement/proof/vote material remained fully verified.
- Run `33878769888` selected the canonical current result but exposed nondeterministic malformed-provider participation. The permanent test was changed to observed topology/authentication readiness and ordered hostile locators rather than blind sleeps.
- A later rust-libp2p `remaining_established` assertion exposed duplicate-connection bookkeeping churn under simultaneous/redundant dials.
- Connection-bookkeeping root cause: SwarmCraft selected duplicate discovery connections using local connection order / first-or-newest semantics. Simultaneous cross-dials could therefore make the two endpoints retain different physical connections, causing ConnectionId-bound authentication churn and request-response lifecycle inconsistencies.
- Deterministic repair: compare transport PeerIds. The smaller transport PeerId prefers the dialer-side connection and the larger transport PeerId prefers the listener-side connection, so both endpoints choose the SAME physical connection. ConnectionId tracking, connection-bound authentication, single-flight dialing, deferred replacement authentication until convergence, and peer-local response-channel isolation are preserved.
- Run `33917372427` then passed the full permanent `discovery_network_freshness` target FIVE consecutive rounds: 3 tests per round, `15/15` executions green. Every round passed malicious/stale browse + exact resolve, duplicate-dial/provider-disconnect resilience, and simultaneous bidirectional-dial convergence. No `remaining_established` assertion and no discovery response-channel death recurred. That run later stopped only on the trivial `clippy::collapsible_if` structural lint in `swarm-network/src/discovery.rs`; semantics were unchanged by the collapse.
- Final strict clippy also found one `single_match` helper lint; that too was repaired structurally without behavioral change.
- Hosted-runner response ordering exposed one test-only timing assertion; the healthy provider fixture delay was made deterministic. The symmetric-dial regression was also corrected to require eventual convergence to one authenticated connection rather than forbidding the brief two-connection overlap while asynchronous duplicate close delivery is pending.

### Final accepted evidence

- Five-round network stress: `33917372427` — `15/15` permanent network-test executions PASS before its later clippy-only stop.
- Public browse malicious/stale-provider acceptance: PASS. Stale and malformed providers participate but cannot win; the current fresh-quorum announcement is selected.
- Exact resolve malicious/stale-provider acceptance: PASS. Resolver does not accept first-self-valid data; stale/malformed candidates cannot win.
- Duplicate-dial + provider-disconnect resilience: PASS. Redundant dials converge and a surviving authenticated provider remains usable after another provider disconnects.
- Simultaneous bidirectional dial convergence: PASS. Both endpoints converge to one authenticated application connection under the deterministic direction rule.
- Joint old+new quorum: PASS. Both majorities are required during transition; insufficient-old, insufficient-new, and stale-old-only cases are rejected.
- FINAL-028 verifier suite: `6/6 PASS`.
- Durable recovery freshness: `durable_recovery_promise_fences_stale_freshness_and_current_majority_recovers` PASS.
- Exact-head Linux job in run `33931301852`: PASS. It proved exact SHA/clean tree at start and end; locked metadata; format; workspace all-target check; strict `-D warnings` clippy; network/protocol/core/storage/consensus/CLI suites; FINAL-028 and discovery-network freshness; authentication replay/admission/invite/friend/DNS hardening; Agent 1 3-peer and 5-peer partition safety, Solo-loss, live membership, automatic invite join, three-daemon recovery, recovery-successor crash/resume; Agent 2 authority/history/replay/migration; Agent 3 canonical-head integrity, missing-head rollback failure, stale fencing rejection, cross-process promise non-equivocation; durable recovery; impaired QUIC lost-ACK/restart recovery; and all required workspace/CLI integration target compilation.
- Windows same-SHA portability in run `33931301852`: PASS for freshness serialization, deterministic signing bytes, canonical signer/proof ordering, proof bounds, and clean exact-SHA start/end.
- macOS same-SHA portability in run `33931301852`: PASS for the same portability surface and clean exact-SHA start/end.

### Post-validation cleanup contract

After exact-head acceptance, only temporary Agent 4 validation/materialization machinery and this ledger were changed. The cleanup paths are:
- `.github/agent4_connection_lifecycle_probe.py`
- `.github/agent4_final028_patch.py`
- `.github/agent4_final028_repair.py`
- `.github/agent4_finalize_ledger.py`
- `.github/agent4_finalize_network_test.py`
- `.github/agent4_finalize_source.py`
- `.github/agent4_finalize_source_v2.py`
- `.github/agent4_finalize_source_v3.py`
- `.github/agent4_finalize_source_v4.py`
- `.github/agent4_finalize_source_v5.py`
- `.github/agent4_finalize_source_v7.py`
- `.github/agent4_finalize_source_v8.py`
- `.github/agent4_finalize_source_v9.py`
- `.github/agent4_finalize_source_v10.py`
- `.github/agent4_finalize_source_v11.py`
- `.github/clippy-failure.txt`
- `.github/workflows/agent4-final028.yml`
- `.github/workflows/agent4-final028-v4.yml`
- `.github/workflows/agent4-production-proof.yml`
- `.github/workflows/agent4-cleanup.yml`
- `implementation/agent-4-network.md`

No Rust source, permanent test, Cargo metadata, or permanent product workflow is permitted to change after the validated production SHA. The final post-validation compare must show only the cleanup deletions above plus `implementation/agent-4-network.md`.

STATUS: READY FOR INTEGRATION

READY FOR INTEGRATION: YES

Exact validated production SHA to integrate: `77300031751d0e3df0fc5a7c9aac1c2b2625989f`

Agent 4 did not merge itself.

READY FOR INTEGRATION
