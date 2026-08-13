# SwarmCraft Roadmap

This roadmap intentionally starts with the smallest architecture that can prove the idea.

The order matters.

Trying to build distributed chunk simulation before reliable recovery would create a spectacular pile of very advanced bugs.

---

# Phase 0: Research and protocol skeleton

Goal:

> Prove that the architecture is coherent before touching deep Minecraft internals.

Tasks:

- define project terminology;
- choose repository structure;
- choose license;
- define world ID;
- define peer ID;
- define canonical encoding;
- prototype signed records;
- prototype content-addressed blobs;
- implement local durable log;
- create deterministic fake-peer simulator;
- write failure scenarios.

Exit criteria:

- two fake peers can exchange signed state;
- corrupted blobs are rejected;
- stale history is detectable;
- simulator can create partitions/crashes.

---

# Phase 1: Peer networking

Goal:

> Two machines can securely discover and communicate.

Tasks:

- QUIC/libp2p prototype;
- peer handshake;
- encrypted transport;
- LAN mDNS discovery;
- direct peer addresses;
- reconnect logic;
- capability negotiation;
- rate limits;
- resumable blob transfer.

Exit criteria:

- two peers exchange a 1 GB synthetic snapshot reliably;
- transfer resumes after connection loss;
- hash corruption is detected;
- peer identity remains stable.

---

# Phase 2: Snapshot swarm

Goal:

> A directory representing a Minecraft world can be replicated across peers.

Tasks:

- snapshot manifests;
- content-addressed blobs;
- Zstd compression;
- parallel peer downloads;
- snapshot retention;
- integrity verification;
- local garbage collection;
- replica inventory exchange.

Exit criteria:

- three peers hold the same snapshot;
- one peer is deleted;
- a fourth peer reconstructs the world from the remaining two;
- corrupted replica data is ignored.

This phase can be built without Minecraft integration.

---

# Phase 3: Minecraft save integration

Goal:

> Create consistent snapshots from a live Minecraft world.

Tasks:

- Fabric mod skeleton;
- local IPC;
- detect server lifecycle;
- request save barrier;
- export consistent world snapshot;
- restore snapshot;
- Minecraft/mod compatibility fingerprint;
- integration tests with temporary worlds.

Exit criteria:

- create world;
- play;
- snapshot;
- shut Minecraft down;
- restore on another machine;
- world opens correctly.

---

# Phase 4: Manual host migration

Goal:

> Move a running world from Alice to Bob without manually copying files.

Tasks:

- authority record;
- graceful authority relinquish;
- final checkpoint;
- target restore;
- automatic server launch/attachment;
- player reconnection flow.

Exit criteria:

1. Alice hosts.
2. Bob is synchronized.
3. Alice selects "transfer authority."
4. Bob becomes authority.
5. Alice reconnects to Bob.
6. No world-file copying is performed manually.

At this point the central concept is already visible.

---

# Phase 5: Automatic host migration

Goal:

> Authority changes automatically when the current authority disappears.

Tasks:

- authority lease;
- health detection;
- epoch transitions;
- fencing tokens;
- election algorithm;
- stale authority rejection;
- crash recovery.

Exit criteria:

- kill Alice's process;
- Bob takes authority;
- world resumes from the latest safe checkpoint;
- Alice comes back and cannot write using stale authority.

Test hundreds/thousands of forced crashes.

---

# Phase 6: World sleep/wake

Goal:

> No peer needs to remain online permanently.

Tasks:

- durable shutdown state;
- latest-state comparison on startup;
- peer discovery after full outage;
- canonical recovery;
- world wake flow.

Exit criteria:

1. Alice, Bob, Charlie synchronize.
2. Everyone shuts down.
3. Alice remains offline.
4. Bob comes back tomorrow.
5. Bob restores world.
6. Charlie joins later and synchronizes.

No permanent VPS is involved.

This is the milestone where SwarmCraft becomes a true serverless-hosting experience.

---

# Phase 7: Solo mode

Goal:

> A single player can advance the world.

Tasks:

- solo epochs;
- durability indicators;
- reconciliation rules;
- solo-history conflict detection;
- branch preservation;
- manual conflict recovery UI.

Exit criteria:

- Alice plays alone;
- Bob returns and safely accepts Alice's history;
- synthetic competing solo branches are detected and never silently merged.

---

# Phase 8: Better replication

Goal:

> Minimize the amount of progress at risk.

Research:

- operation journal;
- incremental region replication;
- fs-level journal awareness;
- high-frequency metadata replication;
- background replica daemon;
- erasure coding.

Exit criteria:

- authority crash loses at most a configured recovery window under normal conditions;
- recovery point is clearly reported.

---

# Phase 9: NAT traversal and public usability

Goal:

> Normal players can use SwarmCraft without configuring routers.

Tasks:

- hole punching;
- multiple discovery sources;
- optional community relays;
- relay encryption;
- bootstrap node list;
- relay self-hosting docs;
- connection diagnostics.

Exit criteria:

- works across representative home NAT environments;
- no single relay is mandatory.

---

# Phase 10: UX

Goal:

> It stops feeling like distributed-systems research.

Possible UI:

```text
World: Slop SMP
Status: Healthy

Authority:
  Mousa-PC

Online:
  4 peers

Replicas:
  6 copies

Latest replicated checkpoint:
  3 seconds ago

World safety:
  HIGH
```

Buttons:

```text
Join
Invite
Seed in background
Transfer authority
View replicas
Create recovery snapshot
Diagnostics
```

---

# Phase 11: Production hardening

Tasks:

- fuzzing;
- property-based protocol tests;
- soak tests;
- crash injection;
- disk corruption tests;
- protocol downgrade tests;
- malicious-peer tests;
- dependency audit;
- signed releases;
- metrics;
- log redaction;
- backup/recovery documentation.

No major architecture should be considered production-ready before this phase.

---

# Phase 12: Distributed simulation research

Only now explore:

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

This may ultimately require a custom server implementation or deep modifications beyond a Fabric mod.

Treat it as a new research track, not an MVP requirement.

---

# First public demo target

The ideal first demo video:

```text
Alice starts a world.
Bob joins.
Charlie joins.

Alice's computer is hard-killed.

Bob automatically becomes host.

Alice restarts and reconnects.

Everyone exits Minecraft.

Bob starts the world again later
without Alice being online.
```

No VPS.

No manual world copying.

No permanent host.

That demonstration alone would communicate the project better than fifty pages of theory.

---

# Definition of success

SwarmCraft succeeds when this sentence becomes true:

> If one valid replicated copy of the world survives, the community can bring the world back.
