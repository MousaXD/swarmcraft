# Agent 1 — Consensus Configuration Safety

## Status

STATUS: READY FOR INTEGRATION

BRANCH: `fix/agent-1-consensus`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH CREATION SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb`

FINAL PRODUCTION SHA: `67493374544d91ad7bbb36be17e9312adb5654f6`

FINAL EXACT-HEAD VALIDATION RUN: `33619420045` — SUCCESS

INTEGRATED SHA: pending integration into `integration/audit-remediation-v1`

## Mission

Repair the canonical voter-set and authority-safety model so supported partitions and membership transitions cannot create two independently writable canonical histories.

## Findings owned

- FINAL-001 — divergent membership sets can form independent quorums
- FINAL-002 — automatic Solo fallback can race majority recovery
- FINAL-006 — higher recovery rounds are not value preserving
- FINAL-039 — strict monotonic generation/counter exhaustion handling
- FINAL-045 — legacy consensus/test models diverge from production semantics

## Audit inputs read

- `implementation/README.md`
- this ledger
- `audits/FINAL-AUDIT.md` from `audit/final-integration-report`
- `audits/02-authority-consensus.md` from `audit/authority-consensus`

The final audit maps Agent 1 to FINAL-001, FINAL-002, FINAL-006, FINAL-039, and FINAL-045. The authority audit provides the concrete AC-01/02/03/06/07 scenarios. Agent 3 owns the cross-process storage-lock/non-equivocation primitive and its composed cross-process proof; Agent 1 does not claim that work.

## Ownership boundaries

Primary ownership completed here:

- `crates/swarm-consensus`
- authority/recovery/membership quorum logic in `crates/swarm-cli`
- consensus-linked protocol/storage call sites needed to make the safety model fail closed
- process/integration tests for elections, leases, recovery, partitions, membership transitions, and counter exhaustion

No package-provider, Desktop UX, runtime-installation, Agent 2, or unrelated feature work was started.

## Implementation checklist

- [x] Define a canonical committed membership generation used by authority quorum calculations.
- [x] Prevent an uncommitted membership update from immediately redefining the active voter set.
- [x] Implement safe membership transitions with explicit joint old+new quorum semantics.
- [x] Fence stale/removed/banned membership universes with durable prepares and committed-certificate replay.
- [x] Remove unsafe automatic writable Solo fallback after unclean multi-member quorum loss.
- [x] Preserve safe single-member quorum-of-one behavior while rejecting unproven multi-member Solo transitions.
- [x] Make higher recovery rounds preserve the previously accepted candidate/value for the target generation.
- [x] Replace Agent 1-owned security-significant next-generation/history saturation, raw overflow, and wrap paths with checked fail-closed exhaustion behavior.
- [x] Add 3-peer and 5-peer divergent-membership/partition process regressions.
- [x] Add 3-peer and 5-peer Solo-enabled unclean-quorum-loss versus majority-recovery regressions.
- [x] Add recovery candidate crash/resume after certificate persistence coverage through `recovery_successor_dies`.
- [x] Add MAX-1/MAX generation and snapshot-history boundary regressions.
- [x] Add live joint-membership replication coverage.
- [x] Compile every `swarm-cli` integration test on the final production SHA.
- [x] Replace the old self-modifying regression workflow with a read-only exact-event-SHA gate.
- [x] Remove committed Python bytecode/generated artifacts and ignore future `__pycache__` / `.pyc` output.

## Milestones

### Milestone 1 — generation/recovery/Solo fencing

Implementation commit: `8ee2f81fa43a30deb196aeb85364fb13840928f2`

- Added `AuthorityGeneration::checked_next()` with fail-closed exhaustion.
- Converted core crash-recovery, recovery promotion, Solo-to-quorum, inbound epoch, recovery round, and canonical sequence paths to checked arithmetic.
- Removed unsafe automatic writable Solo fallback on multi-member quorum loss.
- Rejected unproven multi-member Solo transitions.
- Strengthened durable recovery promises so higher rounds cannot switch the accepted candidate/value.

### Milestone 2 — joint-membership protocol and durable prepare

Implementation commit: `b69b9e210ab217ddf84d119b2d4dd9a424ac7f41`

- Added signed/hash-bound membership proposal/vote/certificate records.
- Added joint old+new majority validation with 3→5, 5→3, and 1→2 regressions.
- Added durable membership promises and persisted membership certificates.
- Added restart-safe conflicting-prepare rejection.
- Aligned shared consensus recovery evaluation with production value locking.

### Milestone 3 — daemon joint-membership activation

Implementation commit: `e1a3dd09d24ef12a14f23af1507650dc770c0004`

- Replaced direct join/leave voter-set activation with durable prepare/vote/commit handling.
- Added bounded membership proposal/commit wire handling.
- Fenced authority/recovery/control transitions while a membership prepare is pending.
- Added committed-certificate crash replay and exact duplicate idempotency.
- Prevented legacy direct membership delivery from bypassing voter-set consensus.
- Added live 1→2 membership replication and fixed the late-vote-after-commit race.

### Milestone 4 — adversarial partition, convergence, and counter closure

Hardening production commit: `4b1fed01171ced3b30750882a58d3d0242489920`

Closure/safety production commit: `67493374544d91ad7bbb36be17e9312adb5654f6`

- Added 3-peer and 5-peer membership partition campaigns.
- Added 3-peer and 5-peer Solo-enabled unclean-quorum-loss campaigns proving a minority cannot become writable while a legitimate majority can recover.
- Added stale removed-voter convergence/revocation coverage.
- Disseminated committed membership to the union of old and new voter sets so removed prepared voters converge and clear their prepare.
- Prevented historical membership certificates from rolling back newer same-world membership generations.
- Closed checked-arithmetic gaps for wake epoch/fencing, snapshot epoch/sequence, membership/config sequence, simulator recovery generation, and storage snapshot numbering.
- Added explicit MAX boundary regressions for authority generation and snapshot numbering.
- Removed 13 accidentally committed `implementation/__pycache__/*.pyc` artifacts and added repository ignore rules.
- Replaced the previous write-back regression workflow with a read-only exact-head workflow that checks the GitHub event SHA and runs the complete Agent 1 closure matrix.

## Final source counter audit

Final production SHA `67493374544d91ad7bbb36be17e9312adb5654f6` passed the automated canonical-counter source audit in run `33619420045`.

Canonical authority/history counters now fail closed instead of saturating or wrapping. This includes authority epoch/fencing generations, membership/config sequences, snapshot generation/history numbering, recovery generation, and the legacy simulator/migration paths touched by Agent 1.

Remaining saturation in Agent 1-adjacent code is non-canonical liveness/transport arithmetic, such as in-memory permit heartbeat progression, blob resume offsets, and monotonic lease deadlines. Those values do not mint a new canonical authority/history generation.

The closure audit also found and fixed one previously missed raw overflow path in `Storage::next_snapshot_number()`: `snapshot_number + 1` is now checked and returns a fail-closed `CounterExhausted` error at `u64::MAX`.

## Exact-head validation

### Pre-closure hardening evidence

Run `33615103885` — SUCCESS. It validated the hardened production tree that became `4b1fed01171ced3b30750882a58d3d0242489920`, including workspace check, `-D warnings` clippy, consensus tests, CLI library tests, live membership, the full partition/Solo campaign, recovery successor crash/resume, and CLI integration-test compilation.

That worker also exposed accidental committed Python bytecode, which was removed in the final production commit instead of being accepted as handoff state.

### Final production exact-head gate

Run `33619420045` — SUCCESS on exact SHA `67493374544d91ad7bbb36be17e9312adb5654f6`.

| Validation | Result |
|---|---|
| Exact event SHA checkout assertion | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --all-targets` | PASS |
| `cargo clippy -p swarm-protocol -p swarm-consensus -p swarm-storage -p swarm-network -p swarm-cli --all-targets -- -D warnings` | PASS |
| Canonical counter saturation/wrap/raw-`+1` source audit | PASS |
| `cargo test -p swarm-consensus` | PASS |
| `cargo test -p swarm-cli --lib` | PASS |
| Authority-generation MAX exhaustion regression | PASS |
| Snapshot-number MAX exhaustion regression | PASS |
| `live_join_replication` | PASS |
| `consensus_partition_safety` — all 3-peer/5-peer partition and Solo-loss cases | PASS |
| `three_daemon_recovery` | PASS |
| `recovery_successor_dies` | PASS |
| `cargo test -p swarm-cli --tests --no-run` | PASS |

All Agent 1-owned implementation and exact-head validation gates are green.

## Remaining Agent 3 cross-process integration dependency

This is intentionally not claimed as Agent 1-owned completion:

- Agent 3 must provide/retain OS-backed per-world locking and cross-process non-equivocation for conflicting durable recovery promises.
- Composed integration must prove the final recovery certificate travels through the production transport/daemon path to the successor process and that a successor crash/restart converges without dual authority.
- The Agent 1 `three_daemon_recovery` and `recovery_successor_dies` regressions are green, but they do not replace Agent 3's storage/process atomicity proof or the final composed integration acceptance.

This dependency belongs to integration/Agent 3 and does not leave unfinished Agent 1-owned production work.

## Known integration conflict areas

Reconcile rather than overwrite when integrating:

- `crates/swarm-cli/src/daemon.rs` — overlaps Agent 3 storage/network integration and Agent 9 recovery runtime work.
- `crates/swarm-cli/src/migration.rs` — overlaps Agent 3/Agent 9 migration and recovery call sites.
- `crates/swarm-storage/src/lib.rs` and consensus-linked storage state — overlaps Agent 3 storage ownership; preserve the checked snapshot-number exhaustion fix.
- `crates/swarm-network/**` call sites used by daemon membership/recovery replication — overlaps Agent 4 network work.
- consensus/recovery protocol state used by membership/recovery certificates — coordinate with Agent 2 protocol integration.

Do not resolve these by taking one branch wholesale. Preserve Agent 1's quorum, fencing, committed-membership, value-locking, and checked-counter invariants while incorporating the downstream owner changes.

## Handoff

READY FOR INTEGRATION: YES

Validated production SHA: `67493374544d91ad7bbb36be17e9312adb5654f6`

Target: `integration/audit-remediation-v1`

Post-merge composed validation still required: Agent 1 exact-head matrix plus Agent 3 cross-process recovery-promise/non-equivocation and production transport/restart proof.

READY FOR INTEGRATION
