# SwarmCraft 0.4.0 Implementation Status

This document is the current source of truth for what the repository **actually implements** versus what remains roadmap or product-vision work.

Application version and wire protocol version are separate concepts. SwarmCraft 0.4.0 still uses protocol version 1 unless a protocol-breaking change explicitly requires otherwise.

## Executive summary

SwarmCraft is an **advanced technical preview**, not merely an architecture prototype and not yet a seamless consumer product.

The repository implements the difficult control-plane and runtime foundations:

- cryptographic peer/world identity;
- signed world configuration and membership;
- content-addressed snapshot storage and verification;
- authenticated libp2p/QUIC networking;
- snapshot replication and resumable transfer;
- authority leases, fencing and quorum-backed crash recovery;
- durable recovery ballots;
- explicit solo history and conflict preservation;
- Fabric lifecycle/save integration;
- backend-managed Java/Minecraft/Fabric runtime setup;
- explicit EULA handling and deterministic server-mod readiness;
- shared authority-migration/runtime orchestration;
- existing-world import;
- backend Host Readiness;
- a player-facing Tauri desktop shell;
- cross-platform CI and installer packaging.

Safe authority recovery is now connected to automatic successor Minecraft runtime orchestration. The largest visible migration gap is **seamless client redirection/reconnection after the successor is running**, not successor runtime startup itself.

Two safety limitations are intentional: a two-voter world cannot automatically recover after one voter disappears because the survivor lacks majority quorum, and sleeping multi-member worlds remain fail-closed until a dedicated sleep-bound quorum wake protocol exists.

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
- Publication ownership pins bind in-progress blob publication to the owning transaction rather than only to a blob hash.
- Replica verification bounds decompression by the signed declared uncompressed size.
- GC coordination covers normal local publication as well as replication publication, with crash-stale lock handling and deterministic race regression coverage.

### Peer networking and replication

- libp2p runtime with TCP/Noise/Yamux and QUIC support.
- Authenticated signed application peer handshake on top of transport identity.
- Persisted transport identity across hard peer restarts.
- Replacement connections can supersede stale libp2p connections without losing application authentication.
- LAN mDNS discovery.
- Kademlia address/discovery support.
- Bootstrap peer configuration.
- AutoNAT state.
- DCUtR hole-punch support.
- Relay client/reservation and relay dialing support.
- Structured connectivity diagnostics backed by **current active paths**, not sticky historical-success booleans.
- Bootstrap/relay infrastructure is classified separately from application reachability.
- Bounded wire message sizes for high-risk request classes.
- Snapshot manifest negotiation.
- Missing-blob negotiation.
- Chunked blob transfer with resume offsets.
- Resume state is content-addressed and can continue from a different replica holding the same blob.
- Replica acknowledgements.
- Live join followed by immediate snapshot replication without requiring a reconnect.
- Fourth-peer reconstruction from surviving replicas is covered by a permanent acceptance gate.
- Corrupt replica data is rejected, and poisoned partial blobs are discarded so a clean retry can proceed from another replica.
- A permanent impaired-network gate proves reconnect/resume behavior under latency variation, packet loss and bandwidth shaping.
- A weekly/manual multi-gigabyte QUIC soak defaults to 2 GiB, repeatedly hard-restarts the sender, deliberately loses acknowledgements, re-authenticates the durable peer identity and resumes from the receiver's committed offset.

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
- Three-daemon hard-kill recovery is a permanent process-level acceptance gate.
- Two-member crash recovery intentionally returns `BlockedByQuorum` when one of two voters disappears; no one-of-two recovery shortcut exists.

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
- Safe Stop World reports success only after the save/checkpoint/shutdown barrier, process exit, canonical snapshot publication and durable sleep state.
- Corrupt/unreadable sleep state fails closed in direct host launch, standby and migration/runtime supervision.

### Managed runtime installation

- Backend-owned `swarmcraft-runtime` sidecar for status, plan, install, repair, verify and managed launch.
- Managed compatible Java resolution rather than relying on an inherited build JVM.
- Official Mojang/Fabric metadata resolution and artifact hashing.
- Fabric API preparation.
- SwarmCraft Fabric bridge resolution with exact version and checksum verification.
- Immutable `vX.Y.Z` release assets are preferred; the rolling `main-latest` snapshot is accepted only if it contains the exact requested versioned bridge JAR and checksum.
- OS-backed runtime install locking so crashed setup processes do not permanently wedge a world while concurrent live installers remain excluded.
- Rollback-safe artifact replacement.
- Explicit Minecraft server EULA acceptance; no implicit or automatic acceptance.
- Durable machine-local `RuntimeLaunchConfig` used by normal hosting and automatic migration.
- Runtime proof is not considered Host Ready until the exact configured runtime completes authenticated Fabric compatibility/readiness verification.

### Server-mod readiness

- Canonical third-party server-mod requirements bind exact mod ID, version, side/environment and artifact hash into world compatibility.
- Local JAR inspection reads Fabric metadata without executing mod code.
- Missing, wrong-version, wrong-hash, duplicate/conflicting, invalid or client-only required mods block readiness.
- Machine-local user-mod bytes remain separate from signed world history.
- Deleting/replacing a previously verified mod invalidates readiness rather than inheriting stale green proof.
- Arbitrary third-party mod bytes are **not** silently downloaded or redistributed.

### Migration and manual transfer

- Automatic authority recovery, manual authority transfer and supported wake paths share one Rust migration/runtime orchestration path.
- The successor restores the canonical snapshot, prepares its configured runtime, launches Minecraft, verifies Fabric readiness and publishes the running endpoint.
- Migration status is exposed to Desktop as structured phases.
- Desktop exposes the manual host-transfer action when backend capabilities and safety checks permit it; JavaScript does not grant authority itself.
- A migration successor that lacks valid runtime/mod/sleep prerequisites blocks rather than launching an unsafe host.

### Host Readiness

- `swarmcraft world host-readiness <world> --json` exposes the backend decision used by Desktop.
- A green successor must be a current reachable eligible member with the exact canonical snapshot/state, verified runtime, verified required mods, no conflict and a recovery quorum that survives without the current authority.
- Stale historical reachability is not reused as a green decision.
- Two-member Alice/Bob shutdown safety is fail-closed when Bob would be left without quorum; explicit handoff remains separate from crash recovery.
- Runtime or mod mutation after verification invalidates readiness.

### Existing-world import

- Typed Rust import path plus packaged `swarmcraft-import` sidecar.
- Desktop exposes **Import existing world** as a normal player flow.
- Import validates exact compatibility metadata and explicit third-party mod requirements.
- Source world bytes remain unchanged.
- Canonical state is assembled and verified in hidden staging, then published atomically.
- Failed/interrupted imports do not expose a visible half-world and can be retried safely.
- EULA state, Java/runtime binaries and `RuntimeLaunchConfig` are deliberately not imported.
- Imported worlds return to the normal Runtime Wizard + Play flow.

### Desktop application

- Tauri 2 desktop shell.
- Four bundled SwarmCraft sidecars: `swarmcraft`, `swarmcraft-host`, `swarmcraft-runtime`, `swarmcraft-import`.
- World list and selected-world detail UI.
- Create world flow.
- Existing-world import flow.
- Signed invite creation.
- Signed invite join flow.
- Leave request flow.
- Runtime Wizard for backend-managed setup and explicit EULA handling.
- Play/host flow with authority-eligibility checks.
- Manual host-transfer action backed by migration APIs.
- Graceful sleep/stop controls.
- Background seeding toggle.
- Backend-derived Host Readiness display.
- Runtime/server-mod remediation UI.
- World safety display.
- Compatibility display.
- Preserved-conflict inspection.
- Peer/membership inspection.
- Snapshot verification, export and recovery controls.
- Replication daemon controls and structured connectivity diagnostics.

### CI and packaging

- Linux, Windows and macOS Rust build/test coverage.
- Formatting and strict Clippy gates.
- RustSec dependency audit against the committed lockfile.
- Process-level acceptance tests for:
  - hard peer restart with stable transport identity, re-authentication and authenticated request recovery;
  - hostile network input and handshake hardening;
  - fourth-peer snapshot reconstruction from surviving replicas, including missing/corrupt-source fallback and cross-replica resume;
  - publication ownership, replica verification, GC/retention races and storage failure injection;
  - existing-world import;
  - direct, standby and migration corrupt-sleep fail-closed behavior;
  - Host Readiness negative states and two-member quorum behavior;
  - live join and immediate replication;
  - host lifecycle and final sleep snapshot;
  - migration orchestration and runtime setup failure hardening;
  - three-daemon hard-kill authority recovery;
  - recovery successor disappearing before epoch promotion;
  - solo-history acceptance and divergence detection.
- Dedicated QUIC impairment gate with latency variation, packet loss, bandwidth limiting, repeated hard restarts and lost-ack resume recovery.
- Multi-gigabyte interrupted QUIC soak on its dedicated scheduled/manual workflow.
- Fabric server-mod build plus embedded Fabric API verification.
- Native Desktop package builds for Linux `.deb` + AppImage, Windows NSIS, macOS ARM64 `.dmg`, and macOS x86_64 `.dmg`.
- Rolling `main-latest` development snapshot workflow.
- Tagged release workflow support for checksums, versioned Fabric artifacts and optional platform signing credentials.
- Main and tagged Desktop packaging stage the same four required runtime sidecars.

---

## Partially implemented

These areas contain real code but do **not** yet satisfy the full product/roadmap exit criteria.

### Seamless automatic host migration

The distributed control plane can elect and fence a new authority, and the shared migration path can automatically start the successor's configured Minecraft runtime after a safe authority transition.

Still incomplete:

- universally directing/reconnecting Minecraft clients to the new running authority without manual coordination;
- broader repeated real-Minecraft crash/migration campaigns across multiple physical devices and network conditions.

The remaining product gap is therefore client continuity and field evidence, not basic successor runtime startup.

### Manual authority transfer

The backend authority-transfer machinery, shared runtime orchestration and Desktop transfer action exist.

Still incomplete relative to a polished consumer flow:

- richer target-selection/readiness guidance where several successors are available;
- seamless player reconnection UX during the handoff;
- broader real-device transfer acceptance across adverse networks.

### World sleep/wake

Durable signed sleep records and safe single-member wake semantics exist.

For more than one non-banned member, wake intentionally remains blocked because a dedicated quorum-backed transition bound to the sleep record/canonical snapshot does not yet exist. SwarmCraft does not substitute first-click-wins or ordinary crash recovery for that missing protocol.

### Snapshot swarm breadth

The roadmap-shaped fourth-peer reconstruction scenario is automated: a source replica can disappear, surviving replicas can have asymmetric availability, corrupt data is rejected, a partial transfer can resume from a different replica, and the new peer restores the exact verified world.

Publication ownership, GC coordination and retention races now have stronger correctness gates. Remaining maturity work can still include:

- a production scheduler that downloads different missing blobs from multiple peers in parallel rather than relying primarily on source fallback;
- longer retention/GC soak campaigns under sustained churn;
- broader hardware/disk-failure profiles beyond deterministic failure injection.

### NAT traversal and internet usability

The code supports AutoNAT, DCUtR, direct-first dialing and relays, with current-path structured diagnostics.

This must **not** be described as universally proven NAT traversal. Representative field testing across real home routers, symmetric NAT, CGNAT, mobile hotspots, firewall policies, blocked UDP and independent-ISP IPv6 networks remains pending. See [NETWORK_VALIDATION.md](NETWORK_VALIDATION.md).

### Desktop product experience

The desktop application now owns the normal managed Runtime Wizard path, import, Host Readiness and migration presentation rather than requiring ordinary users to manually assemble Java/Minecraft/Fabric paths.

Technical-preview friction remains around:

- explicit EULA acceptance, which is intentionally required;
- locally supplying arbitrary third-party server-mod JARs when a world requires them;
- seamless client reconnection after migration;
- safe multi-member wake;
- public/friend discovery and broader first-run polish.

---

## Not implemented yet

- Seamless automatic Minecraft client reconnection/redirection after authority migration.
- Safe sleep-bound quorum wake election for multi-member worlds.
- Central or federated public-world search/lobby services.
- Friends/social discovery.
- Automatic third-party mod redistribution.
- Operation-level Minecraft journal replication.
- Incremental region/chunk replication as the primary recovery path.
- Erasure-coded world storage.
- Production-grade metrics/telemetry infrastructure.
- Universal production signing/notarization setup for every release environment.
- Comprehensive real-world NAT/carrier certification.
- Distributed region/tick simulation.
- Bedrock Edition support.

---

## Roadmap phase assessment

The roadmap is intentionally aspirational and phases have not landed in a perfectly linear order.

| Phase | Current assessment | Notes |
| --- | --- | --- |
| 0 — Research/protocol skeleton | Complete for preview | Core identity, storage, signed state and deterministic protocol machinery are established. |
| 1 — Peer networking | Complete for preview | Authenticated networking, durable reconnect, resume semantics and impaired multi-GiB transfer are gated; representative NAT/carrier certification is tracked in Phase 9. |
| 2 — Snapshot swarm | Mostly complete | Fourth-peer reconstruction, corruption rejection, cross-replica resume and stronger publication/GC safety are gated; parallel scheduling and long-duration retention maturity remain. |
| 3 — Minecraft save integration | Complete for preview | Fabric IPC, restore, save/shutdown barrier and final snapshot flow are implemented and tested. |
| 4 — Manual host migration | Implemented for preview | Backend transfer, shared runtime orchestration and Desktop action exist; richer handoff/reconnection UX remains. |
| 5 — Automatic host migration | Runtime path complete for preview, client UX partial | Recovery/election/fencing plus successor runtime startup are real; seamless player reconnect remains. |
| 6 — World sleep/wake | Safe solo path + fail-closed multi-member | Durable sleep semantics exist; multi-member quorum wake protocol remains intentionally unavailable. |
| 7 — Solo mode | Complete for preview | Explicit solo history, reconciliation and divergence preservation are implemented/tested. |
| 8 — Better replication | Partial | Background replicas, resume and source fallback exist; incremental/journal/erasure-code work remains. |
| 9 — NAT/public usability | Partial | Protocol support and diagnostics exist; representative field certification does not. |
| 10 — UX | Advanced preview | Managed runtime, import, readiness and migration UI exist; reconnect/lobby/multi-member wake polish remains. |
| 11 — Production hardening | Partial/strong preview | CI, fuzz smoke, failure injection and process recovery are strong; longer campaigns, signing and field validation remain. |
| 12 — Distributed simulation | Not implemented | Deliberately future research. |

---

## Current MVP boundary

The crash-recovery demo must use a topology that can genuinely retain majority quorum. A representative target is:

1. Alice hosts a world;
2. Bob and Carol hold verified replicas;
3. Alice's authority process is hard-killed;
4. Bob safely wins authority with a surviving quorum;
5. Bob automatically restores/prepares/launches the correct Minecraft runtime;
6. players reconnect and continue from the accepted state;
7. stale Alice later returns and synchronizes without being able to overwrite canonical history;
8. the world can later sleep and a supported peer can restore it without the original creator.

The repository now proves the storage/replication, recovery/fencing and successor-runtime portions of that sequence and separately proves the real clean-machine Minecraft setup/launch/stop/restart path. Seamless client reconnection in step 6 and safe multi-member wake remain the major product/protocol gaps.

An Alice/Bob-only crash test is **not** a valid positive automatic-failover target: after Alice disappears, Bob alone is one of two voters and must remain `BlockedByQuorum`.

---

## Claim discipline

When describing SwarmCraft externally:

Safe claims:

- authenticated decentralized world replication is implemented;
- peer reconnect/resume behavior is permanently tested under synthetic impairment and multi-gigabyte transfer;
- signed snapshots and world history are implemented;
- quorum-backed authority recovery and fencing are implemented;
- automatic successor Minecraft runtime orchestration after a safe authority transition is implemented;
- managed Java/Minecraft/Fabric setup with explicit EULA acceptance is implemented;
- deterministic server-mod readiness and existing-world import are implemented;
- safe stop/sleep and corrupt-sleep fail-closed behavior are implemented;
- solo-history conflict preservation is implemented;
- Fabric save/lifecycle integration is implemented;
- a Tauri desktop technical preview and cross-platform installers exist.

Claims to avoid for now:

- "host migration is seamless";
- "players automatically reconnect after any host crash";
- "two-player worlds can always fail over after either player crashes";
- "multi-member sleeping worlds use first-click automatic wake";
- "works behind every NAT without configuration";
- "production ready";
- "public decentralized lobby is complete";
- "third-party mods install automatically";
- "Minecraft simulation itself is distributed across peers".

Those distinctions are deliberate. SwarmCraft should under-claim rather than blur protocol capability into product completeness.
