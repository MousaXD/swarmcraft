# Local backup retention and storage admission (V1)

This policy is device-local. It never modifies signed membership, authority,
visibility, checkpoint ancestry or host consent. A quota is not permission to
delete an accepted checkpoint or change the original world's identity.

## Inspect and remediate

```bash
swarmcraft world storage-status <world-id>
swarmcraft world storage-policy <world-id> --max-world-bytes 10737418240
swarmcraft world storage-policy <world-id> --max-world-bytes 10737418240 --reserve-free-bytes 536870912
swarmcraft world storage-policy <world-id> --unlimited
swarmcraft world storage-cleanup <world-id> --confirm
```

These commands use the usual data root (or `--data-dir`). Each policy update
sets the complete local policy; the free-space reserve defaults to 256 MiB and
cannot be set below 64 MiB. `--unlimited` removes the world quota, not the
filesystem reserve. The JSON policy persists across restarts. Invalid or
unreadable policy data denies new payload admission; explicitly setting a valid
policy repairs it without changing any signed metadata.

Status reports the logical bytes beneath that world's store (including partials,
metadata, pins, scratch files and isolated replay), actual filesystem space,
configured reserve, and available headroom for additional payload. A positive
headroom is not a promise that the next chunk or entire checkpoint fits.
World quotas are admission limits on new encoded payload, **not** hard OS disk
quotas: control records and manifests may use additional bytes so emergency
control operations are not deliberately blocked. Other worlds, the Minecraft
working tree, installed runtimes, external backups, filesystem allocation
overhead and unrelated applications are not covered by a world quota. Free-space
checks are best effort against those concurrent writers. No simulated reserve
test establishes safety against every real ENOSPC timing.

## What happens under pressure

- Before writing a new encoded blob or appending a replica chunk, the local
  writer checks the remaining world quota and actual filesystem reserve.
  Writers for one world serialize admission with an OS-owned lock. Network
  handlers refuse rather than wait behind a long local compression job.
- Local compression checks actual output writes, not just an estimate of
  uncompressed size. An unsuccessful publication leaves its previous signed
  checkpoint untouched and removes its own temporary compression file on normal
  error unwind. A hard crash may still leave scratch files.
- A receiver that rejects a chunk before writing returns the authenticated
  `STORAGE_PAUSED` error without exposing its filesystem or free-space details
  to the remote peer. The sender backs off and retries the same offset while the
  negotiation is valid. Interrupted I/O can require normal renegotiation.
  No complete replica receipt is invented. Multi-source reconstruction does
  not blame remote sources for local admission failure.
- Isolated stale-replay downloads count against the outer world's quota.
  Publication may need additional space to copy the verified isolated payload
  while retaining the original. Insufficient space refuses the new publication
  intent; a publication interrupted after its intent remains fenced until it
  can be safely completed.
- A new managed runtime is denied before epoch mutation or resetting its working
  directory when storage admission or restore preflight fails. Restore checks
  the destination filesystem before altering existing files. A running JVM is
  not an OS-quota-managed process. If final checkpointing fails, the existing
  unconfirmed-runtime fence and retained files remain; do not delete them or
  forcibly clear the marker to get a green status.

If a transfer pauses, free unrelated space, run confirmed cleanup if safe, or
increase the local quota. Replication can resume from durable partial offsets.
If the last save of a stopped host is unconfirmed, preserve the runtime and
follow its explicit recovery/archival workflow rather than assuming the final
checkpoint was published. Space recovery is not a new authority grant.

## Cleanup and retention

V1 historical admission and stale-replica replay require the complete signed
checkpoint chain. The supported retention policy therefore preserves **all**
committed checkpoints, not just the newest N. Retention of a real world cannot
prune older signed manifests even if the lower-level retention API is given a
smaller count. Missing historical links, inconsistent recovery roots, incomplete
publication, and pinned historical replay block destructive cleanup.

Confirmed cleanup first verifies the newest signed recoverable checkpoint and
then performs mark-before-sweep with the existing blob GC lock. All committed
manifests, mandatory transfer/sleep/recovery/conflict roots, active replication
pins and local publication pins protect their referenced complete blobs. Only
unreferenced complete blobs are candidates. Unknown files, resumable partials,
runtime working trees, isolated replay data and unproven temporary artifacts
are not deleted. Durable legacy replication pins left after a hard crash are
conservatively retained; reclaiming them requires a separate ownership proof.

Use the existing verified snapshot export for a deliberate external backup.
Do not delete numbered manifests, epoch bases, signed transition evidence or
pending runtime files to meet a quota. Bounded signed history compaction,
transaction-owned crash cleanup for all replication pins, broader persistent
filesystem exhaustion/power-loss campaigns, and Desktop management/diagnostics
remain follow-up work. The [bounded Linux stress campaign](../research/stress-campaign/REPORT.md)
now covers actual tmpfs ENOSPC, process file-size limits, killed compression,
retention/retry and exact signed restores across three independent profiles. This
document does not claim support for arbitrary historical pruning or a complete
cross-platform disk-full certification.
