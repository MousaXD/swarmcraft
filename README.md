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

**Current application version: 0.4.0 technical preview.** Wire protocol version remains `1`.

SwarmCraft is no longer only an architecture prototype. The repository contains an executable Rust core, authenticated peer networking, snapshot replication, authority/recovery logic, a Fabric lifecycle bridge, a Tauri desktop application, managed Minecraft runtime preparation, existing-world import, and cross-platform packaging workflows.

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
- backend-managed Java, Minecraft, Fabric Loader, Fabric API and SwarmCraft Fabric bridge preparation;
- explicit Minecraft server EULA acceptance and persisted machine-local runtime configuration;
- deterministic required server-mod verification by metadata and artifact hash;
- shared Rust runtime orchestration for normal launch, safe authority migration and supported wake paths;
- backend Host Readiness for the player-facing **Can I turn off this PC?** decision;
- safe existing-world import through the Rust backend and normal Desktop flow;
- fail-closed corrupt/unreadable sleep-state handling across direct launch, standby and migration;
- desktop flows for world creation, import, joining, invites, play, sleep, seeding, compatibility, conflicts and diagnostics;
- Linux, Windows and macOS CI/package builds plus RustSec dependency audit;
- four bundled Desktop sidecars on every supported target: `swarmcraft`, `swarmcraft-host`, `swarmcraft-runtime`, and `swarmcraft-import`.

### Not complete yet

SwarmCraft 0.4.0 is still a technical preview, not a claim of universal production readiness or a completely invisible multiplayer handoff.

Important remaining work includes:

- seamless automatic Minecraft client redirection/reconnection after authority migration;
- a dedicated quorum-backed wake protocol for sleeping multi-member worlds;
- representative field validation across home routers, symmetric NAT, CGNAT, mobile carriers, blocked-UDP networks and independent-ISP IPv6 paths;
- automatic redistribution of arbitrary third-party server-mod JARs, which are currently supplied locally and verified against canonical requirements;
- public/friend world discovery and lobby services that remain non-authoritative;
- longer hostile-peer, fuzz, soak, disk-failure and repeated real-Minecraft migration campaigns;
- production signing/notarization operations where repository credentials are available.

For exactly two voting members, crash failover intentionally remains fail-closed: if Alice disappears, Bob alone cannot form majority quorum. Use an explicit authority transfer while both peers are present, or a three-voter topology for automatic crash recovery. Multi-member wake likewise remains fail-closed rather than using first-click-wins semantics.

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
Bob = quorum-backed successor
```

The authority is a role, not ownership.

The runtime uses epochs, fencing tokens, signed leases and quorum-backed recovery to prevent stale authorities from silently re-entering canonical history.

The control-plane recovery logic is implemented and process-level recovery is tested. When a successor safely wins authority, the shared migration/runtime orchestration path can restore canonical state, prepare the configured Minecraft runtime, launch it, verify Fabric readiness and publish the running authority endpoint. Seamless client redirection/reconnection remains separate product work.

---

## Solo mode and partitions

A world may explicitly allow solo advancement when quorum is unavailable.

Solo progress is recorded as lower-durability history instead of being mislabeled as quorum-backed canonical safety. When peers reconnect, compatible solo ancestry can be reconciled. Competing solo branches are preserved as a conflict and are **never silently merged**.

For partitioned networks, SwarmCraft prefers consistency over pretending that two independently advanced Minecraft histories are interchangeable.

---

## Replication and sleep

Snapshots are content-addressed, signed and verified before acceptance. Peers negotiate missing blobs and can resume transfers.

When all peers are offline, nothing runs. The world simply sleeps. Durable state remains on replicas, and a valid replica can later restore the world.

Sleep records are signed and bound to the canonical snapshot/authority generation. Single-member wake uses the supported safe path. Multi-member wake intentionally remains blocked until SwarmCraft has a sleep-bound quorum transition rather than treating the first peer to click Play as authority.

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

The central product milestone is now best represented by a topology that can actually preserve majority quorum:

1. Alice creates a SwarmCraft world.
2. Bob and Carol join and obtain durable replicas.
3. Alice runs the authoritative Minecraft session.
4. Alice's process is killed.
5. Bob safely wins authority from the latest accepted state with a surviving quorum.
6. Bob's Minecraft runtime starts automatically through the shared migration/runtime path.
7. Players reconnect and continue the same world.
8. Alice returns and synchronizes without stale-authority writes.
9. Everyone shuts down.
10. A valid peer later restores the world without Alice being online.

The repository implements and permanently tests the storage, replication, fencing/recovery and successor-runtime portions of that path, with a real clean-machine Minecraft acceptance gate for runtime setup, launch, stop, restart and world restoration. Step 7, seamless automatic client reconnection/redirection, remains the largest visible UX gap. A two-voter Alice/Bob crash topology is intentionally not used as positive automatic-failover evidence because Bob alone would not have quorum.

---

## Current Minecraft target

The 0.4.0 preview currently targets:

- Minecraft Java `26.1.2`;
- Fabric Loader `0.19.3`;
- Fabric API `0.155.2+26.1.2`;
- Java `25+`.

Per-world signed compatibility manifests may impose additional exact mod/datapack requirements.

---

## Validation and release discipline

Normal CI covers Rust format/lint/test gates, process-level replication/recovery/migration scenarios, import and corrupt-sleep regressions, Fabric build, RustSec, fuzz smoke, impaired QUIC resume, Desktop tests and native packaging on Linux, Windows and both macOS architectures.

The separate live player-journey workflow uses a fresh SwarmCraft data directory and official Minecraft/Fabric/Adoptium services, forces managed-Java resolution, requires explicit EULA acceptance, launches real Minecraft twice, stops through the safe durability barrier, restores known world data and verifies monotonically advancing canonical snapshots without divergence.

Real public-network NAT behavior remains a separate manual validation requirement.

See:

- [Final player-journey acceptance](docs/FINAL_PLAYER_JOURNEY_ACCEPTANCE.md)
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
