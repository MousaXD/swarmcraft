# Auditor 9 — CI, Tests, Installers, and Release Engineering

## Audit identity

- Repository: `MousaXD/swarmcraft`
- Audit branch: `audit/ci-release`
- Audited baseline: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Baseline gate: **PASS**. Live `main` resolved to the expected SHA before the audit branch was created.
- Production modifications: **none**
- Audit date: 2026-09-02

This audit asks a narrow question: **does green CI, packaging, and release automation prove what SwarmCraft currently treats it as proving?**

## Executive verdict

**VERDICT: FAIL**

SwarmCraft has substantially better CI coverage than a workspace-only Rust project: root Rust runs on Linux, Windows, and macOS; desktop native packages are built on Linux, Windows, macOS ARM, and macOS Intel; Java/Fabric is built; RustSec runs; a fuzz smoke target runs; process-level acceptance tests exist; network-soak and a clean-machine live player journey exist; and the rolling `main-latest` installer workflow emits SHA-256 sidecars.

The failure is release-control integrity, not lack of tests. The release-producing workflows are independent of the validation workflows. On the exact audited SHA, `Main Desktop Installers` completed successfully and published `main-latest` while the same SHA's `CI` workflow still reported `in_progress`, with the macOS Intel desktop package job still running. The versioned release workflow has the same architectural issue: it starts on any `v*` tag, performs packaging, and publishes without requiring the repository's CI, version guard, RustSec, live player journey, or network-soak status for that SHA.

There is also a meaningful GitHub Actions supply-chain weakness: release workflows grant `contents: write` at workflow scope while executing multiple third-party actions through mutable tags such as `@v4`, `@v2`, and `@stable` instead of immutable commit SHAs.

## Exact-head live evidence

Observed for `main` SHA `354be3b1066428ecab6987590b7c7dbd80fe0870`:

| Workflow | Run | Event | Observed status | Relevance |
|---|---:|---|---|---|
| `Main Desktop Installers` | `33576322502` | push to `main` | **SUCCESS** | Published rolling installers and `main-latest` |
| `CI` | `33576322543` | push to `main` | **IN PROGRESS** at audit cut | The release-producing workflow did not wait for it |
| `Release version guard` | exact-head main-push run | push to `main` | **SUCCESS** | Version consistency check ran independently |
| `Player journey live acceptance` | exact-head main-push run | push to `main` | **SUCCESS** | Clean-machine live journey ran independently |
| `Network Soak` | exact-head main-push run | push to `main` | **SUCCESS** | Path-filtered soak happened to run for this SHA |

The `CI` jobs endpoint showed, among other jobs, successful Linux and Windows desktop packaging and Fabric build, while `Desktop package (macos-x86_64)` was still `in_progress`. The `main-latest` GitHub prerelease was already published at 2026-09-02T01:03:08Z and targets the exact audited SHA. This is direct evidence that installer publication is not gated on full CI completion.

## Workflow inventory

The audited baseline contains exactly these workflow files:

1. `.github/workflows/agent1-lockfiles.yml`
2. `.github/workflows/agent2-final-validation.yml`
3. `.github/workflows/agent3-curseforge-provider.yml`
4. `.github/workflows/ci.yml`
5. `.github/workflows/final-ui-screenshots.yml`
6. `.github/workflows/fuzz.yml`
7. `.github/workflows/main-installers.yml`
8. `.github/workflows/network-soak.yml`
9. `.github/workflows/player-journey-live.yml`
10. `.github/workflows/release.yml`
11. `.github/workflows/version-guard.yml`

No inspected workflow uses `continue-on-error` to convert a failing validation step into a green job. Several steps intentionally use `if: always()` for cleanup/evidence upload, which is appropriate and does not suppress the primary test result.

## CI / validation matrix

| Component | Workflow | Platform | Compile | Lint | Tests | Package | Live validation |
|---|---|---|---|---|---|---|---|
| Root Rust workspace | `ci.yml` `rust` | Linux / Windows / macOS | Yes, through clippy/test build | Yes, `clippy --workspace --all-targets --all-features --locked -D warnings`; fmt on Linux | Yes, all features, locked | Indirectly reused by desktop jobs | Process acceptance separately on Linux |
| Core process acceptance | `ci.yml` `acceptance` | Linux | Test compilation | N/A | Yes, targeted network/storage/CLI/consensus process tests | No | Yes, process-level local acceptance |
| Rust fuzz smoke | `ci.yml` `fuzz-smoke` | Linux nightly | Yes | N/A | `cargo fuzz` canonical-record target, ~20 s | No | Adversarial parser exercise only |
| Longer fuzz | `fuzz.yml` | Linux weekly/manual | Yes | N/A | Same fuzz target, ~300 s | No | Scheduled/manual only |
| Network impairment | `ci.yml` `network-impairment` | Linux | Yes, target compiled locked | N/A | Ignored resume-under-loss test under `tc` | No | Simulated packet impairment |
| Network soak | `network-soak.yml` | Linux | Yes | N/A | Multi-GiB/impairment soak | No | PR/main only for selected paths, plus schedule/manual |
| Desktop frontend JS | `ci.yml` `desktop` | Linux only | N/A | No dedicated JS lint | `node --test apps/desktop/tests/*.test.mjs` | N/A | Tauri bridge source assertion |
| Desktop Rust/Tauri | `ci.yml` `desktop` | Linux / Windows / macOS ARM / macOS Intel | **Yes**, Tauri package build | **No direct desktop Rust clippy** | **No direct desktop Rust test job** | Yes: deb/AppImage, NSIS, DMG | Packaging only |
| `swarm-provider` | Transitively via desktop package | Linux / Windows / macOS ARM / macOS Intel | **Yes for default dependency graph, transitively** | **No direct current-main lint** | **No direct current-main provider tests** | No standalone artifact | Indirect only |
| Fabric server mod | `ci.yml` `fabric` | Linux + Java 25 | Yes, `gradle build` | No dedicated lint | Gradle lifecycle if tests exist | JAR uploaded | Candidate JAR also used by player journey |
| Dependency vulnerabilities | `ci.yml` `dependency-audit` | Linux | N/A | N/A | RustSec audit | No | Registry advisory check |
| Clean-machine player journey | `player-journey-live.yml` | Linux | Candidate Fabric + Rust binaries | N/A | Scripted acceptance | No desktop installer | Yes, official services + managed Java path |
| Rolling installers | `main-installers.yml` | Linux / Windows / macOS ARM / macOS Intel | Yes | No | No pre-publish test gate | Yes + SHA-256 | **No dependency on CI/live acceptance** |
| Versioned release | `release.yml` | Linux / Windows / macOS ARM / macOS Intel | Yes | No | No pre-publish test gate | Yes + Fabric + checksums | **No dependency on CI/live acceptance** |
| Version consistency | `version-guard.yml` | Linux | N/A | N/A | Python consistency script | No | Runs on PR/main push, not tags |
| UI screenshot evidence | `final-ui-screenshots.yml` | Linux | Browser fixture only | N/A | Mocked browser assertions | PNG evidence | Historical branch-specific, not main gate |

## Findings

### A9-001 — HIGH — Release publication is not gated on successful validation of the same SHA

**Affected:** `.github/workflows/main-installers.yml`, `.github/workflows/release.yml`, `.github/workflows/ci.yml`, `.github/workflows/player-journey-live.yml`, `.github/workflows/version-guard.yml`, branch protection

**Evidence:**

- `main-installers.yml` triggers directly on a push to `main` and has no dependency on the separately running `CI`, version guard, player journey, or network-soak workflows.
- Its `publish` job depends only on its own Linux/Windows/macOS/Fabric packaging jobs.
- On the audited SHA, installer run `33576322502` completed `SUCCESS` and the `main-latest` prerelease was published while CI run `33576322543` was still `in_progress`.
- `release.yml` triggers on `push.tags: v*`; its publish job depends on packaging jobs, not on the repository validation workflows.
- Live `main` was reported by GitHub as `protected: false`; there are no required checks preventing a direct unvalidated commit from becoming `main` before post-push automation starts.

**Failure scenario:** a commit reaches `main`, its installer builds succeed, but a root test, desktop test that is not run there, RustSec, live player journey, or late matrix job fails. `main-latest` can nevertheless be replaced with artifacts from that commit. The versioned tag workflow can similarly publish a release from a commit whose required validation never passed.

**Impact:** users can receive artifacts that the project's own validation suite has not accepted. Green installer packaging is therefore not equivalent to a green product/release candidate.

**Remediation:**

1. Make validation a reusable workflow and call it from release/installer workflows, or put release publication behind a same-SHA `workflow_run`/explicit status gate that verifies every required check completed successfully.
2. Keep packaging jobs separate if desired, but do not publish until required validation for the exact SHA is green.
3. Protect `main` and require the intended checks before update/merge.
4. For tags, verify the tag target SHA has required green checks or rerun the full gate inside `release.yml` before publishing.

**Test required to close:** intentionally create a candidate SHA with one required test failing while packaging succeeds and prove neither `main-latest` nor a versioned release is published.

**Confidence:** HIGH.

### A9-002 — HIGH — Mutable GitHub Action references execute with release-write tokens

**Affected:** primarily `.github/workflows/release.yml`, `.github/workflows/main-installers.yml`; also `.github/workflows/final-ui-screenshots.yml`

**Evidence:**

- Release workflows set `permissions: contents: write` at workflow scope, so build jobs inherit a write-capable `GITHUB_TOKEN` rather than restricting write access to the final publisher.
- The workflows execute third-party actions using mutable references such as `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `actions/setup-java@v4`, `gradle/actions/setup-gradle@v4`, and artifact actions at `@v4`, not immutable commit SHAs.
- `final-ui-screenshots.yml` also has workflow-level `contents: write` and pushes generated files after executing mutable actions and an unversioned `pip install playwright`; its impact is limited to its dedicated evidence branch but the pattern is the same.

**Failure scenario:** a compromised upstream action release/tag, or an unexpected retarget of a mutable action reference, executes in a build job with repository write capability and can tamper with repository refs or release state using the job token.

**Impact:** CI supply-chain compromise can become release/repository compromise rather than being limited to a failed build.

**Remediation:**

- Pin every third-party GitHub Action to a reviewed full commit SHA and keep the human-readable release in a comment.
- Default to `contents: read`; grant `contents: write` only on the final publish job.
- Add artifact attestations/provenance for public release artifacts.
- Pin auxiliary installer dependencies such as Python packages used by write-capable workflows.

**Test required to close:** repository policy/lint that rejects non-SHA `uses:` references and verifies only publisher jobs have write permissions.

**Confidence:** HIGH.

### A9-003 — MEDIUM — Excluded Desktop/provider crates are not first-class lint/test gates on current main CI

**Affected:** `Cargo.toml`, `apps/desktop/src-tauri/Cargo.toml`, `crates/swarm-provider/Cargo.toml`, `.github/workflows/ci.yml`, legacy Agent 1/2/3 workflows

**Evidence:**

- Root `Cargo.toml` excludes both `apps/desktop/src-tauri` and `crates/swarm-provider` from the workspace.
- Root `cargo clippy --workspace` and `cargo test --workspace` therefore do not lint or test those crates.
- Current `ci.yml` does package the Tauri application on all four supported desktop runner variants. That **does compile the desktop crate and, transitively, its default `swarm-provider` dependency**, so the simple claim that either default build can stop compiling entirely while normal CI remains green is not supported.
- However, current-main CI does not run direct `cargo clippy` or `cargo test` against the Desktop Rust manifest and does not directly lint/test the provider manifest.
- The historical Agent 1/2/3 validation workflows contain several of these direct checks, but they are tied to old validation branches/manual execution rather than being main gates.

**Failure scenario:** a Desktop/provider unit test or lint contract regresses while release compilation/package creation still succeeds. CI remains green because the relevant direct test/lint suite is stranded in historical validation workflows.

**Impact:** code can pass packaging while failing domain tests or warning policy in excluded crates.

**Remediation:** migrate the useful Desktop/provider checks into `ci.yml` or a reusable current-main validation workflow; then retire/archive the agent-specific workflows.

**Test required to close:** add a deliberately failing Desktop/provider unit test and prove the standard PR/main workflow fails without invoking a legacy agent workflow.

**Confidence:** HIGH.

### A9-004 — MEDIUM — Versioned release tags are not bound to application version metadata

**Affected:** `.github/workflows/release.yml`, `.github/workflows/version-guard.yml`, `scripts/check_release_version.py`

**Evidence:**

- `release.yml` accepts any tag matching `v*` and does not run the release-version guard.
- The version guard runs on pull requests and pushes to `main`, not tag pushes.
- `scripts/check_release_version.py` checks a hard-coded expected app version (`0.4.0`) across selected metadata, but it does not compare the current Git tag to that version.
- Its root-lock package allowlist also omits at least `swarm-catalog`; the separately excluded `swarm-provider` manifest is not part of the guard.

**Failure scenario:** tag the audited tree as `v9.9.9` or another mismatched `v*`. The release workflow will build/package version `0.4.0` artifacts and create a release named from the mismatched tag.

**Impact:** release identity and artifact identity can disagree, undermining upgrade logic, supportability, and provenance.

**Remediation:** derive the expected version from one authoritative metadata source; on tag release require `GITHUB_REF_NAME == v<version>`; validate all shipped component manifests/lockfiles, including excluded crates.

**Test required to close:** a mismatched tag must fail before any publish job or release creation.

**Confidence:** HIGH.

### A9-005 — MEDIUM — Fabric release builds are not reproducible from the Git SHA alone

**Affected:** `minecraft/fabric/gradle.properties`, `minecraft/fabric/build.gradle`, Fabric jobs in CI/release workflows

**Evidence:**

- `loom_version=1.17-SNAPSHOT` is used as the Fabric Loom plugin version.
- `build.gradle` applies `net.fabricmc.fabric-loom` from that snapshot coordinate.
- A `-SNAPSHOT` plugin coordinate is mutable by design; no dependency verification/locking mechanism in the audited files freezes the exact plugin artifact consumed by a future rebuild.
- The runner images and several setup actions are also moving labels/tags, compounding bit-for-bit reproducibility limits.

**Failure scenario:** the remote `1.17-SNAPSHOT` plugin contents change. Rebuilding the same SwarmCraft commit later can use different build tooling and potentially emit a different JAR or fail.

**Impact:** exact source SHA plus repository lockfiles is insufficient to reproduce the Fabric artifact.

**Remediation:** use an immutable released Loom version or a content-addressed/pinned artifact; enable Gradle dependency verification/locking where applicable; record build-tool versions in release provenance.

**Test required to close:** rebuild the same tagged source in clean environments from only committed metadata and verify identical dependency/tool identities and, where feasible, artifact digest.

**Confidence:** HIGH.

### A9-006 — MEDIUM — Desktop packaging is not explicitly locked to the committed Desktop Cargo.lock

**Affected:** `.github/workflows/ci.yml`, `.github/workflows/main-installers.yml`, `.github/workflows/release.yml`, `apps/desktop/src-tauri/Cargo.lock`

**Evidence:**

- A dedicated Desktop `Cargo.lock` is committed.
- Root CLI/runtime builds use `--locked`.
- The actual Tauri package command is `cargo tauri build ... --ci` without an explicit `--locked`/preflight locked-metadata gate in the current primary workflows.
- Historical Agent validation workflows did run explicit locked Desktop metadata/check validation.

**Failure scenario:** Desktop manifest and lockfile drift. A clean CI runner may resolve/update dependencies during package construction instead of failing immediately on stale lock state, making dependency resolution less deterministic than the root workspace gate.

**Impact:** packaging can consume dependency resolution not proven by the committed lockfile, depending on Cargo/Tauri invocation behavior.

**Remediation:** add an explicit `cargo metadata --manifest-path apps/desktop/src-tauri/Cargo.toml --locked` preflight and use Tauri/Cargo locked mode where supported.

**Test required to close:** intentionally make Desktop Cargo.toml inconsistent with Desktop Cargo.lock and prove CI/release packaging fails before building.

**Confidence:** MEDIUM-HIGH.

### A9-007 — MEDIUM — Versioned releases may publish unsigned/unnotarized desktop installers as normal releases

**Affected:** `.github/workflows/release.yml`

**Evidence:**

- Windows Authenticode signing is conditional on signing secrets; absence is explicitly tolerated and the workflow continues.
- macOS Developer ID signing/notarization is conditional; the workflow can produce an ad-hoc signed preview when secrets are absent.
- Linux is checksum-only.
- `gh release create` for `v*` tags does not mark the release as a prerelease solely because signatures are missing.

**Failure scenario:** a version tag is pushed while signing/notarization credentials are absent or incomplete. Packaging and publication succeed, producing a public versioned release whose installers have weaker platform trust characteristics than users may infer from a normal release.

**Impact:** platform warnings/install friction and weaker publisher authenticity for public releases. SHA-256 sidecars prove file integrity only relative to checksums generated in the same pipeline; they are not publisher signatures.

**Remediation:** define a release class policy. For production/public tags, fail closed if required Windows signing and macOS Developer ID/notarization are unavailable. Permit unsigned builds only on explicitly named preview/dev channels such as `main-latest`.

**Test required to close:** run a production-tag dry run with signing secrets absent and verify publication is blocked.

**Confidence:** HIGH.

### A9-008 — LOW — Validation evidence retention and workflow lifecycle are inconsistent

**Affected:** CI artifact uploads, `network-soak.yml`, `player-journey-live.yml`, legacy Agent workflows, `final-ui-screenshots.yml`

**Evidence:**

- `main-installers.yml` explicitly retains intermediate artifacts for 14 days.
- player-journey evidence uses 7 days.
- several CI/release/soak uploads rely on repository-default artifact retention.
- Agent 1/2/3 validation workflows target historical branches/manual conditions and are no longer authoritative main gates.
- `final-ui-screenshots.yml` is branch-specific historical evidence automation and is not a main product gate.

**Impact:** audit evidence lifetime depends partly on repository settings, and stale workflows make it harder to identify the authoritative validation surface.

**Remediation:** standardize evidence retention by evidence class, migrate required legacy checks to current CI, then archive/remove historical workflows after preserving run links in audit/progress documentation.

**Confidence:** HIGH.

## Positive controls already present

The audit found several controls worth preserving:

- `ci.yml` has no path filter on normal PR/main execution, so core CI is hard to accidentally skip based on changed file type.
- Root Rust lint/tests use `--locked`; clippy is strict with `-D warnings`.
- Root Rust executes on Linux, Windows, and macOS.
- Desktop package construction covers Linux, Windows, macOS ARM, and macOS Intel.
- Linux desktop frontend tests and a Tauri global-bridge assertion run before packaging.
- Fabric artifact structure is checked for embedded Fabric API.
- RustSec runs as a dedicated CI job.
- Both a fast fuzz smoke and a longer scheduled/manual fuzz run exist.
- Network impairment and network-soak tests are real network-behavior tests, not only mocks.
- `player-journey-live.yml` removes inherited Java from the journey path so managed-Java resolution is exercised and uses official services.
- Main snapshot and versioned packaging create SHA-256 checksum sidecars; the current `main-latest` release also exposes GitHub-computed SHA-256 asset digests.
- `main-latest` is correctly marked as a prerelease and targets the exact main SHA it packages.
- No inspected validation job uses `continue-on-error` to hide a primary failure.

## Trigger / blind-spot assessment

### Main CI

`ci.yml` runs on all pull requests and pushes to `main`. This is appropriate. Its Rust matrix is cross-platform; formatting is Linux-only, which is sufficient for deterministic rustfmt checking. The primary blind spot is excluded-crate direct lint/tests, not ordinary compilation.

### Network soak

`network-soak.yml` is intentionally path-filtered for PR/main changes to networking/storage plus scheduled/manual runs. Therefore a main green status on unrelated changes should not be interpreted as a fresh soak result. The audited baseline did have a same-SHA successful soak run.

### Fuzz

The longer fuzz workflow is weekly/manual only. PR/main CI gets a short fuzz smoke. A green PR therefore proves the smoke target survived a short run; it does not mean substantial fuzzing occurred for that SHA.

### Player journey

The clean-machine player journey runs on pushes to `main` and PRs to `main`/its historical integration branch. This is strong live evidence, but because it is a separate workflow, release publication does not currently wait for it.

### Legacy Agent validation

Agent 1/2/3 workflows contain useful checks, especially direct Desktop/provider validations, but their branch conditions make them historical validation vehicles rather than current-main policy. Their useful gates should be promoted, then the files should be retired or clearly marked historical.

## Cargo / lockfile assessment

- Root workspace version: `0.4.0`.
- Desktop package version: `0.4.0`.
- Tauri config version: `0.4.0`.
- Fabric `mod_version`: `0.4.0`.
- Root Cargo workspace excludes Desktop and `swarm-provider`.
- Desktop has its own committed Cargo.lock.
- No standalone `crates/swarm-provider/Cargo.lock` is present on audited main; provider dependency resolution is effectively frozen when consumed through the Desktop lockfile, but there is no independent provider lock/test gate.
- Root libp2p Git dependency is pinned to an exact revision, which is stronger than a moving Git branch/tag.

## Release / tag / branch state

At audit time:

- The only Git tag returned by the live Git refs API is `main-latest`, pointing exactly to audited main SHA `354be3b1066428ecab6987590b7c7dbd80fe0870`.
- The live GitHub release is `main-latest`, a prerelease targeting the same SHA and containing desktop/Fabric artifacts plus SHA-256 sidecars.
- No GitHub release exists for `v0.4.0` or `v0.5.0`.
- A live `release/0.5.0` branch exists at `8b85e5a5fe52ed6d906f5c3e8ad3f9bc6db528d5`, eight commits ahead of the audited main SHA with changes concentrated in version metadata and lockfiles. This branch is **not** part of the audited source baseline and was not treated as production truth; it is recorded because release-branch state is in Auditor 9 scope.

The release branch being ahead is not itself a defect. The critical requirement before publication is that a future `v0.5.0` tag be tied to the intended commit, metadata version, required green checks, signing policy, and immutable release inputs.

## Checksums, provenance, and reproducibility

Checksums are present, which is positive, but they should not be confused with independent supply-chain provenance. The checksum files are generated by the same workflow that builds the artifacts. There is no audited GitHub artifact attestation/provenance step. Native signing is optional in the versioned workflow. Toolchain/action inputs include mutable channels/tags (`stable`, `nightly`, runner `-latest`, action major tags), and Fabric Loom is a snapshot dependency. Therefore current builds are **not reproducible from Git SHA alone** in the strong supply-chain sense.

## Artifact retention

- `main-installers.yml`: explicit `retention-days: 14` for intermediate packages.
- `player-journey-live.yml`: evidence retained 7 days.
- Several CI/release/network artifact uploads rely on repository-default retention.

Recommendation: define an explicit retention policy for ordinary CI artifacts, acceptance evidence, release staging, and security/soak evidence so later audits do not depend on mutable repository defaults.

## Recommended remediation order

1. **Gate publication on same-SHA validation success** for both `main-latest` and versioned tags; protect `main` with required checks.
2. **Reduce GitHub Actions token privilege and pin actions to immutable SHAs.** Grant write only to publisher jobs.
3. **Bind release tag to app version** and run the version guard inside the tag-release path.
4. **Promote Desktop/provider direct lint/tests** from historical Agent workflows into authoritative main CI.
5. **Enforce Desktop locked dependency state** before Tauri packaging.
6. **Replace Fabric Loom snapshot tooling** with an immutable build-tool version and add Gradle dependency verification/locking as appropriate.
7. **Fail closed on signing/notarization for production release tags**, while keeping `main-latest` explicitly preview-grade.
8. Standardize retention and archive historical validation workflows after their useful checks are migrated.

## Required regression tests after fixes

- Packaging succeeds but a required CI job fails: no rolling or versioned release is published.
- Required CI remains running: publisher waits/blocks rather than publishing early.
- Mismatched `vX.Y.Z` tag versus metadata: release fails before artifact publication.
- Stale Desktop Cargo.lock: primary CI fails.
- Failing Desktop/provider unit test: normal PR/main CI fails without any agent-specific workflow.
- Repository workflow lint rejects mutable `uses:` references and write permission on builder jobs.
- Production-tag build without signing/notarization credentials: publication fails by policy.
- Fabric build dependencies/tooling resolve to immutable recorded versions.

## Final answer to the core audit question

**Does green CI mean what the project thinks it means?**

Partially. A completed green `CI` run now covers a meaningful cross-platform and process-level surface, and normal default Desktop/provider compilation is harder to silently break than the workspace exclusions alone suggest. But a green installer/release workflow is **not** evidence that CI was green, and the project currently publishes rolling artifacts independently of CI completion. Versioned releases have the same structural gap and additionally lack tag/version binding. Until publication is coupled to exact-SHA validation and the release workflow supply chain is hardened, the release engineering posture does not justify treating successful packaging as an accepted release candidate.

**VERDICT: FAIL**
