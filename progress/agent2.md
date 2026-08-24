# Agent 2 — Modrinth Integration

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/modrinth-provider`
- Current implementation head before this ledger-only commit: `618672b9867f30da1f2c7cbd464d6442deb1d290`
- Green/final head: pending CI validation.

## Mission

Implement a backend-owned Modrinth provider for browsing/searching projects, selecting compatible versions/files, resolving dependencies, and downloading/verifying permitted artifacts for Fabric server worlds.

## Dependencies to read

- `progress/README.md`
- Read Agent 1 when consuming shared Minecraft/Fabric version identifiers or compatibility contracts.

## Dependencies consumed

- `progress/README.md` at `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- `progress/agent1.md` at `41c9b5b650aac1e320195f6e1855945f2722abc4`; Agent 1 was `NOT STARTED` and had not published shared catalog identifier types. This provider therefore accepts explicit Minecraft/loader strings and does not invent Agent 1 catalog types.

## Work completed

- Verified the requested base commit and created `agent/modrinth-provider` from it.
- Reviewed the Runtime Installer artifact publication pattern and mirrored its temp-file/hash/fsync/rename/rollback safety invariants.
- Reviewed current official Modrinth v2 API documentation for search facets, project/version endpoints, dependency/file metadata, rate-limit headers, stable IDs, file hashes, and identifying `User-Agent` behavior.
- Added backend-owned provider-neutral package types under `swarm_cli::package_provider` and a production Modrinth implementation under `swarm_cli::package_provider::modrinth`.
- Added exact Minecraft/Fabric/environment/release filtering and explicit client-only rejection for required server preparation.
- Added dependency resolution that preserves optional dependencies, detects cycles, unresolved required dependencies, incompatible selected versions, and missing compatible dependency versions.
- Added exact artifact download by provider project/version/hash identity with HTTPS/CDN trust boundary, configurable bounded size, temporary file, provider SHA-1/SHA-512 verification, local SHA-256, fsync, atomic/rollback-safe publication, and partial-file cleanup.
- Added structured provider failures for malformed data, rate limiting, unavailable provider, removed resources, compatibility failures, dependency failures, hash mismatch, interrupted download, restricted retrieval, and I/O errors.
- Added a small excluded `swarm-provider` Desktop adapter crate that re-exports the canonical `swarm-cli` provider source rather than duplicating provider logic. This keeps the root committed `Cargo.lock` graph unchanged while allowing Tauri to use strongly typed requests/results.
- Added thin Tauri commands: `modrinth_search`, `modrinth_project`, `modrinth_versions`, `modrinth_resolve`, `modrinth_download`.
- Added deterministic mocked provider tests to the locked root workspace plus an ignored opt-in live Modrinth validation.

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
- Optional dependencies remain optional and are returned in `ResolvedModGraph.optional_dependencies`; they are not silently promoted to required.

Artifact identity/retrieval:

- `ModArtifactLocator { provider, project_id, version_id, sha1, sha512 }`.
- `ModArtifact { filename, url, locator, primary, size, hashes, file_type, retrieval }`.
- `ArtifactRetrieval::ProviderDownload` or `ArtifactRetrieval::ManualRequired { reason }`.
- `ArtifactHashes` carries provider SHA-1/SHA-512 and local SHA-256 after download.
- Automatic download requires an exact provider hash locator and re-fetches the exact version before matching the file. Filename alone is never trusted.

Structured failure contract:

- `ProviderFailure { provider, kind, message, retry_after_seconds, remediation, details }`.
- `ProviderFailureKind::{InvalidRequest, RateLimited, Unavailable, NotFound, MalformedResponse, Incompatible, DependencyCycle, UnresolvedDependency, HashMismatch, DownloadInterrupted, RetrievalRestricted, Io}`.

Tauri commands:

- `modrinth_search(query)`
- `modrinth_project(projectId)`
- `modrinth_versions(projectId, filter)`
- `modrinth_resolve(request)`
- `modrinth_download(request)`

Provider endpoints are centralized in backend code under the official `https://api.modrinth.com/v2` base. Production requests identify as `MousaXD/swarmcraft/<version> (https://github.com/MousaXD/swarmcraft)`.

## Files changed

- `Cargo.toml`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/main.rs`
- `apps/desktop/src-tauri/src/modrinth_commands.rs`
- `crates/swarm-cli/src/lib.rs`
- `crates/swarm-cli/src/package_provider.rs`
- `crates/swarm-cli/src/package_provider/modrinth.rs`
- `crates/swarm-cli/tests/modrinth_provider.rs`
- `crates/swarm-provider/Cargo.toml`
- `crates/swarm-provider/src/lib.rs`
- `progress/agent2.md`

## Tests and evidence

Implemented deterministic coverage for:

- search parsing, pagination, rate-limit metadata and server compatibility facets;
- exact Minecraft/Fabric/server filtering and incompatible client/loader/Minecraft versions;
- required/optional/incompatible/embedded dependency parsing;
- malformed JSON;
- HTTP 404;
- HTTP 429 and retry/reset metadata;
- provider unavailable transport failure;
- dependency cycle detection;
- unresolved required dependency;
- conflicting required versions of the same project;
- client-only exact version rejected for required server resolution;
- hash mismatch cleanup without replacing an existing artifact;
- interrupted download cleanup with no partial publication;
- verified publication and local SHA-256 recording;
- ignored opt-in live provider validation gated by `SWARMCRAFT_LIVE_MODRINTH=1`.

Execution evidence: pending GitHub CI on the current branch because this execution container has no Rust toolchain or outbound Git transport.

## Decisions / invariants

- Do not sign or install ambiguous “latest” requirements.
- Preserve exact provider project/version/file identifiers and hashes for Agent 4.
- Modrinth stable project/version IDs are the canonical provider identifiers; mutable slugs are display/navigation metadata only.
- Respect Modrinth provider/source download boundaries. No peer-to-peer redistribution is introduced.
- Provider/API failures surface as structured failures, never empty-success fallbacks.
- Production metadata requests use Modrinth API v2 and a uniquely identifying SwarmCraft `User-Agent`.
- Compatibility stays backend-owned. Search/version resolution requires exact Minecraft version and Fabric loader inputs for server preparation.
- Clearly client-only and unknown-environment versions are not accepted as required server mods.
- Automatic artifact download is HTTPS-only, bounded, provider-hash-verified, and atomically published only after fsync.
- A file that is not a trusted Modrinth CDN JAR with a provider-published SHA-1 or SHA-512 is exposed as `manual_required`; the provider does not bypass that state.

## Known issues / blockers

- Agent 1 had not published a shared catalog identifier contract at the consumed base SHA. This is not a blocker because provider compatibility inputs remain explicit strings and can consume Agent 1 IDs later without changing Modrinth HTTP semantics.
- Final status is blocked only on exact-head formatting/compile/test validation.

## Handoff for dependent agents

Agent 4 should consume `swarm_cli::package_provider` rather than reimplement provider JSON or HTTP details. Canonicalization should pin `provider=modrinth`, exact `project_id`, exact `version_id`, an exact artifact locator/hash identity, Minecraft version, Fabric loader/environment, and the required resolved dependency set. Optional dependencies are separately represented and must remain optional unless Agent 4 explicitly changes canonical policy.

`DownloadedArtifact` is machine-local installation output and contains the exact provider IDs, filename, provider source URL, provider hashes, computed local SHA-256 and final path. Agent 4 should canonicalize identity/hashes, not the machine-local path.

Restricted/unavailable retrieval is fail-closed through `ArtifactRetrieval::ManualRequired` or `ProviderFailureKind::RetrievalRestricted` with a remediation string. Do not substitute another version/file.

Exact final green SHA: pending CI validation.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 @ `41c9b5b650aac1e320195f6e1855945f2722abc4` — started Agent 2 on `agent/modrinth-provider`; consumed required progress dependencies and official Modrinth API contract; implementation in progress.
- 2026-08-24 @ `618672b9867f30da1f2c7cbd464d6442deb1d290` — provider types/client/resolver/download safety, Tauri bridge and deterministic tests implemented; moved canonical source into the existing locked `swarm-cli` workspace package before CI validation.
