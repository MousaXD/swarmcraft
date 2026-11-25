# Snapshot replication, retention, and garbage collection

SwarmCraft snapshots remain content-addressed and manifest-driven. This document describes the storage-side replication scheduler and the conservative retention/GC rules layered on top of the existing resumable blob protocol.

For device-local cache selection, hot-world scheduling, ordinary payload
limits, UTC windows and replica coverage, see [Replica cache policy](REPLICA_CACHE_POLICY.md).

## Replication invariants

The scheduler does not weaken existing verification.

- A snapshot manifest remains the authoritative list of blob descriptors.
- Blob identity is the BLAKE3 hash of the uncompressed content using the existing SwarmCraft blob domain.
- Encoded size, decoded size, and content hash are checked before a partial blob is promoted into the complete blob namespace.
- Resume offsets are read from the destination's durable `.part` file. They are bound to content identity, not to a particular peer, so another replica can continue the same transfer.
- A corrupt source cannot publish a complete blob. Final verification rejects it and the existing receiver removes the poisoned partial before another source retries.
- Only one scheduler assignment owns a blob hash at a time. Parallelism is across different missing blobs, never competing writers for the same `.part` file.
- Snapshot finalization happens only after every referenced blob verifies.

`ReplicationOptions` bounds parallel blob work and per-read chunk size. Defaults are four concurrent blobs and 256 KiB chunks. The hard storage-side limits are 32 concurrent blobs and 4 MiB chunks. Network integrations should normally keep the existing 256 KiB wire limit.

## Multi-source selection

`ReplicaInventory` records which peers advertise complete blobs. `BlobSourceSelector` deterministically assigns each missing blob to the least-loaded source by already-assigned encoded bytes, with peer ID as a stable tie-breaker. Other advertising peers remain ordered fallbacks.

The scheduler therefore spreads a manifest across replicas instead of selecting one snapshot source:

```text
manifest
  blob A  <- peer 1
  blob B  <- peer 2
  blob C  <- peer 3
  blob D  <- peer 1
```

If a source disappears after writing part of a blob, the next source starts at the destination's durable partial offset. If final verification then fails, the scheduler treats the inherited prefix as suspect: after the receiver resets the poisoned partial, the current fallback gets one clean retry from offset zero before it is blamed. If a source delivers a corrupt blob through completion, verification rejects it and the next source starts that blob from offset zero. Failure is scoped to the blob/source attempt rather than poisoning the whole reconstruction when a healthy fallback exists.

`BlobSource` is intentionally transport-agnostic. `LocalReplicaSource` is the in-process implementation used by storage tests and tooling. The live libp2p daemon uses a receiver-driven async coordinator for ordinary accepted-epoch replication, with the same per-blob ownership, durable offsets, and verified storage primitives.

## Replication observability

Every reconstruction returns a `ReplicationReport` containing:

- total and completed blobs;
- bytes received;
- resumed blob count;
- source failure count;
- corrupt-source rejection count;
- resumed-prefix verification retry count;
- maximum observed parallel blob work;
- the set of sources used;
- per-source attempted/completed blobs, bytes, failures, and corruption rejections.

The scheduler also emits `tracing` events for reconstruction start/completion and source fallback events.

## Retention policy

`RetentionPolicy` is conservative. A snapshot is retained when any of the following is true:

1. it is the latest committed snapshot;
2. it is among the configured `keep_latest` newest snapshots;
3. its snapshot number is explicitly protected by the caller;
4. its manifest hash is referenced by an authority-transfer base;
5. its manifest hash is referenced by a sleep recovery point;
6. its manifest hash is referenced by a durable recovery promise;
7. its manifest hash is referenced by a recovery certificate;
8. its manifest hash is the base or head of the current solo branch or a preserved solo conflict.

The default policy keeps the three newest snapshots, in addition to all mandatory roots above. `keep_latest = 0` still keeps the latest snapshot.

If a recovery/control file exists but cannot be decoded, pruning aborts instead of assuming the root is absent. If a control record references a snapshot that is not present locally, pruning also aborts. Ambiguity therefore retains data rather than risking recovery state.

## Garbage collection

Retention uses two explicit phases.

1. `prune_snapshots` removes only unretained snapshot manifests. It never deletes blobs.
2. `garbage_collect_blobs` acquires an exclusive world-level GC lock, re-reads all currently committed manifests, marks every referenced blob plus active replication pins, then sweeps only recognizable complete blob files that are unmarked.

`apply_retention` runs those phases in that order.

GC never removes:

- a blob referenced by any currently committed snapshot;
- a blob pinned by active reconstruction;
- `.part` resumable-transfer files;
- snapshot/restore temporary files;
- malformed or unknown files.

This is intentionally less aggressive than a filename-wide cleanup.

### Active replication vs. GC

A reconstruction pins all missing blob hashes before transfer starts. Pin creation checks the GC lock both before and after writing durable pin files. GC creates its lock atomically before it reads pins.

That closes the race:

- if replication pins first, GC sees the pins and preserves those hashes;
- if GC locks first, replication aborts before writing complete blobs and can retry later.

The scheduler holds the pin lease until the snapshot is finalized or the reconstruction returns.

A crash can leave a stale pin or GC lock. SwarmCraft treats that as a space leak / blocked cleanup, not as permission to delete uncertain data. Automatic stale-lock deletion is deliberately not implemented because proving that no writer is still active is an operational concern, not a filesystem timestamp decision.

## Interrupted cleanup safety

Pruning manifests is monotonic toward a smaller retained set but does not touch blob bytes. If the process stops after pruning and before sweeping, the only effect is extra orphaned blobs. On the next GC, live roots are recomputed from the manifests that actually remain on disk.

During the sweep, each complete orphan blob is independently removed. An interruption can leave additional orphans, but cannot invalidate a retained snapshot because all retained references were marked before deletion.

## Test coverage

Permanent storage tests cover:

- one new peer reconstructing one snapshot from multiple replicas;
- bounded concurrent downloads of different blobs with exact restore;
- a source disappearing mid-blob and cross-replica resume;
- corrupt-source rejection followed by healthy fallback;
- a corrupt partial from a disappearing source being discarded before a clean retry of the healthy fallback;
- an already-partial transfer resuming from another replica;
- corrupt local replicas being excluded from inventory;
- referenced blobs surviving GC;
- orphan blobs being reclaimed;
- active replication pins preventing premature reclamation;
- interruption between manifest pruning and blob sweep preserving exact recovery;
- the latest snapshot remaining retained even with `keep_latest = 0`.

Existing snapshot-swarm, resume, impaired-network, and QUIC soak tests remain unchanged.

## Live daemon: bounded receiver-driven ordinary replication

The daemon advertises `snapshot-multi-source-pull-v1` in its signed, connection-bound PeerHello. When an ordinary direct-extension manifest is accepted and at least two distinct current members supporting the capability are connected, the receiver may return `ManifestPullAccepted`. This is negotiation only, not a replica acknowledgement. It pins the complete manifest, exact signed membership hash, and exact signed authority epoch hash. Sources must hold the referenced committed snapshot in that epoch and may return only ranges of its referenced blobs. Non-authority sources must have enabled background seeding. All requests use current application authentication and committed membership; hints and capability advertisements are not authorization.

The coordinator in `daemon_parallel_replication.rs` schedules at most four outstanding reads across at most four active worlds, with one request per logical source and one writer per `(world, blob hash)`. The wire chunk limit remains 256 KiB. Active game traffic reduces receiver concurrency to one and applies the existing pacing policy. The serving side also uses the existing outgoing replication pacer. Only a correctly correlated response on the authenticated connection is accepted, with exact target, hash, offset, bounded bytes, and final-range consistency checks. The storage layer verifies the complete blob before promotion.

Failures back off the specific blob/source; a new source resumes at the durable offset, with one clean retry when an inherited prefix fails final verification. Completed unrelated blobs remain pinned and reusable. Storage pressure pauses writes rather than blaming a source. A stalled ordinary pull permits subsequent legacy push negotiation without deleting data. Legacy peers and initial/historical replay remain on their existing independent paths; a pull cannot install epoch or membership records, supersede certified history, or coexist with legacy push writers for the same pending world.

Before finalization, the receiver rechecks the exact scope, manifest signature, and direct extension. Under the storage transaction lock, publication also checks the exact epoch, fencing token, membership hash, current descriptor, active recipient, canonical predecessor, and absence of first-contact or solo-conflict fences. A racing existing slot requires renegotiation instead of returning unfenced success. Replica acknowledgement is sent only after verified publication and another current-membership check.

### Local process evidence

The controlled three-daemon loopback fixture in `crates/swarm-cli/tests/parallel_replication.rs` commits a genuine joint membership, transfers independent blobs over authenticated QUIC, and verifies byte-exact restoration. In the initial run on 2026-11-25, a roughly 6 MiB payload restored in 4.74 s with two contributing sources versus 8.33 s with one contributing source after optional seeding was disabled. The two-source run observed two simultaneous reads. A separate roughly 12 MiB scenario corrupts one source, stops it after progress, verifies fallback and clean retry, and checks that an unrelated verified blob retains its inode and contents. These local timings are a controlled comparison, not a cross-network performance guarantee. The test uses genuine daemon processes and signed control records, but does not launch Minecraft or exercise installed cross-platform builds.

The storage scheduler and `ReplicationReport` remain available for in-process tools. The live coordinator reports contributing sources, bytes, resumed blobs, failures, and maximum parallel reads in daemon logs. Remote inventory advertisement and multi-source certified historical replay are intentionally not enabled by this change.
