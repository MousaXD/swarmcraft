# Agent 3 — CurseForge provider

## Recovery status

`INTEGRATED`

- Branch: `agent/curseforge-provider`
- Exact live head: `2ec9005591de71fffe8e504607a4ffb3145ff9c8`
- Live ancestry audit: this exact head is an ancestor of `integration/player-launcher-v1` with zero Agent 3 commits left ahead.

## Integrated contract

- Official CurseForge REST API only; no site scraping.
- Credential is machine-local via `SWARMCRAFT_CURSEFORGE_API_KEY`; no key is committed or hardcoded.
- Search/project/compatible-file selection, deterministic dependency closure, and download behavior are integrated into Desktop.
- Exact project/file provenance and hashes are preserved in the canonical manifest/runtime hint.
- Runtime install/repair reacquires only the frozen exact file. Provider-restricted or unavailable downloads fail closed with explicit manual-artifact remediation.
- Downloaded JAR identity/hash is verified against the signed canonical requirement before publication.

## Validation evidence

The exact-head final integration workspace check, strict Clippy, tests, Desktop frontend/bridge tests, dependency audit, and process acceptance execute with this provider implementation present. Credential/restriction behavior remains explicit rather than being masked by fallback scraping or version substitution.

Final acceptance is owned by Agent 8; no standalone Agent 3 blocker remains.
