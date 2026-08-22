# SwarmCraft Roadmap

This roadmap describes the order in which SwarmCraft's architecture and product should mature. It is **not** a claim that implementation landed in perfectly linear phase order.

For the exact current-code assessment, use [docs/IMPLEMENTATION_STATUS.md](docs/IMPLEMENTATION_STATUS.md).

## Current 0.2.1 position

| Phase | Status | Summary |
| --- | --- | --- |
| 0 — Research and protocol skeleton | ✅ Preview-complete | Identity, protocol, signed records, durable storage and deterministic state are established. |
| 1 — Peer networking | ✅ Preview-complete | Authenticated libp2p/QUIC, durable identity/reconnect, bounded requests and resumable transfer are permanently gated, including impaired multi-GiB soak. |
| 2 — Snapshot swarm | 🟢 Mostly complete | Real resumable snapshot replication exists; multi-source/retention maturity remains. |
| 3 — Minecraft save integration | ✅ Preview-complete | Fabric IPC, restore, save barriers and final snapshot commit are implemented/tested. |
| 4 — Manual host migration | 🟢 Mostly complete | Signed CLI/desktop transfer wizard exists; real-device acceptance and player reconnection polish remain. |
| 5 — Automatic host migration | 🟡 Control plane complete | Election, quorum recovery and fencing work; automatic successor Minecraft launch/reconnect remains. |
| 6 — World sleep/wake | 🟢 Mostly complete internally | Durable sleep/wake semantics exist; orchestration still exposes runtime plumbing. |
| 7 — Solo mode | ✅ Preview-complete | Explicit solo history, reconciliation and divergence preservation are implemented/tested. |
| 8 — Better replication | 🟠 Early/partial | Background replica support exists; incremental/journal/erasure-code work remains. |
| 9 — NAT/public usability | 🟠 Partial | Protocol support exists; representative real-network certification remains. |
| 10 — UX | 🟠 Partial | Desktop app exists; automatic runtime preparation and lobby UX remain. |
| 11 — Production hardening | 🟠 Partial | Strong CI/recovery/audit gates exist; fuzzing, field validation and signing operations remain. |
| 12 — Distributed simulation | ⚪ Future research | Not implemented by design. |

The most important MVP gap is now the bridge between **safe authority recovery** and **automatic Minecraft runtime migration/player reconnection**.

---

# Phase 0: Research and protocol skeleton

**Status: preview-complete, with continuing protocol hardening.**

Goal:

> Prove that the architecture is coherent before touching deep Minecraft internals.

Tasks:

- define project terminology;
- choose repository structure and license;
- define world ID and peer ID;
- define canonical encoding;
- implement signed records;
- implement content-addressed blobs;
- implement local durable storage/history;
- build deterministic failure scenarios and tests.

Exit criteria:

- peers can exchange and validate signed state;
- corrupted blobs are rejected;
- stale history is detectable;
- failure/recovery behavior can be tested deterministically.

---

# Phase 1: Peer networking

**Status: preview-complete for the Phase 1 transport exit criteria.**

Goal:

> Two machines can securely discover and communicate.

Tasks:

- QUIC/libp2p transport;
- signed peer handshake;
- encrypted transport;
- LAN mDNS discovery;
- direct peer addresses;
- reconnect logic;
- capability negotiation;
- bounded request sizes/rate protections;
- resumable blob transfer.

Exit criteria:

- peers can transfer large synthetic snapshots reliably;
- transfer resumes after connection loss;
- corruption is detected;
- application peer identity remains stable independently of network address.

Current implementation includes libp2p/QUIC, mDNS, Kademlia, AutoNAT, DCUtR and relay support. Permanent gates cover hard reconnect, a 64 MiB transfer under WAN-like packet impairment, and a default 2 GiB transfer with packet loss, bandwidth shaping, repeated hard sender restarts, deliberately lost acknowledgements, re-authentication and resume-offset negotiation. Representative home NAT, CGNAT, mobile and IPv6 certification remains Phase 9 field work; see [docs/NETWORK_VALIDATION.md](docs/NETWORK_VALIDATION.md).

---

# Phase 2: Snapshot swarm

**Status: mostly complete.**

Goal:

> A directory representing a Minecraft world can be replicated across peers.

Tasks:

- snapshot manifests;
- content-addressed blobs;
- Zstd compression;
- resumable/parallel peer downloads;
- snapshot retention;
- integrity verification;
- local garbage collection;
- replica inventory/acknowledgement exchange.

Exit criteria:

- multiple peers hold the same verified snapshot;
- loss of one replica does not destroy the world;
- a new peer can reconstruct the world from surviving replicas;
- corrupt replica data is ignored.

Real live-join replication and resumable blob negotiation are implemented. Multi-source reconstruction strategy and retention/GC maturity remain areas for improvement.

---

# Phase 3: Minecraft save integration

**Status: preview-complete.**

Goal:

> Create consistent snapshots from a live Minecraft world.

Tasks:

- Fabric mod integration;
- local authenticated IPC;
- server lifecycle detection;
- save barrier;
- consistent world snapshot export;
- snapshot restore;
- Minecraft/mod compatibility fingerprint;
- process-level integration tests.

Exit criteria:

- create/play a world;
- request a safe save;
- snapshot and shut Minecraft down;
- restore on another runtime;
- commit a final verified snapshot.

The current Fabric bridge and host process implement these foundations.

---

# Phase 4: Manual host migration

**Status: mostly complete for preview.**

Goal:

> Move a running world from Alice to Bob without manually copying files.

Tasks:

- authority transfer record/state machine;
- graceful authority relinquish;
- final checkpoint;
- target restore;
- automatic target server launch/attachment;
- player reconnection flow;
- CLI and desktop UX.

Exit criteria:

1. Alice hosts.
2. Bob is synchronized.
3. Alice selects **Transfer authority**.
4. Bob accepts and becomes authority.
5. Bob's Minecraft runtime starts safely.
6. Alice/players reconnect to Bob.
7. No manual world-file copying occurs.

The complete signed transfer flow now runs end-to-end through the CLI and the Desktop Transfer host wizard. Player reconnection polish and broader real-device acceptance across adverse networks remain.

---

# Phase 5: Automatic host migration

**Status: distributed control plane implemented; end-to-end product flow partial.**

Goal:

> Authority changes automatically when the current authority disappears.

Tasks:

- authority lease;
- health detection;
- epoch transitions;
- fencing tokens;
- election algorithm;
- stale authority rejection;
- durable recovery ballots/certificates;
- automatic successor runtime launch;
- player reconnection.

Exit criteria:

- hard-kill Alice's process;
- Bob safely wins authority from the latest accepted state;
- Bob's Minecraft runtime starts automatically;
- gameplay can resume without manual world copying;
- Alice returns and cannot write using stale authority;
- repeated crash/recovery cycles preserve safety and liveness.

Quorum election, fencing, durable recovery ballots and stale-peer protection are implemented and covered by process-level tests. Automatic Minecraft launch/reconnection on the elected successor is the critical remaining integration step.

---

# Phase 6: World sleep/wake

**Status: mostly implemented internally.**

Goal:

> No peer needs to remain online permanently.

Tasks:

- durable shutdown state;
- latest-state comparison on startup;
- discovery after full outage;
- canonical recovery;
- safe world wake;
- player-friendly wake orchestration.

Exit criteria:

1. Alice, Bob and Charlie synchronize.
2. Everyone shuts down cleanly.
3. Alice remains offline.
4. Bob returns later.
5. Bob restores/wakes the latest accepted world.
6. Charlie joins later and synchronizes.

No permanent VPS is involved.

Durable sleep records and wake generation checks exist. The remaining work is making the multi-peer wake experience automatic and ordinary-player friendly.

---

# Phase 7: Solo mode

**Status: preview-complete.**

Goal:

> A world whose signed policy permits it can advance with one player while representing the reduced safety honestly.

Tasks:

- solo epochs;
- durability indicators;
- reconciliation rules;
- solo-history conflict detection;
- branch preservation;
- manual conflict recovery UX.

Exit criteria:

- Alice plays alone under an explicit solo policy;
- Bob returns and safely accepts compatible Alice history;
- independently advanced solo branches are detected and never silently merged;
- both conflicting branches remain recoverable.

The protocol/runtime and acceptance tests cover the core safety behavior. Conflict-recovery UX can continue to improve.

---

# Phase 8: Better replication

**Status: early/partial.**

Goal:

> Minimize the amount of progress at risk and reduce replication cost.

Research/work:

- operation journal;
- incremental region replication;
- filesystem-level journal awareness;
- high-frequency metadata replication;
- background replica daemon behavior;
- erasure coding;
- better replica placement/retention.

Exit criteria:

- authority crash loses at most a configured recovery window under normal conditions;
- recovery point is clearly reported;
- replication cost scales better than repeated full-world checkpoints.

Background seeding exists today; the lower-loss incremental strategies remain future work.

---

# Phase 9: NAT traversal and public usability

**Status: protocol support implemented; field validation incomplete.**

Goal:

> Normal players can connect without manually configuring routers in representative environments.

Tasks:

- hole punching;
- multiple discovery sources;
- optional community relays;
- encrypted relay transport;
- bootstrap node list;
- relay self-hosting docs;
- connection diagnostics;
- representative network validation.

Exit criteria:

- works across a documented matrix of representative home NAT/CGNAT/mobile/IPv6 environments;
- relay fallback is demonstrated where direct paths fail;
- no single relay is authoritative or mandatory for world survival.

Do not mark this phase complete merely because AutoNAT/DCUtR/relay code exists. See [docs/NETWORK_VALIDATION.md](docs/NETWORK_VALIDATION.md).

---

# Phase 10: UX

**Status: partial, with a real desktop app now shipping in preview builds.**

Goal:

> SwarmCraft stops feeling like a distributed-systems research console.

Current desktop capabilities include world creation/joining, invites, safety/compatibility state, play/host controls, background seeding, conflict inspection and diagnostics.

The intended mature experience should converge toward:

```text
World: Slop SMP
Status: Healthy

Authority:
  temporary / mostly invisible

Online:
  4 peers

Replicas:
  6 copies

Latest replicated checkpoint:
  3 seconds ago

World safety:
  HIGH
```

Desired primary actions:

```text
Join
Invite
Play
Seed in background
Transfer authority
View replicas
Create recovery snapshot
Diagnostics
```

Major remaining UX work:

- automatic Java/Minecraft/Fabric preparation;
- mod/datapack compatibility acquisition flow;
- automatic host migration/reconnection UX;
- public/friend discovery/lobby;
- reducing manual runtime paths and diagnostics to advanced-only surfaces.

---

# Phase 11: Production hardening

**Status: partial.**

Work includes:

- fuzzing;
- property-based protocol tests;
- long soak tests;
- repeated crash injection;
- disk-full/corruption tests;
- protocol downgrade tests;
- malicious-peer tests;
- dependency audit;
- signed/notarized releases;
- metrics;
- log redaction;
- backup/recovery documentation;
- representative real-network validation.

Current CI already includes strict Rust gates, dependency audit, process-level recovery scenarios, Fabric builds and cross-platform desktop packaging. That is strong preview evidence, not production certification.

No major architecture should be considered production-ready before this phase is substantially complete.

---

# Phase 12: Distributed simulation research

**Status: not implemented; intentionally future research.**

Only after replication, migration and recovery are solid should SwarmCraft explore:

> Can multiple peers simultaneously simulate different parts of one Minecraft world?

Possible architecture:

```text
Region 1 -> Alice
Region 2 -> Bob
Region 3 -> Charlie
```

Research topics:

- chunk ownership;
- region leases;
- cross-region entity migration;
- boundary transactions;
- redstone;
- projectiles;
- explosions;
- portals;
- command execution;
- global game state;
- mod compatibility.

This may require a custom server implementation or modifications far beyond the current Fabric lifecycle bridge.

Treat it as a separate research track, not an MVP requirement.

---

# First public demo target

The central demo remains:

```text
Alice starts a world.
Bob joins.
Charlie joins.

Alice's computer is hard-killed.

Bob safely wins authority.
Bob's Minecraft runtime starts automatically.
Players continue on Bob.

Alice restarts and reconnects as a synchronized peer.

Everyone exits Minecraft.

Bob starts the world again later
without Alice being online.
```

No VPS.

No manual world copying.

No stale authority silently rewriting history.

When that complete flow is repeatable under crashes and packet loss, SwarmCraft has proven its central product idea.

---

# Definition of success

SwarmCraft succeeds when this sentence becomes operationally true, not merely true in a data-structure unit test:

> If one valid replicated copy of the world survives, the community can bring the world back.
