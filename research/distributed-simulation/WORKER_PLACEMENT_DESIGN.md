# #58 — Resource-aware placement reference design

Research only. The #56 Fabric NO-GO and the single-authority production
architecture remain unchanged. This is an explicit opt-in **custom Python
simulation**, not a daemon, a Minecraft-compatible engine, a deployment
service, a remote worker enrollment protocol, or a canonical world writer.

## Consent and admission

The operator invokes one bounded experiment with fixed local synthetic worker
slots (0–3). Every slot has a positive process-charge/CPU offer or an explicit
zero/zero player-only choice, plus a monotonically changing revision. The
consent ledger rejects missing choices, stale revisions, and changed limits
under the same revision. A positive offer only permits an isolated probe;
the worker receives no region state until selected. Zero never spawns a
worker. Unselected probes must exit before any simulation initialization.
The #57 native resource controls are a separate production feature: they
are not imported into this lab, and the lab cannot enroll real users or
consume production world files. A Python worker has **no Java heap**.
Its requested memory budget is total cgroup memory charges, not JVM heap
or guaranteed process RSS.

The lab requires a fresh mapped-user/private network namespace and an
explicitly named disposable delegated cgroup-v2 systemd scope with CPU and
memory controllers. It refuses ordinary host networking or a production
cgroup. No root privilege, host controller changes or other applications
are needed. Each probe is attached to its own capped leaf before executing
the worker; its effective CPU affinity, current and ancestor quotas,
memory charge max/high, zero swap, group OOM, zero CPU burst where
supported, process membership, and memory events are checked.
Capacity is the minimum of CPU affinity and visible finite ancestor
quotas, not idle CPU. Machine memory headroom is bounded by measured
MemAvailable and finite ancestor memory max/high less current charges.
The runner budgets even probe-only workers against shared capacity,
reserves one CPU-equivalent and 1 GiB for its coordinator/machine headroom,
and uses distinct physical cores. Linux is required; there is no
Windows/macOS hard-cap fallback. These are conservative local lab
parameters, not production defaults.

A candidate must supply a matching-revision observation, with verified
enforcement, sampled RTT no older than ten seconds, at least 250 millicores
of effective CPU, and at least 34 MiB of usable worker memory. Usable
memory is the smaller of memory.high and max minus 8 MiB headroom.
The controller obtains six authenticated echo samples after one warm-up,
uses the maximum (nearest-rank p95 of six) and rejects paths over 8 ms.
The 25 ms one-way impaired candidate uses actual kernel netem in both
directions, not an injected sleep. The lab does not model asymmetric
internet congestion, bandwidth contracts or relay trust.

## Placement policy

Count actual current agents and edge agents in each region of the accepted
synthetic cut. Estimate CPU microseconds as 200 per region + 2 per resident
agent + 3 per edge agent. Estimate process memory as 32 MiB per worker plus
2 MiB per owned region + 1 KiB per resident/possible adjacent incoming
agent. These fixed, deliberately conservative heuristics are design
choices for the reference rules, not measured Minecraft costs. The kernel
enforces the actual charge and CPU limits independently.

Enumerate all contiguous linear partitions of the eight-region ring in
region-ID order across one to four distinct eligible slots. The ring-wrap
boundary is included in the crossing cost. Reject incomplete assignments,
over-memory assignments, and estimated critical paths above 50 ms.
Compute estimated service cost from the largest per-worker CPU cost
scaled by its effective millicores, three times the largest measured
RTT, plus two microseconds per estimated inter-worker edge event. Ties
are deterministic. Choose the smallest feasible worker count unless
another count reduces the selected estimated cost by at least 20 percent
and 1 ms. An additional worker may also be necessary simply to fit the
world into the separate per-worker memory caps. No useful feasible
placement means the world remains at its previous verified lab cut.
No region is split or assigned to two slots.

The estimator is not an actual pacing guarantee or a measured admission
SLA. The live experiment verifies the selected region ownership and real
worker computation. It checks policy, sample age, CPU, memory, shared
headroom and estimated region load before and after every tick. The
independent serial shadow in this lab supplies current region load for
those checks; it is not a production scheduler. A change invalidating
any assumption freezes the experiment, with no exported stage or
subsequent generation. There is no online capacity renegotiation.

## Clean scale-out and scale-in

Only after all phase acknowledgments, comparison of each accepted root
against the independent serial traversal, and verification of the full
snapshot may the experiment retire the current coordinator. It closes
all sessions, checks each old worker's clean exit and empty cgroup,
removes its leaves and veth interfaces, and only then starts new
ephemeral authenticated sessions with the same verified cut and tick.
Explicit changed consent needs a new revision. A separate comparison
branch is a sequential experiment starting from the same copied
in-memory reference cut, never concurrently running writers.

A changed CPU limit is not silently applied to a live placement. The
fault test changes only its own disposable leaf after one verified tick;
the next tick is refused, the stage cannot export a cut, and the
scope is poisoned against another generation. A process loss, malformed
receipt, expired measurement, cleanup failure or incomplete scope also
prevents successful stage publication. This is stop-all/restart from an
in-memory cut under one trusted coordinator, not live region migration,
durable fenced ownership, crash recovery, or a quorum-certified commit.
The #60–62 authority and durable migration requirements remain separate.

## Reproduce

Linux user/network namespace support, iproute2/netem, Python 3 and
systemd user delegation of CPU+memory are required, with five distinct
available physical cores and measured shared memory headroom.

From repository root, use a new private output path. Never run this
scope helper on a production service.

```sh
umask 077
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-placement-58-local-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=100 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/placement_lab.py \
  --output /tmp/new-placement-lab.json
cd research/distributed-simulation
python3 verify_placement_evidence.py /tmp/new-placement-lab.json --negative-checks
unshare --user --map-root-user --net sh -ec '
  ip link set lo up
  python3 -m unittest discover -s . -p "test_*.py" -v
'
```

The independent verifier recomputes serial roots from fixed seeds,
replays the deterministic planner and validates the retained evidence
invariants. It cannot cryptographically attest to historical measurements
or certify a malicious worker. See [the report](WORKER_PLACEMENT_REPORT.md).
