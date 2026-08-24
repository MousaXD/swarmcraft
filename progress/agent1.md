# Agent 1 — Minecraft + Fabric Catalog

## Status

`NOT STARTED`

## Branch / exact head

- Branch: `agent/minecraft-fabric-catalog`
- Exact head: `TBD`

## Mission

Own backend catalog/resolution for player-selectable Minecraft and Fabric versions. Replace guessed/free-text compatibility inputs with structured source-backed choices while keeping Runtime Installer authoritative for actual installation and verification.

## Dependencies to read

- `progress/README.md`
- No implementation-agent dependency required to start.

## Dependencies consumed

- None yet.

## Work completed

- None yet.

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

- None yet.

## Tests and evidence

- None yet.

## Decisions / invariants

- Mojang and Fabric official sources remain authoritative.
- User-facing selectors must use backend-returned IDs/versions rather than inventing version rules in Desktop.
- Runtime Installer remains responsible for artifact installation/hash verification.
- Never resolve an unspecified value to an arbitrary “latest” after world compatibility has been signed.

## Known issues / blockers

- None recorded yet.

## Handoff for dependent agents

Agents 4 and 7 depend on this work. Before handoff, document exact JSON/Rust contracts, cache behavior, compatibility rules, error states, and the exact commit SHA that they should consume.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
