# Independent byte-journal and erasure-coding experiments

**Sabathon #34** | 25 November 2026 | Research only; no production wire or storage format.

## Decision

The two experiments address different costs and must not be conflated:

- An ordered byte-operation journal can greatly reduce changed-payload bytes for sparse known writes, but adds replay dependencies and restore cost.
- Erasure coding can reduce retained redundant payload bytes while tolerating two independent shard-domain failures, but adds encoding, reconstruction, repair and placement costs.

Both results justify further isolated work. Neither justifies activation in the live signed checkpoint path. In particular, the journal experiment is not a deterministic semantic Minecraft operation log, and the parity experiment is not a replacement for authority quorum.

## Reproduction

From the repository root:

```sh
flock -x /tmp/swarmcraft-sabathon-cargo.lock bash -c '
  umask 077
  export CARGO_BUILD_JOBS=2 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0
  cargo test -p swarm-storage --test operation_journal_lab \
    --test erasure_coding_lab --locked --offline --jobs 2 -- --nocapture
'
```

Both tests use actual V1 streaming capture, verified blobs and restore as their independent baseline. All data is synthetic, temporary, locally owned and isolated; the V1 storage-only fixture signatures do not establish authority. No production call site, dependency, runtime, network protocol or signed world format was changed.

## A. Ordered byte-operation journal

### Method

Create four 2 MiB region-shaped files and several small metadata files. Start both alternatives from the same verified full V1 checkpoint. Three independent sparse rounds perform one 4 KiB replacement in each region and a metadata replacement. The first round also creates, removes and renames files. Actual source file mutations are independent of the replay implementation.

The test records explicit known byte operations, preimage hashes, before/after tree roots, exact full-base manifest hash, sequence and predecessor batch hash. Each canonical postcard batch is compressed with Zstd level 3, bounded, hashed in a separate domain and synced into an immutable no-clobber laboratory record. The replica accepts only the pinned record bytes. Restoring a requested point starts from the actual verified full V1 base, checks every sequence, predecessor and operation preimage, recomputes the tree root after each batch and writes only a completed verified scratch tree. The candidate uses a bounded in-memory tree (64 MiB) and at most 64 batches, with no arbitrary executable operations or external path access.

### Measurements

Final local run, payload and manifest/record bytes in decimal units. The V1 delta includes actual changed encoded blobs and the serialized full manifest; the journal delta includes the actual compressed serialized record with its pin. No network framing, transport latency, filesystem allocation overhead or production consensus journal overhead is included.

| Sparse round | V1 transferred bytes | Journal record bytes | V1 capture | Journal preparation + append | V1 restore | Journal base + replay + restore |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 6,299,911 | 12,828 | 160.2 ms | 15.9 ms | 54.5 ms | 111.1 ms |
| 2 | 6,299,876 | 12,742 | 158.0 ms | 14.9 ms | 58.5 ms | 121.3 ms |
| 3 | 6,299,876 | 12,742 | 163.6 ms | 15.0 ms | 57.3 ms | 124.3 ms |

The full base was 6,299,903 bytes. Retaining that base and all three rounds' actual encoded payload/manifests cost 25,199,566 bytes with V1; retaining the base and the three journal records cost 6,338,215 bytes (approximately 74.8% less). Individual sparse journal records were approximately 99.8% smaller. Replayed operation counts increased from 8 to 13 to 18, and cumulative journal bytes read from 12,828 to 25,570 to 38,312, in addition to the full base. Journal restoration was slower than the ready full snapshot on this dataset.

Preparation includes reading and hashing the post-operation source tree and creating the batch; append includes encoding, validation and durable record write. These measurements do **not** include capturing arbitrary Minecraft operations, freezing a runtime, coordinating a distributed writer, or achieving write-ahead durability before each game mutation. The synthetic fixture supplies known operations, so the measured append/capture cost is not a live save latency guarantee. Dense arbitrary changes and long journal chains require separate measurement; a 512 KiB laboratory record limit deliberately refuses oversized batches instead of silently dropping operations.

### Fault tolerance

The test rejects wrong base pins, omitted final records, out-of-order records, a missing middle batch, truncated and corrupted records, overwrite of an immutable sequence, wrong operation preimages and escaping paths. A verified prefix and the original base still restore when a middle record is missing; the later suffix does not. After the exact record is restored, reopening the journal reproduces the final state byte-for-byte against the independently restored V1 checkpoint. The journal never changes the base's canonical head.

A missing record can prevent every dependent later restore even when those later records survive. Periodic full checkpoint compaction, explicit contiguous durable receipt tracking, bounded replay, repair redundancy and deliberate fallback to the last actually recoverable prefix are required. A real semantic journal needs deterministic operation definitions across Minecraft versions and mods, ordering and idempotency, side-effect and RNG semantics, real crash/write-ahead evidence, signed generation fencing, and a consistent base. This experiment establishes byte-operation tradeoffs only.

## B. Erasure coding of existing encoded V1 blobs

### Method

An independent four-region fixture is captured with the actual V1 storage pipeline. Each distinct existing encoded blob is striped into four equally sized data shards, zero-padded at the end, plus two parity shards. P is XOR and Q uses distinct coefficients in GF(256). This is a small research-only implementation of a systematic 4+2 code. A pinned archive descriptor contains the exact V1 manifest, exact blob descriptors, shard lengths and per-shard hashes. Archive metadata is replicated on all six simulated failure domains.

A receiver checks the exact archive pin, rejects invalid shard lengths or hashes as erasures, solves at most two erasures, reconstructs all parity and padding, verifies all six expected shard hashes and then feeds the reconstructed **original encoded bytes** into the production V1 blob receiver. A valid parity equation alone never establishes original content integrity. The restored V1 snapshot and separate full-replica restore must match the original file set and every byte.

### Measurements

Final local run, including actual shard files and six archive metadata copies:

| Retained representation | Bytes |
|---|---:|
| One full V1 copy | 6,300,001 |
| Two full copies | 12,600,002 |
| Three full copies | 18,900,003 |
| 4+2 coded archive across six domains | 9,460,962 |

The archive occupied approximately 1.502 times one copy, 24.9% less than two full copies and 49.9% less than three. Three full copies can tolerate two complete-copy losses when placed independently; the code requires six independently available failure domains and at least four valid shards per stripe. These are not equivalent placement counts.

With one data domain removed, another's same-length shard contents corrupted, and one surviving metadata copy corrupted, reconstruction read 30 shard files and 7,878,237 bytes including metadata and damaged bytes. Six stripes yielded 6,299,435 verified original encoded bytes. The repair created 12 replacement shards (3,149,722 bytes) plus two exact metadata copies (3,932 bytes), in a separate repair tree. The source's surviving data was not overwritten.

| Local operation | Duration |
|---|---:|
| Copy one full V1 checkpoint | 110.3 ms |
| Restore already complete V1 replica | 51.8 ms |
| Encode and persist six-domain archive | 766.2 ms |
| Rebuild V1 payload and persist repairs | 1,256.7 ms |
| Restore reconstructed V1 checkpoint | 52.2 ms |

A direct restore from a complete copy is far cheaper than rebuilding a missing copy from parity in this experiment. These timings are local debug-build observations, not network throughput, optimized-library benchmarks or guaranteed repair times. Reconstruction checks all available shard files (including a damaged one), rather than claiming the optimal four-shard read cost. The six domains are temporary directories on one machine; losing that machine loses all six. Physical placement, network cost and churn have not been measured.

### Fault tolerance

All 255 nonzero field inverses are checked. The test exercises all 22 zero/one/two-erasure patterns and all 20 triple-loss patterns at seven lengths, including short and padded final shards (154 recoverable patterns, 140 excessive-loss refusals), as well as mixed corruption and missing shards. The larger actual-V1 fixture verifies metadata fallback, two failed domains, byte-exact recovery, repaired shard hashes and metadata, and refusal when a third domain disappears. It also shows that a self-consistent coded stripe built from corrupted original bytes still fails the real V1 content-hash receiver. A failed reconstruction does not publish a canonical head.

Before production, replace or independently audit the research codec, establish signed archive/placement metadata and repair receipts, bound streaming memory/CPU and hostile inputs, coordinate durable publication and retention pins across actual failure domains, test interrupted writes/reconstruction and correlated failures, quantify WAN repair and costs, and integrate local quotas, bandwidth and placement policy. Erasure coding provides no confidentiality by itself, no membership, no voting power, and no permission to play or host.

## Outcome

Both prototypes demonstrate specific controlled size improvements and expose their restore/fault costs. They are intentionally independent of each other and of the #33 block-checkpoint representation. Keep V1 canonical and verified until production designs and real installed/field tests independently satisfy all safety gates.
