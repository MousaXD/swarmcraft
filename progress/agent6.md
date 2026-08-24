# Agent 6 — Friend + Public Discovery

## Status

`NOT STARTED`

## Branch / exact head

- Branch: `agent/friend-public-discovery`
- Exact head: `TBD`

## Mission

Implement privacy-aware friend discovery and public/unlisted world discovery without weakening signed membership, visibility, or authority semantics.

## Dependencies to read

- `progress/README.md`
- `progress/agent5.md`

Consume Agent 5's exact current-address/connectivity advertisement contract before designing discovery payloads.

## Dependencies consumed

- None yet.

## Work completed

- None yet.

## Contracts / APIs added or changed

- None yet.

Expected ownership includes:

- signed discovery/announcement records;
- public/unlisted visibility behavior;
- friend/contact identity discovery model;
- search/list/query backend APIs;
- expiry/freshness and anti-stale behavior;
- privacy and abuse-resistant defaults;
- Desktop-facing discovery contracts.

## Files changed

- None yet.

## Tests and evidence

- None yet.

## Decisions / invariants

- Discovery never grants membership. Private worlds remain invite/membership controlled.
- Public/unlisted advertisement must respect signed world visibility state.
- Discovery records must expire and must not turn historical reachability into current reachability.
- Do not conflate friend/contact identity with world membership or authority eligibility.
- Any centralized/federated service dependency must have a clear trust boundary and failure mode.

## Known issues / blockers

- Final discovery transport/advertisement shape depends on Agent 5's connectivity contract.

## Handoff for dependent agents

Agent 7 consumes friend/world discovery APIs and UI states. Agent 8 needs deployment/service assumptions and offline/failure behavior. Record exact signed record shapes, privacy rules, expiry semantics, service/config requirements, tests, and exact green SHA.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
