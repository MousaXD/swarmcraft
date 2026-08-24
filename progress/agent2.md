# Agent 2 — Modrinth Integration

## Status

`NOT STARTED`

## Branch / exact head

- Branch: `agent/modrinth-integration`
- Exact head: `TBD`

## Mission

Implement a backend-owned Modrinth provider for browsing/searching projects, selecting compatible versions/files, resolving dependencies, and downloading/verifying permitted artifacts for Fabric server worlds.

## Dependencies to read

- `progress/README.md`
- Read Agent 1 when consuming shared Minecraft/Fabric version identifiers or compatibility contracts.

## Dependencies consumed

- None yet.

## Work completed

- None yet.

## Contracts / APIs added or changed

- None yet.

Expected ownership includes:

- Modrinth search/browse API client;
- Minecraft/Fabric/environment filtering;
- project/version/file identity;
- dependency metadata;
- source URL/provenance handling;
- artifact hash verification;
- backend/Tauri contracts for Desktop browsing and installation.

## Files changed

- None yet.

## Tests and evidence

- None yet.

## Decisions / invariants

- Do not sign or install ambiguous “latest” requirements.
- Preserve exact provider project/version/file identifiers and hashes for Agent 4.
- Respect Modrinth project licensing and download rules.
- Provider/API failures must surface as structured unavailable/error states, not silently fall back to unrelated artifacts.

## Known issues / blockers

- None recorded yet.

## Handoff for dependent agents

Agent 4 consumes provider identity/dependency/download contracts. Agent 7 consumes browse/install UX contracts. Record exact types/JSON, provider rate-limit/cache behavior, permitted download behavior, and exact green commit SHA before marking ready.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
