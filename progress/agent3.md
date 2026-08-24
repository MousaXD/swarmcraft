# Agent 3 — CurseForge Integration

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/curseforge-provider`
- Starting head: `41c9b5b650aac1e320195f6e1855945f2722abc4`
- Exact implementation head: `TBD`

## Mission

Implement a backend-owned CurseForge provider for project/file browsing, compatible file selection, dependency metadata, and permitted download/install flows for Fabric server worlds.

## Dependencies to read

- `progress/README.md`
- Read Agent 1 when consuming shared Minecraft/Fabric version identifiers or compatibility contracts.
- Read Agent 2 if a shared provider abstraction is already established. Reuse it rather than creating a competing provider model.

## Dependencies consumed

- Base coordination state: `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- `progress/agent1.md` at base SHA: no implementation contract published yet.
- `progress/agent2.md` at base SHA: no shared provider abstraction published yet.

## Work completed

- Verified `backup/local-work-20260824` is exactly the requested starting SHA.
- Reviewed the current official CurseForge REST API documentation and download/auth semantics before implementation.

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

- `progress/agent3.md`

## Tests and evidence

- Base/ref verification: `backup/local-work-20260824` compared identical to `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- Implementation tests not run yet.

## Decisions / invariants

- Respect CurseForge API terms, project permissions, and download restrictions.
- Do not proxy or peer-redistribute artifacts that are not permitted for redistribution.
- Exact provider/file identity must be available to Agent 4 for canonical locking.
- If automatic download is unavailable, return a structured remediation requirement rather than pretending installation succeeded.
- Since Agents 1 and 2 have no published implementation contract on the required base, keep Agent 3 provider types CurseForge-owned and integration-friendly rather than claiming a shared canonical schema.

## Known issues / blockers

- No CurseForge API credential is assumed present. Missing credentials must be a normal structured unavailable state and must not break the rest of SwarmCraft.

## Handoff for dependent agents

Agent 4 consumes provider identity/dependency/installability contracts. Agent 7 consumes browse/install UX contracts. Record API/auth requirements, exact types/JSON, download restrictions, test fixtures, and exact green commit SHA before marking ready.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 — `41c9b5b650aac1e320195f6e1855945f2722abc4` — verified base, consumed required ledgers, reviewed official CurseForge API, and started `agent/curseforge-provider`.
