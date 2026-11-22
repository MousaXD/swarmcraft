# Distributed simulation feasibility research

**Decision:** [RFC 0001](RFC.md) — **NO-GO** for general-purpose Minecraft Java/Fabric active multi-machine simulation using current SwarmCraft. Research-only; existing single-authority replication and failover remain the supported architecture.

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
