# Incremental region-block checkpoints — controlled evaluation

**Sabathon #33** | 25 November 2026 | Research only; no new live format or automatic online checkpoint policy.

## Question and decision

The current V1 manifest references one content-addressed blob per file. It already deduplicates identical files, but a small mutation inside a large region file requires encoding and replicating that whole file again. Test whether a fixed-size, sector-aligned block representation reduces the cost of sparse region changes while preserving exact full restoration.

**Decision:** advance the block approach to further isolated design work. Keep the existing signed V1 format and shutdown checkpoint path. Do not enable a new live wire/storage format, automatic save cadence, or mutation of an active Minecraft world based on this experiment. Dense changes, initial capture, retention overhead, and an independently verified immutable source view need further work before a cutover.

## Reproduction

From the repository root, use the normal local-first test workflow:

```sh
flock -x /tmp/swarmcraft-sabathon-cargo.lock bash -c '
  umask 077
  export CARGO_BUILD_JOBS=2 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0
  cargo test -p swarm-storage --test incremental_checkpoint_lab --locked --offline --jobs 2 -- --nocapture
'
```

The experiment is in crates/swarm-storage/tests/incremental_checkpoint_lab.rs; the scheduling sensitivity model is in tests/support/checkpoint_schedule_lab.rs relative to that crate. These files are compiled only into tests, not the shipped daemon or storage library. They add no dependencies. All stores and restore trees are owned temporary directories.

### Fixture and controls

- Eight 4 MiB region-shaped files (32 MiB), with deterministic opaque sector payloads and zero padding; additional level metadata, duplicate and empty files. These are synthetic bytes, **not** valid or launched Minecraft worlds.
- Initial capture, three sparse rounds (one changed 4 KiB sector in every region, small metadata changes, including a file addition, deletion and rename), and one dense full rewrite.
- Real V1 streaming snapshot construction, durable commit, content-addressed Zstd storage, replica finalization and production restore form the baseline. Storage-only fixture signatures do not establish authority.
- The research candidate splits each file into fixed 256 KiB blocks (64 sectors). It reads and hashes every source byte, reuses a block descriptor only by content identity, encodes new blocks through the actual storage crate, verifies every referenced block and the ordered whole-file digest, then syncs and publishes a no-clobber laboratory manifest in a separate namespace.
- Both representations seed their own independent receiver with the first checkpoint, transfer actual missing encoded bytes through the existing resumable blob primitives, and publish the receiver's manifest only after verification. Count actual encoded bytes plus serialized manifest bytes; exclude transport framing, encryption, control messages and network latency.
- Capture timing includes source scan, hashing, encoding, verification and durable manifest publication. Transfer timing includes local replica copy and verification. Restore timing includes local reconstruction. Alternate execution order to reduce systematic cache bias; do not assert a wall-clock speed ratio in CI.
- Exact path sets and contents are compared among the quiescent input, V1 restored tree, and block restored tree for every round. Reopen both stores and restore the original and latest checkpoints after subsequent captures. No research manifest is installed as a canonical V1 snapshot.

## Results

Observed in the second local run, 25 November 2026. Decimal MB are used below; byte counts include the serialized manifest.

| Case | V1 durable capture | Block durable capture | V1 transferred bytes | Block transferred bytes |
|---|---:|---:|---:|---:|
| Initial | 1.095 s | 1.120 s | 25,195,480 | 25,228,156 |
| Sparse round 1 | 1.089 s | 0.224 s | 25,195,453 | 1,582,212 |
| Sparse round 2 | 1.020 s | 0.231 s | 25,195,436 | 1,582,297 |
| Sparse round 3 | 1.159 s | 0.288 s | 25,195,435 | 1,582,070 |
| Dense rewrite | 1.321 s | 1.609 s | 25,195,712 | 25,230,909 |

Median sparse capture: **1.089 s versus 0.231 s** (approximately 79% less time, 4.7x speedup for this capture stage). Median sparse transfer: **25,195,436 versus 1,582,212 bytes** (approximately 93.7% fewer bytes). Independent initial run measured 1.029 s versus 0.256 s median sparse capture with the same deterministic byte counts. The final full-suite run reproduced 1.083 s versus 0.262 s with the same byte counts and reopened both replica stores before cross-checking the original and latest snapshots. Sparse local transfer durations in the second run were 0.822–0.880 s for V1 and 0.170–0.199 s for the candidate. Sparse restore durations were 0.658–0.695 s and 0.638–0.727 s respectively. Initial and dense captures do not show the same advantage, and the block manifest is larger (5,852 bytes versus 948 in the sparse rounds).

This is a **quiescent filesystem capture-stage** comparison, not an actual Minecraft tick-pause measurement. It includes real encoding, durable blob writes and integrity checks, but the laboratory candidate has no concurrent GC, signed consensus or production commit/pin journal; its scratch restore also does not replace a playable tree. V1 performs production preflight restore verification. A production candidate would need all equivalent crash, fencing and retention protections, and may incur additional cost. The input dataset and local filesystem cache affect timings. Do not use the numbers as a universal network, TPS, or save-barrier guarantee.

### Integrity and failure cases

The laboratory format rejects wrong manifest pins, appended/noncanonical manifest bytes, invalid or escaping paths, impossible block lengths or counts, inconsistent descriptors, rearranged blocks with otherwise valid hashes, missing blobs and corrupt same-length encodings. Whole-file hashes bind ordering, lengths and content. The restore function returns only a completed scratch directory; on failure the scratch is discarded. Numbered manifest files are immutable. A failed new capture leaves the prior manifest unchanged. There are no recovery, membership, authority or host-grant records.

These checks prove the isolated reconstruction model, not production atomic activation or resistance to arbitrary remote untrusted manifest parsing. A real format must define bounded streaming decoding, signed state roots, GC ownership, restart recovery, cross-platform durability, and migration from the existing full-file format.

## Snapshot scheduling evaluation (model only)

A candidate scheduler should distinguish an explicit request or dirty threshold from the time of the last **replica-verified** recovery point. A local capture, an unverified receipt, the wrong generation, or a stale attempt ID cannot move that recovery point. Only an independently verified receipt for the exact pending pin and generation advances it to the captured time. New changes and explicit requests arriving during a capture remain pending. One in-flight attempt, retry backoff, an exact accepted generation, an immutable source, conflict-free state and available storage are required. Deadline expiry is a risk indicator, not permission to bypass a gate.

The model compares:

- a fixed 120-second target using representative measured V1 sparse capture + transfer cost;
- a 30-second minimum and 60-second target, with a 128 KiB estimated dirty threshold, using the representative measured block cost.

Assumptions: ten-minute trace, 32 KiB of estimated dirty sector changes every ten seconds, an immutable capture opportunity every 30 seconds, and one-second sampling. Per-attempt costs use representative benchmark measurements rather than real workloads accumulated during that trace. Full estimated transfer cost is charged on each attempt, even if its receipt is lost or it is in flight at the end. This deliberately models a budget, not observed network traffic.

| Modeled scenario | Attempts / verified by end | Estimated total transfer | Maximum sampled dirty age |
|---|---:|---:|---:|
| Fixed V1 | 5 / 4 | ~126 MB | 121 s |
| Incremental adaptive | 10 / 9 | ~15.8 MB | 60 s |
| Incremental, interrupted | 8 / 7 | ~12.7 MB | 150 s |

The interrupted scenario removes immutable capture opportunities for two minutes, denies a storage window and loses one receipt. It records 150 seconds sampled above the target, instead of fabricating an on-time recovery guarantee. Fixed-cost model regressions check the policy independently from the variable benchmark timings.

### What is not enabled

Today Fabric's non-shutdown save barrier saves and replies, then the game can continue mutating files. That reply is not an immutable filesystem view. The shipping supervisor still verifies the save-and-stop/exit path and commits the final checkpoint under its existing generation and head fence.

Before an online schedule is possible, establish a platform-specific, fail-closed frozen-copy or equivalent consistent source protocol; prove the freeze/resume boundary with actual Minecraft; measure pause, end-to-end replica lag, recovery-window violation, CPU, disk, and bandwidth with dense/large/real saves; recheck exact membership and authority at publication; preserve prior verified checkpoints and recovery roots. Never equate more frequent local captures with guaranteed recoverability.

## Next research

Evaluate operation journals separately from block snapshots. A byte-delta journal can test size and replay cost but cannot prove deterministic semantic Minecraft operations, plugin/mod side effects, or cross-region causal order. Evaluate erasure coding separately from either format, including real shard loss, corruption, repair, data expansion, and verification against a full replica baseline. Neither should become a live format without independent durability, authorization and recovery evidence.
