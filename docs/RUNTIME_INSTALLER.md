# Managed Runtime Installer

SwarmCraft now has a backend-owned runtime preparation layer in `swarm-cli`.

The normal player path is intended to consume this contract instead of asking the player for Java, Minecraft server, Fabric, Fabric API, or SwarmCraft bridge JAR paths.

## Backend API

Rust callers use:

```rust
RuntimeInstaller::new(&paths, &storage)
installer.inspect(world)
installer.plan(world)
installer.install(world, options, progress)
installer.repair(world, options, progress)
installer.verify(world)
```

The dedicated machine-readable sidecar is:

```text
swarmcraft-runtime status <world>
swarmcraft-runtime plan <world>
swarmcraft-runtime install <world> [--accept-eula] [--game-endpoint <endpoint>]
swarmcraft-runtime repair <world> [--accept-eula] [--game-endpoint <endpoint>]
swarmcraft-runtime verify <world>
```

`status`, `plan`, and `verify` print one JSON document to stdout. `install` and `repair` print progress objects as JSON lines to stderr and the final `RuntimeInstallReport` JSON document to stdout.

Desktop should treat the JSON enums as the source of truth. It must not duplicate Minecraft/Java/Fabric compatibility rules in JavaScript.

## Component status

`RuntimeStatus.components` covers:

- Java;
- Minecraft server;
- Fabric Loader server launcher;
- Fabric API;
- SwarmCraft Fabric integration;
- managed server directories;
- EULA acceptance;
- required user server mods.

Each component is one of:

```text
ready
missing
incompatible
corrupt
required
unavailable
```

`RuntimeStatus.ready` is true only when every required component is `ready`.

The server-mod component intentionally does **not** auto-download arbitrary third-party mods. If the canonical world compatibility manifest requires user mods that are not runtime/platform components, status is `unavailable` until the server-mod manager can validate them.

## Progress phases

Installer callbacks and `swarmcraft-runtime` stderr use these phases:

```text
checking
downloading_java
downloading_server
installing_fabric
installing_fabric_api
installing_swarmcraft_mod
preparing_directories
waiting_for_eula
verifying
ready
failed
```

## Managed layout

Machine-local artifacts live outside signed/canonical world state:

```text
<SwarmCraft data>/
  runtimes/
    java/
    minecraft/
    fabric/
    fabric-api/
    swarmcraft-fabric/
  runtime-components/
    <world-id>/
      runtime-lock.json
      install.lock
      server/
      mods/
      config/
```

`runtime-lock.json` records the exact selected versions, artifact paths, source provenance, SHA-1 where supplied by the upstream, and a local SHA-256 used for later corruption detection.

The existing migration runtime remains disposable at `<data>/runtime/<world-id>` and is not used as durable package storage.

## Source policy

Automatic mode only resolves artifacts from fixed trusted HTTPS origins:

- Mojang's official version metadata/server downloads;
- Fabric Meta for the server launcher;
- Fabric Maven for Fabric API and its published SHA-1;
- Eclipse Adoptium for managed Java and its SHA-256;
- the official `MousaXD/swarmcraft` GitHub release for the SwarmCraft Fabric bridge and its release SHA-256.

`SWARMCRAFT_FABRIC_MOD_JAR` is a local-file development/advanced override. It is never interpreted as a URL.

Downloads are written to temporary files, checked before publication, fsynced, and renamed into their final location. Failed hash checks remove the temporary artifact. A per-world `install.lock` rejects concurrent installers.

The implementation uses the platform `curl`, archive extraction, and SHA utilities instead of downloading or executing third-party installer scripts.

## Java

The exact Java major version is resolved from Mojang's version metadata during planning/installation. Local status uses the recorded value when available and a conservative version heuristic only before the first successful resolution.

A compatible system `java` may be reused. Otherwise SwarmCraft resolves a matching Eclipse Adoptium runtime into the managed Java directory. It never overwrites the user's system Java.

## Fabric API and the migration runtime

Migration intentionally owns authority/runtime safety and currently installs one first-party bridge JAR into its rebuilt `mods` directory. The Fabric bridge build therefore nests the selected Fabric API artifact using Fabric Loom's `include` mechanism. The installer still downloads, verifies, records, and stages the standalone Fabric API artifact as a managed component.

This keeps package management out of migration/authority code while ensuring the bridge release used by automatic setup carries Fabric API into the live server runtime.

A release asset produced before this change must not be assumed to contain the nested dependency. Automatic setup should be validated against a release built from this revision or later.

## EULA

EULA acceptance is never implicit.

Installation may prepare all artifacts while reporting:

```text
eula: required
```

Only an explicit `--accept-eula` (or an equivalent explicit Desktop user action passed to `RuntimeInstallOptions.accept_eula`) permits the installer to persist `RuntimeLaunchConfig { accept_eula: true, ... }` for migration/wake/recovery.

## Signed compatibility and legacy worlds

Minecraft version and Fabric Loader version are part of durable world genesis/compatibility state. The installer reads them; it does not silently rewrite them.

In particular, a legacy world whose signed genesis records Fabric Loader as `unknown` cannot safely be auto-upgraded to an arbitrary loader version by this machine-local installer. That requires an explicit product-level compatibility migration rather than a local package-install side effect.

## Desktop contract

Agent 2 can add a thin Tauri adapter around the dedicated sidecar. Recommended functions:

```text
runtime_status(world)
  -> swarmcraft-runtime status <world>

runtime_plan(world)
  -> swarmcraft-runtime plan <world>

runtime_install(world, acceptEula, gameEndpoint?)
  -> swarmcraft-runtime install <world> ...

runtime_repair(world, acceptEula, gameEndpoint?)
  -> swarmcraft-runtime repair <world> ...

runtime_verify(world)
  -> swarmcraft-runtime verify <world>
```

For install/repair, stderr lines are progress JSON, not human-readable diagnostics. The final stdout document is the authoritative result.

The Desktop packaging branch still needs to stage `swarmcraft-runtime` as a Tauri sidecar before this adapter is available in shipped installers. That small packaging/UI bridge belongs to the Desktop agent so this branch does not take over frontend ownership.
