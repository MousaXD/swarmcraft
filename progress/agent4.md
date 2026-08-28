# Agent 4 — Canonical modpack

## Recovery status

`INTEGRATED`

- Branch: `agent/canonical-modpack`
- Exact live head: `75351794926e1ba183e5615f948a53c7017084bf`
- Live ancestry audit: this exact head is an ancestor of `integration/player-launcher-v1` with zero Agent 4 commits left ahead.

## Integrated contract

- Deterministic canonical modpack model with exact provider identity, dependency edges, retrieval policy, filenames, and hashes.
- Stable canonical normalization/fingerprinting independent of UI ordering.
- Canonical provider provenance is carried into the signed runtime compatibility requirement.
- Canonical world creation validates the authoritative Minecraft/Fabric pair and persists the runtime compatibility contract.
- Browser-selected package metadata is not the final runtime authority: exact downloaded Fabric JAR identity is inspected by Rust and hashes are verified before world publication.
- Runtime provider acquisition reconstructs and enforces the frozen canonical source; it does not silently substitute a new provider release.

## Validation evidence

The exact live Agent 4 head is fully contained in final integration. Exact-head workspace format/check/strict Clippy/tests and process-level acceptance are green on the integrated source tree, and Desktop launcher tests cover exact provider-provenance mapping and fail-closed missing dependency behavior.

Final acceptance is owned by Agent 8; no standalone Agent 4 blocker remains.
