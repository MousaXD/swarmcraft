# SwarmCraft Desktop

SwarmCraft Desktop is the player-facing Tauri 2 launcher for replicated Minecraft worlds.

The frontend intentionally stays small and dependency-free:

```text
apps/desktop/
├── src/
│   ├── index.html          # semantic launcher structure
│   ├── style.css           # shared visual/component system
│   ├── backend-adapter.js  # Tauri contract and future migration seam
│   └── app.js              # state rendering and interactions
├── tests/
│   └── frontend-contract.test.mjs
└── src-tauri/
    ├── src/
    └── tauri.conf.json
```

## Player flows

The launcher centers normal use around:

- creating a world;
- joining with a signed invite;
- choosing a world and playing;
- inviting friends;
- seeing safety, host and connectivity summaries;
- keeping or verifying a background replica;
- sleeping/stopping a running world;
- preparing for player-facing host transfer and migration progress;
- opening advanced diagnostics only when technical setup or recovery is needed.

Conflict, solo/degraded, canonical, authority-eligibility, membership and relay semantics remain distinct. A storage replica never implies authority eligibility, and discovery never implies membership.

## Backend integration

`src/backend-adapter.js` is the only frontend seam for Tauri command contracts. Existing command names and payload shapes remain unchanged.

Migration-core is intentionally not simulated in JavaScript. The adapter currently reports migration status/transfer/wake capabilities as unavailable. The UI already knows how to render the agreed migration phases:

- Preparing successor
- Saving world
- Transferring authority
- Restoring world
- Starting Minecraft
- Waiting for host
- Ready
- Migration failed

When migration-core exposes desktop commands, connect them in the adapter and enable the corresponding capabilities rather than implementing authority or recovery decisions in frontend code.

Structured connectivity is consumed from world status when the backend reports fields such as `Connectivity`, `Connectivity state`, `Connection`, `Network path`, or `Reachability`. The player-facing states are Direct, Relay, Connecting, Offline, Limited connectivity, and Action required. When no structured state exists, the UI says that it is not reported rather than guessing.

## Current runtime limitation

The current direct-host Play command can still require advanced runtime setup:

- Java executable;
- compatible Fabric server JAR;
- SwarmCraft Fabric mod JAR;
- Minecraft server EULA acceptance.

Those controls live in Diagnostics instead of the primary world flow. Automatic migration/wake remains unavailable until migration-core exposes the backend contract.

## Validation

Run the frontend contract checks with:

```text
node --test apps/desktop/tests/frontend-contract.test.mjs
```

For the Rust/Tauri shell, also run when the required Rust dependencies are available:

```text
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Visual changes should be inspected at the configured default `980x760` window and minimum `720x560` window. The Tauri global bridge must remain enabled with `app.withGlobalTauri: true` because the frontend consumes `window.__TAURI__.core.invoke`.
