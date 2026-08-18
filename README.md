# SwarmCraft

> A decentralized Minecraft world that survives its host.

**Current application version: 0.4.0 technical preview.** Wire protocol version remains `1`.

SwarmCraft combines signed world history, content-addressed replication, temporary fenced authority, a Fabric lifecycle bridge, and a Tauri desktop launcher. The normal player journey now includes backend-managed Java/Minecraft/Fabric preparation, explicit Minecraft EULA acceptance, verified runtime/mod readiness, safe stop/sleep, existing-world import, and automatic successor runtime orchestration after a quorum-backed authority transition.

## What 0.4.0 implements

- persistent Ed25519 peer identity and deterministic world identity;
- signed membership, configuration, snapshot, epoch, lease, recovery, sleep and solo-history records;
- content-addressed BLAKE3/Zstandard snapshot storage with corruption verification;
- authenticated libp2p/QUIC networking with mDNS, Kademlia, AutoNAT, DCUtR and relay support;
- resumable snapshot replication, source fallback and fourth-peer reconstruction;
- quorum-backed authority recovery, monotonic epochs and fencing tokens;
- shared migration/runtime orchestration for automatic recovery, manual transfer and supported wake paths;
- Fabric save/shutdown barriers, restore, final canonical snapshot and durable sleep records;
- fail-closed corrupt/unreadable sleep-state handling across direct launch, standby and migration;
- backend Host Readiness for the player-facing “Can I turn off this PC?” decision;
- managed Runtime Installer for compatible Java, Minecraft, Fabric Loader, Fabric API and the SwarmCraft Fabric bridge;
- explicit Minecraft EULA handling, never automatic acceptance;
- deterministic third-party server-mod requirements and local hash verification;
- safe existing-world import through the Rust backend and normal Desktop flow;
- Tauri desktop packages for Linux, Windows, macOS Apple Silicon and macOS Intel;
- four bundled sidecars on every Desktop target: `swarmcraft`, `swarmcraft-host`, `swarmcraft-runtime`, and `swarmcraft-import`.

## Deliberate limitations

SwarmCraft 0.4.0 is still a technical preview, not a claim of universal production readiness.

- **Two-voter crash failover stays fail-closed.** In an Alice/Bob world, Bob alone cannot form the majority quorum after Alice disappears. Use explicit authority transfer while both peers are present, or a three-member topology for automatic crash recovery.
- **Multi-member wake stays fail-closed.** A sleep-bound quorum wake election is not implemented yet. SwarmCraft does not use first-click-wins wake semantics.
- **Player reconnection after migration is not yet seamless.** The successor runtime can be orchestrated automatically, but clients are not universally redirected/reconnected without coordination.
- **Real internet connectivity still needs field certification.** Automated QUIC impairment and relay/DCUtR coverage are strong, but they are not proof for every home NAT, CGNAT, carrier, firewall or IPv6 environment.
- **Third-party server-mod bytes are not redistributed automatically.** Canonical requirements are synchronized; players provide required JARs locally and SwarmCraft verifies exact metadata/hash compatibility.
- Production Authenticode and Apple notarization depend on repository release credentials. Development snapshots clearly identify unsigned/ad-hoc signing status.

## Player journey

A normal fresh-world path is:

1. Create or join a world.
2. Press **Play**.
3. Runtime Wizard inspects backend state.
4. Accept the Minecraft EULA explicitly when required.
5. SwarmCraft prepares and verifies the managed runtime.
6. Required third-party server mods are verified.
7. Minecraft launches through the shared Rust authority/runtime path.
8. **Stop World** waits for the Fabric save/shutdown barrier, final signed canonical snapshot, and durable sleep record before reporting success.

Existing saves can enter through **Import existing world**. Import copies world data into canonical SwarmCraft state but deliberately does not import EULA acceptance or machine-local runtime configuration.

## Current Minecraft target

- Minecraft Java `26.1.2`
- Fabric Loader `0.19.3`
- Fabric API `0.155.2+26.1.2`
- Java `25+`

Per-world signed compatibility metadata may impose additional exact mod requirements.

## Validation

Normal CI covers Rust formatting/lint/tests, process-level storage/network/authority/migration acceptance, corrupt-sleep regressions, existing-world import, Desktop frontend tests, native package builds on all supported targets, Fabric build + embedded Fabric API verification, dependency audit, fuzz smoke, and impaired QUIC resume.

A separate live acceptance workflow exercises a fresh data directory against official Minecraft/Fabric/Adoptium services, requires managed Java resolution, launches real Minecraft twice, persists runtime configuration, restores known world data, and stops through the safe snapshot/sleep barrier.

See:

- [Final player-journey acceptance](docs/FINAL_PLAYER_JOURNEY_ACCEPTANCE.md)
- [Implementation status](docs/IMPLEMENTATION_STATUS.md)
- [Release gates](docs/RELEASE_GATES.md)
- [Network validation](docs/NETWORK_VALIDATION.md)
- [Architecture](ARCHITECTURE.md)
- [Protocol](PROTOCOL.md)
- [Security model](SECURITY.md)

## License

Apache-2.0. See [LICENSE](LICENSE).
