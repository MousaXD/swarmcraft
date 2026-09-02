# Agent 2 — Modrinth provider

## Recovery status

`INTEGRATED`

- Branch: `agent/modrinth-provider`
- Exact live head: `355d1f0762fe04391643eabcb802bc4641b1b0a8`
- Live ancestry audit: this exact head is an ancestor of `integration/player-launcher-v1` with zero Agent 2 commits left ahead.

## Integrated contract

- Official Modrinth API search/project/version integration.
- Deterministic compatible version ordering and required dependency closure.
- Exact provider file identity and provider hashes.
- HTTPS-only provider/CDN access with bounded downloads and atomic publication.
- Runtime reacquisition uses frozen canonical project/version/file provenance. It never resolves a newer compatible version during Bob's install/repair.
- Exact downloaded JAR is subsequently checked against canonical Fabric mod ID/version/hash before becoming a world mod.

## Validation evidence

The final integration exact-head workspace/Linux/macOS/Windows test and strict-Clippy suites include the Modrinth implementation and its deterministic provider tests. Historical Agent 2 strict-Clippy defects are superseded by the integrated green source tree.

Final acceptance is owned by Agent 8; no standalone Agent 2 blocker remains.
