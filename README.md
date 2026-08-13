# SwarmCraft

> A decentralized Minecraft world that survives its host.

SwarmCraft is an open-source experiment in building Minecraft multiplayer around a **world**, not a permanent server.

Traditional Minecraft multiplayer has a simple topology:

```text
Players -> Server -> World
```

If the server goes offline, the world goes offline.

SwarmCraft aims for:

```text
             World
        canonical history
          /    |    \
       Peer   Peer   Peer
```

Peers replicate world state, discover one another, elect temporary authority when required, and recover the world from the latest valid replicated state.

The long-term goal is a Minecraft world with **no permanent host** and **no permanent owner of the authoritative machine**.

---

## Why?

A normal Minecraft server has several weaknesses:

- one machine is the permanent authority;
- someone has to pay for hosting;
- the world disappears when that machine disappears;
- the owner controls backups and availability;
- scaling usually costs more as more players join.

SwarmCraft explores the opposite model:

- players contribute storage, bandwidth, and eventually compute;
- the world can migrate automatically between players;
- copies of the world are replicated across peers;
- the original creator can leave permanently without killing the world;
- the network can recover after ordinary peer failures;
- no mandatory central world database is required.

This is not "LAN hosting with extra steps."

The intended architecture is closer to:

**Minecraft + BitTorrent-style distribution + replicated state machines + automatic host migration.**

---

## Project status

**Very early design / research stage.**

The first implementation should deliberately avoid trying to distribute every Minecraft tick across many machines.

The initial target is:

1. peer identity;
2. peer discovery;
3. replicated world snapshots;
4. append-only world history;
5. automatic authority election;
6. seamless host migration;
7. crash recovery;
8. NAT traversal;
9. integrity verification.

Distributed region simulation can come later.

---

## Core idea

A SwarmCraft world has a persistent identity.

```text
World ID
   |
Genesis state
   |
Snapshot 1
   |
Transaction log
   |
Snapshot 2
   |
...
```

The world is identified by cryptographic metadata rather than by the IP address of one server.

A player could eventually join a world using something like:

```text
swarmcraft://world/<world-id>
```

Their client would:

1. resolve available peers;
2. connect to the swarm;
3. download the latest valid state;
4. verify its history;
5. join the Minecraft session;
6. become another replica.

---

## What happens when everyone goes offline?

Nothing runs.

The world simply sleeps.

The latest committed world state remains stored across peers.

When a peer returns, it restores the latest valid state it knows about. When more peers return, they compare histories and synchronize.

There is no requirement for a 24/7 Minecraft process.

---

## What is the source of truth?

Not a computer.

The source of truth is the **canonical committed history of the world**.

Each accepted state transition references the previous accepted state.

Conceptually:

```text
State 900
hash: A1

    |
    v

State 901
previous: A1
hash: B7

    |
    v

State 902
previous: B7
hash: C4
```

Peers can independently verify that a proposed history follows the rules of the protocol.

The exact consensus rules are still part of the research and implementation work.

---

## Temporary authority

Minecraft simulation often needs one clear ordering of events.

SwarmCraft therefore does **not** assume that every peer can modify the same state independently at the same instant.

Instead, the network can elect a temporary authority.

```text
Alice = authority
Alice disconnects
Bob = authority
Bob disconnects
Charlie = authority
```

The authority is a role, not ownership.

The world identity and replicated history survive the authority.

---

## Solo play

A decentralized world must remain usable when only one player is online.

SwarmCraft should support a solo-authority mode:

```text
1 peer online
-> peer temporarily advances the world
-> progress is stored durably
-> new state is replicated when another peer appears
```

Progress created while only one copy exists is inherently less durable.

This cannot be magically eliminated without another storage node.

---

## Replication

Important state should exist on multiple devices whenever possible.

Example:

```text
Alice   -> snapshot 1250
Bob     -> snapshot 1250
Charlie -> snapshot 1250
```

If Alice disappears permanently, Bob and Charlie still have the world.

The system should distinguish:

- **locally durable** state;
- **replicated** state;
- **quorum-confirmed** state;
- **historical snapshot** state.

---

## Network partitions

Network partitions are one of the hardest problems.

Example:

```text
Alice + Bob + Charlie | Dave + Eve
```

If the network splits and both sides independently advance the same Minecraft world, inventories, redstone, entities, and blocks can conflict.

SwarmCraft should prefer **consistency over silently forking the canonical world**.

Possible policy:

- the partition retaining the required authority/quorum continues;
- the other side pauses authoritative simulation or enters an explicitly non-canonical mode;
- peers reconcile when connectivity returns.

Solo mode requires special treatment and should be recorded clearly in the history.

---

## Architecture

The proposed implementation is split into two major layers.

```text
+----------------------------------+
| Minecraft Java Edition           |
| Fabric integration mod           |
| Java                             |
+----------------+-----------------+
                 |
                 | local protocol / IPC
                 v
+----------------------------------+
| SwarmCraft Core                  |
| Rust                             |
|                                  |
| peer identity                    |
| networking                       |
| discovery                        |
| replication                      |
| authority election               |
| snapshots                        |
| history validation               |
| encryption                       |
| persistence                      |
+----------------------------------+
```

### Minecraft layer

Recommended:

- Java;
- Fabric;
- minimal responsibility;
- translate Minecraft events/state into protocol operations;
- apply validated remote state;
- control server lifecycle.

### Distributed core

Recommended:

- Rust;
- Tokio;
- libp2p and/or QUIC;
- BLAKE3;
- Ed25519;
- compact binary serialization;
- durable local storage;
- Zstandard compression.

See [ARCHITECTURE.md](ARCHITECTURE.md).

---

## Proposed repository layout

```text
swarmcraft/
├── README.md
├── LICENSE
├── docs/
│   ├── ARCHITECTURE.md
│   ├── PROTOCOL.md
│   ├── SECURITY.md
│   └── ROADMAP.md
├── crates/
│   ├── swarm-core/
│   ├── swarm-network/
│   ├── swarm-storage/
│   ├── swarm-protocol/
│   └── swarm-cli/
├── minecraft/
│   └── fabric/
├── tests/
│   ├── integration/
│   ├── partition/
│   └── recovery/
└── tools/
```

The documentation package currently keeps the docs at repository root for easy review. Move them into `docs/` when creating the actual repository if preferred.

---

## Design principles

### 1. The world must outlive its creator

No creator-owned master file should be required after the world is established.

### 2. No permanent server

A peer can temporarily coordinate Minecraft simulation, but no specific machine must remain online.

### 3. Verify, do not blindly trust

Peers should validate hashes, signatures, protocol versions, and history before accepting state.

### 4. Offline-first durability

A peer should be able to persist valid world progress before contacting another peer.

### 5. Replicate aggressively

When peers become available, important state should be replicated quickly.

### 6. Explicit consistency rules

Network partitions must never be handled by vague "last writer wins" behavior.

### 7. Minecraft integration stays replaceable

The distributed core should not depend heavily on Minecraft internals.

### 8. Security is part of the protocol

Identity, permissions, replay protection, malicious peers, and corrupt replicas should be considered from the beginning.

---

## Non-goals for the first version

The MVP should **not** attempt:

- fully distributed Minecraft tick simulation;
- arbitrary conflict-free merging of two independently played worlds;
- anonymous Byzantine consensus at internet scale;
- blockchain or cryptocurrency;
- global public-world discovery;
- perfect protection from a malicious majority;
- support for every mod loader;
- support for Bedrock Edition.

---

## MVP definition

A successful first milestone could demonstrate:

1. Alice creates a SwarmCraft world.
2. Bob joins.
3. Both obtain durable copies.
4. Alice is the active Minecraft authority.
5. Alice intentionally disconnects.
6. Bob automatically becomes authority.
7. Minecraft gameplay resumes using the same world.
8. Alice reconnects and synchronizes.
9. Both shut down.
10. Bob returns later and restores the correct world without Alice.

If this works reliably under crashes and packet loss, the project has proven its central idea.

---

## Long-term possibilities

Once host migration and replication are solid, the architecture could explore:

- region-based simulation;
- per-region authority;
- chunk sharding;
- distributed mob simulation;
- background seeding daemons;
- erasure-coded world storage;
- public swarm bootstrap nodes;
- optional community relays;
- spectator replicas;
- cross-world federation;
- decentralized backups;
- serverless community worlds.

---

## Important warning

Distributed systems are unforgiving.

A bug in ordinary software may crash an app.

A bug in a distributed world protocol may create:

- duplicated inventories;
- divergent histories;
- permanent forks;
- corrupt snapshots;
- invalid authority changes;
- exploits that propagate to every replica.

For that reason, SwarmCraft should prioritize deterministic tests, failure simulation, recovery testing, and conservative protocol evolution.

---

## Documentation

- [Architecture](ARCHITECTURE.md)
- [Protocol](PROTOCOL.md)
- [Security Model](SECURITY.md)
- [Roadmap](ROADMAP.md)
- [Contributing](CONTRIBUTING.md)

---

## License

A permissive license such as Apache-2.0 or MIT is a natural fit for the project.

Apache-2.0 may be preferable if explicit patent language matters to the project.

Choose intentionally before accepting external contributions.

---

## Contributing

The project is currently at the architecture stage.

Useful early contributions include:

- distributed-systems review;
- Minecraft server lifecycle research;
- Fabric integration experiments;
- libp2p prototypes;
- snapshot formats;
- deterministic recovery tests;
- threat modeling;
- NAT traversal testing.

See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## One-sentence vision

> A Minecraft world should be able to survive every individual machine that has ever hosted it.
