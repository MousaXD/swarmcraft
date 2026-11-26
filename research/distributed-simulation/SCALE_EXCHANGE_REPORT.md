# #63 — Fused authenticated worker commit and snapshot exchange

26 November 2026. Isolated reference runtime, not Minecraft or
independent physical-machine qualification.

## Exchange contract

The fenced reference adapter previously used four sequential request/reply
barriers: prepare, seal, worker-local commit, and signed snapshot. The
new default sends `commit_snapshot` only after the complete seal barrier
has established the exact proposed root. Each worker executes the
existing seal-bound local commit, then returns the original acknowledgment
and the exact existing authenticated snapshot in one bounded frame.
The request does not grant permission to skip prepare or seal.

The coordinator still requires an acknowledgment from every worker for
the exact tick and proposed root. The fenced adapter verifies each
returned receipt's tick, root and expected channel session. Publication
uses the unchanged authority path: authenticated grant receipt,
complete region coverage, global identity and conservation checks,
independent replay where applicable, current contribution/fence checks,
and synchronous FULL durable commit. Client acknowledgments still follow
that durable commit. The original receipt body, MAC and durable
receipt hash are unchanged. PolicyWorker's additional ownerless
binding and MAC are produced and verified through the existing policy
path, not through a weaker signing shortcut.

The worker-local commit remains tentative with respect to the authority.
A missing, malformed, incorrect or unauthenticated response freezes
the coordinator and adapter, without a durable cut or client receipt.
There is no fallback to the separate request, automatic retry, or
reuse of the claimed generation after an ambiguous reply. Recovery
requires fresh grants against the accepted durable base. The fused
exchange shares the original prepare/seal/commit absolute deadline;
it does not acquire a new snapshot deadline.

The legacy separate exchange remains explicitly selectable as a
research comparison. Non-fenced Coordinator users retain their
original three-barrier protocol. Production networking, authority,
gameplay and application release metadata are unchanged.

## Measurement method

The full paced benchmark adds schema 5, including the selected exchange
mode and actual authenticated channel sequence deltas per worker per
tick. It reconciles these with the aggregate channel counters and
independently verifies the same timing, durable state, client receipt,
resource and kernel evidence. Three request and three reply frames per
worker per accepted tick are required in fused mode; separate mode
requires four of each. This counts authenticated application messages,
not TCP packets. Schema 1–4 receipts remain supported.

Exclusive timing scopes have slightly different boundaries: fused mode
attributes the combined commit acknowledgment and snapshot exchange
to worker_receipts; separate mode includes the commit acknowledgment
in worker_tick. Compare the sum of those two phases, not either phase
alone. This is a reduction in protocol dependency rounds, not a
guarantee of 25% lower wall-clock service time.

## Verified full paced matrix

[Raw schema-5 evidence](evidence/2026-11-26-scale-fused.json),
SHA-256 `977f7ac48a01d86726a17233ed854ea3eaec94862d38397f758d836eb9159f9d`.
The wrapper exited 0 and all worker scopes were released. Independent
replay verified 24 runs, 2,185 durable ticks and 4,370 client receipts,
36 worker and 48 client process instances. There were 2,162 completions
within the 3,840 scheduled slots, 142 original slots on time and
23 operations that completed after the window. Zero runs qualified
and 1,717 completed operations exceeded 50 ms service time.
The recorded 3,237 worker-tick instances each use three authenticated
request and three reply messages; 9,711 frames in each direction,
19,422 total, reconcile with the channel sequence counters.
The separate path would require four of each per worker-tick.
Every accepted tick retains two read and two write transactions,
8,740 successful commits in total and zero rollbacks.

Mean fixed-window TPS across two repetitions:

| Workload | Separate, 1 worker | Fused, 1 worker | Separate, 2 workers | Fused, 2 workers |
| --- | ---: | ---: | ---: | ---: |
| Spread 256, local | 20.0000 | 19.6875 | 20.0000 | 17.9375 |
| Spread 2,048, local | 20.0000 | 15.6875 | 20.0000 | 13.1875 |
| Spread 8,192, local | 14.7500 | 8.3125 | 14.1250 | 10.5625 |
| Clustered 8,192, local | 14.4375 | 8.7500 | 13.9375 | 6.1250 |
| Spread 256, 5 ms one-way | 15.5000 | 12.3125 | 14.7500 | 11.9375 |
| Spread 256, 25 ms one-way | 4.3750 | 5.3750 | 4.2500 | 5.2500 |

The separate results are from the previous retained schema-4
candidate-reuse experiment, not a newly paired full-network control.
The new host had elevated CPU and I/O load during the run.
For the one-worker 256-agent local case, mean full service time
increased from 23.121 to 45.479 ms, with SQLite durable commit
increasing from 7.602 to 27.714 ms. Heavy and near-link throughput
regressed. Do not attribute these changes solely to the exchange.

For the one-worker 25 ms one-way profile, the sum of the measured
worker exchange phases fell from 204.253 to 153.792 ms (about one
round trip); mean service fell from 226.163 to 184.186 ms despite
increased mean SQLite commit time. The corresponding two-worker
exchange sum fell from 203.865 to 153.249 ms. This is consistent
with removing one sequential round trip, not evidence that the
remaining WAN service target has been achieved.

## Regression boundary

Both modes are exercised over actual private loopback TCP with
independent worker subprocesses. Their authenticated channel counters
match the expected four versus three requests/replies and the
resulting accepted states match serial replay. A subprocess deliberately
exits after its worker-local tentative commit but before sending the
fused reply. The observed local tick advances, yet the coordinator
freezes, the durable save/history remain unchanged, and restart with
the claimed generation is refused. A fresh generation and fresh
workers recover from the exact accepted base.

Additional deterministic fault tests exercise malformed acknowledgments,
missing/wrong-session/altered receipts, a wrong parent, missing
policy MAC, a commit without a seal and lost reply. The fused
snapshot is identical to an explicit snapshot from the same worker
and retains the original receipt hash. Existing consent withdrawal,
retired-generation, independent replay and durable rollback tests
remain mandatory.

All 125 research regression tests passed, including the retained schema-5 replay and real TCP subprocess tests. Workflow-policy, release-version and diff checks passed.

**Decision:** verified structural exchange reduction; no complete
service or multi-worker capacity qualification. Preserve both
comparison modes and prior raw evidence. A controlled interleaved
network comparison on a stable host, sustained load, actual Minecraft
and independent physical-worker qualification remain outstanding.

## Reproduction

Run each mode sequentially in a fresh delegated scope:

```sh
mode=fused # or separate
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-scale-63-exchange-$mode-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=360 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/scale_lab.py \
  --state-cache cached --input-mode batched --candidate-mode reuse \
  --transaction-counts boundary --worker-exchange "$mode" --publication-mode legacy \
  --output "/tmp/sc63-exchange-$mode-new.json"
cd research/distributed-simulation
python3 verify_scale_evidence.py "/tmp/sc63-exchange-$mode-new.json"
python3 -m unittest discover -s . -p 'test_*.py' -v
```

The 25 ms one-way delay profile still needs three sequential worker
round trips, so it cannot meet a 50 ms full durable tick deadline.
Do not enable production regional or ownerless gameplay on the basis
of this optimization.
