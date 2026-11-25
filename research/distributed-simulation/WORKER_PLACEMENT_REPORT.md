# #58 — Resource-aware worker placement: locally verified research

Date: 25 November 2026 UTC / 26 November UAE time.
Scope: one physical Linux workstation, real capped reference-runtime
worker processes with separate private network namespaces and authenticated
TCP. **Not Minecraft, independent physical hosts, production worker
admission, or a field release gate.**

The [placement design](WORKER_PLACEMENT_DESIGN.md) describes the bounded
consent, resource admission and deterministic placement rules. The
experimental runtime remains the #56 custom region model. The production
single-JVM Fabric NO-GO is unchanged. The #57 native CLI/Desktop preference
is not connected to this isolated Python experiment.

## Evidence

Two completed final experiments use 16,384 spread agents (seed 58),
eight regions, 12 initial one-worker ticks, an identical 32-tick
one-worker comparison and scale-out branch from the same cut, then
12 ticks with a smaller one-worker budget after explicit withdrawal.
The separate clustered fixture has 16,384 agents (seed 59, 85 percent
initially in region zero), with 16 ticks. Both completed reports also
contain a real CPU-policy tamper after one verified tick of a separate
small fixture.

- [Run 1 raw receipts](evidence/2026-11-26-placement-1.json)
- [Run 2 raw receipts](evidence/2026-11-26-placement-2.json)
- [Static network regression receipts](evidence/2026-11-26-static-placement-regression.json)

The runner measured actual per-worker affinity/quotas, current/peak cgroup
charges, CPU counters, six authenticated RTT samples, veth counters, and
kernel netem packet counters. Each final scale-out run assigned regions
0–3 to worker 0 and 4–7 to worker 1. Both had 96 MiB process-charge caps
and 1000 millicore CPU quotas; kernel observed peaks were approximately
13.7–14.4 MiB, and each executed about 173–176 ms of region compute over
32 accepted ticks. Neither probe was given production files or authority.
A 32 MiB/250 millicore candidate was refused on memory fit and the
25 ms one-way netem candidate was refused on measured RTT (50.945 and
51.132 ms p95). Both refused probes exited before the selected workers
were initialized. Zero slots never started a process.

| Comparison, same starting and final root | Run 1 | Run 2 |
| --- | ---: | ---: |
| One worker, p50 service ms | 11.797 | 11.589 |
| Two workers, p50 service ms | 6.564 | 6.572 |
| One worker, p95 service ms | 13.577 | 13.259 |
| Two workers, p95 service ms | 9.479 | 8.821 |
| Observed p95 reduction | 30.2% | 33.5% |
| One worker, checked 32-tick loop wall ms | 1175 | 1174 |
| Two workers, checked 32-tick loop wall ms | 1172 | 1290 |
| Service-time samples over 50 ms | 0/64 | 0/64 |

Service duration is only the reference coordinator's tick barrier,
including real compute and transport; it excludes serial shadow
verification and the per-tick resource checks. **The end-to-end checked
loop did not show a sustained speedup**: the second run was slower
with two workers when all checking overhead was included. This is a
bounded useful-work result, not a 20-TPS gameplay claim or approval of
the scheduler for production.

## Acceptance and negative cases

Both final runs:

- Confirmed 104 measured ticks per run (208 total, including the comparison
  branches) match independent monolithic serial roots; final full states
  also match. Old sessions/leaves exited before new generations. The
  main spread branch reaches tick 56 without resetting time.
- Admitted the additional worker only for spread-load placement with
  sufficiently improved modeled critical path; the clustered case
  selected one worker instead of an unhelpful second. On withdrawal,
  the next generation used only worker 0, with an 80 MiB process cap.
- Rejected too little memory (32 MiB), too little CPU (250 millicores
  unable to fit the modeled 50 ms deadline), and all-player-only offers.
  All preserve tick 56 and its root with no work assignment. All-player-only
  performs no probe. Actual kernel headroom and configured limits were
  checked before and after each accepted tick.
- In the isolated policy-change fault, one tick matched the independent
  root before changing its leaf from 1000 to 500 millicores. The next tick
  was refused by the kernel-policy check; no stage was exported, all
  worker processes exited, and the poisoned scope refused a new
  generation. The retained tick is only an in-memory lab observation,
  not a durable commit.

Nine new tests, 27 prior research tests (36/36), and replay of both
final receipts passed. Six intentional evidence alterations per report
were refused. The small #56 regression ran six real static partition
measurements (1/2/4 workers on local/near paths, 24 networked ticks plus
four serial ticks), and the existing process-kill and link-partition
faults; its independent verifier passed. Earlier exploratory placement
runs were used to refine capacity rechecks and fault handling before
the two final retained runs. No hosted CI, production application or
canonical state was changed.

## Limitations / decision

The lab chooses among local slots, not real enrolled devices; its trusted
coordinator checks honest reference computation. The costs/thresholds
are explicitly selected heuristics, and the scope reinitializes all
workers at an in-memory cut rather than live-migrating. Current
measurements and private HMAC channels do not authorize remote
compute, quorum, regional authority, player state or live routing.
Independent hosts, real Minecraft clients and a reviewed compatible
region runtime are required before production #58–63 acceptance.
No production activation, no new compatibility claim, no unsafe
fallback. Next: #59 runtime/ruleset and real region-compatible
player interaction feasibility; retain the #56 adoption gate.
