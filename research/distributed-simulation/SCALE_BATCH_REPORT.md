# #63 — Atomic input batching and read-only committed receipts

Date: 26 November 2026. Isolated durable reference transaction runtime;
not actual Minecraft or independent-host acceptance.

## Transaction contract

The previous two-client path opened six SQLite write transactions
per tick: two individual command submissions, one input seal, one
canonical publication, and two exact command replays to obtain
already committed results. Two of those transactions were read-only
in effect but used BEGIN IMMEDIATE. It also opened a read transaction
for the preflight. The new explicit batched path:

1. Receives the commands over their existing independent authenticated
   TCP connections. The caller supplies the identities from those
   connections, not a player ID asserted by a command.
2. Under one BEGIN IMMEDIATE and synchronous FULL commit, validates
   the exact current world, parent cut, claimed generation and
   contribution scope, journals the bounded distinct commands,
   verifies existing pending commands and seals the complete next
   input batch. The batch admits at most the registered player
   limit; duplicate player/ID, mixed cuts, conflicting replay,
   nonmembers, and late additions to a sealed batch are refused.
   A separate queue-only batch API retains the existing semantics
   for callers that do not yet want to seal.
3. Rechecks the current runtime grant/cut before worker execution,
   and requires the same authenticated current-generation receipts,
   resource guards, independent serial replay and atomic state,
   batch-result and receipt publication transaction as before.
4. Opens a read transaction to retrieve the exact committed results.
   This operation never inserts or requeues a command. It verifies
   the current ledger, the requested cut and its accepted parent,
   exact sealed inputs, stored request identity and contents, the
   committed receipt's world/tick/root/player and, when retrieving
   the current head, the view from the accepted state. It returns
   results in request order. Historical exact retries remain
   supported; full independent historical replay remains part of
   reopen and evidence verification.
5. Sends and validates the two clients' acknowledgments as before.

All input inserts and sealing succeed or roll back together.
No queued acknowledgment is returned before its transaction
commits; the fused API returns controls only after the same
durable commit. A sealed batch is not a committed world cut.
Withdrawal or fencing can prevent publication even after
the seal succeeds. Existing separate submit/seal APIs and
their exact replay behavior remain available. No database
schema, game wire protocol, ruleset, authority or release
version changes are required.

## Measurement contract

The original full 24-run matrix and phase timing are retained.
Schema 3 adds the input mode and exact observed SQLite transaction
boundary counts for each tick. A trace callback counts BEGIN
IMMEDIATE, BEGIN, COMMIT and ROLLBACK only; it retains no SQL
statement contents. The independent verifier checks these
counts and the same complete state, receipt, network, resource
and cleanup contract as schema 2. For the two-player benchmark,
the expected batched path is two write transactions and two read
transactions with four commits and no rollback; the individual
control is six write transactions and one read transaction with
seven commits and no rollback. Both use the same synchronous
FULL journal policy and independent verification.

## Verified full matrix

[Raw evidence](evidence/2026-11-26-scale-batched.json), SHA-256
`6a267d54d5cc8e1ce8320923fadf3d5fed696a38f3010aaa6123f0a538ab130d`.
The isolated benchmark scope completed with wrapper exit 0; no
matching scope remained loaded afterward. Independent replay verified
all 24 runs, 2,069 durable ticks and 4,138 client receipts, and
the transaction evidence reports 4,138 write and 4,138 read
transactions, 8,276 commits and zero rollbacks. Of the 3,840
scheduled slots, 2,046 completed within the window (23 additional
in-flight ticks completed afterward), 226 met their own deadline,
and 1,574 completed operations exceeded 50 ms service time.
All 36 worker and 48 client process instances completed, and all
worker scopes were released.

Mean fixed-window TPS over two repetitions:

| Workload | Cached individual, 1 worker | Batched, 1 worker | Cached individual, 2 workers | Batched, 2 workers |
| --- | ---: | ---: | ---: | ---: |
| Spread 256, local | 17.250 | 19.062 | 15.625 | 17.500 |
| Spread 2,048, local | 13.250 | 13.562 | 13.188 | 12.938 |
| Spread 8,192, local | 7.500 | 7.000 | 7.625 | 7.312 |
| Clustered 8,192, local | 7.250 | 6.938 | 7.438 | 7.188 |
| Spread 256, 5 ms one-way | 10.000 | 13.625 | 9.500 | 14.188 |
| Spread 256, 25 ms one-way | 3.750 | 4.312 | 3.812 | 4.250 |

One local 256-agent run with one worker completed 160/160
within the window, with 39.59 ms p95 service and 150/160
on-time slots. The repeat completed 145/160 within the window.
No run meets the strict 20 Hz deadline contract. The heavy
spread and clustered cases regress relative to the cached
individual run despite reduced transaction boundaries.

Selected tick-weighted mean exclusive phases (milliseconds):

| Phase | 256 local, 1 worker | 8,192 spread, 2 workers | 256 at 25 ms, 1 worker |
| --- | ---: | ---: | ---: |
| Total service | 41.482 | 135.697 | 229.006 |
| SQLite write transaction commit | 22.210 | 40.542 | 9.671 |
| Resource guards | 7.833 | 9.257 | 7.069 |
| Worker tick exchange | 1.783 | 5.317 | 153.099 |
| Worker receipt exchange | 0.871 | 7.949 | 51.534 |
| Publication receipt validation | 2.087 | 28.991 | 1.826 |
| State decode/roots | 1.035 | 15.947 | 0.898 |
| Independent serial replay | 0.828 | 10.144 | 0.773 |

SQLite commit time fell from 36.986 to 22.210 ms in the
low-load one-worker case. The comparison is against the
previous cached individual run, not a newly measured paired
control. Host load varied and SQLite tracing adds overhead,
including SQL expansion of large checkpoint values. The
transaction counts demonstrate the reduced boundaries; the
throughput and phase differences cannot be attributed solely
to batching. The uncached/individual and cached/individual
schema-3 controls are still to be measured.

Next: remove redundant full candidate parsing and serialization
where already-validated immutable region data can be reused,
while retaining global ID/conservation/root checks and independent
serial replay. Measure the impact of trace instrumentation and
retain controlled comparisons. The sequential four-round worker
exchange remains incompatible with the WAN 50 ms target.

All 109 research tests passed, including retained schema-3 replay;
workflow-policy and release-version checks passed. Regression
coverage includes atomic batch rollback, duplicate and
mixed-cut refusal, exact pending retries, immutable sealed inputs,
real subprocess SIGKILL immediately before and after the durable
seal boundary, restart and generation rotation, read-only historical
receipt replay, uncommitted/corrupt receipt refusal, retired-generation
and consent-withdrawal enforcement, and real transaction count checks
on both paths. A valid sealed batch survives withdrawal before
publication but exposes no committed result.

**Decision: service target remains unqualified.** No actual
Minecraft, independent physical-host, active-region/player-capacity
or sustained soak qualification is established. Production regional
and ownerless gameplay remain disabled.

## Reproduction

Run sequentially in a fresh disposable delegated scope:

```sh
mode=batched # or individual for the prior transaction path
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-scale-63-$mode-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=360 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/scale_lab.py \
  --state-cache cached --input-mode "$mode" --candidate-mode reparse \
  --transaction-counts trace --worker-exchange separate --publication-mode legacy \
  --output "/tmp/sc63-$mode-new.json"
cd research/distributed-simulation
python3 verify_scale_evidence.py "/tmp/sc63-$mode-new.json"
python3 -m unittest discover -s . -p 'test_*.py' -v
```

Use `--state-cache uncached --input-mode individual` to reproduce
the full decode control with current instrumentation. Never run
competing benchmark modes concurrently on the same host.
