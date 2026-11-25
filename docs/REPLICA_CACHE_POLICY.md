# Local replica placement and background scheduling (#36)

These preferences belong to one device and one world. They are not signed
world configuration, membership, hosting consent, a recovery certificate,
or permission to delete a save. A cache-only/player-only device may retain
verified snapshots without becoming a host candidate.

## Operator commands

```sh
# Inspect the current local choice, storage quota and verified local head.
swarmcraft world replica-status "$WORLD"

# Stop following new ordinary snapshots on this device. Keep all existing
# committed history, complete blobs, and resumable partial data.
swarmcraft world replica-policy "$WORLD" --follow false

# Follow a frequently used world, with a bounded scheduling preference.
# Ordinary payloads may be sent/requested between 22:00 and 06:00 UTC.
# The per-world requested/served encoded payload rate is 256 KiB/s.
swarmcraft world replica-policy "$WORLD" --follow true --hot \
  --utc-window 22:00-06:00 --max-kib-per-second 256

# Restore the legacy automatic-follow policy (no explicit window/rate).
swarmcraft world replica-policy "$WORLD" --follow true

# Allow this non-authority device to offer its retained snapshots to
# authenticated current members. Seeding remains a separate local choice.
swarmcraft world seed "$WORLD" true

# Limit additional world payload writes (example: 10 GiB local quota).
swarmcraft world storage-policy "$WORLD" --max-world-bytes 10737418240
```

The policy command replaces the entire local policy: omitted window, rate,
and hot preference are cleared. Without a saved policy, existing automatic
replication behavior is retained. A UTC window is start-inclusive and
end-exclusive, supports midnight wrap, and cannot have equal endpoints.
The encoded-payload rate accepts 4–65,536 KiB/s. A malformed, oversized,
or symlink policy is not silently interpreted as an unrestricted policy.
Placement is selected by applying these choices independently on devices
that are already admitted members of the world. No device may force a
remote device to cache or seed its data.

## Scheduling and safety

* A paused/closed receiver refuses new ordinary negotiations and
  withdraws an active pull on the next scheduler tick. Late pull replies
  are not written while the policy is closed. Existing committed saves
  and partial bytes are preserved; resumption renegotiates exact pins.
* An explicit receive window or rate requires controlled receiver-driven
  pulls. A single authenticated eligible source suffices; the receiver
  still verifies the current signed membership, epoch, manifest and blob
  content. Old peers or bootstrap states that cannot support this path
  are deferred rather than falling back to unrestricted incoming pushes.
  Finish the initial synchronization with an unrestricted follow policy
  before applying a controlled window/rate.
* A controlled world has at most one outstanding pull. The daemon
  accounts for requested encoded bytes before the next dispatch and
  charges outgoing actual encoded payload bytes to the same per-world
  budget, without accumulated idle credit. At most one quarter-second
  of configured payload (also bounded by the wire chunk limit) may be
  dispatched initially. Existing active-game pacing and the four global
  pull slots remain in effect. The hot preference advances scheduling
  priority by at most 500 ms; it cannot starve older ready work.
* Ordinary push senders and pull sources recheck local send windows and
  policy changes before dispatch; a non-authority sender also needs
  continuing background-seeding consent. A receiver changing settings
  cannot recall a packet already on the wire.
* Controlled pulls preflight the existing nonblocking world disk quota
  and free-space reserve before requesting a chunk; actual writes still
  acquire storage admission. The quota is enforced against local payload
  growth and preserves emergency metadata. Pausing is not eviction;
  every committed checkpoint remains a retention root.
* Certified historical replay follows its separately verified protocol
  and is not subject to the ordinary cache window/rate. The existing
  storage quota and canonical fences still apply. Ordinary transfer
  boundaries, however, may need an exact replica acknowledgment; a
  user-selected pause or limit can delay transfer, safe sleep or recovery
  until sufficient copies are synchronized. It never releases a gate.

The rate is a daemon-local, per-world encoded-payload dispatch budget,
not an OS-wide traffic shaper or a daily data allowance. Control messages,
encryption/framing, already-issued requests, historical replay, other
worlds, and other applications are not included. Network jitter can
affect observed instantaneous link traffic. A daemon restart starts a
fresh bounded dispatch budget. This feature does not promise an exact
WAN link cap or aggregate filesystem usage across all worlds.

## Recoverability observations

`replica-status` verifies the locally referenced canonical manifest
signature and payload. When a fresh daemon report matches the exact
head, current membership and epoch, it separately lists connected peers
whose recent status matches the head and those with a corresponding
exact acknowledgment observed by the daemon. The preferred replication
factor is treated as the desired total number of copies for this
diagnostic; local verified payload plus observed exact remote receipts
form the displayed copy count and deficit. Missing or expired reports
give an unknown coverage value, never an invented successful copy count.
On reconnect a peer that already has a snapshot may not send a new
receipt immediately, so a receipt-based count may understate coverage.
Neither a remote status hint nor a receipt proves future availability,
an independent physical failure domain, or safe authority transfer.

The daemon polls committed source worlds in rotating batches of at most
four every five seconds, including opted-in seeders without hosting
authority. A correlated cache-pause refusal permits a new exact-manifest
offer after five seconds; ordinary unanswered requests retain the existing
35-second retry interval. Later-generation catch-up requires an exact
reported parent and a signed direct successor from the same accepted
epoch. Cross-generation recovery still needs its certified replay path.
An opted-in hot receiver prioritizes its own bounded pulls; there is no
remote placement command, eviction of canonical history, or forced cache
replication beyond member choices.
If all eligible devices pause, lack quota, or become unavailable, the
target remains unmet rather than bypassing user constraints.
