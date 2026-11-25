# #56 — Reopened region/tick feasibility: networked reference runtime

**Date:** 2026-11-25. **Scope:** research-only custom discrete world, real
TCP connections between separate Linux network namespaces on one
workstation. Not an implementation or qualification of Minecraft,
Fabric, SwarmCraft canonical regional authority or physical multi-host
gameplay.

## Decision

The previous [Fabric NO-GO](RFC.md) remains binding for the existing
single-JVM architecture. The [experimental runtime selection and
predeclared rules](REGION_LAB_DESIGN.md) chose a custom deterministic
reference runtime, not a compatible Minecraft server or a production
fork. Keep it out of the shipped application and canonical protocol.
The expanded roadmap can continue resource-consent design independently;
no production worker placement, region grant, client route or
distributed mode may be activated by this experiment.

The synchronous whole-world barrier is useful as an intentionally simple
correctness reference but is **not** a viable default architecture for
consumer-WAN distributed Minecraft. Moving toward independent region
clocks, merging strongly coupled regions, a separate global-state owner,
and durable certified cross-region transactions requires a new reviewed
design and evidence. The prior Fabric decision is not superseded by this
restricted lab.

## What actually executes

Each worker runs a distinct process on one selected physical CPU core,
holds its owned regions and computes their next state. The synthetic
world is an eight-region ring of 1,024 cells; agents carry unique IDs,
integer positions and inventory. Deterministic pickup/drop, one-cell
movement, previous-tick neighbor signals and resource debits/credits are
real state updates, not arbitrary CPU loops or synthetic RTT formulas.
The coordinator routes only cross-worker events, collects local state
digests/counts and stages acceptance through prepare, seal and commit
acknowledgment. Internal crossings are applied on the same worker.
The lab uses ephemeral per-connection HMAC and direction-bound frame
sequence, a 4 MiB frame bound and a shared per-tick deadline.

A separate monolithic reference traverses whole-world arrays and
processes agents in ID order. Every accepted tick root from the
networked runs is compared against it; every final full state is
compared structurally. Both agent count and total resources are
conserved, and local duplicate IDs and duplicate boundary messages are
rejected. This establishes parity for **only** the specified discrete
rules and honest-process fixture. There is no Byzantine computation
proof, real Minecraft protocol, player authentication, vanilla mod
compatibility, durable region journal, distributed consensus,
reassignment, or independent persistent failure domain.

## Method and reproduction

The host is a Linux Ryzen 5 5600H workstation (12 logical CPUs, six
physical cores, approximately 13 GiB RAM) also running unrelated desktop
applications. The lab selected CPU 0 for the coordinator/monolithic
reference and CPUs 2, 4, 6, 8 on distinct cores for workers. CPU affinity
is not a hard quota; RSS is a process high-water mark, not reserved
memory or one pooled Java heap. No other applications were closed.

An outer mapped-root user/network namespace begins with only loopback
and no default route. Each worker is placed in a separate child network
namespace connected by an actual veth pair. The harness refuses a host
network, binds only its private lab endpoint, uses ephemeral secrets on
stdin, and creates no reusable worker service. The three delay profiles
are no netem, 5 ms one-way on both ends, and 25 ms one-way on both ends.
Measured echo RTT, actual application frame counts, veth counters and
kernel qdisc statistics are retained. There is no artificial sleep
standing in for network transit. Delays are fixed; jitter, loss and
real internet/relay behavior are not covered by these profiles.

The main matrix is two repetitions of 32 ticks, with seeds 56 and 57,
spread 8,192 and 16,384 agents, clustered 8,192 (85% initially in one
region), and boundary-heavy 8,192. Each seed has one direct monolithic
serial reference and the same networked rules on one, two and four
workers under each of the three profiles. Initialization, first echo
warmup and final full-state export are outside the measured tick window.
Per-tick service duration includes prepare, real network I/O, delivery,
digest/receipt validation and commit acknowledgments. Timings are
unpaced: a service duration over 50 ms misses the nominal 20 TPS
processing budget. They do not measure sustainable real gameplay TPS,
periodic scheduler jitter or player throughput. p50/p95/p99 use
nearest-rank samples; with 32 samples p99 is the run's maximum.
Raw samples and roots are retained.

Run from repository root on a Linux host supporting unprivileged user
and network namespaces, veth and netem, with Python 3, iproute2 and five
available distinct physical cores. Do not use sudo or a production
network/profile.

```sh
cd research/distributed-simulation
unshare --user --map-root-user --net sh -ec '
  ip link set lo up
  python3 -m unittest discover -s . -p "test_*.py" -v
'
unshare --user --map-root-user --net python3 network_lab.py \
  --ticks 32 --repeats 2 --workers 1,2,4 --profiles local,near,wan \
  --scenarios spread:8192,spread:16384,clustered:8192,boundary:8192 \
  --output /tmp/new-region-lab.json
python3 verify_lab_evidence.py /tmp/new-region-lab.json
```

Use a new output path; the lab refuses to replace existing evidence.
The verifier independently recomputes every serial root and crossing
count and checks the retained timing and network receipts. It is an
integrity/reproducibility check of the experiment, not cryptographic
attestation of historic execution.

## Existing Minecraft baseline (not a direct comparison)

The earlier #35 [capacity report](../game-capacity/REPORT.md) exercised
actual vanilla Minecraft 26.1.2 / Java 25 on this same workstation.
Its 16 connected synthetic game clients sustained 19.990 TPS for a
30.065 second stage, 7.9 ms median of sampled server-average MSPT and
1,397.6 MiB peak sampled Java RSS at a 1,536 MiB selected heap. This
is actual Minecraft evidence but is not a Fabric, distributed Minecraft
or per-tick tail-latency benchmark. The reference agents in this lab
are **not Minecraft players**. The meaningful same-rules performance
comparison below is against the custom monolithic reference and
one networked worker, not a ratio to the earlier Java measurements.

## Measured results

The full [networked evidence JSON](evidence/2026-11-25-networked-region-lab.json)
and [p50/p95/p99, RTT, traffic and deadline table](evidence/2026-11-25-networked-region-summary.md)
cover 72 networked runs, 2,304 accepted distributed ticks, and eight
monolithic reference runs (256 ticks). Every recorded distributed tick
root matched its exact same-seed serial reference; final full states,
boundary event counts, ownership, agent/resource invariants and network
counter receipts all passed. The independent offline verifier recomputed
the roots and crossing counts and checked the measurement summary.

### Tick processing latency

Median of the two per-run p95 values in milliseconds; each cell contains
64 total measured ticks across both seeds. The direct reference is not
networked. The networked one-worker mode still includes three TCP
barriers, validation and regional staging.

| Workload | Direct serial | Local 1 | Local 2 | Local 4 | ~10 ms RTT 1 | ~10 ms RTT 2 | ~10 ms RTT 4 | ~50 ms RTT 1 | ~50 ms RTT 2 | ~50 ms RTT 4 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Spread 8,192 | 7.48 | 8.19 | 4.42 | 3.03 | 40.71 | 38.05 | 35.23 | 161.58 | 157.06 | 156.16 |
| Spread 16,384 | 16.65 | 13.35 | 6.86 | 6.41 | 47.38 | 40.56 | 39.31 | 170.01 | 162.85 | 157.72 |
| Clustered 8,192 | 7.09 | 6.78 | 5.72 | 6.36 | 40.16 | 39.86 | 38.85 | 161.07 | 159.67 | 159.51 |
| Boundary-heavy 8,192 | 8.46 | 32.71 | 28.29 | 23.50 | 60.60 | 62.96 | 56.13 | 184.96 | 181.36 | 179.19 |

Four local workers reduce p95 relative to one networked worker by
approximately 2.70x and 2.08x on the two spread workloads. The
clustered fixture improves only marginally (two workers actually have
the lower median p95). The boundary-heavy four-worker lab remains
slower than direct monolithic simulation despite some improvement
over the one-worker regional protocol. Variation between seeds and
runs, short samples and workstation scheduling limit generalization.
The local spread cases are promising *for the reference rules*, not
evidence that more Minecraft players can be sustained. All the
direct/local cases fit the measured service budget at the tested loads,
so there is no measured capacity limit or proof of the prior 25%
sustained-gameplay throughput gate.

### Real network and deadline results

| Configured one-way delay | Observed median framed echo RTT | Tick service >50 ms | Result |
| --- | ---: | ---: | --- |
| No netem | 0.094 ms | 0/768 | All measured local-link ticks fit |
| 5 ms at both ends | 10.272 ms | 156/768 | All misses in boundary-heavy runs |
| 25 ms at both ends | 50.406 ms | 768/768 | Every measured tick misses |

The three serial barrier exchanges impose an unavoidable measured
network penalty in this reference design. This is real TCP/veth/kernel
netem evidence, not extrapolation from assumed RTT. It does not
exclude different asynchronous or region-local-clock protocols.

The full matrix routed 1,202,688 cross-worker agent movements and 984
cross-worker resource transfers, counting repeated events across runs,
not distinct agents. In the boundary-heavy fixture all 8,192 agents
cross a region edge on every tick. With two workers, 2,048 of those
cross a worker boundary; with four workers, 4,096. This is a deliberately
adversarial shape, not a normal Minecraft migration rate. At four local
workers the actual coordinator-side combined veth traffic averaged
176,151 bytes/tick in the boundary case versus 6,565 bytes/tick in
the 8,192 spread case. At one worker boundary events stay local and
the corresponding coordinator veth traffic is only 1,997 bytes/tick.
Frame and veth counters are distinct; the latter include TCP/link
overhead and control packets. These are local link totals, not WAN
bandwidth, relay costs or a throughput guarantee.

| Workload, local link | Workers | Sum of worker CPU ms/tick | Coordinator CPU ms/tick |
| --- | ---: | ---: | ---: |
| Spread 8,192 | 1 | 5.81 | 0.38 |
| Spread 8,192 | 4 | 6.22 | 0.72 |
| Spread 16,384 | 1 | 11.05 | 0.42 |
| Spread 16,384 | 4 | 13.31 | 1.02 |
| Clustered 8,192 | 1 | 5.23 | 0.36 |
| Clustered 8,192 | 4 | 6.25 | 0.69 |
| Boundary-heavy 8,192 | 1 | 23.81 | 0.46 |
| Boundary-heavy 8,192 | 4 | 30.76 | 11.95 |

CPU values are medians of per-run process CPU time divided by accepted
ticks, not elapsed wall time; parallel CPU sums may exceed tick
latency. More workers consume more resources, especially for boundary
routing. The JSON also retains per-process reported peak RSS (roughly
26--30 MiB in the local cases); on Linux an inherited process
high-water mark and earlier allocations can influence that reading.
These are not simultaneous RSS samples, hard reservations or a
measurement of JVM heap pooling.

## Fault and correctness evidence

The small smoke matrix (18 networked runs) first exposed a cleanup
race in which the kernel removed a veth during explicit cleanup.
The harness was corrected to tolerate only verified disappearance,
not arbitrary deletion failure. The repeat and full matrix passed.
Ten original in-memory oracle tests and 17 new region/routing/wire
tests passed. The new tests cover monolithic parity across layouts,
ring wrap, neighbor signal and resource propagation, duplicates,
wrong-position moves, stale prepares, wrong-owner receipts,
nonconservation, seal binding, wrong commit acknowledgment, HMAC
failure/reflection/replay, oversized frames and partial-frame deadline.

Actual process and network faults were injected after one accepted
boundary-heavy tick in the full matrix:

| Injected fault | Refusal | Accepted cut |
| --- | --- | --- |
| Kill worker after prepare | EOF, 4.13 ms | Tick 1 and root unchanged |
| Bring worker's actual veth down after seal | Timeout, 751.32 ms | Tick 1 and root unchanged |

Both runs refused all further coordinator ticks and refused snapshot
export. All scoped worker processes exited. The surviving worker in
the kill case remained at local tick 1 with staged work; in the
partition case the surviving worker had already applied tentative
local tick 2 while the coordinator remained at accepted tick 1.
**This divergence is observed, not hypothetical.** It is why the
lab must freeze and discard the run, and why an in-memory barrier
must not be adopted as a canonical commit/recovery protocol. The
prior root is retained in memory only; there is no durable recoverable
world cut, quorum-certified commit decision, client-visible atomicity
or worker restart proof. A real runtime must fence every publication
and route against a durable accepted generation and consistent cut,
and prove replay-safe recovery after coordinator/worker loss before
serving clients. The fault receipts are not production fencing evidence.

## Compatibility and next implementation boundary

| Runtime / rules | Scope of decision |
| --- | --- |
| Current Minecraft 26.1.2 / Fabric 0.19.3 / Java 25 | Preserve supported single-authority architecture and pinned candidate policy; distributed execution remains NO-GO. |
| Folia | Same-machine region scheduling reference; no tested cross-machine SwarmCraft adapter. |
| MultiPaper | Upstream 1.20.1 Purpur beta and central master; incompatible with the current Fabric tuple and safety contract; not adopted. |
| Minestom | Possible future custom-protocol foundation; vanilla survival and ordinary Fabric mods are not provided; not adopted. |
| Selected custom reference engine | Eight fixed regions, deterministic discrete movement, inventories, resource transfer and neighbor signals only. No Minecraft protocol, mods, saves, real clients, or general server compatibility. |

The next independent actionable work is #57, explicit per-device CPU
and memory preferences with honest measured/admitted/active states.
It must not enable distributed execution or grant authority. For
#58--63, any proposed Minecraft-facing runtime needs an explicit
supported version/mechanics/protocol matrix and a reviewed
transaction/recovery architecture. The required real comparison on
at least two physically independent workers and a separate control/
voting failure domain is still missing. The original GO gates on
real gameplay, mod compatibility, canonical recovery, per-device
resource enforcement, sustained throughput, and zero detected
corruption are not passed. Ownerless governance is a separate
track and must not inherit authority from this lab.

No production Rust, Java, Desktop, schema, workflow, release,
canonical store or existing game state was changed or activated.
No external worker, actual Minecraft session, hosted CI or real
WAN path was used.

### Evidence provenance

The measured source was the local worktree based on
`0c10a438d8d6887a6e8e3212503f6d127ec8df41`, with the new research
files not yet committed. Hashes of the exact tested sources and raw
receipt (SHA-256):

- `region_runtime.py`: `c64331df64cfc3afeb0778293958c6cdf309d7aa6076ae424ee6bb9930b9f5b4`
- `network_lab.py`: `a9d4edd6bb937587852a8f726d09b6729fbd9648b559ceb122246462a7c9b3ed`
- `test_region_runtime.py`: `73332bee6f0b9899c8e639760bd4d865069a98d2f7c81c1022a6ede572f83e16`
- `verify_lab_evidence.py`: `a054460aaa1ddbd0a9ff3f55ee71dd32ab11510845cfbbfc6ddf565c749d49a2`
- `evidence/2026-11-25-networked-region-lab.json`: `dba1686b224be349c023bbbcae61935fe6f45ed0d958a900eebd297d3c48d8e0`

This is a local reproducibility receipt, not a clean release build
attestation or remote independent qualification.
