# Authority recovery acceptance checklist

This checklist turns the v0.1.0-preview crash-recovery contract into reproducible acceptance scenarios.

## Automated checks

The normal CI matrix must pass on Linux and Windows before any manual recovery demo is considered valid:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

The Fabric server mod, desktop shell, and RustSec dependency audit must also remain green.

The consensus suite includes a deterministic 1,000-crash soak. Every completed migration must advance epoch and fencing token exactly once and the previous token must fail renewal.

## Three-peer hard-crash scenario

Use three canonical members: Alice, Bob, and Charlie. All three start from the same verified snapshot and compatibility fingerprint.

1. Start all three daemons and standby host supervisors.
2. Confirm Alice owns the accepted epoch and only Alice's Minecraft runtime is active.
3. Confirm Alice receives a changing multi-member `authority.permit` from fresh lease quorum.
4. Hard-kill Alice without a graceful sleep record.
5. Bob and Charlie must not recover before Alice's last lease expires plus the recovery settle delay.
6. Bob and Charlie exchange fresh `WorldStatusV1` and must agree on the exact canonical epoch, sequence, snapshot hash, state root, and compatibility fingerprint.
7. The deterministic election must choose the same successor on both survivors.
8. The chosen successor must obtain a durable next-generation reservation majority.
9. Reservation quorum alone must not create a live authority permit.
10. The successor publishes exactly one Recovery epoch with epoch and fencing token incremented by one.
11. Recovery-epoch quorum alone must still not create a live authority permit.
12. The successor creates the zero-change promotion snapshot and promoted membership record in the new epoch.
13. The successor obtains fresh current-generation lease quorum.
14. Only now may the local permit heartbeat begin changing and the standby host launch Minecraft.
15. At no point may two Minecraft authorities be simultaneously permitted to write canonical state.

## Old-authority restart scenario

Continue from the previous scenario after Bob or Charlie has recovered the world.

1. Restart Alice's daemon and standby supervisor with Alice's old local epoch still on disk.
2. Alice must not restart Minecraft from the stale epoch.
3. Alice requests fresh world status from connected canonical members because it cannot obtain quorum for its old lease.
4. Alice may adopt the immediately next Recovery epoch only after directly observing a fresh canonical majority already agreeing on the exact promoted state.
5. Alice is not counted as part of that majority until it adopts the recovered epoch.
6. Alice then receives the promoted membership and snapshot through authenticated replication.
7. Alice remains a replica/standby unless a later canonical authority transition selects it.
8. Alice's old fencing token must remain invalid.

## Partition scenario

1. Start Alice as authority with Bob and Charlie as replicas.
2. Isolate Alice from Bob and Charlie while keeping Alice's process alive.
3. Alice's permit must stop changing when it loses fresh quorum, causing Fabric fencing rather than continued canonical writes.
4. Bob and Charlie may recover only after the old lease-expiry window and only if they form the canonical majority.
5. When the partition heals, Alice must converge to the newer Recovery epoch and must not regain authority from its stale generation.

## No-quorum scenario

For a three-member canonical world, leave only one member visible after an unclean authority failure.

Expected result:

- no automatic crash takeover;
- no `SOLO` fallback regardless of elapsed wall-clock time;
- no live authority permit;
- no Minecraft authority launch.

Safety intentionally wins over availability here.

## Clean sleep/wake scenario

This is intentionally different from crash recovery.

1. Gracefully stop the active authority through the Fabric shutdown barrier.
2. Commit the final snapshot and signed sleep record.
3. Take every peer offline.
4. Bring back one eligible peer that holds the exact sleeping snapshot.
5. That peer may wake the world in `SOLO` mode because the previous authority explicitly relinquished canonical ownership.
6. Epoch and fencing token still advance monotonically.

## Second failure during recovery

If the chosen successor obtains durable next-generation reservations and then dies before Recovery epoch quorum, the current preview must stall safely instead of time-expiring the reservation and risking split brain.

This known liveness limitation is acceptable for v0.1.0-preview only if it remains documented in `docs/AUTHORITY_RECOVERY.md`. A later recovery-round/ballot mechanism should address it without weakening majority intersection.

## Release acceptance

A preview build is not ready to merge/release until:

- the full CI matrix is green on the final commit;
- the deterministic failure and 1,000-crash soak tests pass;
- the three-peer hard-crash scenario completes end to end;
- the old-authority restart scenario converges correctly;
- the no-quorum scenario never starts an authority;
- the clean sleep/wake scenario still works;
- Windows and Linux packaged builds can run the daemon, standby supervisor, and Fabric bridge together.
