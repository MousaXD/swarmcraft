# Agent 3 — CurseForge Integration

## Status

`NOT STARTED`

## Branch / exact head

- Branch: `agent/curseforge-integration`
- Exact head: `TBD`

## Mission

Implement a backend-owned CurseForge provider for project/file browsing, compatible file selection, dependency metadata, and permitted download/install flows for Fabric server worlds.

## Dependencies to read

- `progress/README.md`
- Read Agent 1 when consuming shared Minecraft/Fabric version identifiers or compatibility contracts.
- Read Agent 2 if a shared provider abstraction is already established. Reuse it rather than creating a competing provider model.

## Dependencies consumed

- None yet.

## Work completed

- None yet.

## Contracts / APIs added or changed

- None yet.

Expected ownership includes:

- CurseForge API/auth configuration;
- search/browse project/file metadata;
- Minecraft/Fabric/environment filtering;
- dependency metadata;
- exact file identity and hashes where available;
- provider download restrictions/fallback remediation;
- backend/Tauri contracts for Desktop browsing and installation.

## Files changed

- None yet.

## Tests and evidence

- None yet.

## Decisions / invariants

- Respect CurseForge API terms, project permissions, and download restrictions.
- Do not proxy or peer-redistribute artifacts that are not permitted for redistribution.
- Exact provider/file identity must be available to Agent 4 for canonical locking.
- If automatic download is unavailable, return a structured remediation requirement rather than pretending installation succeeded.

## Known issues / blockers

- None recorded yet.

## Handoff for dependent agents

Agent 4 consumes provider identity/dependency/installability contracts. Agent 7 consumes browse/install UX contracts. Record API/auth requirements, exact types/JSON, download restrictions, test fixtures, and exact green commit SHA before marking ready.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
