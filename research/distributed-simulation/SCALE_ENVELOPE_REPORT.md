# #63 — Paced durable reference-runtime scale envelope

Date: 25 November 2026 UTC / 26 November UAE time.

Follow-up: [Exact-state cache and phase profile](SCALE_PROFILE_REPORT.md)
repeats the full matrix with the optimized reference authority;
the original retained measurements below remain unchanged.

This is a bounded, repeated performance experiment of the custom
reference transaction runtime. It is not a Minecraft gameplay
benchmark, independent-host certification or authorization to enable
production regional hosting.

## Measurement contract

The benchmark exercises the entire existing durable transaction
path, rather than just the in-memory worker compute/barrier used in
earlier experiments. Two persistent, authenticated TCP reference
clients send one movement/inventory command each per attempted tick
and acknowledge the exact committed receipt. Background synthetic
agents use the existing deterministic reference rules. The
coordinator queues commands durably, seals the batch, waits for
current-generation region receipts, checks the complete candidate
against the monolithic reference, commits to the trusted local
SQLite authority, and returns the results. No safety or synchronous
durability checks are removed for the measurements.

All cases use eight active regions, 1,024 reference cells, the same
initial fixture seed and scripted player commands. The one-worker
and two-worker cases use the same starting state and rules; the
latter splits ownership at region 4. Separate temporary authorities
with distinct world identities are never competing writers.
Runtime state roots are directly comparable at matching ticks.
Fixed layouts are chosen explicitly for comparison, rather than
letting the admission optimizer decide which layout to measure.
They do not change the actual contribution/admission policy.

Each worker is pinned to one physical core and uses a real
delegated cgroup-v2 128 MiB memory cap and 1,000 millicore CPU
budget. There is an additional coordinator core; local clients
inherit that core. Two workers therefore have twice the aggregate
worker resource budget, not an equal-total-resource comparison.
Actual kernel caps and shared headroom are checked before work
and before each publication. Worker and client processes use a
private network namespace arrangement; worker connections are
authenticated. Worker-to-coordinator delay is implemented by real
netem in both directions, with 0, 5 or 25 ms one-way delay.
Client links remain local loopback. These are controlled latency
profiles, not independent physical hosts, internet congestion or
packet loss measurements.

A run starts timing after worker initialization and client join.
For eight seconds, its target is one tick every 50 ms (160 slots,
20 Hz). Work starts no earlier than its scheduled slot and never
overlaps another tick. If a tick overruns, backlog is carried
forward rather than skipped or reset. No new tick starts after
the eight-second boundary; an in-flight tick can finish afterward
and is retained for correctness replay, but is not counted as a
completion within the window. Actual completion requires both
clients' acknowledgments. Service latency starts when work starts;
scheduled latency starts at the original slot, including backlog.
A run qualifies only if all 160 slots complete within their own
50 ms deadlines and the observation window. Reported throughput
uses completions within the fixed window, not the reciprocal of
service p95 or only successfully timed slots.

The matrix compares spread loads of 256, 2,048 and 8,192 total
agents, clustered load of 8,192 agents (85% initially in region 0,
apart from two explicitly positioned players), and the 256-agent
fixture under both degraded delay profiles. There are two repeats
per worker count and scenario, with order reversed for the second
repeat. All but the two scripted players are synthetic entities,
not simulated full Minecraft clients or a player-count capacity
claim. These short observation windows cannot establish long-term
soak stability; failure in these windows is enough to refuse
qualification at this configured target.

Offline verification replays each accepted cut and full saved
world using the independent serial rules, and reconstructs every
client command, view and receipt. It also checks journal records,
tick schedule, CPU/memory observations, worker grants, real
network counters and impairment counters, and scope cleanup.
It cannot cryptographically attest to historical process execution
or the authenticity of recorded clocks and resource measurements.

## Retained measurements

[Complete raw receipt](evidence/2026-11-26-scale.json).

All 24 runs completed. Independent replay accepted all 1,538 durable
ticks, 3,076 unique client commands/receipts and complete final saves.
Of the 3,840 scheduled slots (192 seconds of observation windows),
1,514 ticks completed inside their windows; 24 additional in-flight
ticks completed afterward. Zero runs qualified at 20 Hz, and zero
original scheduled slots completed on time. There were 1,383 service
latencies above 50 ms out of 1,538 completed operations. The latter
does not count backlog or unserved slots.

Each cell lists the two observed repetitions, in repetition order.
The throughput comparison uses the mean of the two fixed-window
rates; p95 is the service time of each individual repetition, not
a pooled percentile.

| Workload | 1 worker TPS | 2 workers TPS | 1 worker p95 ms | 2 workers p95 ms | Mean TPS change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Spread 256, local | 15.38/15.00 | 14.50/14.75 | 95.11/95.03 | 105.76/92.03 | -3.7% |
| Spread 2,048, local | 9.50/8.88 | 9.12/9.38 | 136.60/172.09 | 154.64/146.89 | +0.7% |
| Spread 8,192, local | 3.12/3.62 | 4.00/3.75 | 471.73/365.12 | 310.16/311.18 | +14.8% |
| Clustered 8,192, local | 4.75/3.38 | 4.62/3.12 | 243.76/451.31 | 272.23/429.37 | -4.6% |
| Spread 256, 5 ms one-way | 12.00/11.75 | 11.38/11.25 | 91.28/94.07 | 96.70/100.44 | -4.7% |
| Spread 256, 25 ms one-way | 4.00/4.00 | 4.00/4.00 | 254.40/257.07 | 254.43/260.54 | +0.0% |

### Interpretation and next engineering target

The highest observed completion rate was 15.375 ticks/s with one
worker and 256 spread agents. The same fixture with two workers
was slightly slower in both repetitions. At 2,048 agents the
mean gain was negligible. At 8,192 spread agents, the extra worker
increased mean completion rate from 3.375 to 3.875 ticks/s
(+14.8%) and improved both p95 observations, but this is far below
the target. The 8,192-agent clustered workload did not benefit;
5 ms one-way delay reduced the low-load rate, and 25 ms one-way
delay limited both layouts to 4 ticks/s in both repetitions.

The existing full-state validation and central publication path
remain material scaling constraints. At 8,192 agents, coordinator
CPU time occupied about 75–78% of elapsed time for spread load and
88–89% for clustered load. Aggregate worker compute CPU time was
only about 7 ms per accepted tick in those cases; this is not a
wall-clock phase breakdown or proof that one particular operation
caused the delay. The next optimization should separately measure
journal writes, serialization/root validation, receipt collection
and client acknowledgment, then reduce redundant work or batch
compatible durable operations without weakening durability, exact
input binding, fencing or independent correctness checks.

**Decision: measured locally, scale qualification not met.** No
tested configuration establishes increased sustainable capacity at
20 Hz. These short windows are sufficient to expose the failure
but cannot prove long-term throughput or a positive scale envelope.
The benchmark did not vary the number of actual player clients or
active regions (two and eight, respectively). Actual Minecraft
clients/runtime, independent physical devices, sustained soak
evidence and quorum-backed production authority remain prerequisites.
No production regional hosting is enabled. Do not substitute
earlier compute-only latency measurements for full durable service.

## Verification and reproduction

The full research suite passed 86 tests (six new); deliberate
altered evidence, incorrect timing and incomplete matrices were
refused. Real worker scopes were released in all 24 runs. The
benchmark uses no hosted CI and does not modify release metadata.

Run only in a fresh disposable delegated systemd scope on Linux:

```sh
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-scale-63-local-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=360 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/scale_lab.py \
  --state-cache uncached --input-mode individual --candidate-mode reparse \
  --transaction-counts trace --worker-exchange separate --publication-mode legacy \
  --output /tmp/new-sc63.json
cd research/distributed-simulation
python3 verify_scale_evidence.py /tmp/new-sc63.json
python3 -m unittest discover -s . -p 'test_*.py' -v
```

Continue #64 ownerless/no-OP signed creation policy research while
keeping this performance and real-game acceptance gate open.
