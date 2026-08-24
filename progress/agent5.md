# Agent 5 — Invite + Internet Bootstrap

## Status

`NOT STARTED`

## Branch / exact head

- Branch: `agent/invite-internet-bootstrap`
- Exact head: `TBD`

## Mission

Make normal invites automatically carry usable connectivity/bootstrap information without asking players to type libp2p multiaddresses. Keep discovery/connectivity separate from membership and authority.

## Dependencies to read

- `progress/README.md`
- No implementation-agent dependency required to start.

## Dependencies consumed

- None yet.

## Work completed

- None yet.

## Contracts / APIs added or changed

- None yet.

Expected ownership includes:

- selecting safe current direct/relay/bootstrap addresses from backend connectivity state;
- automatically populating signed invites with usable contact information;
- relay/bootstrap fallback semantics;
- address freshness/expiry behavior;
- privacy-safe invite payloads;
- structured backend/Tauri state for invite readiness/remediation.

## Files changed

- None yet.

## Tests and evidence

- None yet.

## Decisions / invariants

- Invite connectivity hints never grant membership.
- Relay reachability never grants authority.
- Do not embed stale historical-success addresses as if currently reachable.
- Avoid leaking unnecessary local/private addresses when they are not useful to the recipient.
- Manual bootstrap entry may remain an advanced override, but normal invite creation should not require it.

## Known issues / blockers

- Real-world NAT/CGNAT coverage remains a certification concern even after automatic invite construction works.

## Handoff for dependent agents

Agent 6 consumes the connectivity advertisement contract for discovery. Agent 7 consumes the player-facing invite readiness flow. Record exact address-selection rules, signed invite field behavior, privacy decisions, tests, and exact green SHA.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
