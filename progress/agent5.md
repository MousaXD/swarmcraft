# Agent 5 — Invite + Internet Bootstrap

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/automatic-invites`
- Exact head: `41c9b5b650aac1e320195f6e1855945f2722abc4` (implementation baseline; ledger update follows on branch)

## Mission

Make normal invites automatically carry usable connectivity/bootstrap information without asking players to type libp2p multiaddresses. Keep discovery/connectivity separate from membership and authority.

## Dependencies to read

- `progress/README.md`
- No implementation-agent dependency required to start.

## Dependencies consumed

- Baseline `backup/local-work-20260824` at `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- No implementation-agent dependency required.

## Work completed

- Read the shared progress protocol and this ledger before implementation.
- Inspected existing current-path diagnostics, AutoNAT/DCUtR/relay/bootstrap handling, signed invite creation, pending join staging, and daemon bootstrap consumption.
- Confirmed existing daemon already consumes `InviteV1.bootstrap_addrs` automatically for pending joins; the missing piece is safe backend-owned automatic population plus readiness/remediation semantics.

## Contracts / APIs added or changed

- None committed yet.

Planned owned contract:

- backend `InviteConnectivity`-equivalent derived from live networking diagnostics/state;
- bounded, validated shareable address selection for direct/public and relay reachability;
- normal invite creation automatically uses current shareable addresses;
- manual `--bootstrap` remains an advanced override, not a normal requirement;
- explicit no-proven-path result/remediation rather than false internet-ready claims.

## Files changed

- `progress/agent5.md`

## Tests and evidence

- Baseline source inspection only; no implementation tests run yet.

## Decisions / invariants

- Invite connectivity hints never grant membership.
- Relay reachability never grants authority.
- Do not embed stale historical-success addresses as if currently reachable.
- Avoid leaking unnecessary local/private addresses when they are not useful to the recipient.
- Manual bootstrap entry may remain an advanced override, but normal invite creation should not require it.
- Preserve `InviteV1` signed-token authority semantics; connectivity hints remain signed data but are treated only as untrusted dial candidates by the joiner.
- Existing current-path diagnostics are the source of truth; do not reconstruct NAT state in JavaScript.

## Known issues / blockers

- Real-world NAT/CGNAT coverage remains a certification concern even after automatic invite construction works.
- No blocker to implementation identified.

## Handoff for dependent agents

Agent 6 consumes the connectivity advertisement contract for discovery. Agent 7 consumes the player-facing invite readiness flow. Final handoff will record exact address-selection rules, signed invite behavior, failure/remediation states, tests, and exact green SHA.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 — `41c9b5b650aac1e320195f6e1855945f2722abc4` — started Agent 5 on `agent/automatic-invites`; inspected current-path diagnostics, invite creation, pending join, and automatic daemon bootstrap consumption.
