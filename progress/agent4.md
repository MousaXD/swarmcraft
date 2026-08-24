# Agent 4 — Canonical Modpack + Import

## Status

`NOT STARTED`

## Branch / exact head

- Branch: `agent/canonical-modpack-import`
- Exact head: `TBD`

## Mission

Own the canonical representation of third-party mod requirements and connect it to new-world creation and existing-world import. Convert provider selections into exact signed requirements without embedding machine-local bytes in world history.

## Dependencies to read

- `progress/README.md`
- `progress/agent1.md`
- `progress/agent2.md`
- `progress/agent3.md`

Do not freeze canonical provider fields until the exact green upstream contracts/SHAs are recorded.

## Dependencies consumed

- None yet.

## Work completed

- None yet.

## Contracts / APIs added or changed

- None yet.

Expected ownership includes canonical requirements such as exact:

- provider/source;
- project ID;
- version/file ID;
- Fabric mod ID;
- semantic/display version;
- Minecraft version;
- loader/environment;
- artifact hash;
- required dependencies and their exact locked identities.

Also owns:

- adding mods during Desktop world creation;
- canonical modpack construction from selected providers;
- modded-world import improvements;
- local-artifact fallback for provider-restricted downloads;
- migration/compatibility behavior for older worlds if schema evolution is required.

## Files changed

- None yet.

## Tests and evidence

- None yet.

## Decisions / invariants

- Canonical world history stores requirements/identity, not arbitrary machine-local JAR bytes.
- Never sign `latest`, mutable URLs, or unresolved dependencies.
- Hash mismatch, wrong environment, missing dependency, or provider restriction fails closed.
- Existing server-mod Host Readiness semantics must remain valid.
- Schema/protocol changes require explicit compatibility/migration handling and documentation.

## Known issues / blockers

- Blocked from final contract freeze until Agents 1–3 publish their exact provider/runtime contracts.

## Handoff for dependent agents

Agent 7 consumes the final canonical modpack/create/import contracts. Agent 8 must know any schema-version or backward-compatibility implications. Record all canonical field definitions, signed-vs-machine-local boundaries, migration rules, and exact green SHA.

## Activity log

- 2026-08-24 — ledger created; waiting for provider/runtime contracts before final schema work.
