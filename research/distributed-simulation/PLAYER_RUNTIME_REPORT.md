# #59 — Continuous reference player world

Date: 25 November 2026 UTC / 26 November UAE time.
**Locally verified research only; production Minecraft compatibility and
independent physical-host gameplay remain unverified.** The existing
single-authority Fabric implementation and the #56 production NO-GO
are unchanged.

## Runtime and protocol choice

Extend the #56 deterministic ring model instead of introducing a new
Minecraft server fork or changing real Minecraft save formats. The
reference ring is a fixed 1,024-cell world split into eight 128-cell
regions; workers execute four regions each over private authenticated
framed TCP. The player-facing coordinator owns the ephemeral session
and fixed roster, not production identity or regional authority.

An authenticated per-client lab channel supplies the player identity.
A client joins with its exact world ID and protocol version and can
submit one bounded direction (-1, 0, +1) for the next tick; missing
input holds position. Inputs cannot supply another player ID. Each
input is dispatched only to the region currently containing that
player. Workers validate exact region, resident identity, and unique
input before staging, while existing background entities, signal and
resource mechanics continue. Cross-region transfer uses the existing
prepare/seal/commit protocol; the coordinator publishes only after all
workers acknowledge and a complete checked snapshot agrees with the
cut. The player view covers five contiguous cells, including across
a worker boundary and the ring seam, with identity, tick, position,
inventory and world-bound cut digest. The digest also binds the fixed
roster and ruleset.

This is a deliberately small, scripted, synchronous reference ruleset.
The player protocol is a new private lab protocol, not the Minecraft
wire protocol or SwarmCraft's production wire protocol. No user-world
import, production daemon/desktop integration, remote enrollment,
durable cut, signed regional grant, migration or consensus is provided.
Failure freezes publication; recovery or resumption is not authorized.

## Compatibility matrix

| Runtime / client / content | #59 result |
| --- | --- |
| Python standard-library ring runtime, private lab player protocol 1, reference client processes | Verified below on one Linux workstation |
| Reference ring movement, bounded neighbor view, signals, resource pickup/drop and cross-region transfer | Verified against a separate serial traversal and independent view projection |
| Existing Minecraft Java 26.1.2 / Fabric Loader 0.19.3 / Java 25 baseline | Existing single-authority adapter, not tested for #59; no active regional support |
| Fabric API 0.155.2+26.1.2, existing bridge, arbitrary Fabric mods | No distributed compatibility established |
| Vanilla Minecraft client wire protocol, chunks, NBT saves, crafting, portals, redstone, commands, plugins and modded rules | Not implemented by reference runtime; no compatible game client |
| Folia, MultiPaper and other forked server protocols | Not selected or certified; see #56 and RFC 0001 for the earlier assessment |

The actual supported production adapter range is defined separately
in the repository runtime support contract. This table does not
expand it or imply that an untested tuple works.

## Positive evidence

- [Run 1 raw receipt](evidence/2026-11-26-player-1.json)
- [Run 2 raw receipt](evidence/2026-11-26-player-2.json)
- [Live worker fault receipts](evidence/2026-11-26-player-faults.json)

Two real worker processes per run were pinned to distinct physical
cores, in separate private network namespaces; the coordinator and
three scripted client processes used an isolated coordinator
namespace and loopback client sockets. Two clients each submitted
144 inputs over the real authenticated TCP channel. The third tried
to spoof another identity and was disconnected before the first tick;
its stationary character remained in the simulation. Four subsequent
ticks confirmed that the world continues with disconnected players
held in place. The world has 256 agents, including the three player
identities. The clients cross the 3/4 and 7/0 worker boundaries
forward, reverse, then forward and continue into another region.
The verifier independently replays every accepted tick using the
monolithic reference, projects the five-cell view independently from
flattened world arrays, and checks the full final state, actual
client-observed receipts and crossing counters.

| Measurement | Run 1 | Run 2 |
| --- | ---: | ---: |
| Accepted ticks | 148 | 148 |
| Client-observed views (including joins) | 291 | 291 |
| Player transitions between workers | 6 | 6 |
| Player transitions between regions | 8 | 8 |
| Paced tick-loop p50, ms | 3.237 | 3.260 |
| Paced tick-loop p95, ms | 4.569 | 4.224 |
| Paced tick-loop p99, ms | 7.267 | 5.424 |
| Paced tick-loop max, ms | 9.043 | 7.871 |
| 50 ms schedule misses | 0 | 0 |

The loop measurement includes input collection, worker barrier, full
snapshot, view construction and publication; it excludes scheduled
sleep and offline receipt replay. These are two short (about 7.4 s
each) paced 20 Hz experiments on a small reference fixture, not a
sustainable throughput measurement, benchmark of Minecraft or
one-worker/two-worker speedup comparison. Physical independence,
WAN behavior and an actual player UI were not tested.

## Refusal and fault evidence

A separate real client received the join view and one accepted
cross-worker movement before each fault: SIGKILL of the destination
worker after prepare, and private veth link-down after seal. Both
faults preserved the player-facing tick 1/root, froze view, input,
join and advance operations, and closed the client connection without
publishing tick 2. All scoped worker processes exited and lab links
were removed. These are deliberately not durable recovery tests;
workers may have tentatively advanced locally.

The unit suite additionally exercises stale, future, invalid and
duplicate input, protocol/world mismatch, wrong-region control,
frozen snapshots and world/roster digest binding. The receipt
verifier rejects altered views, identities, roots, timing and fault
evidence. Neither the verifier nor the trusted lab coordinator is
a Byzantine security proof.

Reproduce on Linux with available mapped user/network namespaces
and five distinct physical cores:

```sh
cd research/distributed-simulation
python3 -m unittest discover -s . -p 'test_*.py' -v
unshare --user --map-root-user --net python3 player_lab.py --output /tmp/new-sc59.json
unshare --user --map-root-user --net python3 player_lab.py --faults --output /tmp/new-sc59-faults.json
python3 verify_player_evidence.py /tmp/new-sc59.json
python3 verify_player_evidence.py /tmp/new-sc59-faults.json
```

## Decision and next gate

Keep the reference runtime isolated. #59 is verified only for this
custom ruleset and protocol. Actual independent-machine gameplay,
Minecraft client/mod compatibility, production worker admission,
certified regional authority and durable transactional recovery
remain open. Next #60: define a single certified region writer and
generation, global-state ownership and consistent accepted cuts;
continue research without enabling production distribution.
