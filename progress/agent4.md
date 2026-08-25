# Agent 4 — Canonical Modpack

## Status

`BLOCKED`

## Branch / exact head

- Branch: `agent/canonical-modpack`
- Administrative branch base SHA: `41c9b5b650aac1e320195f6e1855945f2722abc4`
- Implementation head: none; dependency gate has not opened.
- Current branch head is the ledger-only blocked-state commit created by this update; GitHub branch state is authoritative.

## Mission

Own the canonical deterministic world-owned modpack definition that unifies exact Minecraft/Fabric selection, exact provider package/version identity, resolved dependency relationships, and artifact integrity metadata without leaking provider API response shapes or machine-local paths into canonical world state.

## Dependencies to read

- `progress/README.md`
- `progress/agent1.md`
- `progress/agent2.md`
- `progress/agent3.md`

Implementation MUST NOT begin until all three dependency branches are genuinely `READY FOR INTEGRATION` on exact green SHAs.

## Dependencies consumed

None. No Agent 1, Agent 2, or Agent 3 head has been merged or treated as a canonical input because all three exact current heads fail their integration gate.

Observed only, not consumed:

- Agent 1 branch head: `68c6713d6658b0bcc6011803f9684564e3e562c1`
- Agent 2 branch head: `c5d76875c33645bd64c6bc0109c8adef68d68621`
- Agent 3 branch head: `344f086eaa7499ba2e4dfa86f6e27cd3410f5d5a`

All three branches still descend directly from common merge base `41c9b5b650aac1e320195f6e1855945f2722abc4`; no integration branch containing green heads exists yet.

## Work completed

- Verified the live remote heads for Agents 1–3 rather than trusting stale ledgers.
- Verified exact-head GitHub Actions evidence for each dependency.
- Confirmed the three upstream ledgers still report `IN PROGRESS`; none publishes an exact green READY SHA.
- Created `agent/canonical-modpack` from the common dependency merge base only to preserve Agent 4 coordination state. No upstream feature head was merged.
- Performed non-destructive architecture inspection of the existing world compatibility/persistence path and Agent 2 provider-neutral contract.

No canonical schema or runtime behavior has been implemented because the dependency gate is closed.

## Contracts / APIs added or changed

None. No production contract was changed while blocked.

### Architectural observations for implementation after the gate opens

1. Existing canonical world compatibility already lives inside signed `swarm_protocol::WorldConfigV1` as `RuntimeCompatibilityManifestV1` and is persisted by `swarm_storage::Storage::{save_world_config, load_world_config}` to `metadata/world-config.postcard`.
2. `RuntimeCompatibilityManifestV1` already owns exact `minecraft_version`, `loader_id`, `loader_version`, and normalized artifact requirement lists. Its fingerprint normalizes artifact ordering before postcard hashing, so the existing world-config path is the first place to extend/version rather than creating a parallel modpack store.
3. Existing `ArtifactRequirementV1` contains only `artifact_id`, `version`, exact `artifact_hash`, `side`, and optional `provider_hint`. It does not currently express provider namespace + project ID + exact provider version/file ID + dependency graph + retrieval state. Agent 4 must reconcile this through an explicit versioned protocol/migration design once upstream contracts are green; blindly widening signed/postcard V1 state is not acceptable.
4. Existing `server_mods::requirements_from_jars` deterministically inspects Fabric JARs, rejects duplicate mod IDs, computes exact artifact hashes, and feeds `ArtifactRequirementV1`. `add_local_mod` and Host Readiness verify local bytes against the world compatibility manifest. Agent 4 should preserve this integrity boundary rather than replacing it with provider URLs or filenames.
5. Agent 2 currently exposes provider-neutral package contracts under `swarm_cli::package_provider`, including exact project/version/artifact locators, dependency kinds, deterministic resolved graphs, retrieval state, and hashes. However its live `ProviderId` enum is currently Modrinth-only and `ArtifactHashes` has SHA-1/SHA-512/SHA-256 but not CurseForge MD5.
6. Agent 3's ledger documents exact CurseForge project/file identity, provider SHA-1/MD5, computed local SHA-256, required/optional/incompatible dependency semantics, and manual-artifact remediation. Shared representation with Agent 2 therefore still needs deliberate reconciliation after both branches are green.

## Files changed

- `progress/agent4.md` only.

No production Rust, Desktop, protocol, storage, migration, or workflow files were changed.

## Tests and evidence

Agent 4 tests: not run, because implementation is prohibited while dependencies are not READY.

Dependency gate evidence observed on 2026-08-25:

### Agent 1

- Exact head: `68c6713d6658b0bcc6011803f9684564e3e562c1`
- Workflow run: `32744102086` — `Agent 1 Catalog Validation` — `FAILURE`
- Job: `97485342280` — `validate` — `FAILURE`
- Workspace format/check/clippy/tests and catalog tests passed.
- Failure step: `Desktop format`.
- Desktop locked metadata/check/clippy/tests were skipped after that failure.

### Agent 2

- Exact head: `c5d76875c33645bd64c6bc0109c8adef68d68621`
- Workflow run: `32735413224` — `Agent 2 final validation` — `FAILURE`
- Job: `97456913305` — `validate` — `FAILURE`
- Workspace format/check/clippy/tests and deterministic Modrinth tests passed.
- Failure step: `Desktop check`.
- Desktop clippy/tests and provider adapter check were skipped after that failure.

### Agent 3

- Exact head: `344f086eaa7499ba2e4dfa86f6e27cd3410f5d5a`
- Exact-head workflow runs include:
  - `32743739906` — `.github/workflows/agent3-curseforge-provider.yml` — `FAILURE` with no jobs materialized.
  - `32743742646` — `.github/workflows/ci.yml` — `FAILURE`.
  - `32743741549` — `.github/workflows/agent1-lockfiles.yml` — `FAILURE`.
  - `32743746485` — `Release version guard` — `FAILURE`.
- No exact-head green integration evidence exists.

## Decisions / invariants

- GitHub exact-head CI evidence outranks stale progress ledgers.
- No upstream SHA is recorded as consumed until that exact SHA is `READY FOR INTEGRATION` and green.
- The branch base `41c9b5b650aac1e320195f6e1855945f2722abc4` is administrative/provisional only; it is not the future implementation base. Once the gate opens, Agent 4 must intentionally integrate the exact green Agent 1/2/3 heads and record the resulting base/merge state.
- Do not create a second modpack persistence store while `WorldConfigV1.compatibility` already owns signed world compatibility state.
- Do not sign provider URLs, machine-local paths, mutable slugs, or `latest` selectors.
- Provider namespace is part of canonical package identity. Modrinth IDs and CurseForge IDs are not globally comparable.
- Dependency ordering and package ordering must be explicit and deterministic; never rely on HashMap or provider response order.
- Conflicting exact dependency requirements must fail with a typed conflict; do not choose a winner silently.
- Existing Host Readiness/local-JAR integrity verification must remain authoritative for installed bytes.

## Known issues / blockers

### Hard dependency blockers

1. Agent 1 is not READY: exact head `68c6713d6658b0bcc6011803f9684564e3e562c1` fails exact-head validation at Desktop formatting.
2. Agent 2 is not READY: exact head `c5d76875c33645bd64c6bc0109c8adef68d68621` fails exact-head validation at Desktop check.
3. Agent 3 is not READY: exact head `344f086eaa7499ba2e4dfa86f6e27cd3410f5d5a` has no green exact-head validation; multiple workflows fail.

Until all three are fixed and publish exact green READY heads, Agent 4 MUST NOT implement or integrate canonical schema changes.

### Contract reconciliation to revisit only after upstream green

- Extend/reconcile provider namespace so CurseForge is represented alongside Modrinth without provider-specific branching downstream.
- Reconcile hash vocabulary: Modrinth SHA-1/SHA-512/SHA-256 vs CurseForge SHA-1/MD5/local SHA-256.
- Decide explicit versioned migration from existing `RuntimeCompatibilityManifestV1` / `ArtifactRequirementV1` rather than silently mutating persisted postcard V1 data.
- Preserve existing Fabric mod ID/hash readiness semantics while adding exact provider project/version/file identity and dependency edges.

## Handoff for dependent agents

Agent 7 MUST NOT consume Agent 4 yet.

Agent 4 status is `BLOCKED`; there is no canonical-modpack implementation SHA. The next Agent 4 execution must first re-fetch Agents 1–3, confirm all three are `READY FOR INTEGRATION` with exact green CI, then intentionally integrate those exact heads before writing schema/runtime code.

## Activity log

- 2026-08-24 — original ledger created; waiting for provider/runtime contracts before final schema work.
- 2026-08-25 @ base `41c9b5b650aac1e320195f6e1855945f2722abc4` — created `agent/canonical-modpack` as a blocked-state coordination branch; verified Agents 1–3 exact remote heads and exact-head CI failures; performed non-destructive architecture inspection; status set to `BLOCKED`; no upstream dependency consumed and no production implementation started.
