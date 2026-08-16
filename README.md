# SwarmCraft

> A decentralized Minecraft world that survives its host.

SwarmCraft is an open-source experiment in building Minecraft multiplayer around a **persistent world swarm**, not a permanent server machine.

Traditional Minecraft multiplayer looks like this:

```text
Players -> Server -> World
```

If the server disappears, the world usually disappears with it.

SwarmCraft is building toward this instead:

```text
             World
        canonical history
          /    |    \
       Peer   Peer   Peer
```

Peers can replicate signed world state, discover and authenticate one another, elect temporary authority, recover from host failure, and preserve the world across complete shutdowns.

The long-term goal is a Minecraft world with **no permanent host** and **no permanent owner of the authoritative machine**.

---

## Project status

**Current application version: 0.2.1 technical preview.**

SwarmCraft is no longer only an architecture prototype. The repository contains an executable Rust core, authenticated peer networking, snapshot replication, authority/recovery logic, a Fabric lifecycle bridge, a Tauri desktop application, and cross-platform packaging workflows.

### Implemented today

- persistent Ed25519 peer identity and deterministic world identity;
- signed membership, configuration, snapshot, epoch, lease, recovery, sleep and solo-history records;
- content-addressed BLAKE3 snapshot storage with Zstandard compression and integrity verification;
- QUIC/libp2p transport with authenticated peer handshakes;
- mDNS, Kademlia, AutoNAT, DCUtR and relay support;
- resumable blob replication and replica acknowledgements;
- quorum-backed authority leases, fencing tokens and crash recovery;
- durable recovery ballots that allow a later successor when an earlier recovery candidate disappears;
- explicit solo advancement, solo-history reconciliation and conflict preservation;
- Fabric server lifecycle IPC, save barriers, restore and final snapshot commit;
- desktop flows for world creation, joining, invites, play, sleep, seeding, compatibility, conflicts and diagnostics;
- Linux, Windows and macOS CI/package builds plus RustSec dependency audit.

### Not complete yet

The project has **not** completed the seamless end-to-end host-migration product experience.

Important remaining work includes:

- automatically launching the Minecraft runtime on the peer that wins authority after a crash;
- reconnecting players to the new authority without manual coordination;
- exposing manual authority transfer as a complete player-facing workflow;
- automatically preparing compatible Minecraft/Fabric/mod environments instead of asking users for runtime JAR paths;
- field-validating NAT traversal and relay fallback across representative home networks, CGNAT, mobile carriers and IPv6 deployments;
- public/friend world discovery and lobby services that remain non-authoritative;
- deeper fuzzing, soak, disk-failure and malicious-peer testing;
- production signing/notarization operations.

Distributed region simulation is research for later and is not part of the current preview.

For the detailed implementation matrix, see [docs/IMPLEMENTATION_STATUS.md](docs/IMPLEMENTATION_STATUS.md).

---

## Why?

A normal Minecraft server has several weaknesses:

- one machine is the permanent authority;
- somebody must keep that machine online;
- backups and availability depend on the host;
- the original owner can become a single point of failure;
- moving the world usually requires manual coordination and file copying.

SwarmCraft explores the opposite model:

- players contribute storage and bandwidth;
- important world state is replicated across peers;
- authority is temporary and fenced rather than tied to ownership;
- the original creator can leave without invalidating the world identity;
- valid replicated state can survive ordinary peer loss;
- the canonical history is cryptographically verifiable rather than defined by one machine.

This is not "LAN hosting with extra steps."

The architecture is closer to:

**Minecraft + content-addressed replication + signed distributed state + automatic authority recovery.**

---

## Core idea

A SwarmCraft world has a persistent cryptographic identity.

```text
World ID
   |
Genesis / signed configuration
   |
Snapshot 1
   |
Canonical epoch + membership history
   |
Snapshot 2
   |
...
```

The source of truth is the **canonical accepted history of the world**, not the IP address or disk of one server.

Each peer can verify the records it accepts: hashes, signatures, protocol versions, membership, authority generation, fencing tokens and snapshot ancestry.

A current world ID is represented as:

```text
scworld:<cryptographic-id>
```

Signed invitations carry membership/bootstrap information for joining private worlds.

---

## Temporary authority

Minecraft simulation still needs one clear ordering of events, so SwarmCraft does not attempt to let every peer mutate the same world independently.

Instead, one eligible peer temporarily runs the authoritative Minecraft simulation.

```text
Alice = authority
Alice disappears
Bob = elected successor
```

The authority is a role, not ownership.

The runtime uses epochs, fencing tokens, signed leases and quorum-backed recovery to prevent stale authorities from silently re-entering canonical history.

The control-plane recovery logic is implemented and process-level recovery is tested. Automatically turning a newly elected successor into a running Minecraft server is still an integration milestone.

---

## Solo mode and partitions

A world may explicitly allow solo advancement when quorum is unavailable.

Solo progress is recorded as lower-durability history instead of being mislabeled as quorum-backed canonical safety. When peers reconnect, compatible solo ancestry can be reconciled. Competing solo branches are preserved as a conflict and are **never silently merged**.

For partitioned networks, SwarmCraft prefers consistency over pretending that two independently advanced Minecraft histories are interchangeable.

---

## Replication and sleep

Snapshots are content-addressed, signed and verified before acceptance. Peers negotiate missing blobs and can resume transfers.

When all peers are offline, nothing runs. The world simply sleeps. Durable state remains on replicas, and a valid replica can later restore the world.

The current runtime has durable sleep records and wake logic. Fully invisible wake/host orchestration is still being refined.

---

## Architecture

SwarmCraft currently has three practical layers:

```text
+----------------------------------+
| Minecraft Java Edition           |
| Fabric integration mod           |
| Java                             |
+----------------+-----------------+
                 |
                 | loopback IPC
                 v
+----------------------------------+
| SwarmCraft Rust runtime          |
|                                  |
| identity / protocol              |
| storage / snapshots              |
| networking / replication         |
| membership / authority           |
| recovery / solo history          |
+----------------+-----------------+
                 |
                 | sidecar commands
                 v
+----------------------------------+
| Tauri desktop application        |
| HTML / CSS / JavaScript + Rust   |
+----------------------------------+
```

The Rust workspace is split into focused crates under `crates/`, including protocol, storage, networking, consensus, IPC, core services and CLI/runtime orchestration.

See [ARCHITECTURE.md](ARCHITECTURE.md) and [PROTOCOL.md](PROTOCOL.md).

---

## Repository layout

```text
swarmcraft/
├── apps/
│   └── desktop/              # Tauri desktop application
├── crates/
│   ├── swarm-cli/
│   ├── swarm-consensus/
│   ├── swarm-core/
│   ├── swarm-ipc/
│   ├── swarm-network/
│   ├── swarm-protocol/
│   └── swarm-storage/
├── minecraft/
│   └── fabric/               # Fabric lifecycle bridge
├── docs/                     # status, release, recovery and validation docs
├── tests/                    # test guidance / additional fixtures
├── .github/workflows/        # CI, installers and releases
├── ARCHITECTURE.md
├── PROTOCOL.md
├── ROADMAP.md
├── SECURITY.md
└── README.md
```

---

## Design principles

1. **The world must outlive its creator.** No creator-owned master file should be required after establishment.
2. **No permanent server.** Authority may move; world identity must not depend on one machine.
3. **Verify, do not blindly trust.** Hashes, signatures, generations and compatibility matter.
4. **Offline-first durability.** Valid progress must be durably representable before another peer appears.
5. **Replicate aggressively.** More independent valid copies mean better durability.
6. **Make consistency rules explicit.** Never hide forks behind vague last-writer-wins behavior.
7. **Keep Minecraft integration replaceable.** Distributed state should not depend unnecessarily on Minecraft internals.
8. **Treat security as protocol design.** Membership, replay protection, stale authority and corrupt replicas are first-class concerns.

---

## Non-goals for the first production-ready version

SwarmCraft is not currently trying to provide:

- fully distributed Minecraft tick simulation;
- automatic semantic merging of two independently played worlds;
- anonymous Byzantine consensus at internet scale;
- blockchain or cryptocurrency;
- perfect protection from a malicious majority;
- every Minecraft mod loader;
- Bedrock Edition support.

---

## MVP definition

The central product milestone remains:

1. Alice creates a SwarmCraft world.
2. Bob joins and obtains a durable replica.
3. Alice runs the authoritative Minecraft session.
4. Alice's process is killed.
5. Bob safely wins authority from the latest accepted state.
6. Bob's Minecraft runtime starts automatically.
7. Players reconnect and continue the same world.
8. Alice returns and synchronizes without stale-authority writes.
9. Everyone shuts down.
10. Bob later restores the world without Alice being online.

The repository already proves much of the storage, replication and authority-recovery control plane. Steps 6 and 7 are the main remaining end-to-end integration gap.

---

## Current Minecraft target

The 0.2.x preview currently targets:

- Minecraft Java `26.1.2`;
- Fabric Loader `0.19.3`;
- Fabric API `0.155.2+26.1.2`;
- Java `25+`.

Per-world signed compatibility manifests may impose additional exact mod/datapack requirements.

---

## Validation and release discipline

Normal CI covers Rust format/lint/test gates, process-level replication/recovery scenarios, Fabric build, RustSec and native desktop packaging. Real public-network NAT behavior remains a separate manual validation requirement.

See:

- [Implementation status](docs/IMPLEMENTATION_STATUS.md)
- [Release gates](docs/RELEASE_GATES.md)
- [Network validation](docs/NETWORK_VALIDATION.md)
- [Authority recovery](docs/AUTHORITY_RECOVERY.md)
- [Recovery acceptance](docs/RECOVERY_ACCEPTANCE.md)

---

## Documentation

- [Architecture](ARCHITECTURE.md)
- [Protocol](PROTOCOL.md)
- [Security model](SECURITY.md)
- [Roadmap](ROADMAP.md)
- [Product vision](docs/PRODUCT_VISION.md)
- [Implementation status](docs/IMPLEMENTATION_STATUS.md)
- [Contributing](CONTRIBUTING.md)

---

## License

SwarmCraft is licensed under Apache-2.0. See [LICENSE](LICENSE).

---

## Contributing

SwarmCraft is active preview software. Useful contributions include distributed-systems review, recovery testing, networking/NAT validation, Minecraft lifecycle integration, compatibility tooling, UX work, fuzzing and failure injection.

See [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md).

---

## One-sentence vision

> A Minecraft world should be able to survive every individual machine that has ever hosted it.
