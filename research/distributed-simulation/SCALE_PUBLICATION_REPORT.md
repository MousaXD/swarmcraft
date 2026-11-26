# #63 — Exact receipt encoding and independent replay optimization

26 November 2026. Isolated durable reference transaction runtime only.

## Safety and exact encoding

The authority previously canonicalized each receipt body for its HMAC,
then serialized the complete original receipt array a second time for
the durable receipt hash. The optimized path canonicalizes the body
once, computes the same domain-separated HMAC over those bytes and
reuses the exact bytes in the canonical original receipt envelope.
Outer keys and fields are preserved in canonical order, including
ownerless policy binding and MAC. A different body object supplied by
an adapter is not substituted into the original envelope. The receipt
array digest is incrementally computed in original canonical session
order. No worker-supplied precomputed encoding is trusted.

The state commitment uses the identical big-endian binary layout and
SHA-256 as the scalar region summary, packing validated agent triples
in bounded 512-agent blocks. The legacy scalar commitment remains
available. The optimized path still validates all untrusted region
data, grant coverage, global IDs, conserved resources and candidate
root inside the current durable transaction. No root or receipt
validation is skipped.

The optimized transaction replay derives inputs from the same
durably sealed batch and computes the entire next world directly
from the accepted state. It does not consume worker proposals,
events, candidate state, or candidate summaries. It specializes
the independent monolithic algorithm for checked direction/action
pairs, avoids eagerly evaluating background movement for controlled
agents, computes the background phase once and selects resource
debits by the same congruence class. All debits are selected before
credits, and global ID order and same-cell contention are preserved.
The full replayed state must equal the validated candidate, and
its independently recomputed root must equal the proposed root.
Cold audit and offline evidence verification retain the scalar
monolithic implementation and scalar root encoding.

Current-generation and contribution checks, exact sealed input
validation, client receipt creation, two durable write transactions,
atomic all-region publication, FULL SQLite durability and failure
behavior are unchanged. There are no schema or production protocol
changes; this is a local reference implementation.

## Verified paired publication cost

[Raw evidence](evidence/2026-11-26-scale-publication-cost.json),
SHA-256 `3a54c72f2acfaeb845a34cab423f8e31f1a35b2b956e31668cc34743e41aba77`.

The existing file-backed SQLite cost lab now has a separate schema-2
publication experiment. It holds candidate reuse and boundary-only
transaction observation constant and alternates legacy versus
optimized publication order across two repetitions. Each workload
and mode has a fresh database, two warmup ticks and ten measured
commit/receipt operations. Across both workloads and modes, eight
fresh databases, 16 warmups and 80 measured operations completed.
Independent scalar replay, exact roots, committed client receipts,
reopened journal/final-state hashes and transaction counts verify.
Synthetic signed receipts are generated before the timed interval;
this does not measure real network workers or full paced service.

| Agents | Publication | Median wall (ms) | Median CPU (ms) | Mean receipt validation (ms) | Mean independent replay (ms) |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 | legacy | 6.771 | 3.161 | 1.173 | 0.545 |
| 256 | optimized | 6.535 | 2.940 | 0.945 | 0.453 |
| 8,192 | legacy | 31.168 | 27.348 | 15.944 | 7.791 |
| 8,192 | optimized | 28.239 | 24.467 | 14.792 | 6.877 |

At 8,192 agents, median measured CPU and wall time decreased by
approximately 10.5% and 9.4% respectively. The paired experiment
isolates the combined publication optimization, not the individual
effect of each change or actual network service throughput.

## Verified full paced matrix

[Raw schema-6 evidence](evidence/2026-11-26-scale-publication.json),
SHA-256 `de8d6c804b5639449291dfcb970513aa441e7c124260c64f11ba41447c610946`.
The wrapper exited 0 and all worker scopes were released. Independent
scalar replay verified 24 runs, 3,052 durable ticks, 6,104 committed
client receipts, 36 worker process instances and 48 client process
instances. Of 3,840 scheduled slots, 3,036 completed within their
windows and 1,267 met their original deadlines; 16 ticks completed
after their observation window and 1,760 operations exceeded 50 ms
service time. Five runs meet the strict 20 Hz contract: all four
256-agent local runs and the first one-worker 2,048-agent run.
All eight of those local workloads complete every scheduled tick
within the eight-second window, but the other three 2,048-agent
runs miss 1, 10 and 2 individual deadlines.

The 4,561 worker-tick instances reconcile with 13,683 authenticated
frames per direction, 27,366 total. All ticks retain two read and
two write transactions: 12,208 successful commits and zero rollbacks.

Mean fixed-window TPS across two repetitions:

| Workload | Prior fused, 1 worker | Optimized, 1 worker | Prior fused, 2 workers | Optimized, 2 workers |
| --- | ---: | ---: | ---: | ---: |
| Spread 256, local | 19.6875 | 20.0000 | 17.9375 | 20.0000 |
| Spread 2,048, local | 15.6875 | 20.0000 | 13.1875 | 20.0000 |
| Spread 8,192, local | 8.3125 | 15.8125 | 10.5625 | 15.6875 |
| Clustered 8,192, local | 8.7500 | 15.6875 | 6.1250 | 14.9375 |
| Spread 256, 5 ms one-way | 12.3125 | 18.8125 | 11.9375 | 17.6875 |
| Spread 256, 25 ms one-way | 5.3750 | 5.6250 | 5.2500 | 5.5000 |

Heavy and impaired-link cases remain unqualified. Two workers do
not increase sustainable capacity in this matrix. The preceding
schema-5 fused run occurred under elevated host I/O and CPU load.
Its one-worker low-load mean SQLite durable commit cost was 27.714
ms versus 7.475 ms in this run; consequently the historical
throughput differences are not attributable solely to publication
optimization. Compared with the earlier schema-4 local
candidate-reuse result, heavy spread mean throughput is 15.8125
versus 14.7500 TPS with one worker and 15.6875 versus 14.1250 with
two workers; these comparisons are also historical.

Selected tick-weighted mean phase times (ms):

| Phase | 256 local, 1 worker | 8,192 spread, 2 workers | 256 at 25 ms, 1 worker |
| --- | ---: | ---: | ---: |
| Full service | 22.495 | 63.431 | 175.600 |
| Publication receipt validation | 1.036 | 11.598 | 1.083 |
| Independent replay | 0.613 | 9.991 | 0.662 |
| SQLite durable commit | 7.475 | 8.029 | 9.296 |
| Resource guards | 7.210 | 8.769 | 7.011 |
| Worker tick and receipts | 2.571 | 11.693 | 153.875 |

The remaining heavy path includes receipt parsing/authentication,
independent replay, worker transfer and resource observation. The
three sequential worker barriers still impose a WAN latency floor.
Stable-host interleaved full-network controls and sustained soak
remain open. No real Minecraft or independent physical worker
qualification is established, and production regional and ownerless
gameplay remain disabled.

## Regression boundary

The scalar and block commitments agree for empty regions, partial
and exact block sizes, maximum legal agent count and bounded
maximum inventory and bank values. Independent specialized replay
matches the scalar oracle across 72 multi-tick spread, clustered
and boundary fixture transitions, varied controlled actions, reverse
region iteration order, same-cell contention, saturation and ring
wrap. Malformed replay controls are refused. Both authority modes
publish identical durable receipt hashes, checkpoint bytes and
history for the same authenticated candidate and sealed inputs.
The reopened database passes scalar audit. Real policy-bound
receipts preserve the original full outer-envelope hash.

All 132 research regression tests passed, including retained schema-6
and paired-publication evidence replay. Workflow-policy, release-version
and diff checks passed locally.

## Reproduction

Run these experiments sequentially, never competing for the host:

```sh
mode=optimized # or legacy for a full-network control
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-scale-63-publication-$mode-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=360 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/scale_lab.py \
  --state-cache cached --input-mode batched --candidate-mode reuse \
  --transaction-counts boundary --worker-exchange fused \
  --publication-mode "$mode" --output "/tmp/sc63-publication-$mode-new.json"

cd research/distributed-simulation
python3 verify_scale_evidence.py "/tmp/sc63-publication-$mode-new.json"
python3 scale_candidate_lab.py --experiment publication --output /tmp/sc63-publication-cost-new.json
python3 scale_candidate_lab.py --verify /tmp/sc63-publication-cost-new.json
python3 -m unittest discover -s . -p 'test_*.py' -v
```
