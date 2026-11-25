# Replication pacing during gameplay

The daemon detects actual admitted local and remote game forwarding sessions
through process-local RAII activity counters. A denied, revoked, completed,
or aborted stream releases its guard. This observation is **not** route,
authority, membership, consent, or replication evidence.

While gameplay forwarding is active, the daemon shares one outbound encoded
blob pacing budget across all peers and worlds:

- Default ceiling: 64 KiB/s; configure with
  `SWARMCRAFT_REPLICATION_PLAY_KIBPS` (integer 4-128; invalid input fails startup).
- At most 16 KiB per paced chunk, reduced further with the current rate.
- At least 125 ms between sends, no accumulated idle burst credit.
- An authenticated, matching blob ACK slower than 500 ms halves the current
  rate to a floor of 4 KiB/s; eight consecutive ACKs at or below 150 ms
  increase it by 4 KiB/s up to the configured ceiling.
- The 50 ms retry scheduler selects least-recently-served eligible peer/world
  pairs first; each pair retains a single in-flight chunk, exact resume offset,
  existing membership and authority checks, and existing rate-limit backoff.
- When gameplay stops, normal 256 KiB chunks are restored. Existing in-flight
  idle chunks cannot be retracted when gameplay begins. Lease, quorum, control
  and manifest messages are not paced by this blob budget.

This is an encoded outbound blob-byte scheduler, not a hard total wire-byte
limit or network QoS guarantee. ACK delay also includes receiver storage and
scheduling costs and is not a Minecraft latency measurement. Remote peers
without an active forwarding session on their own daemon are not throttled
by another daemon's local state. Real congestion and per-world fairness
require representative field testing.

## Automated acceptance target

The synthetic game-echo gate exercises authenticated QUIC, the production
pacer, real encoded blob storage and verified replica content. It requires
at least 12 overlapping 64-byte game echo RTT samples, p95 <=400 ms, and
completion of a 128 KiB source blob within 30 seconds at a 32 KiB/s ceiling.
CI additionally applies 1 Mbit/s and 12 ms loopback impairment on its isolated
runner. It does not change a developer's network configuration.

Local unshaped result: 131,084 encoded bytes in 4.17 seconds, 41 echo samples,
p95 1.03 ms. This gate uses a test driver, not the complete daemon replication
path; separate daemon and authenticated forwarding lifecycle tests cover
integration and fail-closed behavior. Neither result certifies Minecraft
playability over a constrained real uplink. Track the exact-SHA impaired CI
result separately from the local result.
