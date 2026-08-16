# Authority Recovery in SwarmCraft 0.2.x

This document describes the crash-recovery path implemented by the current preview runtime.

SwarmCraft has one Minecraft simulation authority per world at a time. It does not distribute ticks or chunks across multiple simultaneous authorities.

The recovery design prioritizes **canonical safety first, then liveness**. A peer does not become authoritative merely because it noticed a failure first.

## Recovery overview

For a multi-member world, the normal path is:

```text
current authority
    |
    | signed leases + fresh quorum acknowledgements
    v
live authority permit

current authority disappears
    |
    v
lease/recovery delay expires
    |
    v
survivors compare canonical state
    |
    v
deterministic eligible successor
    |
    v
signed durable recovery ballot
    |
    v
majority votes / recovery certificate
    |
    v
new Recovery epoch + higher fencing token
    |
    v
fresh current-generation lease quorum
    |
    v
live authority permit
```

The ballot/certificate step is durable and monotonic. It replaced the v0.1 preview's single next-generation reservation mechanism.

---

## Normal authority leases

For a multi-member world, the accepted authority periodically sends a signed `AuthorityLeaseGrantV1` for the exact accepted `(epoch, fencing_token)` generation.

A replica accepts the lease only when protocol, membership, identity and generation checks succeed.

The authority counts fresh acknowledgements from non-banned canonical members. A local `authority.permit` heartbeat is refreshed only while the required quorum is fresh.

The Fabric bridge watches this permit for multi-member worlds. If the permit stops changing, the Minecraft process is fenced and terminated rather than being allowed to continue writing as if it still held canonical authority.

This permit is a local runtime enforcement mechanism. It does not replace the signed canonical epoch/lease state.

---

## Crash detection

A replica considers recovery only when all relevant safety conditions are satisfied, including:

- the accepted authority is no longer an authenticated connected peer;
- the world is not in durable sleep;
- the local peer is a non-banned, authority-eligible member;
- the recovery delay/window has opened;
- enough peers are visible to satisfy quorum;
- the candidate has a verified local copy of the canonical snapshot.

Recovery does not use wall-clock timestamps as canonical ordering. Runtime expiry/delay decisions use local monotonic time, while canonical authority ordering comes from epochs, fencing tokens, signed records and recovery rounds.

---

## Canonical recovery view

Survivors exchange fresh `WorldStatusV1` state.

A peer is included in the recovery view only when its status agrees with the candidate's accepted canonical base on the critical fields used by recovery, including:

- world ID;
- accepted epoch;
- canonical sequence;
- latest snapshot hash;
- state root;
- compatibility fingerprint.

Peers that are stale, incompatible, banned, storage-only or otherwise authority-ineligible do not become candidates merely because they are reachable.

Automatic crash recovery requires a visible majority of the canonical active membership. It does **not** silently fall back to solo mode after an unclean authority crash.

Among eligible candidates with the same accepted state, deterministic authority ranking chooses the successor.

---

## Durable recovery ballot

The elected candidate creates a signed `RecoveryBallotV1` anchored to one exact canonical base.

The ballot binds at least:

- world ID;
- base epoch and fencing token;
- target epoch and fencing token;
- monotonically increasing recovery round;
- candidate peer identity/public key;
- canonical base snapshot hash;
- canonical base state hash;
- canonical membership hash.

The target generation advances monotonically from the accepted base.

A ballot is not permission to run Minecraft. It is a proposal to recover a specific canonical base into a specific next authority generation.

---

## Durable promises and votes

Each voter validates the ballot and persists its highest accepted recovery promise before allowing that vote to matter.

A recovery promise prevents a peer from later helping an older/incompatible round gain authority.

Important properties:

- the round is monotonic for the same recovery base;
- a higher valid round can supersede an abandoned earlier candidate/round;
- old rounds become stale once an intersecting majority has moved forward;
- a ballot on a different canonical base is not interchangeable with the current recovery attempt;
- votes are signed and bound to the ballot/candidate/round.

The local candidate also persists its own promise/vote through the same durable path.

---

## Recovery certificate

Once the candidate collects a quorum of valid votes matching the ballot, it builds a `RecoveryCertificateV1`.

The runtime validates the certificate shape against canonical membership and verifies the included signatures.

The certificate is persisted **before** the new recovery epoch is promoted.

That ordering matters: a crash after the certificate write but before epoch promotion can retry safely without granting an older recovery round authority.

---

## Recovery epoch

After a valid durable certificate exists, the candidate promotes a new `EpochRecordV1` in recovery mode.

The new record advances:

```text
epoch        = base_epoch + 1
fencing      = base_fencing_token + 1
```

and remains anchored to the verified canonical base state.

The recovery epoch is replicated together with its quorum certificate. A peer must not accept a recovery epoch merely because the record is correctly signed by the candidate; the recovery proof matters.

---

## Recovery authority remains quorum-dependent

Winning the recovery ballot is not a permanent blank check.

While running in Recovery mode, the accepted authority must continue to satisfy the runtime's recovery/quorum requirements and obtain fresh current-generation lease acknowledgements before the local authority permit remains live.

If the required quorum disappears, the local permit is cleared and the Minecraft authority is fenced rather than continuing indefinitely from a partition minority.

This keeps three separate concepts distinct:

- **elected recovery successor**;
- **accepted recovery epoch with quorum proof**;
- **currently live Minecraft authority with fresh lease quorum**.

---

## What happens if the elected successor dies?

This changed materially from v0.1.

The old preview could safely stall if a successor died after durable reservation but before recovery completed.

Current 0.2.x recovery uses monotonic ballots/promises. If an earlier recovery attempt is abandoned, a later valid candidate can propose a **strictly higher round on the same canonical base** and collect a new majority certificate.

Majority intersection plus durable promises prevents the abandoned older round from later becoming valid again after an intersecting quorum has advanced.

This behavior is covered by the process-level `recovery_successor_dies` acceptance test and is treated as a permanent regression gate.

---

## Returning stale authority

When the old authority returns, its earlier epoch/fencing token is stale.

It must not regain canonical write permission simply because it still has old local files or a previously valid signature.

The returning peer synchronizes from accepted current state through authenticated replication/recovery rules. Old-generation lease/permit state cannot renew the newer accepted authority generation.

The safety rule is simple:

> A valid signature from an old authority does not make an old generation current.

---

## Network partition behavior

For a multi-member canonical world, a partition minority must not continue claiming normal quorum-backed authority.

If the current authority loses fresh quorum, its permit eventually stops and the Fabric runtime is fenced.

A surviving partition can recover only when it can form the required canonical majority and agree on the exact accepted base state.

Automatic crash recovery never lowers the quorum threshold merely because availability is poor.

Solo mode is a separate, explicitly signed world-policy path and is not a generic escape hatch for unclean multi-member authority failure.

---

## Clean sleep/wake is different from crash recovery

A graceful world shutdown produces a signed durable sleep record after the final verified snapshot is committed.

Wake is therefore not treated as an unclean missing-authority election. An eligible peer that holds the exact sleeping snapshot can advance the world through the wake logic with monotonically increasing epoch/fencing state.

The current host/runtime supports this durable sleep/wake foundation.

---

## Product boundary

The control-plane recovery described here is implemented and process-tested.

The complete player experience is **not yet seamless**.

Current missing integration work includes:

- automatically launching the correct Minecraft runtime immediately when this peer becomes the accepted recovery authority;
- automatically directing/reconnecting players to that successor runtime;
- repeated full Minecraft gameplay handoff testing across real networks.

Do not translate "authority recovery is implemented" into "Minecraft host migration is already seamless."

---

## Related implementation and acceptance docs

- [Implementation status](IMPLEMENTATION_STATUS.md)
- [Recovery acceptance](RECOVERY_ACCEPTANCE.md)
- [Release gates](RELEASE_GATES.md)
- [Network validation](NETWORK_VALIDATION.md)
- [`crates/swarm-cli/src/daemon.rs`](../crates/swarm-cli/src/daemon.rs)
- [`crates/swarm-consensus/`](../crates/swarm-consensus/)
