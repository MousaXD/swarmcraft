# Agent 3 — CurseForge Integration

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/curseforge-provider`
- Starting head: `41c9b5b650aac1e320195f6e1855945f2722abc4`
- Current implementation head: `81c601ae087b92ae6a78ea0e3e98cce1de42530a`
- Exact final head: `TBD`

## Mission

Implement a backend-owned CurseForge provider for project/file browsing, compatible file selection, dependency metadata, and permitted download/install flows for Fabric server worlds.

## Dependencies to read

- `progress/README.md`
- Read Agent 1 when consuming shared Minecraft/Fabric version identifiers or compatibility contracts.
- Read Agent 2 if a shared provider abstraction is already established. Reuse it rather than creating a competing provider model.

## Dependencies consumed

- Base coordination state: `41c9b5b650aac1e320195f6e1855945f2722abc4`.
- `progress/agent1.md` at base SHA: no implementation contract published yet.
- `progress/agent2.md` at base SHA: no shared provider abstraction published yet.

## Work completed

- Verified `backup/local-work-20260824` is exactly the requested starting SHA.
- Reviewed the current official CurseForge REST API documentation and implemented only documented API calls; no curseforge.com scraping and no fabricated CDN URLs.
- Added a credential-gated CurseForge client using `https://api.curseforge.com` and the `x-api-key` header.
- Added Fabric + Minecraft compatibility filtering, structured project/file metadata, deterministic file ordering, recursive required-dependency resolution, optional-dependency preservation, and incompatible-selection detection.
- Added provider-restricted download handling that returns `manual_artifact_required` with exact project/file/name/hash/size/project-page metadata.
- Added automatic download hardening: HTTPS-only provider-returned URLs, 512 MiB bound, same-directory temporary files, streaming SHA-1/MD5/provider verification, local SHA-256, fsync, atomic rename, and cleanup-on-failure.
- Added six provider Tauri commands without implementing final Desktop create-world UX.
- Added deterministic fixture/unit coverage for the provider contract and failure classification; validation execution is pending remote CI because the current execution container has no Rust toolchain.

## Contracts / APIs added or changed

Machine-local credential:

- `SWARMCRAFT_CURSEFORGE_API_KEY`: optional process environment variable. When absent/blank, provider commands return `status = "configuration_required"`, `error.code = "missing_api_credential"`; unrelated SwarmCraft functionality is unaffected.

Tauri commands:

- `curseforge_provider_status()`
- `curseforge_search(query, minecraft, loader, environment, index, pageSize)`
- `curseforge_project(projectId)`
- `curseforge_versions(projectId, minecraft, loader)`
- `curseforge_resolve(fileId, minecraft, loader, environment)`
- `curseforge_download(fileId, destination)`

Common structured envelope:

- Success browsing/resolution: `{ status: "ok", provider: "curseforge", data: ... }`.
- Structured failures: `{ status, provider: "curseforge", error: { code, message, retry_after_seconds } }`.
- Automatic artifact success: `{ status: "downloaded", provider: "curseforge", data: { package, destination, bytes, local_sha256, provider_hashes_verified, ... } }`.
- Provider-restricted automatic download: `{ status: "manual_artifact_required", provider: "curseforge", data: { project_id, file_id, version_id, name, file_name, file_size, hashes, project_url, reason } }`.

Provider file metadata includes:

- `provider = "curseforge"`
- exact `project_id`
- exact `file_id` / `version_id`
- display `name` and exact `file_name`
- CurseForge game-version tags and loader tags
- `release_type`
- requested environment plus explicit `applicability = "unknown"` because CurseForge Core API file metadata does not expose a reliable ordinary-mod client/server side flag
- dependencies with relation kind and required/optional booleans
- download availability state
- provider hashes
- `file_size`
- provider availability/date fields

Dependency representation:

- Relation type 3 -> required; recursively selected.
- Relation type 2 -> optional; exposed with `automatically_selected = false` and never silently promoted to required.
- Relation type 5 -> incompatible; a graph containing both incompatible projects fails with `impossible_dependency_selection`.
- Required dependencies choose deterministically by newest compatible `fileDate`, then highest file ID; resolution is capped at 128 packages.
- Agent 4 remains responsible for canonical lock generation and any canonical schema.

CurseForge API mapping:

- Search: `GET /v1/mods/search` with Minecraft game, Mods class, exact `gameVersion`, and Fabric `modLoaderType`.
- Project: `GET /v1/mods/{modId}`.
- Versions: `GET /v1/mods/{modId}/files` with exact Minecraft/Fabric filters.
- Exact file lookup: official `POST /v1/mods/files` bulk-file endpoint using one exact file ID.
- Download permission/URL: file `downloadUrl` when supplied, otherwise official `GET /v1/mods/{modId}/files/{fileId}/download-url`; 403/404/no URL becomes manual remediation rather than a guessed URL.

## Files changed

- `progress/agent3.md`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/curseforge.rs`
- `apps/desktop/src-tauri/src/main.rs`

## Tests and evidence

Implemented deterministic unit/fixture tests in `curseforge.rs` for:

- missing/blank credential normalization;
- invalid credential mapping;
- rate limiting plus retry-after mapping;
- removed project and removed file mapping;
- provider unavailable mapping;
- malformed response rejection;
- distinct required vs optional dependencies;
- no compatible Minecraft version vs no Fabric build;
- selected file Minecraft/Fabric validation;
- impossible dependency selection;
- exact `manual_artifact_required` remediation metadata;
- HTTPS-only automatic download URLs;
- provider SHA-1/MD5 verification and hash mismatch rejection;
- interrupted/oversized download size rejection;
- deterministic newest-date/highest-file-ID selection.

Execution evidence pending. The current execution container does not provide `cargo`/`rustc`, so owned Rust/Desktop checks will be run through repository CI before handoff.

## Decisions / invariants

- Respect CurseForge API terms, project permissions, and download restrictions.
- Do not proxy or peer-redistribute artifacts that are not permitted for redistribution.
- Exact provider/file identity must be available to Agent 4 for canonical locking.
- If automatic download is unavailable, return a structured remediation requirement rather than pretending installation succeeded.
- Since Agents 1 and 2 have no published implementation contract on the required base, keep Agent 3 provider types CurseForge-owned and integration-friendly rather than claiming a shared canonical schema.
- No API key is committed or logged. Credential configuration is machine-local environment only.
- Never synthesize `edge.forgecdn.net` or another mirror URL. Only a CurseForge API-returned HTTPS URL may be fetched automatically.
- Optional dependencies remain optional. Canonical dependency locking remains Agent 4's responsibility.

## Known issues / blockers

- No CurseForge API credential is assumed present. This is a supported configuration state, not a blocker for the rest of SwarmCraft.
- Remote compile/test validation is still pending.
- `apps/desktop/src-tauri/Cargo.lock` has not yet been regenerated for the new provider dependencies because the execution container has no Rust toolchain; this must be resolved or explicitly validated before READY FOR INTEGRATION.

## Handoff for dependent agents

Agent 4 should consume exact CurseForge `project_id` + `file_id`, provider hashes, file size, release type, compatibility tags, dependency edges, and download-remediation state. Do not treat the provider's deterministic dependency selection as the canonical lock itself; Agent 4 owns canonical locking.

Agent 7 may use the Tauri commands above for browse/install UX. In particular, handle `configuration_required`, `rate_limited`, `incompatible`, `manual_artifact_required`, `download_failed`, and `downloaded` as distinct states.

The exact green final SHA will replace `TBD` after validation.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 — `41c9b5b650aac1e320195f6e1855945f2722abc4` — verified base, consumed required ledgers, reviewed official CurseForge API, and started `agent/curseforge-provider`.
- 2026-08-24 — `9fb985a23eab71f6a9f039a737fbd2d9bdf5245c` — implemented CurseForge provider core, deterministic resolution, structured failures, hardened automatic download, and fixtures/tests.
- 2026-08-24 — `81c601ae087b92ae6a78ea0e3e98cce1de42530a` — wired six CurseForge Tauri commands; remote validation and lockfile refresh remain.
