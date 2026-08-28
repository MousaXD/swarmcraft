# Agent 2 — Modrinth Integration

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/modrinth-provider`
- Latest implementation/validation head before this ledger commit: `621085f34adf6ccdfef780694099637a8c1cd784`
- Green/final head: pending exact-head CI validation.

## Mission

Implement a backend-owned Modrinth provider for browsing/searching projects, selecting compatible versions/files, resolving dependencies, and downloading/verifying permitted artifacts for Fabric server worlds.

## Dependencies to read

- `progress/README.md`
- Agent 1 Minecraft/Fabric catalog contract before final handoff.

## Dependencies consumed

- Coordination rules from `progress/README.md` on the Agent 2 branch.
- Agent 1 current implementation contract at `agent/minecraft-fabric-catalog` head `f450c0ffb2029671006c831bc3cfe26dbd7bb752` (PR #47). Agent 1 exposes source-backed exact values as `MinecraftVersion.id: String` and `FabricLoaderVersion.version: String`, with `CatalogService::validate_fabric_selection(minecraft_version: &str, fabric_loader_version: &str, refresh: bool)` as the authority for validating the pair.
- Reconciliation decision: no Agent 2 code change is required. `ModSearchQuery`, `ModVersionFilter`, and `ModResolveRequest` already consume exact Minecraft and loader strings. Agent 2 does not duplicate Mojang/Fabric catalog lookup or pair validation. Agent 4/7 should feed Agent 1-validated values into these provider requests.

## Work completed

- Verified the requested base and created/continued `agent/modrinth-provider` without rewinding newer valid work. The branch had advanced beyond the previously reported `618672b9867f30da1f2c7cbd464d6442deb1d290`; that newer work was preserved.
- Reviewed current official Modrinth v2 documentation for search facets, project/version endpoints, stable identifiers, version-file hashes, dependency metadata, rate-limit headers, identifying `User-Agent`, and 410 behavior for retired API generations.
- Added backend-owned provider-neutral package types under `swarm_cli::package_provider` and a production Modrinth implementation under `swarm_cli::package_provider::modrinth`.
- Added exact Minecraft/Fabric/environment/release filtering and explicit client-only rejection for required server preparation.
- Added dependency resolution that preserves optional dependencies, detects cycles, unresolved required dependencies, incompatible selected versions, and conflicting required versions of one project.
- Added exact artifact download by provider project/version/hash identity with HTTPS/CDN trust boundary, configurable bounded size, temporary file, provider SHA-1/SHA-512 verification, computed local SHA-256, fsync, rollback-safe publication, and partial-file cleanup.
- Added structured provider failures for malformed data, rate limiting, unavailable provider, removed resources, compatibility failures, dependency failures, hash mismatch, interrupted download, restricted retrieval, and I/O errors.
- Added excluded `swarm-provider` Desktop adapter crate that re-exports the canonical `swarm-cli` provider contract rather than duplicating provider logic.
- Added thin Tauri commands: `modrinth_search`, `modrinth_project`, `modrinth_versions`, `modrinth_resolve`, `modrinth_download`.
- Added deterministic mocked provider tests plus an ignored opt-in live Modrinth validation.
- Expanded the deterministic matrix with positive required-dependency resolution, optional-edge preservation in a successful graph, selected incompatibility rejection, oversize pre-download rejection, exact artifact identity enforcement, manual-remediation behavior, and structured provider-error serialization.
- Removed temporary formatting and lockfile-regeneration workflows after their one-shot validation purpose; no Agent 2 helper workflow remains in the handoff tree.

## Contracts / APIs added or changed

Canonical Rust contract path: `swarm_cli::package_provider`.

Provider ID:

- `ProviderId::Modrinth` serializes as `"modrinth"`.

Core request/result types:

- `ModSearchQuery { query, minecraft_version, loader, environment, release_type, offset, limit }`
- `ModSearchResult { items, offset, limit, total_hits, rate_limit }`
- `ModProjectSummary { provider, project_id, slug, title, description, icon_url, categories }`
- `ModProjectDetails { summary, status, project_type, environments, minecraft_versions, loaders, license }`
- `ModVersionFilter { minecraft_version, loader, environment, release_type }`
- `ModVersion { provider, project_id, version_id, display_name, version_number, minecraft_versions, loaders, environment, release_type, published_at, dependencies, files }`
- `ModResolveRequest { root_version_id, minecraft_version, loader, environment, allowed_release_types }`
- `ResolvedModGraph { provider, root_version_id, versions, optional_dependencies, incompatibilities }`
- `ModDownloadRequest { locator, destination_dir, max_bytes }`
- `DownloadedArtifact { provider, project_id, version_id, filename, path, size, source_url, hashes }`

Dependency representation:

- `ModDependency { kind, project_id, version_id, file_name }`
- `DependencyKind::{Required, Optional, Incompatible, Embedded}`.
- Required dependencies are recursively resolved to exact versions compatible with the requested Minecraft/Fabric/environment/release policy.
- Optional dependencies remain optional in `ResolvedModGraph.optional_dependencies`; they are never silently promoted.
- Incompatible edges are preserved and checked against selected versions/projects; a conflict fails closed.
- Embedded dependencies are metadata, not recursively installed as separate required artifacts.

Artifact identity/retrieval:

- `ModArtifactLocator { provider, project_id, version_id, sha1, sha512 }`.
- `ModArtifact { filename, url, locator, primary, size, hashes, file_type, retrieval }`.
- `ArtifactRetrieval::ProviderDownload` or `ArtifactRetrieval::ManualRequired { reason }`.
- `ArtifactHashes` carries provider SHA-1/SHA-512 and computed local SHA-256 after acquisition.
- Automatic download requires exact provider project/version plus at least one provider hash. The exact version is re-fetched and the file is matched by hash; filename alone is never trusted.
- A provider URL is retrieval metadata only, not canonical identity. Ephemeral/mutable URLs must not be signed into canonical package identity.

Structured failure contract:

- `ProviderFailure { provider, kind, message, retry_after_seconds, remediation, details }`.
- `ProviderFailureKind::{InvalidRequest, RateLimited, Unavailable, NotFound, MalformedResponse, Incompatible, DependencyCycle, UnresolvedDependency, HashMismatch, DownloadInterrupted, RetrievalRestricted, Io}`.
- The type derives `Serialize`; Tauri commands return `Result<_, ProviderFailure>` directly so provider failures remain structured across the bridge rather than being flattened to strings.

Tauri commands:

- `modrinth_search(query: ModSearchQuery) -> Result<ModSearchResult, ProviderFailure>`
- `modrinth_project(projectId: String) -> Result<ModProjectDetails, ProviderFailure>`
- `modrinth_versions(projectId: String, filter: ModVersionFilter) -> Result<ModVersionList, ProviderFailure>`
- `modrinth_resolve(request: ModResolveRequest) -> Result<ResolvedModGraph, ProviderFailure>`
- `modrinth_download(request: ModDownloadRequest) -> Result<DownloadedArtifact, ProviderFailure>`

Provider endpoints are centralized in backend code under `https://api.modrinth.com/v2`. Production requests identify as `MousaXD/swarmcraft/<version> (https://github.com/MousaXD/swarmcraft)`.

## Files changed

- `Cargo.toml`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/main.rs`
- `apps/desktop/src-tauri/src/modrinth_commands.rs`
- `crates/swarm-cli/src/lib.rs`
- `crates/swarm-cli/src/package_provider.rs`
- `crates/swarm-cli/src/package_provider/modrinth.rs`
- `crates/swarm-cli/tests/modrinth_provider.rs`
- `crates/swarm-cli/tests/modrinth_provider_matrix.rs`
- `crates/swarm-provider/Cargo.toml`
- `crates/swarm-provider/src/lib.rs`
- `progress/agent2.md`

Temporary validation-only workflows were created and removed on this branch; they are not part of the final handoff tree.

## Tests and evidence

Deterministic provider coverage now exercises:

- search parsing;
- pagination and rate-limit metadata;
- exact Minecraft compatibility;
- Fabric compatibility and loader rejection;
- server environment filtering;
- incompatible client-only mods;
- required dependency resolution;
- optional dependency preservation;
- incompatible dependency rejection;
- dependency cycles;
- unresolved dependencies;
- conflicting selected versions;
- malformed provider response;
- provider unavailable transport failure;
- HTTP 404;
- rate limiting and retry/reset metadata;
- interrupted download;
- oversize download rejection before transport/publication;
- provider hash mismatch;
- successful verified download;
- computed SHA-256 production;
- cleanup of partial files and preservation of an existing destination on failure;
- exact provider project/version/hash artifact identity;
- manual-remediation state for non-installable/restricted artifacts;
- structured `ProviderFailure` serialization used by Tauri.

Live provider validation remains ignored/opt-in via `SWARMCRAFT_LIVE_MODRINTH=1`; deterministic CI does not depend on Modrinth availability.

Lockfile evidence:

- A one-shot CI job ran Cargo metadata resolution for both the root workspace and excluded Desktop manifest, then immediately re-ran both with `--locked` successfully.
- Neither `Cargo.lock` nor `apps/desktop/src-tauri/Cargo.lock` changed, so both committed lockfiles already represented the Agent 2 dependency graphs. The one-shot workflow removed itself in commit `621085f34adf6ccdfef780694099637a8c1cd784`.

Full formatting/compile/clippy/test and Desktop package CI on the exact handoff head is pending below; do not treat this ledger state as READY until that exact head is green.

## Decisions / invariants

- Do not sign or install ambiguous `latest` requirements.
- Agent 1 remains the version-selection authority. Agent 2 accepts exact validated Minecraft/Fabric strings and does not maintain a second catalog.
- Preserve exact provider project/version/file identifiers and hashes for Agent 4.
- Modrinth stable project/version IDs are durable provider locators; mutable slugs are display/navigation metadata only.
- Provider-published SHA-1/SHA-512 identify the exact provider file; computed SHA-256 is the acquired-byte integrity identity SwarmCraft can freeze locally/canonically.
- Provider URLs are retrieval metadata, never canonical identity.
- Machine-local filesystem paths are installation state, never canonical identity.
- Respect Modrinth provider/source download boundaries. No peer-to-peer redistribution is introduced.
- Provider/API failures surface as structured failures, never empty-success fallbacks.
- Compatibility stays backend-owned. Search/version resolution requires exact Minecraft version and Fabric loader inputs for server preparation.
- Clearly client-only and unknown-environment versions are not accepted as required server mods.
- Automatic artifact download is HTTPS-only, bounded, provider-hash-verified, and atomically published only after fsync.
- A file that is not a trusted Modrinth CDN installable JAR with a provider-published SHA-1 or SHA-512 is exposed as `manual_required`; the provider does not bypass that state or substitute another artifact.

## Known issues / blockers

- Exact-head CI is the only remaining gate. Until formatting, clippy, workspace tests, acceptance checks, dependency audit, and Desktop packaging finish green on the same pushed head, status remains `IN PROGRESS`.

## Handoff for dependent agents

Agent 4 should consume `swarm_cli::package_provider` rather than reimplementing Modrinth JSON/HTTP rules.

Canonical package identity should freeze:

- `provider = modrinth`;
- exact `project_id`;
- exact `version_id`;
- exact provider file identity via SHA-1 and/or SHA-512 locator;
- selected Minecraft version and Fabric compatibility values that originated from Agent 1 validation;
- environment;
- resolved required dependency edges/versions;
- optional dependency edges separately, without promotion;
- provider SHA-1/SHA-512;
- computed artifact SHA-256 once acquired;
- automatic-download vs manual-remediation state.

Do not canonicalize mutable slug, source/download URL, or machine-local filesystem path. Modrinth project/version IDs are the durable locator; hashes are authoritative for exact file bytes.

`DownloadedArtifact` is machine-local acquisition output. Agent 4 should retain IDs/hashes/size/filename as appropriate but must not sign the local path or depend on the source URL for identity.

Restricted/unavailable retrieval fails closed through `ArtifactRetrieval::ManualRequired` or `ProviderFailureKind::RetrievalRestricted` with remediation. Do not silently select another version or file.

Exact final green SHA: pending exact-head CI validation.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 @ `41c9b5b650aac1e320195f6e1855945f2722abc4` — started Agent 2 on `agent/modrinth-provider`; consumed coordination guidance and official Modrinth API contract.
- 2026-08-24 @ `618672b9867f30da1f2c7cbd464d6442deb1d290` — provider types/client/resolver/download safety, Tauri bridge and deterministic tests implemented.
- 2026-08-24 @ `d383fba934f20d5cbcdeca951392b43017b2abeb` — preserved later formatting/cleanup work already present on the branch instead of rewinding to the earlier milestone.
- 2026-08-24 @ `e3a12887e6605626802dad2d7fbed3c004de2af4` — expanded deterministic provider matrix for required/optional/incompatible dependencies, oversize behavior, exact identity, manual remediation, and structured bridge errors.
- 2026-08-24 @ `621085f34adf6ccdfef780694099637a8c1cd784` — root and Desktop Cargo metadata both validated with `--locked`; temporary lockfile helper removed itself with no lockfile diff.
