# SwarmCraft expansion RFC roadmap

Companion long-term R&D roadmap · 23 November 2026, UAE time  
Baseline · SwarmCraft 0.5.1 technical preview on `main` (`aaffac22e32547983f1fd558af6eb32c0da4917b`)  
Primary execution plan · [SABATHON_IMPLEMENTATION_PLAN.md](./SABATHON_IMPLEMENTATION_PLAN.md)

## Scope boundary

This document holds the **long-term expansion tracks (backlog items 56–68)** separated out of [SABATHON_IMPLEMENTATION_PLAN.md](./SABATHON_IMPLEMENTATION_PLAN.md) so the private-world sabathon, preview, and v1 production release gates stay focused and measurable.

Neither track below is part of the private-world sabathon demo or initial production release. In particular, the repository’s feasibility study ([research/distributed-simulation/RFC.md](../research/distributed-simulation/RFC.md)) issued a formal **NO-GO** for general-purpose distributed Fabric simulation on the current single-JVM architecture. Treat both tracks as distinct post-v1 architecture programs with their own RFCs, threat models, and quantitative GO gates.

## Backlog (Items 56–68)

Item numbers continue directly from [SABATHON_IMPLEMENTATION_PLAN.md](./SABATHON_IMPLEMENTATION_PLAN.md) (items 1–55) to preserve cross-references.

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 56 | Expansion · Design prerequisite | Reopen the distributed region/tick feasibility decision for the expanded product scope (revisiting the current Fabric NO-GO). Choose the experimental runtime before implementation | Compare the existing single-host baseline with actual multi-host simulation. Document cross-region correctness, compatibility limits, and measured performance before adopting the runtime |
| 57 | Expansion · Missing | Add per-device CPU and memory contribution, with desired limits, measured capacity, and a player-only choice | Joining grants no compute access automatically. A player can select 4 GiB and a CPU limit, change or withdraw it, and see requested, admitted, and active resources. Zero preserves play where available |
| 58 | Expansion · Missing | Build opt-in worker admission and placement based on usable memory, CPU, network path, and region load | A newly admitted machine takes real simulation work only when useful. Inadequate capacity scales down safely. Platform limits and heap versus process memory are visible |
| 59 | Expansion · Missing | Choose or build a region-aware runtime. Keep one world identity as different machines simulate different regions | Players traverse a continuous world while independent machines tick different regions. The supported Minecraft/protocol/mod matrix is documented and verified |
| 60 | Expansion · Missing | Bind every region to a single authorized writer and generation. Define the ownership of global game state | Region reassignment, overlap, and worker restart never produce two canonical writers for the same region. World-wide saves include a consistent accepted state |
| 61 | Expansion · Missing | Make player, entity, and inventory transitions durable across regions, including block and global interactions in the supported ruleset | Players walk or use a supported portal across workers and preserve identity, state, and inventory through restart. Supported cross-region mechanics pass replay and correctness tests |
| 62 | Expansion · Missing | Handle scale-in, loss, and partition. Drain and reassign live work when volunteers leave | A graceful exit drains eligible work and releases the device budget. A crash freezes or fails over affected regions from accepted checkpoints without duplicating state |
| 63 | Expansion · Unverified | Run real distributed gameplay and publish the scale envelope | Compare the same world, load, and network with one worker and multiple independent workers. Show increased sustainable active regions or player capacity at the published service target. Publish clustered-load and degraded-network limits |
| 64 | Expansion · Missing | Add a creation-time **Ownerless / no OP** world option with a signed governance policy | The founding player receives no permanent creator privileges, player OP, unilateral rule changes, or unilateral control over recovery. Turning on more compute grants no governance privilege |
| 65 | Expansion · Missing | Bind enforced permission rules to the canonical runtime and each regional worker, with explicit CFT vs. BFT trust boundaries | On unmodified runtimes, the founder and successor hosts have ordinary in-game permissions, and `/op`, RCON, and privileged management interfaces are disabled. Documentation explicitly separates software-enforced governance from OS-level host tampering |
| 66 | Expansion · Missing | Define shared authority, validator admission, and loss recovery for ownerless worlds | A verified majority can keep the world available after the founder leaves permanently. No single key can publish a unilateral policy change or rewrite accepted state. A lost majority leaves verified recovery copies |
| 67 | Expansion · Missing | Separate world governance, public/private participation, and volunteer worker trust | Ownerless can be invite-only or public. Joining a public world grants neither a vote nor write authority. Anonymous game access and eligibility to run or certify workers are separate policies |
| 68 | Expansion · Unverified | Certify ownerless and horizontally scaled operation together | In one real game session, the founder exits permanently, other peers keep the same world available with no player OP, opt-in worker resources change, and players continue across reassigned regions when the required quorum and capacity survive |

## Horizontal simulation using volunteered resources

The product promise is **one playable world that admits additional simulation capacity while people join and volunteer resources**. If Bob offers 4 GiB and up to two CPU cores, his device can simulate a suitable region while Alice's device simulates another. Each worker runs within its own admitted budget. Allocation responds to useful additional capacity, actual load, and network quality. A player's joining alone grants no resources, and two machines keep separate memory spaces. The scale target is more active regions or players at an agreed performance level. An overloaded single area can remain a bottleneck.

The current SwarmCraft runtime has one authoritative Minecraft process with warm replicas and successor recovery. The repository's feasibility study ([research/distributed-simulation/RFC.md](../research/distributed-simulation/RFC.md)) found general-purpose distributed simulation **NO-GO** for the current Fabric architecture and contains no real distributed Minecraft benchmark. The [#56 reopened networked reference lab](../research/distributed-simulation/REGION_LAB_REPORT.md)
now measures actual discrete region simulation, TCP/virtual-node latency,
cross-region correctness and freeze on worker loss; it does **not** run
Minecraft, implement durable regional quorum commits or demonstrate
physical independent hosts. The Fabric NO-GO remains and no runtime
has been adopted for production. Treat this as a new runtime track.
The [#58 capped placement experiment](../research/distributed-simulation/WORKER_PLACEMENT_REPORT.md)
adds reference-only explicit consent, measured capacity/RTT, useful-work
selection and clean scale-in. It is not production worker admission.
The production #57 [resource preference](DEVICE_RESOURCE_CONTRIBUTION.md)
does not enroll regional workers. No distributed engine or remote authority
grant is enabled.
Region scheduling work such as [Folia](https://docs.papermc.io/folia/reference/overview/) is a useful architectural reference for ownership and boundaries, but multi-machine SwarmCraft still requires its own network, consistency, and recovery design.

1. **Choose a supported first runtime and workload.** Start with a narrow Minecraft version, documented mechanics, and approved compatibility set. Run real clients, two physically separate worker machines, and a separate control/voting failure domain where needed. Preserve the existing single-host world mode and its saves.
2. **Make resource consent enforceable.** Ask for a memory and CPU limit per device and per world. Show current admission, reserve client and OS headroom, enforce tested CPU and memory boundaries where supported, and display whether the number is a Java heap target or a tested whole-process limit. Support zero, live reductions, and withdrawal. Never make a joining player install or run unapproved server workloads.
3. **Allocate by region and keep one canonical state.** Lease regions with fenced generations, and define a deterministic owner or ordered submission path for time, rules, player data, and other global effects. Use a stable Minecraft entry point. When an admitted worker leaves, drain work, checkpoint safely, and reassign only after the old grant is fenced. Stop affected updates when the remaining authority or resources are insufficient.
4. **Certify gameplay across boundaries.** Cover walking/portals, player identity, inventory, autosave, crash mid-transfer, reconnect, and stale worker return. For each supported mechanic, test ordering and safe recovery across boundaries. Include redstone, entities, fluids, and cross-region updates in the declared support matrix. Keep unsupported mods and mechanics out of the compatibility claim.
5. **Prove useful horizontal capacity.** Before measuring, define hardware, workload, network conditions, tick-time targets, and accepted data-loss/recovery bounds. Compare one active worker against two or more on geographically spread and concentrated loads. Record authoritative work placement, player capacity or active region count, tick latency, bandwidth, and measured memory/CPU. Also show the player experience when a worker reduces a budget or powers off.

**Call horizontal scaling done when** the same world is played through independent active machines, a new 4 GiB and CPU contribution takes observable game work, supported players cross regions with state intact, healthy comparable workloads sustain more than with one active worker, and removing that worker produces an observed safe drain, failover, or clear blocked state.

## Ownerless, no-OP world option

The isolated, non-activatable #64 creation contract is documented in [Ownerless creation](rfcs/ownerless-creation.md). The [#65 permission lab](rfcs/ownerless-permissions.md) tests actual Minecraft/Fabric permissions and policy-pinned reference workers. The [#66 CFT reference authority](rfcs/ownerless-authority.md) adds signed quorum history, durable local vote locks, joint roster changes and verified recovery copies. The [#67 participation contract](rfcs/ownerless-participation.md) adds distinct signed public/private reference identities, independent role eligibility and durable one-use proofs. The [network observation follow-up](rfcs/ownerless-network-observation.md) adds bounded signed TCP observations of an independently pinned head, with local partition/recovery evidence. The [encrypted decision exchange](rfcs/ownerless-network-decision.md) transports independently reviewed proposals and durable validator votes over mutually authenticated Noise channels. Neither is a freshness lease or live canonical authority. Certificate dissemination/reconciliation, canonical Minecraft authority, live admission, and independent-host gameplay gates remain open.

Keep **Ownerless / no OP** as a creation-time option. “2b2t-inspired” describes a persistent world and its gameplay philosophy. This SwarmCraft mode adds the explicit requirement that no player, including the founder and current host, has operator powers. Joining rules are a separate choice, so the world can be public or invite-only. Publish the exact limits of any gameplay, spam, or infrastructure policies at creation.

### Explicit threat-model boundary (CFT governance vs. BFT execution)

Be completely honest about what consumer volunteer hardware can and cannot enforce:

- **What this mode CAN enforce (governance ownerlessness / CFT):**
  - No permanent founder/admin key in the signed world policy; membership, validator set, and allowed policy changes require quorum rules.
  - Unmodified SwarmCraft runtimes disable player OP, RCON, command blocks (if policy-restricted), and privileged console paths, and enforce matching signed rules across hosts and workers.
  - Peers reject snapshots, epochs, or policy transitions that violate signed quorum or compatibility rules.
- **What this mode CANNOT enforce on untrusted consumer PCs (Byzantine / anti-tamper execution):**
  - Whoever controls the OS on the active host controls JVM memory, local disk, and Java bytecode. A malicious host with root/local access can edit `level.dat`, attach a JVM agent, or run a modified binary before signing a snapshot — because SwarmCraft snapshot replication verifies **data integrity and authority signatures**, not **rules-compliant Minecraft tick execution**.
  - Do not claim cryptographic cheat-prevention against a malicious host unless a separate design introduces attested/trusted validator sets, deterministic re-execution, or explicit state-transition proofs.

### Delivery requirements

1. **Bind the rules to the world identity.** Sign the creation policy and bind accepted runtimes, worker grants, and managed `server.properties` / game permission enforcement to it. Remove built-in founder privileges and disable player OP and privileged console/RCON interfaces in this mode. Decide which rule invariants are fixed, which published changes require a quorum, and when a policy change requires a new world identity.
2. **Separate everyday play from maintenance authority.** Joining, playing, seeding data, running simulation, and certifying the next generation are separate roles. Donation size buys no governance privileges. Use an explicit validator enrollment model with a known failure and trust assumption. Recover from accepted checkpoints without the founder's key while enough accepted replicas and votes survive.
3. **Define and publish volunteer-host trust boundaries.** Specify which machines are eligible to execute authoritative work, what peers independently verify (signed snapshots, epochs, quorum certificates, policy hashes), and how the world responds when validator trust or quorum is lost. Keep public game access strictly separate from admission to authoritative execution.
4. **Prove there is no privileged founder or player.** Observe a normal founder session, normal volunteer-host session, founder departures and restarts, replacement of accepted peers under the published rules, and continued play on the same world. Attempts to grant ordinary players operator rights or unilaterally change protected rules through supported interfaces fail and leave state unchanged. A quorum outage produces the documented safe outcome.

**Call the ownerless mode done when** regular players, the creator, and active hosts all have the advertised game permissions on unmodified runtimes, the founder can permanently leave while the remaining quorum keeps the same world operating, a single participant cannot unilaterally change protected rules, and the CFT governance vs. OS-level host-trust boundaries are evidenced and published.

## Expanded release finish line

The original private-world sabathon demo and v1 production gates remain the primary milestones in [SABATHON_IMPLEMENTATION_PLAN.md](./SABATHON_IMPLEMENTATION_PLAN.md). The **full expanded SwarmCraft vision** additionally requires both completion tests above, plus one combined game session where the founder leaves permanently and an active worker is added or removed while the same world continues under the supported quorum and resource conditions. Broader editions, loaders, and optional research retain separate scope and acceptance criteria.
