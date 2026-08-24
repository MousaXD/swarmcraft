# August 24 Feature Completion + Integration Audit

## STATUS

IN PROGRESS

This is the integration ledger for the unfinished August 24 feature wave. GitHub remote branches, exact SHAs, PR metadata, and Actions results are authoritative in this environment because the local container cannot fetch GitHub or run Cargo directly.

## Guardrails

- Do not merge feature validation PRs into `backup/local-work-20260824`.
- Do not merge anything into `main`.
- GitHub CI is the remote compiler, linter, test runner, acceptance lab, and cross-platform validation oracle.
- A stale `progress/agent*.md` does not override branch heads or CI.
- Later integration stages may consume only exact-green upstream heads.

## Baseline remote state

- `main` initially audited at `105b19ade82be606e5a855df4e82ce18bb7e885a`
- `backup/local-work-20260824` at `41c9b5b650aac1e320195f6e1855945f2722abc4`
- PR #44: `agent/modrinth-provider` -> backup
- PR #45: `agent/discovery` -> backup
- PR #46: `agent/automatic-invites` -> backup
- PR #47: `agent/minecraft-fabric-catalog` -> backup
- PR #48: `agent/curseforge-provider` -> backup
- PR #42: backup -> `main`

`backup/local-work-20260824` is meaningful foundation/staging work, not disposable debris, and must not be merged to `main` as part of this wave.

## Branch: agent/discovery

STATUS: READY CANDIDATE

- Green implementation head: `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`
- PR: #45
- CI `32779349426`: SUCCESS
- Network Soak `32779349422`: SUCCESS
- Release Guard `32779349498`: SUCCESS

Bugs repaired through exact-head CI:

- E0308 outbound failure match-arm type mismatch.
- E0502 friend-store borrow conflict.
- `large_enum_variant` from exact-world discovery response size, fixed structurally by boxing the response payload.
- rustfmt deviations.
- stale `STORAGE_SCHEMA_VERSION` test import.
- Clippy `needless_as_bytes`.
- E0004 non-exhaustive normal-daemon wire-request handling, fixed with explicit `DISCOVERY_ENDPOINT_REQUIRED` protocol-boundary responses while retaining exhaustive matching.
- invalid shared-world fixture using an arbitrary `WorldId`, fixed by deriving canonical identity from genesis instead of weakening storage validation.

Contract preserved: PRIVATE hidden, UNLISTED non-browsable but exact-resolvable where designed, PUBLIC discoverable, signed/fresh records only, and discovery never grants membership or authority.

Remaining gate: the documentation-only ledger descendant must pass exact-head CI before final READY promotion.

NEXT: `agent/minecraft-fabric-catalog` immediately after this documentation head is green.

## Branch: agent/minecraft-fabric-catalog

STATUS: BLOCKED

- Last audited head: `68c6713d6658b0bcc6011803f9684564e3e562c1`
- PR: #47
- Normal CI `32721432165`: SUCCESS
- Dedicated catalog validation `32744102086`: FAILURE

Known failure: dedicated validation stops at `cargo fmt --check`, so downstream catalog-specific validation has not yet earned green evidence. Fix formatter output first, then run the complete dedicated suite and repair any deeper failures it reveals.

Required contract: source-backed Mojang Minecraft catalog, Fabric Meta compatible-loader catalog, deterministic/cacheable behavior, and compatibility enforcement.

## Branch: agent/modrinth-provider

STATUS: BLOCKED

- Last audited head: `c5d76875c33645bd64c6bc0109c8adef68d68621`
- PR: #44
- Normal CI, PR Target Guard, and Release Guard: green at last audit
- Dedicated validation `32735413224`: FAILURE overall
- Provider validation, Rust workspace tests, Network Soak, and Actionlint inside that run: SUCCESS
- Desktop build/checks: FAILURE

The old claim that provider unit tests are red is stale. The remaining technical issue is the exact Desktop/Tauri validation failure and must be diagnosed from current logs rather than guessed.

## Branch: agent/automatic-invites

STATUS: BLOCKED

- Last audited head: `110ed6f9558ab2417b281725018fc11dc70ae5fc`
- PR: #46
- Release Guard, PR Target Guard, Network Soak: green at last audit
- CI: FAILURE

Last observed symptom: macOS membership convergence times out after invite acceptance. Diagnose the actual synchronization/reachability cause. Do not hide it with arbitrary sleep inflation.

Contract: normal invites derive connectivity hints from backend networking state; users do not manually construct multiaddresses or infer NAT truth in JavaScript.

## Branch: agent/curseforge-provider

STATUS: READY TECHNICALLY / RELEASE BOOKKEEPING RED

- Last audited head: `344f086eaa7499ba2e4dfa86f6e27cd3410f5d5a`
- PR: #48
- Normal CI: SUCCESS
- PR Target Guard: SUCCESS
- Dedicated CurseForge validation: SUCCESS
- Release Guard: FAILURE

No technical failure was observed. Do not make an isolated version bump until the combined release/version strategy is known.

Contract remains official CurseForge API, no scraping, no committed secrets, graceful missing credentials, compatibility/dependency resolution, permitted artifact download, and deterministic metadata.

## Integration stages

- `integration/package-discovery-foundation`: NOT CREATED. Create only after the required upstream feature branches are technically green.
- `agent/canonical-modpack`: NOT CREATED. Create from exact-green package/discovery foundation.
- `agent/player-launcher-journey`: NOT CREATED. Create after canonical modpack plus required invite/discovery inputs are green.
- `integration/player-launcher-v1`: NOT CREATED. Create only after player journey is exact-green and acceptance-ready.

## Required work order from here

1. Finish exact-head Discovery documentation validation and promote Agent 6.
2. Fix and fully validate Minecraft/Fabric Catalog.
3. Diagnose/fix Modrinth Desktop validation.
4. Diagnose/fix Automatic Invites macOS convergence.
5. Reconfirm CurseForge technical green and resolve release bookkeeping in the combined strategy.
6. Create and validate `integration/package-discovery-foundation`.
7. Build and validate `agent/canonical-modpack`.
8. Build and validate `agent/player-launcher-journey`.
9. Build and validate `integration/player-launcher-v1` with fresh-install/cross-platform acceptance.

## Current next action

Validate the current Discovery documentation head. If green, make the final READY ledger promotion and validate that exact head, then move directly to Catalog formatting and its dedicated suite.
