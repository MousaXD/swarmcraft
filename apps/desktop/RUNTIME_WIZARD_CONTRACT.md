# Desktop Runtime Wizard Contract

The Runtime Wizard is a player-facing consumer of the backend-owned runtime contract. It does not implement Java, Minecraft, Fabric, EULA, server-mod, authority or migration rules in JavaScript.

## Backend ownership

The packaged `swarmcraft-runtime` sidecar exposes structured commands for:

```text
status
plan
install
repair
verify
launch
```

Desktop invokes these through thin Tauri commands in `apps/desktop/src-tauri`. Status/plan/verify are machine-readable JSON. Install/repair expose backend progress and return the final structured install report.

## Normal Play flow

1. Desktop requests `runtime_status` for the selected world.
2. If required components are incomplete, Runtime Wizard renders backend component state.
3. If EULA acceptance is required, the UI requires an explicit player checkbox before sending `acceptEula: true`.
4. Desktop requests backend install/repair as needed. It does not download artifacts itself.
5. Desktop requests backend verification.
6. Required server-mod readiness remains a separate fail-closed proof boundary.
7. Managed launch delegates to the shared Rust runtime/migration orchestration path and persisted `RuntimeLaunchConfig`.

The wizard never treats file existence alone as Host Readiness. Runtime proof becomes authoritative only after the configured runtime launches and completes the authenticated Fabric compatibility/readiness handshake.

## Core Tauri commands

- `runtime_status`
- `runtime_plan`
- `runtime_install`
- `runtime_repair`
- `runtime_verify`
- `runtime_launch`

Each command is a thin bridge to the Rust backend/sidecar. Desktop normalizes presentation fields but does not recalculate backend safety decisions.

## EULA

Minecraft server EULA acceptance is explicit. A player action is required before `acceptEula: true` is sent. Importing an existing world does not import EULA state.

## Existing-world import

Import is a separate typed backend path exposed by `swarmcraft-import` and the Tauri `import_world` command. It imports canonical world data and compatibility requirements, then returns to the normal Runtime Wizard + Play path. Runtime binaries, Java selection, launch configuration and EULA state remain machine-local.

## Failure behavior

Backend errors remain errors. The UI may offer retry/repair/Advanced diagnostics, but it must not fake `ready`, suppress corrupt runtime state, auto-accept the EULA, or bypass server-mod/authority checks.
