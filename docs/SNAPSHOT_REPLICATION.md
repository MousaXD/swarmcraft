# Snapshot replication, retention, and garbage collection

SwarmCraft snapshots remain content-addressed and manifest-driven. This document describes the storage-side replication scheduler and the conservative retention/GC rules layered on top of the existing resumable blob protocol.

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

If a source disappears after writing part of a blob, the next source starts at the destination's durable partial offset. If a source delivers corrupt data, verification rejects the completed partial and the next source starts that blob from offset zero. Failure is scoped to the blob/source attempt rather than poisoning the whole reconstruction when a healthy fallback exists.

`BlobSource` is intentionally transport-agnostic. `LocalReplicaSource` is the in-process implementation used by storage tests and tooling. The live libp2p daemon can populate `ReplicaInventory` from authenticated replica knowledge and reuse `BlobSourceSelector` without redesigning the wire protocol.

## Replication observability

Every reconstruction returns a `ReplicationReport` containing:

- total and completed blobs;
- bytes received;
- resumed blob count;
- source failure count;
- corrupt-source rejection count;
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
- an already-partial transfer resuming from another replica;
- corrupt local replicas being excluded from inventory;
- referenced blobs surviving GC;
- orphan blobs being reclaimed;
- active replication pins preventing premature reclamation;
- interruption between manifest pruning and blob sweep preserving exact recovery;
- the latest snapshot remaining retained even with `keep_latest = 0`.

Existing snapshot-swarm, resume, impaired-network, and QUIC soak tests remain unchanged.

## Live daemon integration boundary

The storage layer now exposes the focused pieces needed for safe source-aware scheduling: `ReplicaInventory`, `BlobSourceSelector`, `ReplicationScheduler`, and `ReplicationReport`.

The current daemon still negotiates `SnapshotManifest` / `BlobChunk` directly. Its existing wire messages do not identify a centralized pull coordinator, so fully adopting concurrent multi-peer scheduling in the live daemon requires a small replication-runtime state machine that owns each `(world, snapshot, blob)` transfer and routes authenticated peer responses into that owner. That integration should reuse these abstractions and must not reintroduce overlapping writers to one `.part` file.
