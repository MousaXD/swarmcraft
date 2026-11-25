# Managed Runtime Installer

SwarmCraft has a backend-owned runtime preparation layer in `swarm-cli`.

The normal Desktop player path consumes this contract instead of asking the player to hunt for Java, Minecraft server, Fabric Loader, Fabric API, or SwarmCraft bridge JAR paths.

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
swarmcraft-runtime launch <world>
```

`status`, `plan`, and `verify` print one JSON document to stdout. `install` and `repair` print progress objects as JSON lines to stderr and the final `RuntimeInstallReport` JSON document to stdout. `launch` delegates to the shared Rust authority/runtime orchestration path using the persisted machine-local runtime configuration.

Desktop treats the JSON enums as the source of truth. It does not duplicate Minecraft/Java/Fabric compatibility rules in JavaScript.

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

The server-mod component intentionally does **not** auto-download arbitrary third-party mods. If the canonical world compatibility manifest requires user mods that are not runtime/platform components, status remains unavailable/not-ready until the server-mod manager validates exact local artifacts.

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
    publication.lock
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

`runtime-lock.json` records exact selected versions, immutable generation
paths, source provenance, upstream SHA-1/SHA-256 where supplied, and a local
SHA-256. Managed Java additionally records the archive SHA-256, exact
executable path and extracted-tree digest. Component status reports the
verification level. A local operator override pins the supplied bytes, not
publisher provenance; a generated Fabric launcher has HTTPS/local SHA-256
and a checked JAR manifest, not an upstream signature or checksum.

The migration runtime at `<data>/runtime/<world-id>` is disposable restored runtime state and is not used as durable package storage.

## Source policy

Automatic mode only resolves artifacts from fixed trusted HTTPS origins:

- Mojang's official version metadata/server downloads;
- Fabric Meta for the server launcher;
- Fabric Maven for Fabric API and its published SHA-1;
- Eclipse Adoptium for managed Java and its SHA-256;
- the official `MousaXD/swarmcraft` GitHub releases for the SwarmCraft Fabric bridge and its release SHA-256.

The Runtime Installer in a published preview uses the `main-snapshot-<commit>` tag embedded at build time. Other builds prefer `vX.Y.Z` and can fall back to the legacy `main-latest` release. Every source must supply the exact requested `swarmcraft-fabric-X.Y.Z.jar` and matching `.sha256` asset.

`SWARMCRAFT_FABRIC_MOD_JAR` is a local-file development/acceptance override. It is never interpreted as a URL.

Downloads are written into disposable same-filesystem staging directories,
checked before publication, fsynced and renamed into immutable shared
generations. A small `current.json` lookup receipt is atomically replaced
only after the generation is valid. Previously referenced files remain
untouched, including the legacy cache layout. Failed downloads or hash
checks cannot replace the prior known-good generation. Explicit repair
can publish a different generation; old world locks retain their
original paths. Do not remove old generations without a verified
reference inventory. SHA-1/SHA-256 checks are performed in Rust.

Installer locking uses stable OS-backed per-world and machine-data-root
publication locks. The lock files are intentionally retained after release
to prevent inode-replacement races. A concurrent second-world installer
must retry after the first releases its shared lock; a process crash does
not leave a permanent PID-file wedge. The installer also holds the world
authority-runtime slot and refuses an unaccounted residual JVM. It prepares
all shared artifacts before updating world-local copies and invalidates
prior runtime/readiness proofs. A partial world-local staging failure is
unready/fail-closed until repaired; prior shared generations remain.
A previously installed world can verify/reuse its complete setup offline;
new-world source resolution still requires upstream metadata.

Authority hosting uses the same class of protection: each world has one machine-local authority-runtime slot (`control/<world>/authority-runtime.lock`). An explicit launch (Desktop Play, `swarmcraft-runtime launch`) fails fast when another process actually holds that slot, while the daemon supervisor retries only genuine lock contention; lock-path and locking I/O failures are surfaced instead of being mistaken for an active host. A freshly prepared runtime directory is seeded from `runtime-components/<world>/server/server.jar` only when a compatible managed runtime lock exists and the staged JAR still matches its recorded SHA-256, so first launch avoids a Mojang re-download without trusting stale or stray staged bytes.

The implementation uses the platform `curl` for HTTPS-only transfer with
bounded retries/time/size, initial and final effective source-host checks
and disabled user curl configuration, and the platform tar tool for
official hash-verified Java archive extraction. It does not download or
execute arbitrary third-party installer scripts. This does not claim
privileged filesystem race confinement, per-hop redirect confinement or
provenance of Fabric secondary dependencies.

## Java

The exact Java major version is resolved from Mojang's version metadata during planning/installation. Local status uses the recorded value when available and a conservative version heuristic only before the first successful resolution.

A compatible system `java` may be reused, with an explicit external
version-only provenance status. Otherwise SwarmCraft resolves a matching
Eclipse Adoptium archive into an immutable managed Java generation, checks
the archive hash and extracted tree before probing Java, and rechecks the
tree before launch. It never overwrites system Java or a previously
referenced managed Java directory. Historical managed Java locks without
tree provenance require explicit repair before managed execution.

The live player-journey acceptance deliberately removes the workflow build JVM from the player runtime environment and requires Runtime Installer to resolve managed Java, so an inherited CI Java installation cannot create a false green.

## Fabric API and the migration runtime

Migration owns authority/runtime safety and rebuilds disposable runtime state from canonical world state plus verified machine-local configuration. The Fabric bridge release nests the selected Fabric API artifact using Fabric Loom's `include` mechanism. The installer also downloads, verifies, records, and stages the standalone Fabric API artifact as a managed component.

This keeps package management out of authority logic while ensuring the bridge used by automatic setup carries Fabric API into the live server runtime.

CI verifies that the built/released SwarmCraft Fabric JAR embeds Fabric API under `META-INF/jars`.

## SwarmCraft bridge release assets

Main snapshots and tagged releases publish a versioned bridge JAR and checksum:

```text
swarmcraft-fabric-X.Y.Z.jar
swarmcraft-fabric-X.Y.Z.jar.sha256
```

Each installed preview resolves the bridge from its own commit-pinned release, so a later preview cannot change the bridge selected by an earlier installer.

## EULA

EULA acceptance is never implicit.

Installation may prepare artifacts while reporting:

```text
eula: required
```

Only an explicit `--accept-eula` or equivalent explicit Desktop user action passed to `RuntimeInstallOptions.accept_eula` permits the installer to persist `RuntimeLaunchConfig { accept_eula: true, ... }` for normal hosting/migration/wake/recovery.

Importing an existing world does not import EULA acceptance.

## Signed compatibility and legacy worlds

Minecraft version and Fabric Loader version are part of durable world genesis/compatibility state. The installer reads them; it does not silently rewrite them.

In particular, a legacy world whose signed genesis records Fabric Loader as `unknown` cannot safely be auto-upgraded to an arbitrary loader version by this machine-local installer. That requires an explicit product-level compatibility migration rather than a local package-install side effect.

## Runtime verification and Host Readiness

Static artifact existence is not enough to claim a machine can safely host.
The same managed lock, Java tree, source/staged JAR hashes and selected
launch paths are verified inside the authority-runtime slot before any
new canonical authority epoch is prepared. Unreadable managed locks
do not fall back to manual configuration. Preserve corrupt metadata for
review rather than deleting it blindly.

Runtime setup and Host Readiness have separate boundaries:

1. Installer verifies the selected managed artifacts/configuration.
2. The configured runtime launches through the shared Rust path.
3. The exact Minecraft/Fabric world completes the authenticated Fabric compatibility/readiness handshake.
4. Only then can current runtime proof become green for Host Readiness.

Changing/replacing a verified runtime artifact or relevant configuration invalidates the proof instead of reusing historical success.

Required third-party server mods remain a separate current machine-local readiness proof.

## Desktop contract

The shipped Tauri application has thin commands around the packaged `swarmcraft-runtime` sidecar:

```text
runtime_status(world)
  -> swarmcraft-runtime status <world>

runtime_plan(world)
  -> swarmcraft-runtime plan <world>

runtime_install(world, acceptEula)
  -> swarmcraft-runtime install <world> [--accept-eula]

runtime_repair(world, acceptEula)
  -> swarmcraft-runtime repair <world> [--accept-eula]

runtime_verify(world)
  -> swarmcraft-runtime verify <world>

runtime_launch(world)
  -> swarmcraft-runtime launch <world>
```

Desktop Runtime Wizard consumes these commands and does not execute download URLs or reconstruct Java commands itself. The player explicitly accepts EULA terms before the frontend sends `acceptEula: true`.

For install/repair, stderr lines are progress JSON and stdout is the authoritative final result.

All supported Desktop package jobs stage `swarmcraft-runtime` together with `swarmcraft`, `swarmcraft-host`, and `swarmcraft-import`.

Manual Advanced runtime configuration remains available for power users, but it passes through backend verification and does not bypass Host Readiness or authority safety.

See the [runtime resilience audit](../research/runtime-resilience/REPORT.md) for actual two-world, outage, corruption and dependency evidence.
