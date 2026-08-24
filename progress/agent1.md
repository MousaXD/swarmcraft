# Agent 1 — Minecraft + Fabric Catalog

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/minecraft-fabric-catalog`
- Exact head: `41c9b5b650aac1e320195f6e1855945f2722abc4` (branch created from mandated base; implementation commits follow)

## Mission

Own backend catalog/resolution for player-selectable Minecraft and Fabric versions. Replace guessed/free-text compatibility inputs with structured source-backed choices while keeping Runtime Installer authoritative for actual installation and verification.

## Dependencies to read

- `progress/README.md`
- No implementation-agent dependency required to start.

## Dependencies consumed

- No implementation-agent dependencies. Coordination rules consumed from `progress/README.md` at base `41c9b5b650aac1e320195f6e1855945f2722abc4`.

## Work completed

- Created `agent/minecraft-fabric-catalog` from the mandated base SHA after verifying the base branch is exactly `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- Read repository-wide `AGENTS.md` and the mandatory progress ledger before implementation.

## Contracts / APIs added or changed

- None yet.

Expected ownership includes structured backend APIs for:

- supported Minecraft release/version listing from official Mojang metadata;
- compatible Fabric Loader versions from Fabric Meta;
- compatibility filtering between selected Minecraft and Fabric Loader;
- Tauri/backend adapter contracts consumed by Desktop selectors;
- source provenance and cache/error semantics.

Do not move compatibility truth into JavaScript.

## Files changed

- `progress/agent1.md`

## Tests and evidence

- Base verification: `backup/local-work-20260824` compared identical to `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- No implementation tests run yet.

## Decisions / invariants

- Mojang and Fabric official sources remain authoritative.
- User-facing selectors must use backend-returned IDs/versions rather than inventing version rules in Desktop.
- Runtime Installer remains responsible for artifact installation/hash verification.
- Never resolve an unspecified value to an arbitrary “latest” after world compatibility has been signed.
- Feature work stays on `agent/minecraft-fabric-catalog`; no direct merge to `main`.

## Known issues / blockers

- None recorded yet.

## Handoff for dependent agents

Agents 4 and 7 depend on this work. Before handoff, document exact JSON/Rust contracts, cache behavior, compatibility rules, error states, and the exact commit SHA that they should consume.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 — `41c9b5b650aac1e320195f6e1855945f2722abc4` — verified mandated base, created `agent/minecraft-fabric-catalog`, read required coordination guidance, status set to IN PROGRESS.
