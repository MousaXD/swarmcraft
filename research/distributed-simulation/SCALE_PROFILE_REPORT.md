# #63 — Exact-state cache and full durable service profile

Date: 26 November 2026. Reference transaction lab only; no actual
Minecraft or independent-host qualification.

Follow-up: [Atomic batching and receipt lookup](SCALE_BATCH_REPORT.md)
reduces transaction boundaries while preserving the same durable
publication checks. The measurements below are the original
individual-submission cached run.

## Change and safety boundary

The original authority reparsed the same complete SQLite world BLOB,
validated every agent/region and recomputed every region summary
on each transaction. A normal two-client tick includes two durable
input submissions, batch seal, runtime preflight, canonical commit,
and two committed receipt lookups. Several use identical accepted
state. Cache only the most recent structurally validated immutable
state, its ordered IDs and region summaries, keyed by the exact raw
state value returned by SQLite in the current transaction.

The cache is neither a persisted authority record nor a snapshot
lease. Every read still opens the existing transaction and fetches
the world row, verifies world/ruleset, player/agent and resource
invariants, recomputes the tick-bound world and cut roots, validates
current grants, epoch/claim records and committed head. Candidate
receipt MACs, full candidate validation, independent serial replay
and its root check, synchronous FULL transactions and canonical
publication are unchanged. No candidate is considered accepted
from cache contents. A changed BLOB is decoded and validated even
when its reported tick and root are unchanged. Returned dictionaries
and ID lists are fresh; nested region tuples and summaries are
immutable. An uncommitted read followed by rollback cannot cause
tentative bytes to be reused for the prior accepted state.

The research lab now records exclusive monotonic wall times per
accepted tick for input receipt, command journal, seal, runtime
preflight, worker staging/commit exchange, receipt transfer,
resource guards, publication receipt validation, independent replay,
client receipt materialization, checkpoint serialization, journal
write, SQLite transaction begin/commit, ledger read, state decode
and roots, client receipt lookup/send and client acknowledgment.
Nested intervals are subtracted from parent intervals; the
unattributed remainder is retained. All phase durations reconcile
exactly to service latency. They are elapsed wall times, not CPU
profiles or a direct measurement of individual fsync syscalls.
SQLite write-transaction COMMIT includes no-op receipt replay
transactions as well as actual durable writes; SQL executions may
also include journal I/O.

The schema-2 evidence verifier independently replays the same world
and receipt rules, validates the original full 24-run contract, and
refuses incomplete, negative, unknown or non-reconciling phase
accounting. Schema-1 historical evidence remains supported unchanged.
The benchmark offers an uncached control implementation that
executes the original full-state validation on each read while
using the same instrumentation. It is a lab-only comparison mode.

## Verified cached run

[Raw profile receipt](evidence/2026-11-26-scale-profile-cached.json)
(SHA-256
`1aca6efc07bff9b82628dba62b7516d1acb4e74fbd6cbc9d0c3d43cf8ae29fa2`).

The full 24-run matrix uses the same fixture seed, agent counts,
two persistent authenticated reference clients, eight regions,
worker layouts, physical-core assignments, cgroup limits, network
impairments, repetitions, and eight-second 20 Hz windows as the
[historical experiment](SCALE_ENVELOPE_REPORT.md). The prior raw
receipt remains unchanged. These are sequential experiments on
the same physical host, not randomized simultaneous controls:
host load, storage latency and the new timing instrumentation
can affect the comparison. The new uncached instrumented control
was not run for this increment.

Independent replay verified 1,883 accepted ticks and 3,766 distinct
client commands/receipts, all complete final saves and resource
and network observations. 1,859 ticks completed within 3,840
scheduled slots (24 in-flight completions afterward). Sixty slots
met their original 50 ms deadline, and 1,586 completed operations
exceeded 50 ms service latency. Zero runs qualified. The highest
observed single rate was 17.75 TPS. All 36 worker and 48 client
process instances completed their lab runs and every disposable
worker scope was released. The outer systemd scope command
reported exit status 1 even though all 24 run lines and the
complete JSON receipt were produced without a Python traceback.
The user-unit journal showed scope start and consumed CPU time;
the unit was no longer loaded afterward. Independent verification
passed; this is retained as complete replay-verified measurement
evidence, not a claim of a clean outer wrapper exit.

Mean fixed-window TPS over two repetitions:

| Workload | Previous 1 | Cached 1 | Previous 2 | Cached 2 |
| --- | ---: | ---: | ---: | ---: |
| Spread 256, local | 15.188 | 17.250 | 14.625 | 15.625 |
| Spread 2,048, local | 9.188 | 13.250 | 9.250 | 13.188 |
| Spread 8,192, local | 3.375 | 7.500 | 3.875 | 7.625 |
| Clustered 8,192, local | 4.062 | 7.250 | 3.875 | 7.438 |
| Spread 256, 5 ms one-way | 11.875 | 10.000 | 11.312 | 9.500 |
| Spread 256, 25 ms one-way | 4.000 | 3.750 | 4.000 | 3.812 |

Local heavy spread throughput increased 122.2% with one worker
and 96.8% with two relative to the historical observations.
However, adding a second worker to the cached heavy spread case
now yields only 1.7% more throughput, far below the scale goal.
At 5 ms and 25 ms delay the observed rates regressed; do not
attribute all differences to the cache or claim universal gains.

## Phase attribution and next target

Selected tick-weighted mean exclusive elapsed phases, milliseconds:

| Phase | 256 local, 1 worker | 8,192 spread, 2 workers | 256 at 25 ms, 1 worker |
| --- | ---: | ---: | ---: |
| Full service | 54.114 | 129.577 | 265.621 |
| SQLite write-transaction COMMIT | 36.986 | 47.253 | 45.950 |
| Resource guards | 7.003 | 8.662 | 7.316 |
| Worker tick exchange | 1.732 | 5.058 | 152.874 |
| Worker receipt exchange | 0.864 | 6.610 | 51.471 |
| Publication receipt validation | 1.709 | 24.144 | 1.818 |
| State decode and roots | 0.871 | 13.226 | 0.874 |
| Independent serial replay | 0.707 | 9.794 | 0.773 |
| Other ledger read work | 1.623 | 6.722 | 1.715 |

The complete phase profile is derivable from the verified raw
receipt. The principal local low-load constraint is now SQLite
write-transaction COMMIT (~68% of service for one worker).
Heavy cases also spend material time in full receipt validation
and independent serial replay. At 25 ms one-way, the worker
exchange and receipt rounds alone take about 204 ms; the current
sequential network barrier cannot sustain 20 Hz there.

The next implementation target is atomic batching of compatible
authenticated player commands and read-only lookup of exact
already-committed receipts, reducing transaction boundaries
without acknowledging a queued command before durable commit,
changing sealed inputs, accepting a partial cut, or removing
current grant/root/replay checks. Then profile receipt validation
and the redundant serialization of validated regions. Keep all
resource checks, fail-closed behavior and publication atomicity.

## Verification and reproduction

All 96 research tests passed, including the retained schema-2
matrix, cache mutation/rollback and phase accounting regressions.
Workflow-policy and release-version checks passed locally.

Run each mode sequentially in a fresh disposable delegated scope,
never concurrently on the same benchmark host:

```sh
mode=cached # or uncached for the instrumented control
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-scale-63-profile-$mode-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=360 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/scale_lab.py \
  --state-cache "$mode" --input-mode individual \
  --candidate-mode reparse --transaction-counts trace --worker-exchange separate \
  --output "/tmp/sc63-$mode-new.json"
cd research/distributed-simulation
python3 verify_scale_evidence.py "/tmp/sc63-$mode-new.json"
python3 -m unittest discover -s . -p 'test_*.py' -v
```

**Decision: service target remains unqualified.** These experiments
do not measure actual Minecraft clients, independent physical
hosts, active-region/player-capacity scaling or sustained soak.
Production regional and ownerless gameplay remain disabled.
