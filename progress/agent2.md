# Agent 2 — Modrinth Integration

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/modrinth-provider`
- Exact head: `41c9b5b650aac1e320195f6e1855945f2722abc4` (branch point; implementation commits pending)

## Mission

Implement a backend-owned Modrinth provider for browsing/searching projects, selecting compatible versions/files, resolving dependencies, and downloading/verifying permitted artifacts for Fabric server worlds.

## Dependencies to read

- `progress/README.md`
- Read Agent 1 when consuming shared Minecraft/Fabric version identifiers or compatibility contracts.

## Dependencies consumed

- `progress/README.md` at `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- `progress/agent1.md` at `41c9b5b650aac1e320195f6e1855945f2722abc4`; Agent 1 was `NOT STARTED` and had not published shared catalog identifier types. This provider therefore accepts explicit Minecraft/loader strings and does not invent Agent 1 catalog types.

## Work completed

- Verified the requested base commit and created `agent/modrinth-provider` from it.
- Reviewed the Runtime Installer artifact publication pattern to preserve its temp-file/hash/fsync/rename safety invariants.
- Reviewed current official Modrinth v2 API documentation for search facets, version filtering, dependency/file metadata, rate-limit headers, stable IDs, and required identifying `User-Agent` behavior.

## Contracts / APIs added or changed

- None committed yet.

Expected ownership includes:

- Modrinth search/browse API client;
- Minecraft/Fabric/environment filtering;
- project/version/file identity;
- dependency metadata;
- source URL/provenance handling;
- artifact hash verification;
- backend/Tauri contracts for Desktop browsing and installation.

## Files changed

- `progress/agent2.md`

## Tests and evidence

- No implementation tests run yet.

## Decisions / invariants

- Do not sign or install ambiguous “latest” requirements.
- Preserve exact provider project/version/file identifiers and hashes for Agent 4.
- Respect Modrinth project licensing and download rules.
- Provider/API failures must surface as structured unavailable/error states, not silently fall back to unrelated artifacts.
- Production requests use Modrinth API v2 and a uniquely identifying SwarmCraft `User-Agent`.
- Compatibility stays backend-owned. Search/version resolution will require exact Minecraft version and Fabric loader inputs for server preparation.
- No peer-to-peer redistribution is introduced by this provider.

## Known issues / blockers

- Agent 1 has not yet published a shared catalog identifier contract at the consumed base SHA. This is not a blocker because provider compatibility inputs remain explicit strings and can later consume Agent 1 IDs without changing Modrinth HTTP semantics.

## Handoff for dependent agents

Agent 4 consumes provider identity/dependency/download contracts. Agent 7 consumes browse/install UX contracts. Record exact types/JSON, provider rate-limit/cache behavior, permitted download behavior, and exact green commit SHA before marking ready.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 @ `41c9b5b650aac1e320195f6e1855945f2722abc4` — started Agent 2 on `agent/modrinth-provider`; consumed required progress dependencies and official Modrinth API contract; implementation in progress.
