# Agent 7 — Player Setup + Migration UX

## Status

`NOT STARTED`

## Branch / exact head

- Branch: `agent/player-setup-migration-ux`
- Exact head: `TBD`

## Mission

Connect the catalog/provider/modpack/connectivity/discovery contracts into the normal Desktop player journey: create, choose versions/mods, invite/join, automatically prepare permitted runtime/mod artifacts, sync the world, launch, and reconnect after host migration.

## Dependencies to read

- `progress/README.md`
- `progress/agent1.md`
- `progress/agent2.md`
- `progress/agent3.md`
- `progress/agent4.md`
- `progress/agent5.md`
- `progress/agent6.md`

Do not begin shared-contract integration until the required upstream files record exact ready-for-integration SHAs.

## Dependencies consumed

- None yet.

## Work completed

- None yet.

## Contracts / APIs added or changed

- None yet.

Expected ownership includes Desktop flows for:

- Minecraft version selector backed by Agent 1;
- Fabric Loader selector backed by Agent 1;
- Modrinth browse/select/install backed by Agent 2;
- CurseForge browse/select/install/remediation backed by Agent 3;
- canonical new-world modpack creation/import backed by Agent 4;
- automatic invite readiness/connectivity backed by Agent 5;
- friend/public discovery backed by Agent 6;
- join setup wizard that resolves runtime, permitted mods, snapshot sync, and readiness;
- explicit local-file remediation when provider download is unavailable;
- post-migration endpoint observation and player reconnection UX without bypassing authority safety.

## Files changed

- None yet.

## Tests and evidence

- None yet.

## Decisions / invariants

- Desktop consumes backend truth; do not duplicate compatibility, authority, Host Readiness, hash, or licensing rules in JavaScript.
- “Automatic mod setup” means automatically retrieve only artifacts the provider/project permits. Otherwise show a precise local/download remediation step and verify the resulting artifact.
- Join must not claim playable until canonical membership, runtime/mod compatibility, snapshot synchronization, and required readiness checks are satisfied.
- Reconnection UX follows the backend-published accepted authority endpoint. It must never select a host by frontend guesswork.
- Existing safe Stop World, fencing, migration, and Host Readiness behavior must remain intact.

## Known issues / blockers

- Intentionally waits on Agents 1–6 for final integration contracts.

## Handoff for dependent agents

Agent 8 consumes the complete player journey. Record every upstream SHA consumed, Tauri/backend adapter changes, UI state machine, failure/remediation states, reconnect behavior, owned tests, screenshots/evidence where useful, and exact green head.

## Activity log

- 2026-08-24 — ledger created; waiting on upstream implementation contracts.
