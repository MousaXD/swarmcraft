# Agent 8 — CI / Release Governance

## Status

STATUS: BLOCKED

BRANCH: `fix/agent-8-ci-release`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH CREATION SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb`

LATEST INTEGRATION BASE CONSUMED: `f6ff3d4659fd69cef63e03d3cbf573c0490d6826`

BASE RECONCILIATION MERGE: `eee08d8e545bb963e9091572a69a2966a84da82a`

IMPLEMENTATION SOURCE HEAD: `6ca930e8177d9104246b40f506b5abef485c7386`

VALIDATED PR HEAD BEFORE THIS LEDGER-ONLY UPDATE: `1ef2ab283c45bb3f7d39dc45422e14891eb30aba`

VALIDATED PR MERGE CANDIDATE: `a691c41dc4cbcf73a7837fd87c6d6aa37c7772a6`

INTEGRATED SHA: pending

## Mission

Make release publication mean that the exact published SHA passed the project’s required validation gates, reduce workflow supply-chain privilege, and make current validation authoritative for every shipped crate and package path.

## Findings owned

- FINAL-021 — publication not gated on successful same-SHA validation
- FINAL-022 — mutable GitHub Action refs with write-capable workflow tokens
- FINAL-035 — excluded Desktop/provider direct lint/test blind spots
- FINAL-036 — release identity/reproducibility controls, Desktop locks, Fabric tooling, signing/notarization
- FINAL-038 — repository governance truth / required-check enforcement
- FINAL-044 — Desktop dependency audit and validation-evidence lifecycle
- FINAL-046 — obsolete validation PR/branch/workflow cleanup

## Dependencies consumed

Agent 8 had no implementation dependency gate.

The remediation base advanced during the work only through Agent 10 ledger changes. `integration/audit-remediation-v1` at `f6ff3d4659fd69cef63e03d3cbf573c0490d6826` was reconciled into Agent 8 at merge commit `eee08d8e545bb963e9091572a69a2966a84da82a`; no Agent 8 production/workflow changes were overwritten.

## Implementation completed

### Authoritative exact-SHA validation

- Added reusable `.github/workflows/required-validation.yml` with terminal status `Required validation gate`.
- Required gate aggregates Core CI, specialist provider/catalog validation, release identity/policy, CI governance regressions, live player journey, and optionally the multi-GiB network soak.
- Release-producing callers require the network soak.
- Added caller/input-scoped concurrency so superseded validation attempts cancel without cross-cancelling the separate soak-enabled release-candidate gate.

### Release publication gating

- `main-installers.yml` waits for Required Validation with soak before any package/publish path.
- `release.yml` reruns Required Validation for the exact tag target with exact `vX.Y.Z` binding and soak.
- Final publishers depend on validation plus every required package job.
- Rolling `main-latest` publication is limited to `refs/heads/main`; PR validation builds packages but skips publication.
- Negative execution evidence proves failed and unresolved validation cannot fall through to publication.

### Supply-chain hardening

- Pinned active third-party Actions to immutable full commit SHAs with readable version comments.
- Defaulted workflow permissions to read; only final publisher jobs receive `contents: write`.
- Publisher checkout disables persisted Git credentials.
- Added `scripts/check_workflow_policy.py` to reject mutable `uses:` refs, excessive write permissions, publication DAG bypasses, missing tag binding, missing credential policy, and mutable Fabric Loom snapshots.
- Replaced snapshot Fabric Loom tooling with released `1.17.2` and added policy rejection of snapshot coordinates.

### Release identity and production credentials

- Aligned excluded `swarm-provider` package metadata with application version `0.5.0`.
- Reworked `scripts/check_release_version.py` to validate app/Desktop/provider/Tauri/Fabric versions, root/Desktop lock graphs, protocol metadata, Fabric tooling, and optional exact release tag.
- Added `scripts/check_release_credentials.py`.
- Production tag releases fail closed unless required Windows signing and Apple signing/notarization credentials are complete.
- Unsigned/ad-hoc output is restricted to the explicitly preview-grade rolling channel.

### Excluded-crate and lock coverage

- Added direct Desktop check, clippy, and tests to authoritative CI.
- Added direct excluded provider check, clippy, and tests.
- Added Desktop RustSec audit against the committed Desktop lock graph.
- Fixed Windows lock preflight shell portability.
- Strengthened lock-currentness checks to force full dependency resolution after regression testing showed `cargo metadata --locked --no-deps` could accept a changed dependency graph.
- Added CI governance regressions that mutate the Desktop dependency graph and inject intentionally failing Desktop/provider tests; the aggregate workflow succeeds only when those negative cases are correctly rejected.

### Historical validation cleanup and evidence lifecycle

- Migrated still-useful specialist checks into current authoritative workflows before removing obsolete workflow files.
- Removed historical `agent1-lockfiles.yml`, `agent2-final-validation.yml`, `agent3-curseforge-provider.yml`, and `final-ui-screenshots.yml` from the active workflow surface.
- Historical validation PRs #44, #47, and #49 are closed and unmerged; the old Agent 1/2 validation refs are gone.
- Standardized artifact retention: ordinary CI packages 7 days, live/main/release evidence 14 days, network-soak evidence 30 days.

## Final dynamic validation evidence

### Required Validation — GREEN

Run: `33583427402`

PR head: `1ef2ab283c45bb3f7d39dc45422e14891eb30aba`

PR merge candidate executed by the pull-request workflow: `a691c41dc4cbcf73a7837fd87c6d6aa37c7772a6`

Result: `SUCCESS`

Green components include:

- `Required validation gate`
- CI governance regressions
  - stale Desktop dependency graph is rejected by full locked resolution
  - intentionally failing provider test is rejected
  - intentionally failing Desktop test is rejected
- specialist catalog/Modrinth/CurseForge checks
- live clean-machine player journey against official services
- Rust format/clippy/test matrix on Ubuntu, Windows, and macOS
- Desktop/provider direct check/clippy/tests
- root and Desktop RustSec audits
- QUIC impairment regression
- fuzz smoke
- Fabric build and embedded Fabric API verification
- Linux, Windows, macOS arm64, and macOS x86_64 Desktop packaging
- release identity/workflow policy, mismatched-tag negative test, missing-signing negative test, and positive credential-set test

### Release-candidate package path with soak — GREEN

Main Desktop Installers run: `33583427535`

Result: `SUCCESS`

The nested exact-SHA Required Validation completed successfully and included `Network soak / Interrupted QUIC multi-GiB soak`, which passed. Downstream Linux `.deb`, Windows `.exe`, both macOS `.dmg` jobs, and the Fabric bridge JAR all succeeded and uploaded artifacts. `Publish rolling main release` was correctly `skipped` because this was a PR, not `refs/heads/main`.

Evidence artifacts include:

- network soak evidence, 30-day retention
- clean-machine live evidence
- Linux `.deb`
- Windows `.exe`
- macOS arm64 `.dmg`
- macOS x86_64 `.dmg`
- Fabric JAR
- ordinary Desktop matrix packages

### Failed-validation publication regression — GREEN

Main Desktop Installers run `33582275682` exercised the negative DAG:

- nested `Required validation gate` failed;
- Linux, Windows, macOS, and Fabric package jobs did not succeed;
- `Publish rolling main release` was cancelled;
- no rolling release was published.

### Unresolved-validation publication regression — GREEN

Observed Main Desktop Installers runs remained held at the reusable validation dependency while Required Validation was unresolved. Downstream publication could not start before validation completion.

## Final test ledger

| Test / Gate | Result | Evidence |
|---|---|---|
| Required reusable DAG accepted | PASS | run `33583427402` |
| Aggregate Required validation gate | PASS | run `33583427402` |
| Full-resolution stale Desktop lock regression | PASS | governance job in `33583427402` |
| Injected failing provider test rejected | PASS | governance job in `33583427402` |
| Injected failing Desktop test rejected | PASS | governance job in `33583427402` |
| Direct Desktop/provider lint/tests | PASS | `33583427402` |
| Root + Desktop RustSec | PASS | `33583427402` |
| Rust Ubuntu/Windows/macOS matrix | PASS | `33583427402` |
| Windows package shell portability | PASS | `33583427402` and `33583427535` |
| Linux/Windows/macOS package builds | PASS | `33583427402`; release-path packages also green in `33583427535` |
| Fabric build/tooling policy | PASS | `33583427402`, `33583427535` |
| Live player journey | PASS | `33583427402`, nested release validation in `33583427535` |
| Network impairment | PASS | `33583427402`, `33583427535` |
| Multi-GiB release-candidate soak | PASS | `33583427535` |
| Mutable-action/write-permission policy | PASS | release identity job in `33583427402` |
| Mismatched tag fails closed | PASS | release identity job in `33583427402` |
| Missing production credentials fail closed | PASS | release identity job in `33583427402` |
| Complete production credential set accepted | PASS | release identity job in `33583427402` |
| Failed validation blocks rolling publication | PASS | `33582275682` |
| Unresolved validation blocks publication | PASS | observed reusable dependency DAG |
| Required status rule installed in repository | BLOCKED | live ruleset `21764953` lacks required-status rule |
| Final stale validation ref deleted | BLOCKED | `ci/discovery-fixture-trigger` still exists |

## Remaining blockers

### BLOCKER 1 — repository required-status enforcement

FINAL-038 cannot be truthfully closed from this execution environment.

Live ruleset `21764953` (`meow`) remains active with deletion, non-fast-forward, and code-quality rules only. It still does **not** contain a required-status-check rule for `Required validation gate`.

The connected GitHub capability exposes ruleset and branch-protection reads only; no ruleset/protection mutation operation is available. The local execution environment has no GitHub credential/token that could be used to perform the administrative API write independently.

Required repository-admin action is documented in `docs/RELEASE_GATES.md`: require the exact terminal status `Required validation gate` on the protected integration/main path.

### BLOCKER 2 — final obsolete remote ref cleanup

FINAL-046 has one safe stale remote ref remaining:

`ci/discovery-fixture-trigger` -> `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`

It is a strict ancestor of `agent/discovery`, which is three commits ahead with no divergence, so deletion is safe. The connected GitHub capability exposes ref creation/update but no ref deletion operation, and the local environment has no authenticated GitHub token.

Required repository-admin action: delete `refs/heads/ci/discovery-fixture-trigger`.

## Remaining work

No known Agent 8 production/workflow defect remains. All executable validation requirements owned by Agent 8 are green.

To unblock handoff:

1. Add repository required-status enforcement for exact status `Required validation gate`.
2. Delete `ci/discovery-fixture-trigger`.
3. Re-read live repository state.
4. If both administrative changes are confirmed and no new code/workflow drift occurred, change this ledger to `READY FOR INTEGRATION` and record the exact handoff head.

## Handoff

READY FOR INTEGRATION: NO

Validated implementation tree: PR head `1ef2ab283c45bb3f7d39dc45422e14891eb30aba`, PR merge candidate `a691c41dc4cbcf73a7837fd87c6d6aa37c7772a6`.

Required Validation: `33583427402` — SUCCESS.

Release-path validation + multi-GiB soak + package build: `33583427535` — SUCCESS.

Known conflict areas: active workflow files and release/version policy scripts. Integration must preserve the aggregate `Required validation gate` contract and the release DAG dependency on it.

## Agent final statement

BLOCKED
