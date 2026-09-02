# Agent 1 — Consensus Configuration Safety

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-1-consensus`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH CREATION SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (campaign planning head; production tree remains the declared campaign baseline plus implementation ledgers)

CURRENT HEAD SHA: `e1a3dd09d24ef12a14f23af1507650dc770c0004` (validated Milestone 3 production head before this ledger-only commit)

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

- [x] Define a canonical committed membership generation used by authority quorum calculations.
- [x] Prevent an uncommitted membership update from immediately redefining the active voter set.
- [x] Implement safe membership transitions with explicit joint old+new quorum semantics.
- [x] Ensure stale/removed/banned membership universes cannot bypass a prepared or committed voter-set transition at the production daemon boundary; adversarial process coverage remains to be added.
- [x] Remove unsafe automatic writable Solo fallback after unclean multi-member quorum loss.
- [x] Preserve explicitly safe single-member semantics while failing closed for multi-member Solo transitions without a committed clean-relinquishment proof.
- [x] Make higher recovery rounds preserve any previously accepted/certified value for the target generation; end-to-end crash/resume coverage remains to be run.
- [ ] Ensure recovery promises/votes cannot equivocate under the production execution model in coordination with Agent 3 storage locking if needed.
- [ ] Replace security-significant saturating next-generation arithmetic with checked fail-closed exhaustion behavior. Authority/recovery generation paths are converted; owned-path audit remains.
- [ ] Add 3-peer and 5-peer divergent-membership partition regression tests.
- [ ] Add minority-old-authority Solo versus majority recovery regression test.
- [ ] Add recovery candidate crash/resume after certificate persistence regression test.
- [x] Add generation MAX-1/MAX boundary tests for the shared authority generation primitive.

## Work completed

- Verified there was no pre-existing `fix/agent-1-consensus` branch to preserve.
- Created `fix/agent-1-consensus` from the campaign planning head without changing the declared production baseline.
- Read the required audit sources and extracted the exact safety invariants and reproduction scenarios before production edits.

### Milestone 1 — generation/recovery/Solo fencing

Implementation commit: `8ee2f81fa43a30deb196aeb85364fb13840928f2`

- Added `AuthorityGeneration::checked_next()` with fail-closed epoch/fencing exhaustion errors and MAX-1/MAX regression coverage.
- Changed recovery ballot generation validation to checked successor arithmetic rather than saturating arithmetic.
- Converted crash-recovery, recovery promotion, Solo-to-quorum promotion, inbound epoch advancement, recovery round, recovery snapshot sequence, and recovery membership sequence paths to checked arithmetic.
- Removed automatic promotion into writable Solo mode when a multi-member authority loses quorum.
- Allowed a single-member world to use its ordinary quorum-of-one path instead of being artificially denied a permit.
- Reject received multi-member Solo transitions unless a future committed clean-relinquishment proof exists, rather than treating the signed `allow_solo_advancement` flag alone as sufficient safety proof.
- Strengthened durable recovery promises so a higher round cannot switch the accepted candidate/value on the same voter for the same target generation.

### Milestone 2 — joint-membership protocol and durable prepare

Implementation commit: `b69b9e210ab217ddf84d119b2d4dd9a424ac7f41`

- Added signed/hash-bound `MembershipProposalV1`, `MembershipVoteV1`, and `MembershipCertificateV1` protocol primitives.
- Added shared joint-consensus validation requiring majorities of both the old and proposed active voter universes.
- Added 3-to-5, 5-to-3, and 1-to-2 membership quorum unit regressions demonstrating quorum intersection.
- Added durable membership proposal promises and persisted membership certificates with crash-safe atomic writes.
- Added storage regression proving a prepared voter cannot switch to a conflicting membership proposal after restart.
- Aligned `swarm-consensus` recovery ballot evaluation with the durable production value-lock rule so a higher round cannot switch candidates.

### Milestone 3 — daemon joint-membership activation

Implementation commit: `e1a3dd09d24ef12a14f23af1507650dc770c0004`

- Replaced direct join/leave voter-set activation with durable prepare/vote/commit handling.
- Added `MembershipProposal` / `MembershipCommit` wire requests and vote/commit response handling with explicit bounds.
- Fenced authority leases, recovery ballots/epochs, ordinary epoch changes, transfers, lease grants, and sleep transitions whenever a membership prepare is durably pending.
- Added durable committed-certificate replay so a crash between certificate persistence and local membership application converges on the certified configuration.
- Restricted legacy/direct `WireRequest::Membership` so voter-set changes require a joint membership certificate and same-voter updates must be exact direct extensions.
- Kept exact duplicate membership delivery idempotent.
- Added live 1-to-2 joint-membership process regression and fixed a post-commit late-vote race by treating an in-flight vote after prepare cleanup as stale/idempotent rather than fatal.
- Preserved current-world synchronization after membership commit so the newly committed peer receives epoch/config/snapshot state without reconnecting.

## Current exact state

Fixed through Milestone 3:

- membership voter-set changes are prepared durably and activated only after an old+new joint quorum certificate;
- a voter that has prepared a transition fails closed for authority/recovery/control transitions until that exact transition commits;
- committed membership certificates survive/recover across crashes and direct membership delivery cannot bypass joint voter-set changes;
- the audited minority-old-authority automatic Solo path no longer becomes writable merely because quorum disappears;
- single-member worlds remain writable through the normal canonical quorum rule;
- authority generation and fencing counters fail closed at exhaustion on converted production paths;
- recovery voters and shared consensus helpers preserve the accepted candidate/value across higher rounds.

Still incomplete:

- process-level 3-peer/5-peer divergent-membership partition regression remains;
- explicit Solo/minority versus majority-recovery process regression remains;
- explicit recovery-certificate crash/resume process regression remains to be run/strengthened against the audit scenario;
- owned-path security-significant saturating-counter audit remains;
- clippy/lint and final exact-head validation remain;
- duplicate-process atomic non-equivocation remains coordinated with Agent 3 storage locking and must be truthfully documented at handoff.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Audit/source review | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Required implementation and audit inputs read. |
| Milestone 1 `cargo fmt --all` | PASS | `8ee2f81fa43a30deb196aeb85364fb13840928f2` | GitHub Actions run `33580807303`. |
| Milestone 1 `cargo check --workspace --all-targets` | PASS | `8ee2f81fa43a30deb196aeb85364fb13840928f2` | GitHub Actions run `33580807303`. |
| Milestone 1 `cargo test -p swarm-consensus -p swarm-storage` | PASS | `8ee2f81fa43a30deb196aeb85364fb13840928f2` | Durable recovery value-lock and generation boundaries. |
| Milestone 1 `cargo test -p swarm-cli --lib` | PASS | `8ee2f81fa43a30deb196aeb85364fb13840928f2` | GitHub Actions run `33580807303`. |
| Milestone 1 `cargo test -p swarm-cli --tests --no-run` | PASS | `8ee2f81fa43a30deb196aeb85364fb13840928f2` | All CLI process/integration tests compile. |
| Milestone 2 `cargo fmt --all` | PASS | `b69b9e210ab217ddf84d119b2d4dd9a424ac7f41` | GitHub Actions run `33581156235`. |
| Milestone 2 `cargo check --workspace --all-targets` | PASS | `b69b9e210ab217ddf84d119b2d4dd9a424ac7f41` | GitHub Actions run `33581156235`. |
| Milestone 2 `cargo test -p swarm-protocol -p swarm-consensus -p swarm-storage` | PASS | `b69b9e210ab217ddf84d119b2d4dd9a424ac7f41` | Joint old/new quorum, durable prepare, shared recovery value-lock. |
| Milestone 3 `cargo fmt --all` | PASS | `e1a3dd09d24ef12a14f23af1507650dc770c0004` production tree | GitHub Actions run `33582595124`. |
| Milestone 3 `cargo check --workspace --all-targets` | PASS | `e1a3dd09d24ef12a14f23af1507650dc770c0004` production tree | GitHub Actions run `33582595124`. |
| Milestone 3 protocol/consensus/storage/network tests | PASS | `e1a3dd09d24ef12a14f23af1507650dc770c0004` production tree | GitHub Actions run `33582595124`. |
| Milestone 3 `cargo test -p swarm-cli --lib` | PASS | `e1a3dd09d24ef12a14f23af1507650dc770c0004` production tree | GitHub Actions run `33582595124`. |
| Milestone 3 `cargo test -p swarm-cli --test live_join_replication -- --nocapture` | PASS | `e1a3dd09d24ef12a14f23af1507650dc770c0004` production tree | Live joint membership + immediate replication. |
| Milestone 3 `cargo test -p swarm-cli --tests --no-run` | PASS | `e1a3dd09d24ef12a14f23af1507650dc770c0004` production tree | All CLI integration tests compile. |

## Required validation before handoff

- [x] format through Milestone 3; rerun on final head required
- [ ] clippy/lint for affected Rust crates
- [x] unit tests through Milestone 3; rerun on final head required
- [x] consensus tests through Milestone 3; rerun on final head required
- [ ] process-level 3-peer recovery/partition tests required for this remediation
- [ ] process-level 5-peer divergent-membership partition tests
- [ ] Solo/recovery race test
- [ ] recovery higher-round value-preservation crash/resume test
- [ ] exact-head CI or dedicated validation

## Blockers

No current implementation blocker. Local checkout execution remains unavailable because the desktop/local repository connector rejects this chat with `CALLER_IDENTITY_REQUIRED`; exact transformations and validation are being executed by branch-scoped, self-cleaning GitHub Actions workers. Milestone 3 required two fixture/race corrections before the live process regression went green; run `33582595124` passed the complete Milestone 3 gate and produced implementation head `e1a3dd09d24ef12a14f23af1507650dc770c0004`.

## Remaining work

Add the required adversarial process/race/crash regressions, complete the owned-path counter audit, coordinate/document the duplicate-process boundary with Agent 3, run clippy plus final exact-head validation, then finalize the exact handoff SHA.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Required integration order: before Agent 2; before Agent 9 together with Agent 6.

Known conflict areas: `crates/swarm-cli/src/daemon.rs`, consensus/recovery protocol state, membership persistence semantics.

Post-merge validation required: full authority/recovery process suite.

## Agent final statement

NOT COMPLETE
