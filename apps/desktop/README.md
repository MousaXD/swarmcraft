# SwarmCraft Desktop

SwarmCraft Desktop is the player-facing Tauri 2 launcher for replicated Minecraft worlds.

The frontend intentionally stays small and dependency-free:

```text
apps/desktop/
├── src/
│   ├── index.html          # semantic launcher structure
│   ├── style.css           # shared visual/component system
│   ├── backend-adapter.js  # Tauri/backend contract boundary
│   ├── import-flow.js      # typed import request/result helpers
│   ├── runtime-wizard.js   # managed runtime setup presentation
│   └── app.js              # state rendering and interactions
├── tests/
│   ├── frontend-contract.test.mjs
│   ├── import-flow.test.mjs
│   └── runtime-wizard.test.mjs
└── src-tauri/
    ├── src/
    └── tauri.conf.json
```

## Player flows

The launcher centers normal use around:

- creating a world;
- importing an existing Minecraft world through the typed Rust importer;
- joining with a signed invite;
- choosing a world and playing;
- running the managed Runtime Wizard when Java/Minecraft/Fabric setup is incomplete;
- explicitly accepting the Minecraft server EULA when required;
- inviting friends;
- seeing safety, host and structured connectivity summaries;
- seeing backend-derived **Can I turn off this PC?** Host Readiness;
- keeping or verifying a background replica;
- verifying/remediating required server mods;
- sleeping/stopping a running world through the safe durability barrier;
- seeing backend-reported host migration progress;
- transferring authority when the backend exposes the transfer capability and safety checks permit it;
- safely waking a sleeping world only when the backend exposes a valid wake path;
- opening advanced diagnostics for technical setup, manual overrides or recovery.

Conflict, solo/degraded, canonical, authority-eligibility, membership and relay semantics remain distinct. A storage replica never implies authority eligibility, and discovery never implies membership.

## Backend integration

`src/backend-adapter.js` is the frontend boundary for Tauri command contracts. Authority, recovery, quorum, fencing, runtime verification and mod-readiness decisions stay in Rust.

The Desktop consumes structured backend capabilities and state including:

- `swarmcraft world migration-status <world> --json` for migration/runtime state;
- the migration transfer backend for the player-facing host-transfer action;
- `swarmcraft world wake <world>` for backend-validated wake intent;
- `swarmcraft world host-readiness <world> --json` for shutdown safety;
- `swarmcraft-runtime` status/plan/install/repair/verify/launch for managed runtime setup;
- `swarmcraft-import` through the Tauri `import_world` bridge for existing saves;
- structured connectivity diagnostics from the networking backend.

If a backend capability is absent, the matching Desktop action stays unavailable instead of being simulated in JavaScript. The finalized migration-core `snake_case` phases are translated into the smaller player-facing progress model used by the launcher.

Manual authority transfer is no longer a fake frontend shortcut. `transferHost()` calls the migration adapter, and the backend remains responsible for the signed transition and authority checks. Desktop merely exposes the action when capability and host eligibility allow it.

Structured connectivity uses backend path state rather than guessing from peer counts. Player-facing presentation distinguishes direct, relayed, connecting, offline/limited and actionable failure states while preserving the underlying backend diagnostics for Advanced views.

## Runtime setup

Normal **Play** uses the backend-managed Runtime Wizard. The player is not expected to manually hunt for Java, Minecraft server, Fabric Loader, Fabric API or the SwarmCraft Fabric bridge on the normal path.

The packaged `swarmcraft-runtime` sidecar owns:

- structured runtime inspection and planning;
- compatible managed Java resolution;
- official Minecraft/Fabric/Fabric API preparation;
- exact SwarmCraft Fabric bridge resolution and checksum verification;
- explicit EULA state;
- repair and verification;
- durable machine-local launch configuration;
- launch through the shared Rust authority/runtime orchestration path.

The Desktop never auto-accepts the Minecraft EULA. When backend state says EULA acceptance is required, Runtime Wizard requires an explicit player checkbox before requesting acceptance.

Manual/Advanced runtime configuration remains available as a fallback for power users, but path existence alone is not Host Readiness. The exact configured runtime must launch and complete the authenticated Fabric compatibility/readiness handshake before its runtime proof becomes green.

## Server mods

Canonical third-party server-mod requirements are backend-owned. Desktop can show readiness and help a player add/remove a local copy that matches an already-canonical requirement; it does not rewrite the signed modpack or silently redistribute arbitrary third-party JARs.

Runtime readiness and server-mod readiness are separate fail-closed Host Readiness boundaries.

## Existing-world import

**Import existing world** is a normal launcher flow backed by the Rust importer.

The player provides the source directory plus exact Minecraft/Fabric compatibility and explicitly declares third-party server-mod requirements. Import publishes canonical world data atomically and leaves the source save unchanged.

EULA acceptance, Java/runtime binaries and `RuntimeLaunchConfig` are intentionally not imported. After import, the world enters the same Runtime Wizard + Play path as any other world.

## Host Readiness and migration safety

The player-facing shutdown question renders the backend `HostReadinessReport`; JavaScript does not recompute whether another device can safely take over.

For exactly two voting members, one survivor after a crash is not a quorum. Desktop therefore preserves `BlockedByQuorum` instead of showing a false **Safe to shut down** result. An explicit authority transfer while both peers are present is a different operation.

Multi-member wake also remains fail-closed until the backend implements a sleep-bound quorum wake transition. Desktop must not implement first-click-wins wake behavior.

## Packaged sidecars

Every supported Desktop bundle requires all four Tauri external binaries:

- `swarmcraft`;
- `swarmcraft-host`;
- `swarmcraft-runtime`;
- `swarmcraft-import`.

The release/package workflows must stage the same set on Linux, Windows, macOS ARM64 and macOS x86_64.

## Validation

Run all Desktop JavaScript contract checks with:

```text
node --test apps/desktop/tests/*.test.mjs
```

For the Rust/Tauri shell, also run when the required native dependencies are available:

```text
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Normal CI runs the full JavaScript test glob and packages the Desktop application on Linux, Windows, macOS ARM64 and macOS x86_64 with the four required sidecars staged.

Visual changes should be inspected at the configured default `980x760` window and minimum `720x560` window. The Tauri global bridge must remain enabled with `app.withGlobalTauri: true` because the frontend consumes `window.__TAURI__.core.invoke`.
