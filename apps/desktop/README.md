# SwarmCraft Desktop

SwarmCraft Desktop is the player-facing Tauri 2 launcher for replicated Minecraft worlds.

The frontend intentionally stays small and dependency-free:

```text
apps/desktop/
├── src/
│   ├── index.html          # semantic launcher structure
│   ├── style.css           # shared visual/component system
│   ├── backend-adapter.js  # Tauri/backend contract boundary
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
- seeing backend-reported host migration progress;
- safely waking a sleeping world when the backend exposes that capability;
- opening advanced diagnostics only when technical setup or recovery is needed.

Conflict, solo/degraded, canonical, authority-eligibility, membership and relay semantics remain distinct. A storage replica never implies authority eligibility, and discovery never implies membership.

## Backend integration

`src/backend-adapter.js` is the frontend boundary for Tauri command contracts. Authority, recovery and quorum decisions stay in Rust.

The desktop now capability-probes the bundled `swarmcraft` CLI before enabling migration features. When migration-core is present it can consume:

- `swarmcraft world migration-status <world> --json` for authoritative migration/runtime state;
- `swarmcraft world wake <world>` for the backend-validated wake request.

If those commands are absent, migration status and wake remain disabled instead of being simulated in JavaScript. The finalized migration-core `snake_case` phases are translated into the smaller player-facing progress model used by the launcher.

Manual authority transfer remains intentionally disabled in the desktop adapter for now. Migration-core implements transfer as a signed, multi-stage prepare/export/accept/commit/activate/observe exchange. The frontend must not collapse that protocol into a fake one-click authority change.

Structured connectivity is consumed from world status when the backend reports fields such as `Connectivity`, `Connectivity state`, `Connection`, `Network path`, or `Reachability`. The player-facing states are Direct, Relay, Connecting, Offline, Limited connectivity, and Action required. When no structured state exists, the UI says that it is not reported rather than guessing.

## Runtime setup

The Play command launches the bundled `swarmcraft-host` runtime. After migration-core integration that binary routes hosting through the shared Rust runtime path; the desktop does not duplicate launch or fencing decisions.

The local runtime can still require advanced setup:

- Java executable;
- compatible Fabric server JAR;
- SwarmCraft Fabric mod JAR;
- Minecraft server EULA acceptance.

Those controls live in Diagnostics instead of the primary world flow.

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
