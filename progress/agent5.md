# Agent 5 — Automatic invites

## Recovery status

`INTEGRATED`

- Branch: `agent/automatic-invites`
- Exact live head: `e13a4fd57e3c26121275db0b1628808e2e036a44`
- Live ancestry audit: this exact head is an ancestor of `integration/player-launcher-v1` with zero Agent 5 commits left ahead.

## Integrated contract

- Signed invite tokens contain authority reachability without requiring ordinary players to type libp2p multiaddresses.
- Empty normal-path bootstrap input derives usable reachability from backend connectivity diagnostics.
- Join remains authority-mediated; possessing or discovering an invite does not bypass membership policy.
- Token bootstrap addresses remain inside the signed/encoded invite rather than being exposed as UI setup material.

## Validation evidence

`automatic_invite_join` is part of exact-head workspace tests. It creates an invite with no manual `--bootstrap`, stages a Bob join via the actual CLI, starts two real daemon processes, advances canonical membership on both peers, clears the pending join, and verifies exact snapshot replication. Workspace tests were green on the integrated validation head.

Final acceptance is owned by Agent 8; no standalone Agent 5 blocker remains.
