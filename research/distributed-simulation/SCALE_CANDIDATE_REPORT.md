# #63 — Verified candidate reuse and lightweight transaction observation

Date: 26 November 2026. Isolated durable reference runtime only.

## Validation and durability contract

An authenticated worker receipt is still checked against the current
world, generation, parent, exclusive region grant and MAC. Each
untrusted region is parsed and validated into an immutable Region
exactly once. The assembled candidate is checked again for complete
ordered region coverage, globally unique bounded agent IDs, exact
identity set, total resources and the independently computed world
root. The transition authority still independently replays the
entire candidate, checks sealed authenticated inputs, verifies current
contribution scope and creates exact committed receipts.

The original redundant full candidate reparse is a selectable
benchmark control. Reuse changes neither the canonical state BLOB
nor the receipt hash. A shallow copy of the validated state mapping
is supplied to the transition callback; the retained regions and
nested data are immutable. The cache is populated from those locally
validated regions and roots only after successful durable COMMIT.
Every subsequent read still fetches the persisted SQLite BLOB, checks
exact bytes against the cache, and revalidates ledger metadata and
grants. Rollback, external mutation, restart and retired-generation
checks are not bypassed. The new test suite checks malformed signed
regions, cross-region duplicate identities, exact receipt hash parity,
caller-owned receipt mutation, cold reopen and mutation of the
committed SQLite state from a second connection.

SQLite trace callbacks expand bound values, including the large
canonical state BLOB. The benchmark now optionally observes successful
BEGIN, COMMIT and ROLLBACK boundaries directly through the common
transaction wrapper rather than expanding every SQL statement.
The observer is not used for authorization or commit decisions and is
disabled in ordinary runtime operation. Tests compare boundary counts
against actual SQLite trace counts on successful individual and
batched ticks and on rollback. Both modes use the same SQL statements,
synchronous FULL durability and independent replay.

The full scale evidence schema 4 adds explicit candidate and transaction
observation modes; the verifier retains all previous fixed-window,
receipt, network, cgroup, cleanup and serial-reference checks. Earlier
schema 1–3 evidence is still accepted and independently replayed.

## Controlled local cost experiment

The separate candidate cost experiment uses actual file-backed SQLite
FULL transactions, but synthetic signed receipts generated from a
deterministic reference fixture, not a real network or distributed
worker processes. It measures only candidate commit and committed
receipt retrieval after the same input seal. It isolates two factors:
reparse versus reuse, and expanded SQL trace versus direct boundary
observation. Each combination uses a fresh database, identical
candidate states for a given workload and repetition, two warmup
ticks and ten measured ticks; the second repetition reverses mode
order. Both 256-agent and 8,192-agent spread fixtures are measured,
with a single CPU affinity. Every accepted cut and receipt is checked,
then the database is closed and reopened and the exact durable journal
and final state are verified. The separate evidence verifier rebuilds
the reference roots, cuts, journal and final-state hashes, and checks
timing accounting and transaction counts. This is not a claim of
independent-host capacity, network service or Minecraft performance.

## Verified paced matrix

[Raw schema-4 evidence](evidence/2026-11-26-scale-reuse.json),
SHA-256 `b12afd47f2ba1735e22cb38a2d62295b6f1968d9ff89beeaaa8cc2f9d5dc4f85`.
The wrapper exited 0 and all worker scopes were released. Independent
replay verified all 24 runs, 2,834 durable ticks and 5,668 client
receipts across 36 worker and 48 client process instances. Of
3,840 scheduled slots, 2,818 completed within their windows,
1,272 met their original deadlines, and 1,560 operations exceeded
50 ms service time. Sixteen ticks completed after their window.
Every accepted tick used two write and two read transactions,
for 11,336 commits and zero rollbacks.

Mean fixed-window TPS across two repetitions:

| Workload | Batched, 1 worker | Reuse, 1 worker | Batched, 2 workers | Reuse, 2 workers |
| --- | ---: | ---: | ---: | ---: |
| Spread 256, local | 19.0625 | 20.0000 | 17.5000 | 20.0000 |
| Spread 2,048, local | 13.5625 | 20.0000 | 12.9375 | 20.0000 |
| Spread 8,192, local | 7.0000 | 14.7500 | 7.3125 | 14.1250 |
| Clustered 8,192, local | 6.9375 | 14.4375 | 7.1875 | 13.9375 |
| Spread 256, 5 ms one-way | 13.6250 | 15.5000 | 14.1875 | 14.7500 |
| Spread 256, 25 ms one-way | 4.3125 | 4.3750 | 4.2500 | 4.2500 |

Five runs qualify under the strict 20 Hz contract: all four
256-agent local runs and the first one-worker 2,048-agent local
run. All four 2,048-agent local runs complete 160/160 within their
windows, but the remaining three miss 2, 2 and 4 individual
deadlines. Heavy and impaired-link runs do not qualify. Two workers
do not improve heavy throughput over one worker in this matrix.

Selected tick-weighted mean exclusive phases (ms):

| Phase | 256 local, 1 worker | 8,192 spread, 2 workers | 256 at 25 ms, 1 worker |
| --- | ---: | ---: | ---: |
| Total service | 23.121 | 70.399 | 226.163 |
| Publication receipt validation | 1.272 | 15.347 | 1.342 |
| Independent serial replay | 0.746 | 12.150 | 0.780 |
| SQLite durable commit | 7.602 | 8.048 | 9.124 |
| Resource guard | 7.192 | 8.941 | 7.007 |
| Worker tick exchange | 1.818 | 4.474 | 152.781 |
| Worker receipt exchange | 0.919 | 7.419 | 51.472 |

This comparison uses the preceding historical batched run, not a
paired full network control. In particular SQLite commit latency
varies with storage/host conditions. Do not attribute the entire
throughput improvement to the code change or to tracing.

## Verified paired candidate cost

[Raw paired evidence](evidence/2026-11-26-scale-candidate-cost.json),
SHA-256 `05e27f852ce3b3f3ecacfee77ae41b41c6ea6ba9984fb2bd8cda0049a6e09e3f`.
All 16 fresh databases, 32 warmups and 160 measured operations
completed, and independent reopen and evidence replay passed.
Median wall and process CPU time in milliseconds (20 samples per
workload/mode):

| Agents | Candidate | Observation | Wall | CPU |
| ---: | --- | --- | ---: | ---: |
| 256 | reparse | trace | 8.299 | 4.567 |
| 256 | reparse | boundary | 7.913 | 4.294 |
| 256 | reuse | trace | 8.025 | 4.149 |
| 256 | reuse | boundary | 7.386 | 3.366 |
| 8,192 | reparse | trace | 51.206 | 47.764 |
| 8,192 | reparse | boundary | 46.566 | 42.758 |
| 8,192 | reuse | trace | 38.821 | 34.335 |
| 8,192 | reuse | boundary | 32.464 | 28.836 |

At 8,192 agents, with observation held at boundary mode, reuse
reduces median CPU time by about 32.6%; with reuse held constant,
boundary observation reduces it by about 16.0%. Relative to the
original reparse/trace combination, the new combination reduces
median CPU by about 39.6% and wall time by about 36.6%. These are
local commit-and-receipt measurements with synthetic signed candidates,
not full network tick costs.

All 116 research regression tests passed, including both new retained
evidence replays. Workflow-policy, release-version and diff checks passed.

**Decision:** the local reference service target is partially
qualified at low load, but #63 remains open. The next bottleneck
is heavy candidate validation, receipt exchange and independent
replay; sequential worker round trips dominate WAN service.
Preserve the same fail-closed and durable checks. A repeated
full-network paired control, sustained soak, actual compatible
Minecraft gameplay, independent physical-host and increased
sustainable player/region-capacity evidence are still missing.
Production regional and ownerless gameplay remain disabled.

## Reproduction

Run full benchmark in the existing isolated delegated memory/cpu
scope, using the same worker cgroup and net namespace guard:

```sh
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-scale-63-reuse-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=360 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/scale_lab.py \
  --state-cache cached --input-mode batched --candidate-mode reuse \
  --transaction-counts boundary --worker-exchange separate --output /tmp/sc63-reuse-new.json

cd research/distributed-simulation
python3 verify_scale_evidence.py /tmp/sc63-reuse-new.json
python3 scale_candidate_lab.py --output /tmp/sc63-candidate-cost-new.json
python3 scale_candidate_lab.py --verify /tmp/sc63-candidate-cost-new.json
python3 -m unittest discover -s . -p 'test_*.py' -v
```

Do not run the paired experiment concurrently with the paced matrix.
The flags `--candidate-mode reparse --transaction-counts trace` select
the previous candidate and observation path.
