# Agent 3 — CurseForge Integration

## Status

`IN PROGRESS`

## Branch / exact head

- Branch: `agent/curseforge-provider`
- Starting head: `41c9b5b650aac1e320195f6e1855945f2722abc4`
- Original implementation milestone: `81c601ae087b92ae6a78ea0e3e98cce1de42530a`
- Latest validation-gate parent observed before this ledger update: `5b6f1093ae56dceef68059a7885501ecd66a763b`
- Exact final green head: `TBD`

## Mission

Implement and validate a backend-owned CurseForge provider for project/file browsing, compatible file selection, dependency metadata/resolution, and permitted artifact acquisition for Fabric worlds. Agent 4 remains the owner of canonical modpack/package-lock schema.

## Dependencies consumed

- Coordination rules from `progress/README.md` on this branch.
- Agent 1 live branch observed at `2e87e2a1e7ba4f375bc47001dbab937271a31f94`; its ledger publishes exact source-backed `MinecraftVersion.id` and `FabricLoaderVersion.version` strings plus `CatalogService::validate_fabric_selection` as the Minecraft/Fabric pair authority.
- Agent 2 live branch observed at `c5d76875c33645bd64c6bc0109c8adef68d68621`; its latest ledger publishes provider-neutral `ModSearchQuery`, `ModVersion`, `ModDependency`, `ArtifactHashes`, `ModArtifactLocator`, `ArtifactRetrieval`, `ResolvedModGraph`, `DownloadedArtifact`, and structured `ProviderFailure` concepts.
- Agent 2's current `ProviderId` is still Modrinth-only on its branch. Agent 3 does not copy that file into this isolated branch or create a competing shared schema. Agent 4/integration can extend the shared enum with CurseForge while mapping the provider-owned fields documented below.

## Shared-contract reconciliation

- Agent 1 `MinecraftVersion.id` maps directly to Agent 3's exact `minecraft` request string. No Minecraft version is inferred from a display label.
- Agent 1 `FabricLoaderVersion.version` is the exact runtime loader version. CurseForge file metadata only exposes a Fabric loader-family compatibility tag, so Agent 3 filters for the `Fabric` family but does **not** pretend CurseForge proves compatibility with one exact Fabric Loader runtime version.
- Agent 2 `DependencyKind::{Required, Optional, Incompatible}` maps directly to CurseForge relation types 3, 2, and 5. CurseForge dependency edges normally identify a project, not an exact dependency file/version; Agent 3 deterministically resolves required projects to exact compatible file IDs before handoff.
- Agent 2's environment model is more precise than ordinary CurseForge file metadata. Agent 3 therefore preserves requested environment but reports provider applicability as `unknown` rather than manufacturing client/server metadata.
- Agent 2 `ArtifactHashes` has SHA-1/SHA-512/SHA-256 fields. CurseForge currently supplies SHA-1 and/or MD5 through its API. Agent 3 preserves the provider MD5 instead of discarding it merely to fit Agent 2's present shape; Agent 4 should retain provider-specific verification metadata or extend the shared neutral representation during integration.
- Provider-neutral identity mapping is `provider = curseforge` + exact CurseForge `project_id` + exact CurseForge `file_id` (`version_id` is an integration alias for that exact file ID). Download URLs are retrieval metadata only.

## Work completed

- Uses only the official CurseForge REST API at `https://api.curseforge.com`; no curseforge.com scraping and no fabricated CDN URLs.
- Added machine-local credential handling through `SWARMCRAFT_CURSEFORGE_API_KEY` and the official `x-api-key` header.
- Added exact Minecraft + Fabric-family filtering, project/file metadata, deterministic file ordering, recursive required-dependency resolution, optional-dependency preservation, and incompatible-selection detection.
- Added structured provider failures for missing/invalid credentials, rate limiting, unavailable provider, removed resources, malformed responses, incompatibility, impossible dependency selection, and download failures.
- Added provider-restricted download handling that returns `manual_artifact_required` instead of guessing a CDN URL or silently selecting another file.
- Added automatic-download hardening: provider-returned HTTPS only, redirect downgrade rejection, 512 MiB bound, same-directory temporary files, streaming SHA-1/MD5 verification, computed local SHA-256, flush + file `sync_all`, atomic rename, and cleanup on failure.
- Added six CurseForge Tauri commands and registered all six in Desktop.
- Added deterministic provider tests with no real API credential requirement.
- Added `.github/agent3_finalize.py` plus the draft-PR exact-head validation gate to apply the final compatibility/manual-remediation hardening, regenerate the excluded Desktop lockfile, and execute the required workspace/Desktop validation before READY is declared.

## Provider locator / identity semantics

- CurseForge **project ID** is the durable provider project locator.
- CurseForge **file ID** is the durable exact provider artifact locator.
- `version_id` in the current JSON bridge aliases the exact CurseForge file ID; it is not a separate mutable version label.
- Provider SHA-1/MD5 hashes are exact provider verification data when present.
- Local SHA-256 is computed from acquired bytes. Agent 4's frozen canonical SHA-256 is authoritative once canonicalization occurs.
- Provider-returned, signed, temporary, CDN, or download URLs are never canonical identity and must not be signed into the canonical package lock.
- A filename, slug, display name, or project page URL is navigation/display metadata, not exact artifact identity.

## Dependency semantics

- Relation type 3 -> `required`; recursively selected.
- Relation type 2 -> `optional`; preserved and never silently promoted or installed as required.
- Relation type 5 -> `incompatible`; selecting both sides fails closed with `impossible_dependency_selection`.
- Required dependencies choose deterministically by newest compatible `fileDate`, then highest file ID.
- Resolution is bounded to 128 selected packages.
- A required edge back to an already-selected project terminates rather than re-expanding forever; this is the applicable cycle behavior for CurseForge's project-level dependency model.
- Agent 4 remains responsible for freezing the resolved graph into canonical lock semantics.

## Automatic-download rules

Automatic acquisition is allowed only when the exact CurseForge file can be identified and CurseForge supplies a usable provider download URL through either the file metadata or the official exact-file download-url endpoint.

The provider then requires:

- an HTTPS URL with a host;
- redirects to remain HTTPS;
- exact JAR destination semantics;
- no overwrite of an existing destination;
- nonzero declared size within the 512 MiB bound;
- streamed byte count equal to provider-declared size;
- all recognized CurseForge SHA-1/MD5 hashes to match;
- local SHA-256 computation;
- temporary-file flush + `sync_all` before same-directory atomic publication;
- partial-file cleanup on every pre-publication failure.

CurseForge's `allowModDistribution` is preserved as provider/project metadata. Agent 3 does not reinterpret it into a stronger download permission than the official API exposes. The provider's exact-file download response is authoritative for whether automatic retrieval is available.

## Manual artifact contract

When the official provider cannot supply an automatic URL for the exact file, `curseforge_download` returns `status = "manual_artifact_required"`. Final hardening preserves enough information for Agent 4/Desktop to demand the exact player-supplied JAR:

- `provider = "curseforge"`;
- exact `project_id`;
- exact `file_id` and `version_id` alias;
- display/version identity plus exact `file_name`;
- `file_size` when known;
- all provider hashes, including SHA-1/MD5 when present;
- clean Minecraft compatibility values;
- loader-family tags and explicit Fabric compatibility;
- dependency identities/kinds;
- project name/slug/page and `allow_mod_distribution` remediation metadata;
- explicit remediation kind/reason code and the exact project/file IDs the player must supply;
- reason automatic retrieval is unavailable.

No CDN URL is invented and no alternate CurseForge file is substituted. Agent 4 can hash a manually supplied JAR and compare its bytes against known provider hashes, then freeze the resulting canonical SHA-256.

## Tauri contracts

Commands:

- `curseforge_provider_status()`
- `curseforge_search(query, minecraft, loader, environment, index, pageSize)`
- `curseforge_project(projectId)`
- `curseforge_versions(projectId, minecraft, loader)`
- `curseforge_resolve(fileId, minecraft, loader, environment)`
- `curseforge_download(fileId, destination)`

Distinct states remain structured and must not be collapsed by Desktop:

- `ok` for successful browse/project/version/resolve calls;
- `available` for configured provider status;
- `configuration_required` for missing/blank/invalid credential state as applicable;
- `rate_limited` with optional retry-after seconds;
- `incompatible` for Minecraft/Fabric/dependency incompatibility;
- `manual_artifact_required` for exact files that cannot be automatically retrieved;
- `download_failed` for acquisition/verification/publication failures;
- `downloaded` only after verification and atomic publication.

Common failure envelope remains `{ status, provider: "curseforge", error: { code, message, retry_after_seconds } }`.

## Files changed / owned in this branch

- `progress/agent3.md`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/curseforge.rs`
- `apps/desktop/src-tauri/src/main.rs`
- `apps/desktop/src-tauri/Cargo.lock` once regenerated by the final gate
- `.github/agent3_finalize.py`
- `.github/workflows/agent3-curseforge-provider.yml`
- `.github/workflows/ci.yml` contains the earlier Agent 3 validation job added on this feature branch; it is not a canonical provider contract.

## Deterministic test matrix

The final exact-head gate executes fixture/unit coverage for:

- missing API key;
- blank API key;
- invalid credential;
- provider unavailable;
- rate limiting and retry-after;
- removed project;
- removed file;
- malformed provider response;
- Minecraft incompatibility;
- Fabric incompatibility;
- required dependency metadata/resolution semantics;
- optional dependency preservation;
- incompatible dependency rejection;
- impossible selection;
- applicable dependency-cycle termination;
- deterministic newest-date/highest-file-ID selection;
- automatic HTTPS download candidate accepted;
- non-HTTPS/absent automatic retrieval rejected or converted to exact manual remediation;
- exact `manual_artifact_required` metadata;
- wrong provider hash rejection;
- exact local SHA-256 computation sensitivity to artifact bytes;
- interrupted download size rejection;
- oversized/zero declared download rejection;
- partial-file cleanup;
- successful same-directory atomic publication;
- HTTPS-only URL enforcement.

No deterministic test requires `SWARMCRAFT_CURSEFORGE_API_KEY` or live CurseForge availability.

## Lockfile / validation status

- Root workspace excludes `apps/desktop/src-tauri`; Agent 3 introduced no root-workspace dependency, so the root `Cargo.lock` should remain unchanged. The final gate still validates it with `cargo metadata --locked` and all requested root checks.
- `apps/desktop/src-tauri/Cargo.lock` is regenerated from the Desktop manifest in the final gate and committed if it changes.
- Final gate runs:
  - `cargo fmt --all -- --check`
  - `cargo check --workspace --all-features --locked`
  - `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
  - `cargo test --workspace --all-features --locked`
  - excluded Desktop format/check/clippy separately with all targets/features and `--locked`
  - deterministic `curseforge::tests::`
  - static registration/state validation for all six Tauri commands.
- Exact green CI evidence is still pending. Status therefore remains `IN PROGRESS`.

## Credential behavior / provider restrictions

- No API key is committed, logged, or embedded in canonical state.
- Missing/blank credential is a supported structured `configuration_required` state and does not break unrelated SwarmCraft functionality.
- Invalid credential is distinguished from provider unavailability and rate limiting.
- Only official CurseForge API calls are used.
- No peer redistribution policy is introduced by this provider.
- Optional dependencies remain optional.
- Unknown ordinary-mod client/server applicability remains unknown.

## Handoff for Agent 4

Consume exact CurseForge project/file identity, provider hashes, file size/name, Minecraft/Fabric-family compatibility, resolved required dependency selections, separate optional/incompatible edges, and automatic-vs-manual retrieval state. Preserve provider-specific MD5 if present. Do not sign download URLs, mutable slugs, project-page URLs, or machine-local paths. Freeze the actual artifact bytes under canonical SHA-256 in Agent 4's model.

Do not treat Agent 3's JSON bridge as Agent 4's canonical schema and do not treat the deterministic provider resolver as the canonical lock itself.

## Known issues / blockers

- Exact-head compile/lint/test and lockfile validation is the only remaining gate.
- No live CurseForge credential is required or assumed.

## Activity log

- 2026-08-24 — ledger created; implementation not started.
- 2026-08-24 @ `41c9b5b650aac1e320195f6e1855945f2722abc4` — verified base and started `agent/curseforge-provider`.
- 2026-08-24 @ `9fb985a23eab71f6a9f039a737fbd2d9bdf5245c` — implemented provider core, deterministic resolution, structured failures, hardened automatic download, and fixtures/tests.
- 2026-08-24 @ `81c601ae087b92ae6a78ea0e3e98cce1de42530a` — wired six CurseForge Tauri commands; remote validation and lockfile refresh remained.
- 2026-08-24 — preserved all legitimate newer Agent 3 branch work beyond the original milestone rather than rewinding.
- 2026-08-24 — reconciled live Agent 1/Agent 2 contracts, identified the clean Minecraft-vs-loader tag boundary and the richer manual-artifact handoff required by Agent 4.
- 2026-08-24 @ `5b6f1093ae56dceef68059a7885501ecd66a763b` — installed an exact-head draft-PR validation gate that applies the final provider/manual-remediation hardening, regenerates Desktop lockfile, runs root + excluded Desktop checks, and publishes an inspectable exact-head status.
