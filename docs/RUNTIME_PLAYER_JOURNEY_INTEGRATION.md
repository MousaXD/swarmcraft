# Runtime Player Journey Integration

This document records the integrated 0.4.0 player-journey contract that is being prepared for `main`.

## Normal player path

**Play** uses backend Runtime Installer status and the Runtime Wizard. Runtime setup is owned by Rust, including compatible Java, official Minecraft/Fabric resolution, Fabric API, the SwarmCraft Fabric bridge, managed directories, explicit EULA state and durable launch configuration.

Runtime verification and required server-mod verification are separate fail-closed Host Readiness boundaries. Desktop does not infer readiness from paths or historical success.

## Shared launch, migration and stop

Managed launch, automatic authority recovery, manual transfer and supported wake paths share Rust runtime/migration orchestration rather than maintaining independent JavaScript launch logic.

**Stop World** reports success only after the Fabric save/shutdown barrier, Minecraft process exit, final signed canonical snapshot, durable signed sleep record and sleeping migration state.

Corrupt or unreadable sleep state is never interpreted as awake. Direct launch, standby and migration block fail-closed.

## Existing-world import

`swarmcraft-import` and the Desktop `import_world` path safely stage, verify and atomically publish imported canonical world state. Import leaves the source world unchanged and does not import EULA or machine-local runtime configuration.

## Host Readiness

The Desktop question **Can I turn off this PC?** renders structured backend Host Readiness. A safe successor must have current reachable membership, exact canonical state, authority eligibility, verified runtime, verified required mods, no conflict and a recovery quorum that survives without the current authority.

A two-member Alice/Bob world therefore remains `BlockedByQuorum` for crash failover. That is an intentional safety result, not a missing frontend shortcut.

## Packaging

All supported Desktop targets bundle four Rust sidecars:

- `swarmcraft`
- `swarmcraft-host`
- `swarmcraft-runtime`
- `swarmcraft-import`

The Fabric artifact embeds Fabric API for the normal path. Main snapshots and tagged releases publish the versioned SwarmCraft Fabric bridge plus SHA-256 checksum so managed runtime setup can resolve the exact adapter version.

## CI and live acceptance

Normal CI covers Rust fmt/clippy/tests, storage/network/process acceptance, import, corrupt-sleep regressions, Host Readiness, migration/recovery, all Desktop tests and packages, Fabric verification, dependency audit, fuzz smoke and impaired QUIC resume.

`player-journey-live.yml` also runs for main-bound pull requests and exercises a fresh data directory against official services with managed Java, explicit EULA, real Minecraft/Fabric launch, safe stop, restart/restore and canonical snapshot advancement.

## Intentional YELLOW gates

- Two-voter crash failover remains `BlockedByQuorum`.
- Multi-member wake remains fail-closed until a sleep-bound quorum wake protocol exists.
- Seamless client reconnection and representative real-world NAT certification remain future validation/product work.

None of these limitations justify weakening quorum, fencing, signed history or runtime verification.
