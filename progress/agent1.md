# Agent 1 — Minecraft + Fabric Catalog

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/minecraft-fabric-catalog`
- Implementation milestone head: `dd678f558d86bfde3aa1c23904c7ab4497928157`

## Mission

Own backend catalog/resolution for player-selectable Minecraft and Fabric versions. Replace guessed/free-text compatibility inputs with structured source-backed choices while keeping Runtime Installer authoritative for actual installation and verification.

## Dependencies to read

- `progress/README.md`
- No implementation-agent dependency required to start.

## Dependencies consumed

- No implementation-agent dependencies. Coordination rules consumed from `progress/README.md` at base `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- Desktop UI guidance consumed from root `AGENTS.md`, `.agents/skills/swarmcraft-ui-design/SKILL.md`, and `.agents/skills/frontend-quality-gate/SKILL.md`.

## Work completed

- Created `agent/minecraft-fabric-catalog` from the mandated base SHA after verifying the base branch is exactly `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- Added shared Rust crate `swarm-catalog` for official Mojang/Fabric provider access, parsing, bounded caching, and compatibility validation.
- Added Mojang version parsing with stable-release default filtering and explicit snapshot inclusion.
- Added Fabric Meta exact-Minecraft loader resolution; empty Fabric responses mean no compatible loader rather than a guessed fallback.
- Added bounded HTTPS transport: official hard-coded endpoints only, HTTPS-only client, redirects disabled, connect/request timeouts, and response-size caps.
- Added 30-minute cache lifetime, fresh-cache reuse, refresh, and stale official-cache fallback with an explicit warning when providers are unavailable.
- Added thin Tauri commands `minecraft_versions` and `fabric_loader_versions`.
- Added Desktop Create World selector controller. At startup it upgrades the old Create World free-text version fields into real selectors, moves them into the normal flow, exposes Retry/Refresh, and adds an Advanced-only “Show Minecraft snapshots” opt-in.
- Added normal Create World backend revalidation: `create_world` verifies the exact Minecraft/Fabric pair against the Fabric catalog before invoking `swarmcraft world create`. An incompatible or unverifiable pair fails before signed world metadata is created.
- Preserved the existing signed `RuntimeCompatibilityManifestV1` path: after validation, the same exact strings continue into `swarmcraft world create`, compatibility fingerprinting, Runtime Installer/runtime-lock, and Host Readiness.
- Added deterministic Rust fixtures/mocks and Desktop selector-state tests plus an ignored live-source validation test.

## Contracts / APIs added or changed

### Rust shared crate: `swarm-catalog`

Core serializable types:

```text
MinecraftVersion {
    id: String,
    type: String,          // serde output name; Rust field is version_type
    release_time: String,
    supported: bool,
}

FabricLoaderVersion {
    version: String,
    stable: bool,
    minecraft_version: String,
}

CatalogResponse<T> {
    provider: "mojang" | "fabric",
    source_url: String,
    fetched_at_unix_seconds: u64,
    cache_expires_at_unix_seconds: u64,
    origin: "network" | "fresh_cache" | "stale_cache",
    warning: Option<String>,
    versions: Vec<T>,
}
```

Callable Rust APIs intended for Agent 4 / Agent 7 reuse:

```text
CatalogService::minecraft_versions(include_snapshots: bool, refresh: bool)
    -> Result<CatalogResponse<MinecraftVersion>, CatalogError>

CatalogService::fabric_loader_versions(minecraft_version: &str, refresh: bool)
    -> Result<CatalogResponse<FabricLoaderVersion>, CatalogError>

CatalogService::validate_fabric_selection(
    minecraft_version: &str,
    fabric_loader_version: &str,
    refresh: bool,
) -> Result<FabricLoaderVersion, CatalogError>

parse_minecraft_catalog(body: &[u8])
filter_minecraft_versions(versions, include_snapshots)
parse_fabric_loader_catalog(minecraft_version, body)
validate_fabric_loader_selection(minecraft_version, loader, versions)
```

Official endpoints owned by the backend:

- Mojang: `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json`
- Fabric: `https://meta.fabricmc.net/v2/versions/loader/{minecraft_version}`; the path segment is encoded by `Url`, not concatenated by JavaScript.

### Tauri contracts

`minecraft_versions` request:

```json
{
  "includeSnapshots": false,
  "refresh": false
}
```

Representative response:

```json
{
  "provider": "mojang",
  "source_url": "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
  "fetched_at_unix_seconds": 1787572800,
  "cache_expires_at_unix_seconds": 1787574600,
  "origin": "network",
  "warning": null,
  "versions": [
    {
      "id": "26.2",
      "type": "release",
      "release_time": "2026-06-23T10:00:00+00:00",
      "supported": true
    }
  ]
}
```

`fabric_loader_versions` request:

```json
{
  "minecraftVersion": "26.2",
  "refresh": false
}
```

Representative response:

```json
{
  "provider": "fabric",
  "source_url": "https://meta.fabricmc.net/v2/versions/loader/26.2",
  "origin": "network",
  "warning": null,
  "versions": [
    {
      "version": "0.19.3",
      "stable": true,
      "minecraft_version": "26.2"
    }
  ]
}
```

Tauri command errors are structured `CatalogErrorPayload` values:

```json
{
  "code": "provider_unavailable | response_too_large | malformed_provider_response | empty_catalog | invalid_input | incompatible_fabric_selection | cache_unavailable | catalog_task_failed",
  "provider": "mojang | fabric",
  "message": "..."
}
```

## Files changed

- `Cargo.toml`
- `crates/swarm-catalog/Cargo.toml`
- `crates/swarm-catalog/src/lib.rs`
- `crates/swarm-catalog/tests/fixtures/mojang_manifest.json`
- `crates/swarm-catalog/tests/fixtures/fabric_loaders_26_2.json`
- `crates/swarm-catalog/tests/live_sources.rs`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/catalog_commands.rs`
- `apps/desktop/src-tauri/src/main.rs`
- `apps/desktop/src/catalog-selectors.js`
- `apps/desktop/src/import-flow.js`
- `apps/desktop/tests/catalog-selectors.test.mjs`
- `.github/workflows/agent1-lockfiles.yml` (temporary read-only lockfile artifact workflow; remove before handoff)
- `progress/agent1.md`

## Tests and evidence

- Base verification: `backup/local-work-20260824` compared identical to `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- Deterministic tests added but full branch CI has not yet been run at this milestone.
- Live provider validation path: `cargo test -p swarm-catalog --test live_sources -- --ignored --nocapture` (not part of deterministic default CI).

## Decisions / invariants

- Mojang and Fabric official sources remain authoritative.
- User-facing selectors consume backend-returned IDs/versions. JavaScript contains no fabricated Minecraft/Fabric compatibility table.
- Mojang `release` is the normal default. Snapshots appear only after explicit Advanced opt-in.
- Fabric compatibility is determined only by Fabric Meta’s exact `/loader/{minecraft_version}` result.
- An empty Fabric list is valid evidence that the selected Minecraft version currently has no compatible loader; Create World stays disabled.
- Stale cached data may be used only if it was previously parsed as valid official-source data, and the response explicitly reports `origin = stale_cache` with a warning.
- Runtime Installer remains responsible for artifact installation/hash verification.
- Never resolve an unspecified value to an arbitrary “latest” after world compatibility has been signed.
- The normal Desktop Create World path fails closed if the exact pair cannot be revalidated. The existing direct CLI `world create` remains an advanced/manual interface and is not made network-dependent in this agent branch.
- Feature work stays on `agent/minecraft-fabric-catalog`; no direct merge to `main`.

## Known issues / blockers

- Both workspace and excluded Desktop lockfiles still need regeneration for the new `reqwest` dependency. A temporary read-only PR workflow is present only to produce those exact lockfiles; it must be removed before handoff.
- Rust formatting/clippy/test evidence is pending CI.
- Visual/min-window UI validation is pending after deterministic Desktop tests are green.

## Handoff for dependent agents

Agent 4 / Agent 7 should consume `swarm-catalog` rather than duplicating provider rules. For any flow that can create or mutate canonical runtime metadata, call `CatalogService::validate_fabric_selection` before signing/persisting the exact pair. Do not infer compatibility from version strings.

The Desktop Tauri JSON shapes above are stable integration contracts for selectors. Cache provenance (`origin`, `warning`) is part of the player-facing offline-state contract.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 — `41c9b5b650aac1e320195f6e1855945f2722abc4` — verified mandated base, created `agent/minecraft-fabric-catalog`, read required coordination guidance, status set to IN PROGRESS.
- 2026-08-24 — `441472e216a68986b507ea55bf74a510da260d03` — shared `swarm-catalog` provider/cache/parsing layer and deterministic fixtures added.
- 2026-08-24 — `dd678f558d86bfde3aa1c23904c7ab4497928157` — Tauri catalog commands, Create World selector state, and fail-closed Desktop creation revalidation wired; CI/lockfile validation next.
