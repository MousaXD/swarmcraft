# Authority recovery in v0.1.0-preview

This document describes the authority crash-recovery path implemented by the preview runtime.

The preview has one Minecraft simulation authority at a time. It does not distribute ticks or chunks across authorities.

## Safety goal

After an unclean authority failure, SwarmCraft must not start a replacement Minecraft authority merely because one peer noticed the failure first.

A replacement becomes live only after three distinct majority gates:

1. next-generation reservation;
2. recovery-epoch acceptance;
3. live authority-lease acknowledgement.

These acknowledgements are intentionally tracked separately. Passing an earlier gate cannot be reused as proof for a later gate.

## Normal authority lease

For a multi-member world, the current authority periodically sends a signed `AuthorityLeaseGrantV1` for the accepted `(epoch, fencing_token)` generation.

A replica accepts the lease only when:

- the sender is the authenticated application peer in the signed lease;
- the peer is a non-banned, authority-eligible member with the expected public key;
- the generation exactly matches the locally accepted epoch;
- the lease peer/key exactly matches the locally accepted epoch authority;
- the lease duration matches the preview policy.

The authority counts only fresh acknowledgements from the canonical membership. It refreshes the local `authority.permit` heartbeat only while a majority is fresh.

The Fabric bridge watches that permit. If the permit stops changing, the old Minecraft authority is fenced and stopped rather than continuing to write canonical state.

## Crash detection

A replica considers crash recovery only when all of the following are true:

- the accepted authority is no longer an authenticated connected peer;
- the last accepted authority lease has expired locally;
- the recovery settle delay has elapsed;
- the world is not in durable sleep;
- the local peer is an eligible member;
- the canonical world has more than one active member.

Lease expiry uses local monotonic time. Wall-clock timestamps are not used to decide whether an authority lease expired.

## Canonical recovery view

Survivors exchange `WorldStatusV1` and include a peer in the recovery view only when its fresh status agrees exactly on:

- world ID;
- accepted epoch;
- latest snapshot sequence;
- latest snapshot hash;
- state root;
- compatibility fingerprint.

The candidate itself must hold the verified canonical snapshot locally.

A crash recovery attempt requires a visible majority of the canonical non-banned membership. Automatic crash recovery never falls back to solo mode.

Among eligible visible candidates with the same canonical state, the existing deterministic authority election chooses one successor. The stable peer-ID tie-break means peers with the same view choose the same candidate.

## Gate 1: durable next-generation reservation

The candidate creates a signed `AuthorityLeaseGrantV1` for exactly:

```text
next_epoch = current_epoch + 1
next_fencing_token = current_fencing_token + 1
```

During recovery this signed lease is used as a next-generation reservation, not as permission to run Minecraft.

Each accepting peer persists the reservation at:

```text
worlds/<world>/metadata/recovery-reservation.postcard
```

Persistence provides the important one-successor-per-generation rule across daemon restarts:

- an older generation cannot overwrite a newer reservation;
- the same candidate/generation may be replayed idempotently;
- a different candidate cannot replace the same reserved generation;
- a later generation may supersede an older reservation only after canonical state has advanced.

The candidate must receive a majority of reservation acknowledgements before creating the recovery epoch.

A reservation acknowledgement is not an epoch acknowledgement and is not a live lease acknowledgement.

## Gate 2: recovery epoch quorum

After reservation quorum, the candidate creates a signed `EpochRecordV1` with:

```text
mode = RECOVERY
epoch = previous_epoch + 1
fencing_token = previous_fencing_token + 1
previous_epoch_hash = hash(previous_epoch_record)
base_state_hash = latest_verified_snapshot.state_root
```

A replica accepts a Recovery epoch only when it matches the durable next-generation reservation and advances epoch/fencing exactly once.

Exact epoch replay is idempotent so a daemon can reconnect or restart and collect acknowledgement again without manufacturing another generation.

The recovered authority does not mint a permit after writing its own epoch. It must first receive `EpochAccepted` from a canonical majority.

This separate gate prevents reservation acknowledgements from being mistaken for proof that peers accepted the new epoch.

## Promotion snapshot and membership

The Recovery epoch is based on the last verified canonical snapshot. Before Minecraft starts, the recovered authority creates a zero-change promotion snapshot in the new epoch:

- a new snapshot number;
- a new sequence;
- `previous_snapshot_hash` pointing to the old canonical snapshot;
- the same verified state root;
- the same content-addressed blob descriptors;
- the new authority identity and signature.

No Minecraft data is mutated by this promotion. Existing verified blobs are reused.

The authority also promotes the membership record into the new epoch and signs it as the accepted authority. Recovery artifact creation is idempotent, allowing the daemon to reconstruct the promotion after a local restart.

## Gate 3: live authority lease quorum

Only after Recovery epoch quorum does the new authority send ordinary current-generation leases.

These responses are stored in a separate live-lease acknowledgement map and must be fresh.

Only a fresh canonical majority can refresh `authority.permit`.

The standby host process requires both:

- local ownership of the accepted authority epoch;
- a changing multi-member authority permit.

Therefore Minecraft starts only after all three recovery gates have succeeded.

## Fencing the previous authority

When the Recovery epoch is accepted, both epoch and fencing token increase.

Any old authority operating with the previous fencing token is stale. Even if it later reconnects, its generation cannot be renewed as the current authority generation.

The deterministic failure simulator asserts that the previous authority cannot renew after the recovery epoch advances.

## Durable sleep is different from crash recovery

`SOLO` remains valid for a world that was cleanly placed into durable sleep and then woken from the exact latest sleeping snapshot.

It is not an automatic crash-recovery fallback.

This distinction is intentional:

- clean sleep contains explicit canonical relinquishment evidence;
- an unclean crash does not.

## Deterministic failure tests

The simulator covers at least these invariants:

- recovery before lease expiry is rejected;
- an accepted authority that is still visible blocks recovery;
- corrupt or divergent replicas cannot take over;
- automatic crash recovery never falls back to solo;
- reservation quorum alone cannot activate the successor;
- epoch quorum without live-lease quorum cannot activate the successor;
- the old fencing token cannot renew after the Recovery epoch advances;
- a conflicting candidate cannot replace an in-progress reserved generation;
- the deterministic candidate tie-break rejects a different visible winner;
- all-offline durable sleep/wake still works independently.

## Known liveness limitation

The current preview deliberately prefers safety over liveness in one second-failure case.

If a candidate obtains durable next-generation reservations and then dies before the Recovery epoch is accepted by a majority, those reservations are not automatically discarded by a timer. Another candidate therefore cannot immediately take the same generation.

This can stall automatic recovery, but it does not create two authorities.

A future hardening step should introduce an explicit monotonic recovery round/ballot that can supersede an abandoned reservation while preserving majority intersection. Do not solve this by independently expiring durable reservations from wall-clock time: asymmetric clocks and stale candidates could otherwise reopen split-brain risk.

## Preview threat model

This mechanism is crash/partition safety for an invite-oriented friend-group preview. It is not Byzantine consensus and does not make a malicious canonical majority safe.

All protocol records still require authenticated channels, membership checks, signatures, exact generation checks, state-hash checks, and fencing.
