# SwarmCraft 0.2.1 Implementation Status

This document is the current source of truth for what the repository **actually implements** versus what remains roadmap or product-vision work.

It replaces the original v0.1.0 foundation status, which became stale as networking, Fabric integration, recovery, desktop UI and packaging landed.

Application version and wire protocol version are separate concepts. SwarmCraft 0.2.1 still uses protocol version 1 unless a protocol-breaking change explicitly requires otherwise.

## Executive summary

SwarmCraft is an **advanced technical preview**, not merely an architecture prototype and not yet a seamless consumer product.

The repository already implements the difficult control-plane foundations:

- cryptographic peer/world identity;
- signed world configuration and membership;
- content-addressed snapshot storage and verification;
- authenticated libp2p/QUIC networking;
- snapshot replication;
- authority leases, fencing and crash recovery;
- durable recovery ballots;
- explicit solo history and conflict preservation;
- Fabric lifecycle/save integration;
- a player-facing Tauri desktop shell;
- cross-platform CI and installer packaging.

The largest missing product milestone is connecting **safe authority recovery** to **automatic Minecraft runtime migration and player reconnection**.

---

## Implemented

### Identity, protocol and storage

- Rust workspace with focused protocol, core, storage, networking, consensus, IPC and CLI/runtime crates.
- Protocol version 1 and storage schema version 1.
- Durable Ed25519 peer identity.
- `PeerId = BLAKE3(public_key)`.
- Deterministic cryptographic `WorldId`.
- Canonical signed world configuration and presentation metadata.
- Signed membership records and authority eligibility.
- Content-addressed BLAKE3 blob descriptors.
- Zstandard-compressed blob storage.
- Deterministic snapshot manifests and state roots.
- Streaming snapshot creation/verification paths for large data.
- Crash-conscious temporary-write, fsync and rename persistence paths.
- Signed snapshot manifests, verification and corruption detection.
- Snapshot restore, recovery and normal-world export.

### Peer networking and replication

- libp2p runtime with TCP/Noise/Yamux and QUIC support.
- Authenticated signed application peer handshake on top of transport identity.
- LAN mDNS discovery.
- Kademlia address/discovery support.
- Bootstrap peer configuration.
- AutoNAT state.
- DCUtR hole-punch support.
- Relay client/reservation and relay dialing support.
- Structured connectivity diagnostics.
- Bounded wire message sizes for high-risk request classes.
- Snapshot manifest negotiation.
- Missing-blob negotiation.
- Chunked blob transfer with resume offsets.
- Replica acknowledgements.
- Live join followed by immediate snapshot replication without requiring a reconnect.

### Authority, recovery and safety

- Deterministic authority candidate ranking.
- Signed authority leases.
- Monotonic epochs and fencing tokens.
- Stale/future authority generation rejection.
- Quorum-aware runtime permits for Minecraft authority.
- Automatic detection of missing authority peers.
- Quorum-backed recovery election.
- Durable signed recovery ballots and votes.
- Persisted recovery certificates.
- Later recovery rounds can supersede an abandoned successor while remaining anchored to the same canonical base.
- Recovery epoch replication.
- Stale returning peers cannot continue writing with an old authority generation.

### Solo history

- Signed per-world policy for allowing solo advancement.
- Explicit solo epochs.
- Durable solo branch ancestry/head records.
- Solo branch refresh as snapshots advance.
- Reconciliation when compatible history returns.
- Explicit conflict preservation for independently advanced solo branches.
- No unsafe automatic semantic merge of divergent Minecraft worlds.

### Minecraft/Fabric integration

- Fabric mod project and CI build.
- Loopback-only local IPC bridge.
- Runtime authentication token for IPC.
- Minecraft/Fabric/world compatibility reporting.
- Save barrier request/response.
- Graceful shutdown barrier.
- Snapshot restore into a runtime world directory.
- Fabric bridge injection into the runtime mods directory.
- Minecraft server process launch.
- Final signed snapshot commit after successful shutdown.
- Durable sleep record.
- Authority permit watchdog that terminates a multi-member server when a live permit is lost.

### Desktop application

- Tauri 2 desktop shell.
- Bundled SwarmCraft runtime sidecars.
- World list and selected-world detail UI.
- Create world flow.
- Signed invite creation.
- Signed invite join flow.
- Leave request flow.
- Play/host flow with authority-eligibility checks.
- Graceful sleep/stop controls.
- Background seeding toggle.
- World safety display.
- Compatibility display.
- Preserved-conflict inspection.
- Peer/membership inspection.
- Snapshot verification, export and recovery controls.
- Replication daemon controls and diagnostics.

### CI and packaging

- Linux, Windows and macOS Rust build/test coverage.
- Formatting and strict Clippy gates.
- RustSec dependency audit against the committed lockfile.
- Process-level acceptance tests for:
  - live join and immediate replication;
  - host lifecycle and final sleep snapshot;
  - three-daemon hard-kill authority recovery;
  - recovery successor disappearing before epoch promotion;
  - solo-history acceptance and divergence detection.
- Fabric server mod build.
- Native desktop package builds for Linux, Windows and macOS.
- Rolling `main-latest` development snapshot workflow.
- Release workflow support for installer checksums, Fabric artifacts and optional platform signing credentials.

---

## Partially implemented

These areas contain real code but do **not** yet satisfy the full product/roadmap exit criteria.

### Seamless automatic host migration

The distributed control plane can elect and fence a new authority after the current authority disappears.

Still missing:

- automatically launching the successor's Minecraft runtime as a direct consequence of winning authority;
- automatically directing/reconnecting players to that new runtime;
- proving the entire gameplay handoff under repeated real Minecraft crash scenarios.

This is the most important remaining MVP integration gap.

### Manual authority transfer

Consensus/state-machine support exists for prepared/accepted/committed authority transfer records.

Still missing:

- a complete CLI command flow;
- a desktop **Transfer authority** action;
- target runtime preparation and automatic launch;
- player reconnection UX.

### World sleep/wake

Durable sleep records, safe latest-snapshot checks and wake epoch logic exist.

Still missing:

- a fully automatic world-wake orchestration path that hides runtime plumbing from normal players;
- broader multi-peer full-outage acceptance coverage matching the roadmap's complete sleep/wake scenario.

### Snapshot swarm breadth

Snapshot replication, corruption checks and resumable blob transfer exist.

Still incomplete relative to the roadmap:

- parallel multi-source reconstruction as a polished replication strategy;
- retention/garbage-collection policy maturity;
- explicit automated proof of the roadmap scenario where a fourth peer reconstructs a world from multiple surviving replicas.

### NAT traversal and internet usability

The code supports AutoNAT, DCUtR and relays.

This must **not** be described as universally proven NAT traversal. Representative field testing across real home routers, CGNAT, mobile hotspots, firewall policies and IPv6 networks remains pending. See [NETWORK_VALIDATION.md](NETWORK_VALIDATION.md).

### Desktop product experience

The desktop application is real and usable for technical preview workflows.

It is not yet the final launcher experience because users may still need to provide:

- Java/runtime configuration;
- a Fabric server JAR;
- the SwarmCraft Fabric mod JAR;
- explicit EULA acceptance.

Automatic per-world Minecraft/Fabric/modpack preparation is future work.

---

## Not implemented yet

- Seamless automatic player reconnection after authority migration.
- Fully productized manual authority transfer.
- Automatic acquisition/preparation of per-world Minecraft/Fabric/mod environments.
- Central or federated public-world search/lobby services.
- Friends/social discovery.
- Automatic third-party mod redistribution.
- Operation-level Minecraft journal replication.
- Incremental region/chunk replication as the primary recovery path.
- Erasure-coded world storage.
- Production-grade metrics/telemetry infrastructure.
- Comprehensive fuzzing and malicious-peer campaign coverage.
- Universal production signing/notarization setup for every release environment.
- Distributed region/tick simulation.
- Bedrock Edition support.

---

## Roadmap phase assessment

The roadmap is intentionally aspirational and phases have not landed in a perfectly linear order.

| Phase | Current assessment | Notes |
| --- | --- | --- |
| 0 — Research/protocol skeleton | Complete for preview | Core identity, storage, signed state and deterministic protocol machinery are established. |
| 1 — Peer networking | Mostly complete | Core networking stack is implemented; exact 1 GiB resume exit criterion is not the main permanent CI gate. |
| 2 — Snapshot swarm | Mostly complete | Real replication exists; multi-source reconstruction/retention maturity remains. |
| 3 — Minecraft save integration | Complete for preview | Fabric IPC, restore, save barrier and final snapshot flow are implemented and tested. |
| 4 — Manual host migration | Partial | State-machine support exists; complete user/runtime transfer workflow does not. |
| 5 — Automatic host migration | Control plane complete, product flow partial | Recovery/election/fencing are real; successor Minecraft launch + player reconnect remain. |
| 6 — World sleep/wake | Mostly complete internally | Durable sleep/wake semantics exist; orchestration is not yet invisible to players. |
| 7 — Solo mode | Complete for preview | Explicit solo history, reconciliation and divergence preservation are implemented/tested. |
| 8 — Better replication | Early/partial | Background replica support exists; incremental/journal/erasure-code work remains. |
| 9 — NAT/public usability | Partial | Protocol support exists; representative field certification does not. |
| 10 — UX | Partial | Desktop UI exists; launcher/runtime automation and lobby experience remain. |
| 11 — Production hardening | Partial | CI/audit/process recovery are strong; fuzzing/soak/signing/field validation remain. |
| 12 — Distributed simulation | Not implemented | Deliberately future research. |

---

## Current MVP boundary

The central MVP demo is not considered fully complete until all of the following happen as one end-to-end flow:

1. one peer hosts a world;
2. another peer holds a verified replica;
3. the authority process is hard-killed;
4. the successor safely wins authority;
5. the successor automatically launches the correct Minecraft runtime;
6. players reconnect and continue from the accepted state;
7. the stale peer later returns and synchronizes without being able to overwrite canonical history;
8. the world can later wake from replicated state without the original creator.

The repository already proves steps 2, 4 and 7 at the distributed control-plane level and implements much of the supporting host/sleep machinery. Steps 5 and 6 are the largest remaining end-to-end gap.

---

## Claim discipline

When describing SwarmCraft externally:

Safe claims:

- authenticated decentralized world replication is implemented;
- signed snapshots and world history are implemented;
- quorum-backed authority recovery and fencing are implemented;
- solo-history conflict preservation is implemented;
- Fabric save/lifecycle integration is implemented;
- a Tauri desktop technical preview and cross-platform installers exist.

Claims to avoid for now:

- "host migration is seamless";
- "players automatically reconnect after any host crash";
- "works behind every NAT without configuration";
- "production ready";
- "public decentralized lobby is complete";
- "mods install automatically";
- "Minecraft simulation itself is distributed across peers".

Those distinctions are deliberate. SwarmCraft should under-claim rather than blur protocol capability into product completeness.
