# Agent 8 — Integration + Acceptance

## Status

`NOT STARTED`

## Branch / exact head

- Branch: `integration/player-launcher-v1`
- Exact head: `TBD`

## Mission

Integrate Agents 1–7 into one coherent player-launcher branch, resolve contract mismatches, run the full acceptance matrix, audit false greens, and leave a precise final handoff. Do not invent replacement implementations for owned feature work unless required to resolve integration defects.

## Dependencies to read

- `progress/README.md`
- `progress/agent1.md`
- `progress/agent2.md`
- `progress/agent3.md`
- `progress/agent4.md`
- `progress/agent5.md`
- `progress/agent6.md`
- `progress/agent7.md`

Only integrate exact heads explicitly marked `READY FOR INTEGRATION` or `DONE` with named green tests.

## Dependencies consumed

- None yet.

## Work completed

- None yet.

## Contracts / APIs added or changed

- None yet.

Agent 8 owns integration fixes and final contract reconciliation, including documenting any deviations from upstream handoffs.

## Files changed

- None yet.

## Tests and evidence

- None yet.

Required final acceptance should cover at minimum:

1. Fresh Alice install/device setup.
2. Create world using Mojang-backed Minecraft selector.
3. Select compatible Fabric Loader from Fabric source.
4. Browse/select Modrinth mods.
5. Browse/select CurseForge mods where available/permitted.
6. Resolve dependencies and freeze exact canonical mod requirements/hashes.
7. Runtime install, explicit EULA, real Minecraft/Fabric launch.
8. Save/checkpoint canonical world state.
9. Create an invite without manually entering bootstrap addresses in the normal path.
10. Fresh Bob install, paste invite, complete canonical membership.
11. Bob resolves exact Minecraft/Fabric requirements.
12. Bob automatically retrieves permitted required mods; restricted artifacts produce precise remediation and pass after exact local artifact verification.
13. Bob receives/verifies the canonical snapshot and can launch/join.
14. Public/unlisted discovery behavior is correct; private worlds do not leak into public discovery.
15. Friend discovery does not grant membership.
16. Kill/lose Alice; successor election/fencing/runtime restore remain safe.
17. New authority endpoint is published and player reconnection UX follows the accepted backend endpoint.
18. Wrong hashes, incompatible mods, stale discovery, unreachable invites, provider failures, and interrupted installs fail closed.
19. Existing import, Stop World, Host Readiness, replication, recovery, package builds, and security gates remain green.

## Decisions / invariants

- Do not merge feature branches directly into `main` during this integration phase.
- Do not mark the milestone complete based only on unit tests. Require process-level and real-Minecraft player-journey evidence for the critical path.
- Preserve provider licensing/redistribution boundaries.
- Preserve canonical signed compatibility, authority fencing, quorum behavior, snapshot integrity, Host Readiness, and backend-owned runtime/mod decisions.
- Any intentionally unsupported scenario must be documented as such rather than hidden behind a green UI state.

## Known issues / blockers

- Waits for Agents 1–7.

## Handoff for dependent agents

This is the final integration ledger. Before declaring completion, record integrated upstream SHAs, merge/conflict decisions, exact final head, all CI/workflow run IDs, remaining limitations, and merge recommendation.

## Activity log

- 2026-08-24 — ledger created; integration not started.
