# Desktop Runtime Wizard Contract

This document defines the Desktop-side contract for the automatic Minecraft runtime setup flow.

At the time this branch was created, `agent/runtime-installer` was not available, so Desktop does not claim automatic installation works yet. The UI treats missing runtime commands as an explicit unavailable capability and routes power users to the existing Advanced/Diagnostics path.

## Player flow

`Play` is intercepted before the legacy manual-path launcher flow.

1. Desktop asks the backend for structured runtime status.
2. If the backend reports `ready: true`, Desktop asks the backend to launch the managed runtime directly.
3. If setup is incomplete, Desktop opens the setup wizard and renders backend-reported component state.
4. If the backend reports EULA acceptance is required, the wizard requires an explicit checkbox before sending `acceptEula: true`.
5. During installation, Desktop polls structured status for progress presentation.
6. Desktop verifies again before launch.
7. Failures expose backend-reported world-data safety and retry safety. Desktop does not invent either property.
8. If automatic runtime commands are absent, install/start controls stay unavailable and the player can open Advanced setup.

## Expected Tauri commands

The runtime integration is isolated in `src/backend-adapter.js`.

### `runtime_status`

Input:

```json
{ "world": "scworld:..." }
```

Output: JSON object or a JSON string following the status shape below.

### `runtime_plan`

Input:

```json
{ "world": "scworld:..." }
```

Output: structured JSON describing planned backend actions. Desktop does not execute download URLs or compatibility rules from this object.

### `runtime_install`

Input:

```json
{
  "world": "scworld:...",
  "acceptEula": true
}
```

`acceptEula` is `true` only after the player explicitly checks the EULA box. Output uses the runtime status shape.

### `runtime_repair`

Input:

```json
{ "world": "scworld:..." }
```

Output uses the runtime status shape.

### `runtime_verify`

Input:

```json
{ "world": "scworld:..." }
```

Output uses the runtime status shape. Desktop requires `ready: true` before managed launch.

### `runtime_launch`

Input:

```json
{ "world": "scworld:..." }
```

Output: process identifier or another success value suitable for player-facing confirmation.

This command is intentionally backend-owned. The managed flow does not reconstruct Java/JAR paths or compatibility rules in JavaScript. If Agent 1 chooses a different launch command, only the adapter should change.

## Runtime status shape

Desktop accepts snake_case and common camelCase transport variants, but backend should prefer one stable JSON schema.

```json
{
  "state": "checking | eula_required | installing | verifying | ready | failed",
  "phase": "checking | downloading_java | downloading_server | installing_fabric | installing_fabric_api | installing_swarmcraft_mod | preparing_directories | verifying | ready | failed",
  "ready": false,
  "detail": "Human-readable summary",
  "eula_accepted": false,
  "eula_required": true,
  "world_data_safe": true,
  "retry_safe": true,
  "components": {
    "java": { "state": "ready", "version": "21", "detail": "managed" },
    "minecraft_server": { "state": "ready", "version": "..." },
    "fabric_loader": { "state": "missing" },
    "fabric_api": { "state": "missing" },
    "swarmcraft_integration": { "state": "missing" },
    "world_directories": { "state": "ready" },
    "server_mods": { "state": "ready" }
  },
  "failure": {
    "message": "...",
    "detail": "..."
  }
}
```

Recognized component states are presentation-only categories: `ready`, `working`, `missing`, `incompatible`, `corrupt`, `failed`, and `unknown`. The backend remains authoritative about which category applies.

## Progress

While `runtime_install` is active, Desktop polls `runtime_status` approximately every 650 ms and renders the reported phase/components. Poll failures are ignored because the installation call owns the actionable error.

No frontend download loop exists. Desktop never fetches Fabric, Java, Minecraft, or mod artifacts itself.

## Failure contract

For a useful failure screen, backend should report:

- what failed in `detail` / `failure`;
- `world_data_safe: true|false` when it can make that assertion;
- `retry_safe: true|false` when it can make that assertion.

If either safety property is absent, Desktop displays “Not reported by backend” and does not enable automatic retry unless `retry_safe` is explicitly `true`.

## Advanced mode

The existing Diagnostics runtime fields remain the manual fallback for Java/server/mod paths and EULA acceptance. The wizard links to that section as **Advanced setup**.

Additional requested overrides such as server directory, JVM options, RAM, launch arguments, Fabric overrides, and server configuration are not fabricated by this branch because no backend contract currently persists/consumes them. They remain an integration dependency.
