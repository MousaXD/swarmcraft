# RFC 0001 — Horizontal Minecraft simulation research

**Issue:** [#86](https://github.com/MousaXD/swarmcraft/issues/86) · **Date:** 2026-11-08 · **Agent:** 06 · **State:** research-only final recommendation

## Executive decision

**NO-GO** for implementing general-purpose, active multi-machine Minecraft Java/Fabric simulation within SwarmCraft's current architecture and ordinary Fabric mod-compatibility constraints. This is **not** a proof that distributed Minecraft simulation is impossible. It is a decision that the demonstrated safety, compatibility and performance prerequisites for this product **do not exist yet**. The proposed future custom-runtime experiment remains **CONDITIONAL**, requiring a new scope and explicit approval.

SwarmCraft presently runs **one** authoritative Minecraft JVM at a time, while peers can hold snapshots/replicas and safely take over as successors. Replication and recovery increase availability, **not** the live computation performed by an overloaded host. The 4 GiB that Alice elects to make available plus the 3 GiB that Bob elects to make available **do not create one 7 GiB Minecraft JVM heap**. Distinct JVMs require distinct heaps, process/native headroom, synchronization and world ownership.

Included are a limited 10-test Python *in-memory model* and single-machine CPU/serialization proxy. These are repeatable **research evidence**, not real Minecraft or WAN end-to-end tests, not a quorum implementation, and not sufficient to establish a horizontally scaled product. The GO gates in #86 have **not** been met; this NO-GO conclusion can nonetheless complete the requested feasibility research.

## 1. Actual SwarmCraft architecture examined

Inspected GitHub `main` at `45174310d1433116e64b9b6978cb4f2aa1c22c15`:

- [ROADMAP.md — Phase 12](../../ROADMAP.md): distributed simulation explicitly designated **future research**; single-authority snapshot and recovery functionality is a different phase.
- [Implementation status](../../docs/IMPLEMENTATION_STATUS.md): current signed identity, verified snapshots, quorum-backed recovery/elections, fencing, Fabric lifecycle, runtime launch and migration status; its product limitations must be preserved.
- [SwarmCraftMod.java](../../minecraft/fabric/src/main/java/dev/swarmcraft/fabric/SwarmCraftMod.java): SERVER_STARTED/STOPPING events, authenticated loopback IPC, safe save/stop barriers dispatched via `server.execute`, and permit watchdog. **No distributed tick scheduler, chunk transfer protocol, region journal or multi-writer game state.**
- [Fabric build coordinates](../../minecraft/fabric/gradle.properties): Minecraft 26.1.2 / Fabric Loader 0.19.3 / Fabric API 0.155.2+26.1.2. `org.gradle.jvmargs=-Xmx2G` is a **Gradle tooling** property, not a Minecraft server resource donation.
- [Authority permit](../../crates/swarm-cli/src/authority_permit.rs): world-authority heartbeat and generation fencing for a hosted runtime; not individual regional host ownership.
- [Recovery rules](../../crates/swarm-consensus/src/recovery.rs): quorum certificates and durable ballot controls for canonical recovery; no cross-region tick transaction semantics.

**Inspection limitation:** GitHub source review, not a source-built/loaded Fabric Minecraft server. No native Minecraft performance trace, mod-compatibility matrix, two-machine gameplay or three-machine partition test was produced. Production code and CI remain untouched.

## 2. Verified external architectural precedents

| Source | Feasibility lesson | Limit |
|---|---|---|
| [Folia project](https://papermc.io/software/folia/), [Folia region architecture](https://docs.papermc.io/folia/reference/overview/) and [README](https://github.com/PaperMC/Folia) | A deeply modified Paper server ticks independent regions concurrently on **threads**, with region buffers/merge rules, per-region data and complex cross-region scheduler/teleport handling. | Same machine, **not** multiple collaborating Fabric JVMs; ordinary plugin semantics require modification. |
| [Folia FAQ](https://docs.papermc.io/folia/faq/) | Performance depends on spatial spread and core/worker balance; some global commands are disabled or restricted. | No general mod compatibility or automatic speedup for crowded hotspots. |
| [MultiPaper source](https://github.com/MultiPaper/MultiPaper) and [architecture description](https://multipaper.io/multipaper/how-it-works.html) | Multi-server, same-world chunk ownership with an external master, synchronized chunk caches, peer messages and separate server processes is plausible with a **modified Paper-derived** runtime. | Different ownership, storage, compatibility, security and coordinator assumptions from SwarmCraft. |
| [MultiPaper caveats](https://multipaper.io/) | Its maintainers warn about duplication/corrupted chunk risks and plugin incompatibilities. | Does not substantiate corruption-free horizontal Fabric gameplay. |
| [Fabric events](https://docs.fabricmc.net/develop/events/) and [ServerTickEvents API](https://maven.fabricmc.net/docs/fabric-api-0.155.2%2B26.2/net/fabricmc/fabric/api/event/lifecycle/v1/ServerTickEvents.html) | Events expose start/end server/world tick callbacks. | Callbacks do not repartition vanilla Minecraft's global scheduler, entity ownership or persistent state. |
| [Minecraft tick reference](https://minecraft.wiki/w/Tick) | Nominal 20 TPS gives 50 ms per tick. | Available processing time is **less** after queuing, network waits, serialization and GC. |

**Compatibility conclusion:** A thin Fabric lifecycle extension is insufficient. The appropriate experimental foundation would be a custom/forked server with explicit region ownership, global-state ownership and mod isolation or compatibility policy. Merely adapting Fabric event hooks will not satisfy atomic interactions and persistence.

## 3. Architecture options and recommendation

| Option | Active simulation scaling? | Gameplay compatibility / safety | Decision |
|---|---|---|---|
| A. Current single Minecraft server + snapshot replicas and quorum successor | No | Current supported model, fail-closed canonical recovery | **Retain default** |
| B. Add volunteer CPU/RAM admission for successor/failover | No | Must consent and reserve RAM; separate issue #84 | Valuable, **not horizontal simulation** |
| C. Folia-style region threads **within one** modified host | Possibly | Thread-affinity, global scheduling, mod incompatibility | Separate custom-runtime study |
| D. MultiPaper-style region/chunk workers **on different** computers | Possibly on spatially distributed loads | Requires fork, fencing, durable cross-region event log, server/client routing and WAN fault tolerance | **NO-GO** under existing product constraints |
| E. Distinct independently hosted worlds behind proxy | Yes, across separate worlds | Not one unified ordinary Minecraft world | Different product, not an answer to #86 |

### 3.1 Hypothetical future design (NOT implemented)

```text
                   +----------------------------------+
                   | Stable client connection gateway |
                   | signed world/region epoch routing|
                   +-----------------+----------------+
                                     |
                    +----------------+-----------------+
                    | Global single-writer state       |
                    | time / weather / game rules /    |
                    | world border / scoreboards /     |
                    | global commands / dimension data|
                    +----------------+-----------------+
                                     |
          Quorum-certified grants / fenced commit certificates
                                     |
             +-----------------------+-----------------------+
             |                       |                       |
       Alice: region A         Bob: region B        Charlie: region C
       tick + journal          tick + journal       tick + journal
             |                       |                       |
             +----------- cross-region events ---------------+
                                     |
                       Durable commit + state roots
                          quorum-backed recovery
```

All pictured gateways, bus, journal format, regional leases, atomic boundary transactions, global sequencer and canonical commit service are **missing new infrastructure**. The diagram is a research sketch, not an implementation contract.

### 3.2 Ownership and fencing invariant

- Partition identity must include world ID, dimension, region/chunk range and map version. A grant binds region, world generation, monotonic region epoch/token, owner, membership/quorum certificate and committed state root.
- **At most one canonical accepted writer** for each region/generation. An old disconnected worker could run locally until a watchdog stops it, but storage and client routing must reject **all stale-generation output**. Rejecting writes at the controller alone is insufficient if clients continue talking to an old server.
- Durable quorum votes must be authenticated, protected against replay and tied to reconfiguration. Every accepted grant must exclude old grants via a real persistent consensus/reconfiguration algorithm. A simple timeout or local vote tally is **not** a distributed proof of uniqueness; wall-clock leases need bounded uncertainty and fencing.
- Region split/merge and ownership migration require an atomic base root and cutoff tick. Uncommitted work is never published as canonical. Persistently ordered journals/checkpoints and a recovery algorithm must reconstitute a **single world-wide consistent cut** across all regions.
- If the member majority or required global owner becomes unavailable, affected causal work must **freeze**. There is no safe implicit “solo distributed” continuation or silent conflict merge.
- Cross-region entity migration needs a unique transfer ID and a prepared source state, reserved destination, single certified commit and idempotent recovery. Exactly one **canonical** owner/location after a crash is mandatory. All external side effects must have equivalent safeguards.

**Critical model limitation:** `model.py` centralizes the current grant in one idealized oracle and moves entities atomically in one memory space. It purposely does **not** implement distributed voting, durable disk writes, clocks, signed tokens, or network partitions; its passing tests cannot validate the above contract.

## 4. Interaction/compatibility threat inventory

| Gameplay feature | Non-negotiable correctness condition | Design implication |
|---|---|---|
| Walking players, mounts, vehicles, dimension portals, teleport and respawn | Atomic identity, connection, inventory and entity-owner handoff | Coordinated freeze/prepare/commit; routing and recovery |
| Mobs, AI, projectiles, falling blocks | No duplicate tick, movement or damage across region boundary | Buffered neighbor influence, causal queues, deterministic conflict resolution |
| Redstone, pistons, observers and scheduled block ticks | Update and dependency order preserved | Merge neighboring active regions or use costly cross-region transactions |
| Fluids, fire, explosions, light propagation | Correct propagation and cross-chunk writes | Abort/freeze on unavailable neighbor; serializable update model |
| Structure generation, chunk loading and saving | One generator/owner and canonical write set across chunks | Generation reservations, persistent journal and atomic saves |
| Hoppers, chests, container transfer and inventories | Exactly-once mutation; no dupe/drop | Transactional inventory transfer, idempotent retries |
| Time, weather, rules, maps, scoreboard, bossbars and advancements | Global consistent ordering/reads | Central or quorum-replicated global sequencer; contention |
| Commands, functions, datapacks, console and server events | Explicit global-vs-region task ownership | API redesign and compatibility restrictions |
| Fabric mixins, mod static state, registries, custom dimensions | Audited region/thread safety and side-effect determinism | Unknown mods **ineligible by default**, explicit allowlist |

An arbitrary Fabric server mod may assume it can read or mutate any world state during the main tick. Regionized execution and cross-host state would break this assumption. Unsupported mods must stay on the normal single-authority mode. “Works on Folia/Paper” is not proof of compatibility with Fabric, a specific Minecraft version, or SwarmCraft.

## 5. Failure semantics and proof gaps

| Fault | Required safe response | Evidence today |
|---|---|---|
| Regional owner hard kill | Freeze uncommitted events; resume last certified root with new epoch only after quorum | **Not implemented/tested** |
| Slow tick, GC pause or missing worker | Apply bounded backpressure; freeze affected interactions | **Not implemented/tested** |
| One-of-three/minority partition | Cannot certify new canonical grant or speculative writes | Central model rejects claimed minority; **not real consensus** |
| Two simultaneous claims | At most one accepted generation; stale messages rejected everywhere | Central model serializes grants; **no distributed tests** |
| Crash during entity transfer | Recover source or destination exactly once, with durable prepare/commit | In-memory model tests commit, abort, idempotence only |
| Stale returning host | Storage, gateway and all workers reject old fencing token | Model only verifies local token |
| Loss of global sequencer | Stop dependent updates until quorum-certified recovery | **Not implemented** |
| Mod accesses foreign chunk/global mutable data | Detect and fail closed, not corrupt shared world | **Not implemented** |
| Disk/partition mismatch or mixed snapshot epochs | Never publish as a valid checkpoint; preserve prior canonical world | **Not implemented** |

Trust and abuse assumptions are hard: volunteers' computers may disconnect, misreport resources, run modified JVMs or inspect world data. Signed identity is not a trusted computation proof. Anti-cheat, privacy, source validation and permission to execute third-party mods on remote machines require separate designs.

## 6. Performance and resource model

**Work ceiling:** with per-region work `W_i`, worker count `p`, and coordination overhead `C`, an optimistic processing bound is `max(sum(W_i)/p, max(W_i)) + C`. A dominant base/redstone farm limits speedup regardless of idle workers. Real regions can merge or trigger global serial work, increasing `C`.

**Heap:** multiple JVMs have separate heaps, native allocations, GC, caches, chunk/POI data, IPC and network buffers. `-Xmx` is a **heap** ceiling, not total resident-memory or device-RAM reservation. Each volunteer must opt in, may choose **0**, and needs OS/player/client/Java/native headroom and admission/revocation; these safeguards belong to separate hosting-budget work.

**Tick/latency:** 20 TPS implies 50 ms available per nominal tick. A design requiring one blocking cross-host RTT of `R` needs at least `R` for that barrier alone; two serial round trips at an assumed 25 ms consume 50 ms *before* server computation. At an assumed 60 ms, even one serial round trip exceeds the budget. This is a **sensitivity calculation**, not a measured link, and not all possible designs require a per-tick barrier. Pipelining/optimism changes conflict resolution and player-state guarantees.

**Bandwidth:** each actively interested worker needs entity/chunk/block state, hot-chunk subscriptions, ticks, retry data, and replicated durable history. Serialization, compression, relay paths, NAT and asymmetric consumer uplinks add variable cost. No real bandwidth, TPS, MSPT, CPU/RSS, end-to-end reconnect or correctness data was collected here.

## 7. Research-only prototype and measured proxy

All code is isolated under this directory and uses Python standard library.

```bash
cd research/distributed-simulation
python3 -m unittest discover -s . -p 'test_*.py' -v
python3 benchmark.py --entities 12000 --regions 12 --rounds 64 --repeats 5 --workers 1,2,4 --output evidence/local.json
```

- `model.py`: idealized central grant/fence and atomic transfer state machine; **not** actual distributed consensus or Minecraft.
- `test_model.py`: **10 passing deterministic tests**, including duplicate minority votes, stale generations, unexpected member, transfer collision, commit/abort, portal region migration and idempotence.
- `benchmark.py`: compares real measured local CPU wall-clock for synthetic arithmetic jobs in one Python process versus two/four local worker processes; JSON encode/decode proxy for assumed boundary messages. Workers run **on one machine**, not over a network, and process startup is excluded from multi-worker timings.
- `evidence/2026-11-08-synthetic-local.json`: full observed samples, checksums, CPU quota and sensitivity assumptions.

**Observed median local compute proxy, 5 repetitions, milliseconds:**

| Assumed load shape | 1 local process | 2 local processes | 4 local processes | Four-versus-one speedup |
|---|---:|---:|---:|---:|
| Spread equally over 12 regions | 74.097 | 40.278 | 27.948 | 2.65× |
| Clustered (85% of synthetic work in one region) | 73.770 | 66.960 | 65.278 | 1.13× |
| Spread with 10% **assumed** boundary events | 70.063 | 38.746 | 25.347 | 2.76× **before communication** |

Environment: Linux x86_64, Python 3.13.5, 5 CPU affinity, cgroup CPU quota 4; 12,000 synthetic arithmetic work items (called “entities” in the CLI but **not Minecraft entities**), 64 arithmetic rounds each. Same checksums across worker counts. This only demonstrates a normal parallel-computing constraint: spatially balanced workloads are easier to distribute than hotspots.

Assumed boundary case: 1,200 **invented-format** JSON events encoded/decoded locally, 200,400 bytes total, 6.93 ms median local CPU; no socket transfer or real Minecraft packet. Assumed RTTs of 5, 25 and 60 ms appear under `network_rtt_budget_sensitivity_NOT_MEASURED` in the JSON and are **not network observations**. Device scheduling, workers and repeat count influence measured medians.

**Interpretation:** these numbers do not demonstrate Minecraft simulation speedup, real regional ownership, Fabric mod correctness, or safe gameplay. A real single-server-vs-multiple-remote-servers benchmark, with verified same world/mods/players and failure tests, remains **absent**. Thus the quantitative GO gate is unsatisfied; performance improvement is a hypothesis only.

## 8. Gates for reconsidering decision

1. **Choose and own a custom server runtime** with concrete region/chunk ownership APIs, global ordering and mod compatibility policy. Do not promise support for arbitrary Fabric mods.
2. **Prove multi-node safety**: authenticated durable grants/reconfigurations, quorum intersection, monotonic write fencing, canonical state roots, crash-consistent cross-region transactions; malicious/late/reordered/dropped messages and recovery tested.
3. **Gameplay semantics:** tests for player/inventory transfer, portals, projectiles, redstone, fluids, TNT/explosions, generation and saves, global commands/datapacks, plus explicit unsupported mod classes. No dupes, lost entity, mixed world checkpoint or double canonical writer detected in the fault campaign.
4. **Real benchmark:** same Minecraft server version/world/mod envelope, single-authority baseline, 2/3 independent active hosts; realistic spread, hot-spawn/clustered and boundary-heavy workloads; record p50/p95/p99 MSPT, achieved TPS, sustained players/entities, deadline misses, CPU, heap/RSS/GC, network bytes/jitter/relay and canonical save hashes.
5. **Host-loss and WAN campaign:** hard kill, simultaneous grants, partial partitions, recovering old host, skew/GC pauses, cross-boundary transfers mid-crash, 2/3 loss, NAT/relay and asymmetric uplinks. Independent review and retained recordings/logs.
6. **Proposed quantitative GO threshold, subject to Agent 00 approval:** at least 25% more *sustained* active-player/entity throughput on **two** spread-load scenarios at 20 TPS, p95 MSPT < 50 ms, within each volunteer's memory/CPU limit; report regressions of clustered/boundary cases. **Zero detected canonical corruption/dupe/loss** is mandatory; “<1% failure” is unacceptable.
7. **Fail-closed production fallback:** retain current single-authority replica/failover and ordinary nonhosting players; do not overwrite conflict history or silently enter distributed mode.

None of those GO gates is validated by this Python prototype.

## 9. Stages, risks, and decision for Agent 00

**R0 (this PR):** research, source precedent, threat/consistency inventory, synthetic reproducible model and measurements, explicit NO-GO. **Research complete.**

**R1 (requires separate approval):** custom-runtime-only fork, constrained mods, instrumented intra-process region simulation; stop if world parity fails.

**R2 (requires separate approval):** durable multi-machine region grants and boundary transactions in a lab, still separate from SwarmCraft release code; inject partitions and crashes.

**R3 (requires separate approval):** three real machines/ISPs, Minecraft gameplay, state-root and fault certification, quantitative GO thresholds before any integration.

**R4 (only after R3):** opt-in narrow experimental runtime and conservative single-server fallback; never turn off safety gates for speed.

**Risk register:** Critical — item duping/vanishing, world corruption, stale accepted writers, cross-region state/root divergence. High — Fabric/mod incompatibility, global state bottleneck, WAN jitter/relay, unknown peer computation, repartition races, disconnect/portal continuity, recovery correctness. Medium-high — JVM RSS/GC admission, bandwidth and NAT, observability, testing matrix, security/privacy of voluntary peers. **None is closed by these experiments.**

**To Agent 00:**
- **Research verdict: READY** for independent review/merge as a **research-only negative feasibility result**.
- **Feature verdict: BLOCKED / NO-GO** for general-purpose multi-host active simulation on current Minecraft Java/Fabric SwarmCraft.
- **Issue #86:** may be marked complete **as feasibility research**, after reviewing the RFC and evidence. This does *not* mean any active scaling feature shipped.
- **Recommendation:** continue robust single-authority failover and voluntary successor resource budgets (#84) and actual three-machine player-journey certification (#85). Do not advertise pooled Minecraft RAM or delivered horizontal world simulation.
