# Distributed simulation feasibility research

**Decision:** [RFC 0001](RFC.md) — **NO-GO** for general-purpose Minecraft Java/Fabric active multi-machine simulation using current SwarmCraft. Research-only; existing single-authority replication and failover remain the supported architecture.

## Reopened feasibility (#56, 2026-11-25)

[Experimental runtime selection](REGION_LAB_DESIGN.md) and
[networked region lab report](REGION_LAB_REPORT.md): actual distinct
worker processes, private veth/TCP links with kernel netem, and
deterministic cross-region movement/resource/signal execution checked
against a monolithic reference. The lab stays isolated under this
research directory. The production Fabric NO-GO remains; physical
multi-machine Minecraft gameplay and durable quorum-backed regional
commits are unverified. The earlier 2026-11-08 synthetic proxy below
is retained unchanged as a separate historical experiment.

```sh
unshare --user --map-root-user --net python3 network_lab.py \
  --output /tmp/new-region-lab.json
python3 verify_lab_evidence.py /tmp/new-region-lab.json
```

See the report for Linux namespace requirements and exact workload
matrix. Never interpret lab agents as Minecraft players or turn a
successful lab receipt into authority.

## Resource-aware worker placement (#58, 2026-11-26)

[Placement design](WORKER_PLACEMENT_DESIGN.md) and
[verification report](WORKER_PLACEMENT_REPORT.md): explicit lab consent
and revision-bound probes, real Linux CPU/memory enforcement, measured
path RTT, load-aware selection, and stop-all scale-in from a verified
in-memory cut. Two experiments executed 208 measured ticks with matching
serial roots. Service latency improved on spread load, but checked-loop
throughput did not demonstrate sustained improvement. No production worker
enrollment or Minecraft regional authority is enabled.

## Continuous player world (#59, 2026-11-26)

[Runtime/protocol and verification report](PLAYER_RUNTIME_REPORT.md):
two real reference client processes traverse worker boundaries while
retaining one world identity and independently checked inventory and
views. Two short paced experiments, 296 accepted ticks; real process-kill
and link-partition cases freeze player-facing publication. The private
lab protocol is not Minecraft; no production compatibility or regional
authority is enabled.

## Exclusive regional writers (#60, 2026-11-26)

[Authority design and verification report](REGIONAL_AUTHORITY_REPORT.md):
private SQLite canonical-cut ledger, one-time generation claims,
exclusive region scopes and authenticated worker receipts. Two
real overlapping-worker experiments, concurrent-process commit
and reassignment races, pre/post-commit process kills, and fresh
worker restart from the durable cut. This is a single trusted local
serialization point, not independent quorum or production authority.

## Durable player and inventory transitions (#61, 2026-11-26)

[Durable transitions report](DURABLE_TRANSITIONS_REPORT.md): isolated
protocol-2 transaction ruleset, sealed durable input batches and atomic
world/command receipts. Two real private TCP client experiments preserve
cross-worker movement and inventory through authority-writer kills
before and after commit, fresh workers and exact command retries.
This is trusted local reference evidence, not Minecraft or quorum
certification.

## Capped worker withdrawal and checkpoint recovery (#62, 2026-11-26)

[Worker recovery report](WORKER_RECOVERY_REPORT.md): durable contribution
revisions and lifecycle fencing, actual capped-worker scale-in,
worker SIGKILL and link partition, verified scope cleanup, and
freshly admitted workers replaying the sealed input from the last
accepted cut. No-capacity conditions pause rather than overcommit.
This is stop-all reference-generation recovery under one trusted
local ledger, not live Minecraft migration or quorum certification.

## Original R0 evidence (2026-11-08)


There are **no modifications** to production Rust crates, Minecraft/Fabric bridge, desktop, workflows or release configuration. The included Python standard-library tools have limited meanings:

- [model.py](model.py) / [test_model.py](test_model.py) — in-memory *central* oracle (not a distributed quorum protocol), fenced grants, entity transfers and 10 deterministic invariant tests.
- [benchmark.py](benchmark.py) — **single-machine** synthetic CPU-process comparison and made-up JSON boundary-message serialization; **not** Minecraft code, actual network sync or player throughput.
- [evidence/2026-11-08-synthetic-local.json](evidence/2026-11-08-synthetic-local.json) — captured environment, raw samples, and **assumed** RTT sensitivity. Do not cite these as real multi-host throughput.
- [RFC.md](RFC.md) — design, architecture diagram, risk/compatibility matrix, failure model, sources, constraints and decision.

Run with Python 3 (no dependencies):

```bash
cd research/distributed-simulation
python3 -m unittest discover -s . -p 'test_*.py' -v
python3 benchmark.py --entities 12000 --regions 12 --rounds 64 --repeats 5 --workers 1,2,4 --output evidence/local.json
```

Times vary with CPU quota and scheduler. The `entities` CLI argument denotes **synthetic arithmetic work items**, not Minecraft mobs or players. A real Fabric server benchmark, three independently hosted machines, WAN partition tests, durable coordinator and cross-boundary correctness certification have **not** been performed. See RFC's GO gates before considering an implementation phase.
