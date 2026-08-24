# Agent 6 — Friend + Public Discovery

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/discovery`
- Exact head: `41c9b5b650aac1e320195f6e1855945f2722abc4` (implementation base; ledger update commit follows)

## Mission

Implement privacy-aware friend discovery and public/unlisted world discovery without weakening signed membership, visibility, or authority semantics.

## Dependencies to read

- `progress/README.md`
- `progress/agent5.md`

Consume Agent 5's exact current-address/connectivity advertisement contract before designing discovery payloads.

## Dependencies consumed

- `backup/local-work-20260824` / `41c9b5b650aac1e320195f6e1855945f2722abc4` as the mandated implementation base.
- Agent 5 dependency checked on 2026-08-24: no `agent/automatic-invites` branch is published yet and base `progress/agent5.md` remains `NOT STARTED`, so no Agent 5 implementation SHA can be consumed yet.

## Work completed

- Created `agent/discovery` from exact base `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- Read the shared progress protocol and Agent 6/Agent 5 ledgers before changing implementation code.
- Began architecture inspection of existing authenticated libp2p/Kademlia, signed world config visibility, and Desktop bridge contracts.

## Contracts / APIs added or changed

- None yet. Discovery reachability fields will not be frozen until Agent 5 publishes its owned advertisement contract.

Expected ownership includes:

- signed discovery/announcement records;
- public/unlisted visibility behavior;
- friend/contact identity discovery model;
- search/list/query backend APIs;
- expiry/freshness and anti-stale behavior;
- privacy and abuse-resistant defaults;
- Desktop-facing discovery contracts.

## Files changed

- `progress/agent6.md`

## Tests and evidence

- No implementation tests executed yet.

## Decisions / invariants

- Discovery never grants membership. Private worlds remain invite/membership controlled.
- Public/unlisted advertisement must respect signed world visibility state.
- Discovery records must expire and must not turn historical reachability into current reachability.
- Do not conflate friend/contact identity with world membership or authority eligibility.
- Any centralized/federated service dependency must have a clear trust boundary and failure mode.
- Agent 5 owns the connectivity advertisement shape; Agent 6 will consume it rather than create a competing address-selection contract.

## Known issues / blockers

- Final discovery transport/advertisement reachability shape depends on Agent 5's connectivity contract.
- Agent 5 has not yet published its implementation branch/SHA, so final integration readiness is currently blocked on that dependency even though discovery core work can proceed independently.

## Handoff for dependent agents

Agent 7 consumes friend/world discovery APIs and UI states. Agent 8 needs deployment/service assumptions and offline/failure behavior. Record exact signed record shapes, privacy rules, expiry semantics, service/config requirements, tests, and exact green SHA.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 — `41c9b5b650aac1e320195f6e1855945f2722abc4` — created `agent/discovery`, verified exact base, read required ledgers, and recorded Agent 5 dependency as not yet published.
