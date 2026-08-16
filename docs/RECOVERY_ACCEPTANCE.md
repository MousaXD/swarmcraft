# Authority Recovery Acceptance Checklist

This checklist turns the current 0.2.x crash-recovery contract into reproducible acceptance scenarios.

It distinguishes **control-plane recovery correctness** from the still-incomplete **automatic Minecraft host-handoff product flow**.

## Automated baseline

The permanent CI matrix must pass on the final commit.

Core gates include:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

Platform-specific Rust gates, Fabric build, RustSec and desktop package jobs must also remain green according to `docs/RELEASE_GATES.md`.

---

## Permanent process-level recovery tests

The repository CI currently includes dedicated acceptance tests for:

- live join plus immediate snapshot replication;
- host process/Fabric lifecycle;
- three-daemon hard-kill recovery;
- recovery successor dying before epoch promotion;
- solo-history acceptance and divergence detection.

These tests exercise real processes/local networking rather than only in-memory state machines.

---

## Three-member hard-crash control-plane scenario

Use three canonical non-banned members: Alice, Bob and Charlie. All start from the same verified canonical snapshot, membership and compatibility fingerprint.

1. Start the peer daemons and establish authenticated connectivity.
2. Confirm Alice owns the accepted authority generation.
3. Confirm Alice's multi-member authority permit is refreshed only while fresh lease quorum exists.
4. Hard-kill Alice without creating a graceful sleep record.
5. Bob/Charlie must not recover before the configured recovery window opens.
6. Survivors exchange fresh `WorldStatusV1` state.
7. Recovery participants must agree on the exact canonical base used by the algorithm: epoch, sequence, snapshot hash, state root and compatibility fingerprint.
8. A visible canonical majority must exist.
9. Deterministic candidate ranking must select the same eligible successor from the same view.
10. The successor creates a signed recovery ballot anchored to that exact base.
11. Each voter persists its recovery promise before its vote is considered durable.
12. The successor must collect a valid majority of votes matching one ballot/round.
13. The recovery certificate must be persisted before epoch promotion.
14. The new Recovery epoch must advance epoch and fencing token monotonically.
15. Peers must reject stale/invalid recovery records that lack the required proof or canonical base agreement.
16. The accepted recovery authority must still obtain/maintain the required fresh lease quorum before its local authority permit remains live.
17. At no point may two different authority generations both hold valid current write permission.

Expected result:

- one recovery generation becomes accepted;
- the previous authority generation is fenced;
- canonical snapshot/history identity is preserved;
- normal replication can continue from the recovered generation.

---

## Recovery successor dies scenario

This is a permanent regression scenario for the 0.2 ballot design.

1. Start a recovery attempt from a valid canonical base.
2. Allow the first candidate/round to become durably promised by some participants.
3. Kill or otherwise abandon that candidate before the Recovery epoch is successfully promoted.
4. A later eligible candidate proposes a **strictly higher recovery round on the same canonical base**.
5. Durable promises must reject attempts to resurrect an older/lower round after an intersecting quorum has advanced.
6. The new candidate collects a fresh majority certificate.
7. Recovery completes without lowering quorum requirements.

Expected result:

- safety is preserved;
- the abandoned old round cannot later become authoritative;
- liveness can recover through the higher round.

This replaces the v0.1 preview behavior that could safely stall after an abandoned reservation.

---

## Old-authority restart scenario

Continue after Bob or Charlie has recovered the world.

1. Restart Alice with old local epoch/fencing state still on disk.
2. Alice must not regain a live multi-member authority permit for the stale generation.
3. Old lease/fencing state must not renew against the newer accepted generation.
4. Alice must observe/synchronize accepted current world state through authenticated protocol paths.
5. Recovery epoch/certificate and canonical snapshot/membership/configuration must validate before Alice treats them as current.
6. Alice remains a replica unless a later valid authority transition selects it.

Expected result:

- Alice converges to the newer accepted state;
- no stale-authority write is accepted merely because Alice was previously valid.

---

## Partition scenario

1. Start Alice as accepted authority with Bob and Charlie as canonical replicas.
2. Isolate Alice from Bob/Charlie while keeping Alice alive.
3. Alice must stop receiving enough fresh lease acknowledgements to maintain quorum.
4. Alice's local permit eventually stops changing and the Fabric permit guard fences the Minecraft runtime.
5. Bob and Charlie may recover only after the recovery window opens and only if they form the required canonical majority on one exact base.
6. When the partition heals, Alice must converge to the newer accepted generation and remain unable to revive its stale fencing token.

Expected result:

- the minority partition cannot continue normal quorum-backed canonical writes indefinitely;
- majority recovery does not require trusting wall-clock ordering or "last writer wins."

---

## No-quorum scenario

For a three-member canonical world, leave only one member visible after an unclean authority failure.

Expected result:

- no automatic crash takeover;
- no quorum threshold reduction;
- no generic `SOLO` fallback for this unclean multi-member crash;
- no current-generation live authority permit;
- no claim that canonical recovery succeeded.

Safety intentionally wins over availability here.

---

## Clean sleep/wake scenario

Clean sleep is intentionally different from crash recovery.

1. Gracefully stop the active authority through the Fabric shutdown barrier.
2. Commit the final verified signed snapshot.
3. Persist the signed sleep record.
4. Take every peer offline.
5. Bring back one eligible peer holding the exact sleeping snapshot.
6. Wake logic must reject a stale replica whose latest snapshot does not match the sleeping state.
7. A valid wake advances epoch/fencing monotonically and clears durable sleep state as appropriate.

Expected result:

- no crash-recovery ballot is required merely because everyone was intentionally offline;
- wake starts from the exact durable sleeping checkpoint.

---

## Solo-history scenario

For a world whose signed configuration explicitly allows solo advancement:

1. Lose quorum without treating the situation as an unclean automatic crash takeover by an arbitrary peer.
2. The accepted authority may enter explicit solo mode only through the signed world policy/runtime rules.
3. Persist signed solo branch ancestry/head state.
4. Advance snapshots while clearly reporting reduced durability/safety.
5. Reconnect a compatible peer with matching ancestry.
6. Compatible history may reconcile and return to quorum-backed operation.

Expected result:

- solo progress is never mislabeled as quorum-backed safety;
- compatible solo history can be adopted safely.

---

## Divergent solo branches

1. Construct or reproduce independently advanced solo branches from a shared base.
2. Reconnect the branches.
3. The runtime must detect that neither branch is a simple compatible continuation of the other.
4. Both branches must remain preserved for recovery/manual resolution.
5. No automatic semantic Minecraft merge is attempted.

Expected result:

- visible conflict state;
- no silent last-writer-wins replacement;
- no destroyed branch merely to make the UI look healthy.

---

## Product-level host migration scenario

This is the **remaining MVP integration target**, not yet a fully automated permanent acceptance gate.

The desired end-to-end scenario is:

1. Alice runs the authoritative Minecraft world.
2. Bob holds a verified synchronized replica and is authority eligible.
3. Alice's process/machine is hard-killed.
4. Bob wins control-plane recovery safely using the ballot/certificate protocol.
5. Bob's host supervisor automatically restores/launches the correct Minecraft/Fabric runtime.
6. Players are redirected/reconnected to Bob.
7. Gameplay continues from the accepted safe checkpoint.
8. Alice later returns as a stale peer and synchronizes without regaining old authority.

Until steps 5 and 6 are automatic and repeatedly tested, documentation must say **automatic authority recovery is implemented** rather than **seamless Minecraft host migration is complete**.

---

## Release acceptance

A preview change touching recovery/authority code is not ready until:

- the full relevant CI matrix is green on the final commit;
- process-level recovery tests pass;
- successor-dies recovery remains live and safe;
- stale-authority behavior remains fenced;
- no-quorum behavior does not invent availability;
- clean sleep/wake behavior still validates exact snapshot state;
- solo-history divergence remains preserved rather than silently merged;
- documentation is updated if the safety contract changes.

See `docs/AUTHORITY_RECOVERY.md`, `docs/RELEASE_GATES.md` and `docs/IMPLEMENTATION_STATUS.md`.
