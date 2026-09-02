# Auditor 2 — Authority, Consensus, Migration, and Split Brain

## Audit baseline

- Repository: `MousaXD/swarmcraft`
- Audited branch: `main`
- Exact audited SHA: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Audit branch: `audit/authority-consensus`
- Production modifications: none
- Method: source review of consensus/protocol/storage/runtime authority paths plus existing process/unit tests and exact-head GitHub Actions metadata.
- Exact-head CI observation during this audit: workflow run `33576322543` (`CI`) was still in progress when checked; `Release version guard` run `33576322733` had completed successfully. This report does not treat an in-progress CI run as proof of safety.

## Executive verdict

The authority system has strong local generation fencing, signed epoch/lease validation, exact snapshot anchoring, durable recovery votes, and a careful manual-transfer path. Those controls are real and materially reduce stale-authority risk.

However, the repository does **not** currently preserve the required invariant that there is one canonical writable history under all supported authority transitions.

Two independent CRITICAL split-brain paths are present:

1. membership changes are not quorum-committed before they redefine the quorum set, so peers with stale membership can form a second locally valid quorum while the current authority still holds a quorum under the newer membership;
2. when `allow_solo_advancement` is enabled, the current authority can automatically enter Solo mode after losing quorum while the opposite majority independently performs crash recovery from the previous epoch, producing two writable branches from one base.

Additional HIGH findings affect recovery-round safety, successor eligibility, and duplicate-daemon non-equivocation.

**VERDICT: FAIL**

---

## Authority transition table

| Transition / failure | Who may write? | Who decides? | Proof checked | Partition behavior | What fences old writers? | Ambiguity resolution | Fail-closed / fail-open |
|---|---|---|---|---|---|---|---|
| Stable multi-member Quorum epoch | Current epoch authority only after fresh local `authority.permit` | Authority daemon plus majority lease acknowledgements | Signed epoch, membership/key eligibility, exact `(epoch,fencing_token)` lease, fresh acks | Permit expires if quorum acks disappear | Exact epoch/token checks; runtime permit heartbeat | None if membership view is identical | Fail-closed while Solo fallback is disabled; otherwise see AC-02 |
| Crash recovery | Deterministically elected eligible candidate only after Recovery epoch and fresh quorum permit | Visible majority on exact local canonical base | Signed ballot/votes, membership hash, snapshot/state hash, next epoch/token, certificate quorum | Majority can recover; minority should stop | Higher epoch/token and stale lease rejection | Recovery rounds | Mostly fail-closed, but AC-01 and AC-03 violate assumptions |
| Manual transfer | Source until checkpoint/transfer stop; target only after committed transfer, next epoch, and permit | Source + target, then observing peers / lease quorum | Prepared → Accepted → Committed signatures, exact checkpoint, exact next epoch/token | Target cannot run multi-member world until permit becomes live | Exact generation, active outbound transfer suppresses source relaunch | Transfer state is linear and hash/snapshot bound | Fail-closed |
| Quorum → Solo | Current authority | Current authority alone if signed world config allows Solo | Current epoch identity, signed config, latest snapshot | Isolated authority may advance and regain permit without quorum | Only local Solo epoch/token; other partition may still hold old epoch | Solo-branch conflict preservation after heal | **Fail-open for availability; unsafe with concurrent majority recovery (AC-02)** |
| Recovery successor dies before epoch promotion | Later candidate may use higher round on same base | New majority | Durable recovery promises and new quorum certificate | Can recover liveness | Round floor + certificate validation | Higher round supersedes abandoned attempt | Unsafe if earlier certificate already existed and later resumes (AC-03) |
| Permanent quorum loss, Solo disabled | Nobody writes multi-member world | No quorum | Missing permit | World stops | Permit timeout | Manual recovery/restore only | Fail-closed |
| Permanent quorum loss, Solo enabled | Current authority can enter Solo and write | Current authority alone | Signed policy and local history | Availability retained at cost of fork risk | No cross-partition fence against a majority on older view | Manual Solo reconciliation | Fail-open |
| Multi-member wake | No automatic writer | Migration supervisor | Durable sleep state; current implementation blocks multi-member wake | Remains stopped | Sleep record + no permit | Requires future quorum-backed wake protocol | Fail-closed |
| Stale peer reconnect | Current accepted generation only | Receiving peer's local accepted state | Exact generation, hash-linked next epoch, membership/signature checks | Stale writes rejected after newer epoch locally accepted | Epoch + fencing token | Sync/replication | Fail-closed **only if peers agree on membership/epoch lineage** |

---

# Findings

## AC-01 — CRITICAL — Stale membership can create two simultaneous valid quorums and two writable authorities

**Files / functions**

- `crates/swarm-cli/src/daemon.rs`
  - `handle_request` → `JoinRequest`, `LeaveRequest`, `Membership`, `RecoveryBallot`
  - `maintain_authority_leases`
  - `drive_recovery_ballot`
  - `maintain_local_authority`
- `crates/swarm-storage/src/world.rs`
  - `Storage::save_membership_record`
- `crates/swarm-consensus/src/lib.rs`
  - `has_quorum`, `quorum_size`

**Invariant**

There must be one canonical writable history, and the quorum definition used to elect/fence authority must itself be canonical.

**Failure scenario**

A concrete 5-peer sequence is sufficient:

1. Start with canonical membership `{A,B,C}` and A as authority in epoch N.
2. While B and C are disconnected or otherwise miss membership replication, A admits D and E. A/D/E can hold the newest signed membership `{A,B,C,D,E}` while B/C still hold `{A,B,C}`.
3. Partition the network as `A,D,E | B,C`.
4. A computes quorum from its local 5-member descriptor. A + D + E is 3/5, so A keeps a live permit and continues writing epoch N.
5. B and C compute quorum from their stale 3-member descriptors. After A's last lease expires, B + C is 2/3, so they can run recovery, certify a Recovery epoch N+1 using the stale membership hash, obtain 2/3 lease acknowledgements, and receive a live permit.
6. A and the recovered B-side can now run writable Minecraft authority concurrently from the same pre-partition history.
7. On heal, the two sides do not have a common membership hash/epoch lineage that guarantees automatic convergence. Fencing tokens are evaluated against each peer's locally accepted epoch, so they do not prevent the split before state convergence.

The same mechanism can let a peer that has been removed or banned in a newer membership continue participating in recovery from a stale membership view.

**Evidence**

- Join/leave membership updates are authority-signed and persisted locally, but there is no quorum-commit step for membership before the new membership becomes active.
- `Storage::save_membership_record` is a local atomic file replacement, not a replicated quorum commit.
- Recovery quorum size is derived from the candidate's local `WorldDescriptorV1` member count.
- Recovery ballots bind a membership hash, which prevents peers on different membership versions from mixing votes, but it does **not** establish which membership version is canonical. That turns membership divergence into independent quorum universes rather than safely resolving it.
- `live_join_replication.rs` proves the joining peer receives membership, but it is a two-peer test and does not prove every existing voting member has durably accepted the new membership before it affects quorum semantics.

**Existing coverage**

- Membership signature and monotonic sequence checks exist.
- Recovery ballots bind `membership_hash`.
- Existing recovery tests seed identical membership on all peers before failure.

**Missing test**

A process-level five-peer test with deliberately divergent membership replicas, followed by a `3/2` partition, must assert that only one side can ever obtain a live authority permit. Also test removal/ban propagation where a stale removed peer attempts to contribute to recovery quorum.

**Recommended remediation**

Make membership itself a consensus-controlled, quorum-committed control-plane value before it changes the voting set. A safe design needs an explicit configuration/membership epoch or joint-consensus transition so old and new quorum sets cannot both independently authorize conflicting authority. At minimum:

- do not let an uncommitted membership record redefine `member_count` for authority quorum;
- replicate and durably acknowledge membership changes with the old quorum before activation;
- use joint quorum rules while adding/removing voters;
- bind authority leases/recovery to a canonical committed membership generation, not merely the local latest file;
- prevent removed/banned members from remaining valid voters through stale local descriptors.

**Confidence:** HIGH

---

## AC-02 — CRITICAL — Automatic Solo fallback can race majority recovery and create two writable branches

**Files / functions**

- `crates/swarm-cli/src/daemon.rs`
  - `maintain_local_authority`
  - `solo_mode_allowed`
  - `promote_to_solo`
  - `promote_solo_to_quorum`
- `crates/swarm-cli/src/main.rs` world-config creation
- `crates/swarm-cli/src/world_import.rs`
- `apps/desktop/src-tauri/src/canonical_world_commands.rs`
- `ARCHITECTURE.md` sections 12, 14, 15
- `README.md` “Solo mode and partitions”

**Invariant**

A network partition must not allow both sides to mutate authoritative/canonical world history.

**Failure scenario**

With synchronized 5-member state and A as current authority:

1. Partition `A,E | B,C,D`.
2. A loses normal lease quorum.
3. If signed policy allows Solo, `maintain_local_authority` calls `promote_to_solo`, advancing A to a locally accepted Solo epoch N+1 and later refreshing a live permit without quorum.
4. B/C/D still hold epoch N. They no longer see A, have 3/5 quorum, agree on the exact snapshot, and recover a different authority into Recovery epoch N+1.
5. The majority recovery authority obtains its own epoch/lease quorum and live permit.
6. Both sides can now mutate Minecraft state concurrently from the same base.

This does not depend on malformed messages or malicious peers.

**Evidence**

- `maintain_local_authority` explicitly enters Solo when normal quorum is absent and `allow_solo_advancement` is true.
- While already in Solo mode, the same function refreshes the authority permit without requiring quorum.
- Production world creation/import/Desktop canonicalization currently sets `allow_solo_advancement: true` by default.
- The architecture's partition section explicitly states the CP goal: the minority side must not advance canonical state.
- `swarm-consensus::elect_authority_with_quorum` documentation says automatic crash recovery requires majority and that clean sleep/wake may use Solo because the previous authority explicitly relinquished the world. Production automatic Solo-on-quorum-loss is broader than that safety statement.

**Existing coverage**

- Solo ancestry is signed and conflicts are preserved instead of silently merged.
- `three_daemon_recovery.rs` sets `allow_solo_advancement: false` for the positive crash-recovery scenario.
- The simulator's “never create two authorities” model maintains one central global authority state, so it cannot model two independently partitioned daemons each accepting a different local epoch.

**Missing test**

A real multi-daemon 5-peer `2/3` (and 3-peer `1/2`) partition with Solo enabled while the original authority is running. Assert that the old side never gets a live writable permit once the other side can recover. The current implementation is expected to fail this test.

**Recommended remediation**

Do not automatically enter writable Solo mode after unclean multi-member quorum loss. Restrict Solo advancement to cases that cannot race a recovery majority, such as:

- single-member worlds; or
- a clean, durable relinquishment/sleep transition with a protocol proof that makes the previous authority non-writable.

If the product intentionally supports AP-style Solo forks, they must be treated as non-canonical local branches and must not share the same authority-permit semantics as canonical writable history. The UI/policy must make this explicit, and majority recovery must not simultaneously claim uniqueness.

**Confidence:** HIGH

---

## AC-03 — HIGH — Higher recovery round may switch candidate after an earlier quorum certificate already exists

**Files / functions**

- `crates/swarm-storage/src/state.rs`
  - `Storage::promise_recovery_ballot`
  - `same_recovery_base`
- `crates/swarm-consensus/src/recovery.rs`
  - `evaluate_recovery_ballot`
- `crates/swarm-cli/src/daemon.rs`
  - `drive_recovery_ballot`
  - `promote_recovery_epoch`
  - `RecoveryEpoch` handler
- `crates/swarm-cli/tests/recovery_successor_dies.rs`

**Invariant**

Once a quorum has accepted a value for a target generation, later recovery rounds must not produce a conflicting value for that same generation.

**Failure scenario**

1. Candidate B obtains and durably stores a round-1 quorum certificate for target epoch N+1, then stalls before saving the epoch.
2. B remains unavailable long enough that C starts round 2 on the same base.
3. Durable promises permit an intersecting voter to accept a strictly higher round with a **different candidate** because `same_recovery_base` excludes the candidate.
4. C obtains a round-2 certificate and promotes its own Recovery epoch N+1.
5. B later resumes. Its already-persisted round-1 certificate is sufficient for `drive_recovery_ballot` to continue into `promote_recovery_epoch`; there is no final quorum query proving the round-1 value is still the chosen value.
6. B now persists a conflicting local Recovery epoch N+1. It cannot obtain normal quorum permits if C's value has won the majority, so the write path largely fails closed, but B is stuck on a conflicting accepted generation and cannot accept C's same-number epoch as a direct extension.

**Evidence**

- Higher rounds may replace the candidate as long as canonical base fields match.
- Recovery certificate is persisted before epoch promotion.
- `recovery_successor_dies.rs` deliberately pauses the first successor after certificate persistence, then **kills** it before allowing the higher-round successor to complete. The test later restarts the first successor only after the second value is established, so it does not exercise “first candidate resumes from an already-formed old certificate.”

**Existing coverage**

Strong coverage exists for stale lower rounds, same-round equivocation, exact base anchoring, successor death, certificate signatures, and next-generation fencing.

**Missing test**

Pause B after certificate persistence; allow C to complete a higher-round certificate/epoch; then resume B without deleting B's certificate and verify B does not persist a conflicting same-generation epoch.

**Recommended remediation**

Use a value-preserving consensus rule for higher rounds. A proposer entering a higher round must learn any previously accepted/certified value from an intersecting quorum and carry that value forward rather than freely selecting a new candidate. Alternatively, make a formed certificate a durable lock that voters refuse to supersede with a different value for the same target generation. This requires more than a monotonically increasing round counter.

**Confidence:** HIGH

---

## AC-04 — HIGH — Recovery election ignores host capability and may elect a storage-only/unready authority

**Files / functions**

- `crates/swarm-cli/src/daemon.rs`
  - `maintain_authority_leases`
  - `request_host_capabilities`
  - `world_status`
- `crates/swarm-cli/src/host_readiness.rs`
  - `local_host_capability`
  - `evaluate_host_readiness`
- `crates/swarm-cli/src/migration.rs`
  - `supervise_world`

**Invariant**

A storage-only or runtime-incompatible peer must not become the automatic Minecraft authority if it cannot run the exact world.

**Failure scenario**

1. Smallest deterministic authority-eligible survivor has the exact snapshot but no verified Java/runtime configuration or required server mods.
2. Daemon recovery requests `HostCapabilityV1`, but candidate construction uses membership eligibility plus `WorldStatusV1`; it does not filter the elected candidate on `HostCapabilityV1.runtime`, `server_mods`, or conflict readiness.
3. That peer wins recovery, becomes accepted authority, and can obtain a control-plane permit.
4. The migration supervisor then blocks because runtime/config/mod readiness is missing.
5. Other host-ready peers cannot automatically recover while the newly accepted authority daemon remains connected.

**Evidence**

- `peer_capability` is collected and displayed in host-readiness diagnostics, but is not consulted in the production recovery candidate vector.
- `WorldStatusV1.authority_eligible` is derived from membership flags, not machine-local runtime readiness.
- Host-readiness reporting itself uses runtime/mod readiness to identify a true successor, demonstrating the distinction is already modeled.

**Existing coverage**

Host readiness has detailed readiness states. Recovery tests generally seed homogeneous eligible peers and do not force the deterministic winner to be storage-only.

**Missing test**

Three or five peers with the deterministic lowest-ID survivor missing runtime/mod readiness while another eligible peer is fully ready. Automatic recovery must choose a host-ready peer or remain fail-closed without installing an unusable authority.

**Recommended remediation**

Make automatic-election eligibility include a fresh signed/authenticated host capability proving exact compatibility, verified runtime, required server mods, and conflict-free state. Distinguish storage voters from host candidates in the protocol rather than overloading `authority_eligible`.

**Confidence:** HIGH

---

## AC-05 — HIGH — Duplicate daemon processes can defeat durable recovery non-equivocation

**Files / functions**

- `crates/swarm-cli/src/daemon.rs` → `run`
- `crates/swarm-storage/src/state.rs` → `Storage::promise_recovery_ballot`
- `crates/swarm-storage/src/control.rs` → `Storage::save_epoch_record`
- `crates/swarm-cli/src/migration.rs` → `AuthorityRuntimeGuard` (contrast: runtime is locked, daemon is not)

**Invariant**

One voting peer identity must not sign two conflicting promises/votes for the same recovery round.

**Failure scenario**

Two daemon processes are started against the same data directory/identity on different listen addresses. There is no process-wide daemon ownership lock. Two concurrent `promise_recovery_ballot` calls can both read the same prior state, each decide a different same-round ballot is acceptable, then race independent atomic file replacements. Each process can emit a valid vote signed by the same peer identity before the final file winner is known.

That breaks the majority-intersection argument, because the same logical voter can appear in conflicting certificates. The storage operations are individually crash-atomic but the read/check/write transaction is not mutually exclusive across processes.

**Evidence**

- `daemon::run` does not acquire an exclusive data-directory/identity lock.
- `promise_recovery_ballot` performs load → compare → atomic write without interprocess locking.
- The project already uses `fs2` locks for `AuthorityRuntimeGuard` and runtime installation, demonstrating that same-machine duplicate-process races are considered elsewhere.

**Existing coverage**

Single-process tests prove same-round conflicting ballots are rejected when serialized.

**Missing test**

Launch two daemons with the same data directory and force simultaneous conflicting recovery ballots. The second daemon should fail to start, or recovery promise persistence must serialize across processes.

**Recommended remediation**

Acquire a single exclusive daemon/data-directory lock before loading identity or participating in consensus. Independently, make promise/epoch control-state transitions transactional under a filesystem/database lock so correctness does not depend on one process.

**Confidence:** MEDIUM-HIGH

---

## AC-06 — LOW — Saturating generation increments violate strict monotonicity at `u64::MAX`

**Files / functions**

- `crates/swarm-cli/src/daemon.rs`
  - recovery generation construction
  - `promote_recovery_epoch`
  - `promote_to_solo`
  - non-recovery/recovery transition validation
- `crates/swarm-protocol/src/v2.rs`
  - `RecoveryBallotV1::generation_is_well_formed`

**Invariant**

Every epoch/fencing transition must strictly increase the generation.

**Evidence**

Several production paths use `saturating_add(1)`. At `u64::MAX`, the “next” value equals the previous value, and validation using the same saturating expression can accept a non-increasing transition. Manual transfer correctly uses `checked_add`, showing the safer pattern.

**Impact**

Practically unreachable through normal lifetime, but it is still a formal violation of the fencing invariant and complicates proofs/fuzzing.

**Missing test**

Boundary tests at `u64::MAX` for recovery, Solo, and inbound epoch validation.

**Recommended remediation**

Use `checked_add(1)` and fail closed on generation exhaustion everywhere authority epoch/fencing state advances.

**Confidence:** HIGH

---

## AC-07 — LOW — Legacy/test-only consensus models diverge from production authority semantics

**Files / modules**

- `crates/swarm-consensus/src/migration.rs`
- `crates/swarm-consensus/src/recovery.rs`
- `crates/swarm-consensus/src/simulator.rs`
- production runtime in `crates/swarm-cli/src/daemon.rs`

**Issue**

The consensus crate contains duplicate authority-generation/lease abstractions and helper logic that code search shows is primarily exercised by unit tests, while production daemon logic reimplements key election, lease, recovery-promise, and migration decisions. The simulator also centralizes one global authority state, so it cannot represent all distributed local-state divergence that production can experience.

**Impact**

Tests can remain green while production safety semantics drift. AC-01 and AC-02 are examples of behaviors the central simulator cannot naturally represent.

**Recommended remediation**

Collapse shared invariants into one production-used consensus state machine, or make process-level distributed tests the authoritative safety gate. Remove/rename legacy helpers that no longer govern runtime behavior.

**Confidence:** HIGH

---

# Required adversarial assessments

## Authority epochs and fencing

**Positive controls**

- Inbound leases must match the locally accepted `(epoch,fencing_token)` exactly.
- Inbound recovery/non-recovery epochs must extend local history exactly, including previous epoch hash where applicable.
- Snapshot manifests must match the locally accepted epoch and authority.
- The runtime checks authority generation before launch, restore, checkpoint, and while supervising the live server.

**Residual problem**

These are local fences. They are only globally safe if peers agree on the canonical membership and epoch lineage. AC-01 demonstrates a case where two partitions can each have internally consistent but incompatible local authority universes.

## Simultaneous elections and deterministic tie breaking

- Candidate ranking is deterministic on accepted epoch, canonical sequence, then peer ID.
- Same-round durable promises reject conflicting ballots when operations are serialized on one daemon.
- Quorum intersection prevents two same-round certificates only if every logical voter is non-equivocating and every peer uses the same voter set.
- AC-01 breaks the same-voter-set premise; AC-05 can break non-equivocation; AC-03 weakens value preservation across higher rounds.

## Membership changes during elections

**FAIL.** Membership hash binding detects disagreement but does not decide which membership is canonical. Because membership is not quorum-committed before redefining quorum size, stale and new memberships can each authorize different quorums. See AC-01.

## Stale authority rejection

**PASS locally after newer state is accepted.** A peer that has accepted N+1 rejects N leases/snapshots. The system does not, however, guarantee all partitions accept N+1 before a stale side remains writable; AC-01/AC-02 are the counterexamples.

## 1/1 partition

With two synchronized members and Solo disabled, automatic recovery is fail-closed because one survivor cannot form 2/2 majority. With Solo enabled, the current authority may enter Solo; the other side cannot independently recover with only one vote, so this topology alone does not create the same majority-vs-Solo race as 2/1 or 3/2.

## 3/2 partition

**FAIL under supported states.** AC-01 can create two quorums from divergent membership views. Even with synchronized membership, AC-02 permits a minority containing the old authority to enter Solo while a 3-member majority recovers.

## Network heal

Exact-generation checks prevent silent overwrite, which is good. But they also mean conflicting same-number epochs or divergent Solo/recovery histories do not automatically converge. Solo conflicts are preserved for manual resolution. AC-03 can strand one replica on a conflicting same-generation Recovery epoch.

## Delayed/reordered lease messages

Generally fail-safe. Lease expiry uses local monotonic `Instant`; delayed current-generation leases can postpone recovery but cannot authorize a different generation. After a newer epoch is locally accepted, old leases are rejected.

## Clock skew

No canonical authority ordering depends on wall-clock time. Lease/recovery waiting uses monotonic local time. This is a positive control.

## Replayed epoch records

Identical current epoch is idempotently accepted; stale/non-direct extensions are rejected. Recovery epochs additionally require a certificate. Positive, subject to divergent local membership/epoch state.

## Permanent loss of quorum

- Solo disabled: fail-closed; permit expires and no recovery without majority.
- Solo enabled: fail-open into writable Solo, which is the core of AC-02 for multi-member partitions.

## Storage-only / incompatible peer eligibility

Compatibility fingerprint and snapshot state are checked, but machine host readiness is not an election requirement. Storage-only/unready peers can become authority. See AC-04.

---

# Tests and evidence reviewed

Representative reviewed coverage includes:

- `crates/swarm-consensus/src/lib.rs` quorum/election/fencing tests
- `crates/swarm-consensus/src/simulator.rs` crash/partition simulator
- `crates/swarm-consensus/tests/migration.rs`
- `crates/swarm-cli/tests/three_daemon_recovery.rs`
- `crates/swarm-cli/tests/recovery_successor_dies.rs`
- `crates/swarm-cli/tests/migration_core.rs`
- `crates/swarm-cli/tests/manual_transfer_process_gate.rs`
- `crates/swarm-cli/tests/live_join_replication.rs`
- authority permit and host-readiness unit coverage

The major missing class is **distributed tests with intentionally divergent control-plane state**: different membership versions, competing locally accepted epochs, duplicate daemon instances, and a live old authority in one side of a partition while another side attempts recovery.

---

# Recommended remediation order

1. **AC-01 first:** make membership/quorum-set changes consensus-controlled and jointly committed. No other quorum proof is trustworthy while voter-set divergence can create two majorities.
2. **AC-02:** prohibit automatic writable Solo after unclean multi-member quorum loss, or redesign Solo as explicitly non-canonical local progress that cannot race canonical recovery.
3. **AC-03:** replace free candidate switching across higher recovery rounds with a value-preserving accepted-value protocol.
4. **AC-05:** add a daemon/data-dir singleton lock and transactional consensus-control persistence.
5. **AC-04:** require fresh host capability for automatic authority candidacy.
6. **AC-06/AC-07:** remove saturation ambiguity and consolidate duplicated consensus models/tests.

After fixes, re-run process-level 3-peer and 5-peer partition campaigns with membership churn, candidate crashes at every persistence boundary, duplicate/reordered messages, duplicate local daemon startup, and host-readiness heterogeneity.

---

# Final verdict

The current tree has meaningful and well-implemented fencing mechanisms, but their safety proof assumes a single canonical membership view and no writable Solo/majority-recovery race. Production code does not enforce those assumptions.

**VERDICT: FAIL**
