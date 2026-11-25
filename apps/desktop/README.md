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
- safely waking a sleeping world only when the backend exposes a valid wake path;
- opening advanced diagnostics for technical setup, manual overrides or recovery.

Conflict, solo/degraded, canonical, authority-eligibility, membership and relay semantics remain distinct. A storage replica never implies authority eligibility, and discovery never implies membership.

## Backend integration

`src/backend-adapter.js` is the frontend boundary for Tauri command contracts. Authority, recovery, quorum, fencing, runtime verification and mod-readiness decisions stay in Rust.

The Desktop consumes structured backend capabilities and state including:

- `swarmcraft world migration-status <world> --json` for migration/runtime state;
- `swarmcraft world wake <world>` for backend-validated wake intent;
- `swarmcraft world host-readiness <world> --json` for shutdown safety;
- `swarmcraft-runtime` status/plan/install/repair/verify/launch for managed runtime setup;
- `swarmcraft-import` through the Tauri `import_world` bridge for existing saves;
- structured connectivity diagnostics from the networking backend.

If a backend capability is absent, the matching Desktop action stays unavailable instead of being simulated in JavaScript. The finalized migration-core `snake_case` phases are translated into the smaller player-facing progress model used by the launcher.

Manual authority transfer is **not yet a complete Desktop flow**. Migration Core has a signed staged transfer protocol and shared runtime orchestration, but the current Tauri capability probe exposes migration status and wake only, and the Desktop adapter keeps `transferAuthority` unavailable. The frontend must not collapse the backend transfer protocol into a fake one-click authority change.

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

Close Minecraft or the source server first. Choose **Browse for world folder** (native folder picker on Linux, Windows and macOS), or paste the path to the actual save containing `level.dat` and `session.lock`. Enter a display name, visibility, exact Minecraft/Fabric versions and every required third-party server-mod JAR, or explicitly confirm that no third-party server mods are required.

Select **Check import** to review the source file count, approximate uncompressed size, declared runtime and minimum Java requirement, and required mod identities. This uses a read-only Rust preflight and does not create a world or initialize its storage. The versions and mods are user declarations, not inferred from the contents of the save. The estimate is not a disk reservation or a hosting-readiness guarantee.

Select **Import world** only after checking the review. Changing any form value invalidates the previous review. The Rust importer reacquires the existing source lock and validates compatibility against the reviewed fingerprint before atomic canonical publication. The fingerprint binds compatibility, not the source contents; the source is checked again rather than assumed unchanged since review. Import leaves the original save unchanged.

EULA acceptance, Java/runtime binaries and `RuntimeLaunchConfig` are intentionally not imported. After import, the world enters the same Runtime Wizard + Play path as any other world.

## Minecraft client setup

Select your world, then expand **Minecraft client setup**. The requirements come from the locally retained creator-signed world configuration and its immutable genesis compatibility fingerprint. They are not a live authorization to connect or host.

Choose Minecraft Launcher, Prism Launcher or another Java Edition launcher. Follow the instructions to create a separate installation/instance and game directory, select the exact Minecraft and Fabric versions and the game-appropriate Java runtime, and obtain the listed client/both-side JARs from authorized sources. Keep your existing launcher account and other instances unchanged. The server-only SwarmCraft Fabric bridge is not a client mod.

Optionally enter the full path to that instance's existing `mods` folder and select **Check client profile**. SwarmCraft inspects only its top-level regular JARs, compares the signed IDs, versions and domain-separated artifact hashes, and reports missing, duplicate, unexpected, invalid or mismatched files. The check is a point-in-time inventory observation, not proof that dependencies resolve, that the launcher actually uses the selected folder, or that Java, the game, login or the network connection works. No launcher configuration, account files or game binaries are read or written, and no mods or launchers are installed or started. Use a separate instance rather than deleting or overwriting your usual mods.

Equivalent read-only client inventory request via the runtime sidecar:

```sh
swarmcraft-runtime client-profile <WORLD_ID>
swarmcraft-runtime client-profile <WORLD_ID> --mods-dir /absolute/path/to/instance/mods
```

Launch Minecraft yourself through your chosen launcher. Use Multiplayer and the current verified reconnect instructions in SwarmCraft; the compatibility report does not grant a route or enable hosting.

## Minecraft / Fabric compatibility matrix

New Desktop Create and Import offer the pinned Minecraft 26.1.2 / Fabric Loader 0.19.3 baseline (reference Java 25). The adapter metadata declares a broader technical range, but newer matching releases are not yet qualified for new worlds. Other official versions remain visible with unavailable, not-yet-qualified, or incompatible labels. Import does not convert or upgrade a save; declare only the versions actually used by that save. Existing signed profiles remain unchanged.

Inspect the build's selection policy without initializing a data directory:

```sh
swarmcraft-runtime compatibility-matrix
```

The pinned baseline is a release candidate, not a claim of completed real Fabric lifecycle or authenticated Minecraft client acceptance. See the [qualification matrix and field acceptance requirements](../../research/runtime-compatibility/REPORT.md). Local installation, hosting consent, authority, and live authenticated readiness remain separate checks.

## Exact provider artifacts and manual fallback

When creating a world, the provider picker resolves required dependencies and selects exact versions. If the provider returns a restricted installable JAR with exact identity, size and hash metadata, Create asks for the absolute path to a manually obtained authorized copy before publishing the world. The native canonicalizer checks the declared bytes; the signed profile retains the manual-only restriction even when strong provider hashes exist. Do not use unofficial redistribution sources.

For an existing world, expand **Advanced world controls** and select **Acquire required server mods**. SwarmCraft checks the creator-signed compatibility, validates dependency sides, acquires permitted missing server artifacts directly from Modrinth or CurseForge, and compares their provider and canonical hashes, mod IDs and versions. CurseForge requires the machine-local `SWARMCRAFT_CURSEFORGE_API_KEY` environment variable; an unavailable key or provider is reported as an unavailable acquisition, never a substitute download. Explicit distribution refusal and local/imported files require manual supply.

The report identifies exact provider project/file IDs, filename and expected SwarmCraft artifact hash. Use **Supply required JAR** for an authorized exact local file. Successful partial acquisitions remain available on retry. No existing different artifact is overwritten. This operation checks server JARs only; it neither accepts EULA nor installs or starts the runtime, grants hosting consent, changes launcher accounts, or redistributes mod files through peers. Client setup continues to use the separate launcher workflow above.

```sh
swarmcraft-runtime acquire-mods <WORLD_ID>
```

The JSON response can report incomplete inventory even with a successful command exit. Check its `ready` field and per-artifact states; it is not a game-ready or live hosting authorization.

## Host Readiness and migration safety

The player-facing shutdown question renders the backend `HostReadinessReport`; JavaScript does not recompute whether another device can safely take over.

For exactly two voting members, one survivor after a crash is not a quorum. Desktop therefore preserves `BlockedByQuorum` instead of showing a false **Safe to shut down** result. Backend explicit authority transfer while both peers are present is a different operation, but its complete Desktop workflow remains future integration work.

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

## Startup contract and native acceptance

The HTML starts in **Starting…**, never **Ready**. A classic `bootstrap.js`
runs before ES modules so a failed import, synchronous JavaScript exception or
missing native bridge cannot leave `Loading…` / `Checking…` skeletons
indefinitely. The module marks each startup phase and reports `ready` only
after device initialization, identity verification, and a real world-list
command. Local-only operation is explicitly marked degraded if the networking
service could not start. A bounded watchdog shows a retry action on failure.

The real Linux packaged-app acceptance is
`scripts/acceptance/packaged-desktop-webdriver.py`. It uses `tauri-driver`
and the native WebKitGTK WebDriver, **not Chrome or an injected Tauri mock**.
Required CI extracts the exact same-run Debian package, verifies the four
sidecars are executable, starts the installed GUI under Xvfb with an isolated
`SWARMCRAFT_DATA_DIR`, creates a world using WebDriver input, closes the
Tauri session, relaunches it, and checks the world and node identity persist.
A second run removes the installed CLI sidecar and proves the normal
acceptance fails while the GUI reports an actionable startup error.

To reproduce on Linux, install `webkit2gtk-driver`, `xvfb` and
`tauri-driver 2.1.0`, build a Debian package with the four sidecars, extract
it with `dpkg-deb -x`, then run:

```sh
export SWARMCRAFT_DATA_DIR="$(mktemp -d)"
xvfb-run -a python3 scripts/acceptance/packaged-desktop-webdriver.py \
  --binary /absolute/path/to/extracted/usr/bin/swarmcraft-desktop \
  --artifacts /tmp/swarmcraft-desktop-e2e
```

This is a **Linux native-WebKitGTK GUI** gate. The separate local
[installed Windows/macOS native QA runner](../../docs/NATIVE_DESKTOP_QA.md)
uses WebView2/msedgedriver on Windows and external native Accessibility
on both Mac architectures, without adding hooks to shipping artifacts.
Actual installed Windows/macOS runs remain required; neither platform
should be described as E2E-certified. These tests also do not certify WAN
connectivity, full Minecraft hosting or multi-device horizontal scaling.

## Official catalogs vs. shipped Fabric compatibility

The Mojang and Fabric Meta catalogs can publish valid new versions **before
this SwarmCraft build ships a compatible Fabric bridge**. The Desktop's native
catalog commands now attach `adapter_supported` to every official version,
using the authoritative `swarm_protocol::SHIPPED_RUNTIME_ADAPTER` validator.
The original official-provider `supported` flag is retained; newer releases
stay visible in the Create/Import selectors but cannot be chosen when the
shipped adapter cannot run them. The default is the newest compatible release
and Fabric Loader combination, not blindly the newest official version.

Create/Import must never bypass the backend's runtime contract or treat a
provider-valid tuple as proof of a compatible host runtime. The existing
canonical creation/import guards and authenticated Fabric handshake remain
authoritative. When the adapter contract changes, the native catalog
annotation updates automatically with the shipped protocol implementation.
