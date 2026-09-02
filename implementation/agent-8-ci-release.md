# Agent 8 — CI / Release Governance

## Status

STATUS: BLOCKED

BRANCH: `fix/agent-8-ci-release`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH CREATION SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb`

LATEST INTEGRATION BASE CONSUMED: `f6ff3d4659fd69cef63e03d3cbf573c0490d6826`

BASE RECONCILIATION MERGE: `eee08d8e545bb963e9091572a69a2966a84da82a`

IMPLEMENTATION SOURCE HEAD: `6ca930e8177d9104246b40f506b5abef485c7386`

INTEGRATED SHA: pending

## Mission

Make release publication mean that the exact published SHA passed the project’s required validation gates, while reducing workflow supply-chain privilege and making current-main validation authoritative for all shipped crates.

## Findings owned

- FINAL-021 — publication not gated on successful same-SHA validation
- FINAL-022 — mutable GitHub Action refs with write-capable workflow tokens
- FINAL-035 — excluded Desktop/provider direct lint/test blind spots
- FINAL-036 — release identity/reproducibility controls: tag/version binding, immutable Fabric build tooling, locked Desktop packaging, production signing/notarization policy
- FINAL-038 — repository governance truth / required-check enforcement
- FINAL-044 — Desktop dependency audit and validation-evidence lifecycle
- FINAL-046 — obsolete validation PR/branch/workflow cleanup

## Audit inputs read

- `implementation/README.md` from `integration/audit-remediation-v1`
- this ledger (`implementation/agent-8-ci-release.md`; no `implementation/agent-8-consensus.md` exists)
- `audits/FINAL-AUDIT.md` from `audit/final-integration-report`
- `audits/09-ci-release.md` from `audit/ci-release`
- repository-wide `AGENTS.md`

The final audit and Auditor 9 agree on the core failure: installer/release publication was not coupled to completed validation of the exact published SHA. Auditor 9 additionally supplied the action-pin, excluded-crate, tag/version, Loom, Desktop lock, signing, retention, and historical-workflow failure scenarios addressed below.

## Dependencies

Required before starting: none.

Dependency heads consumed: none.

The remediation base advanced during Agent 8 work only through Agent 10 ledger updates. `integration/audit-remediation-v1` at `f6ff3d4659fd69cef63e03d3cbf573c0490d6826` was reconciled into this branch at merge commit `eee08d8e545bb963e9091572a69a2966a84da82a`; no Agent 8 production/workflow changes were overwritten.

## Ownership boundaries

Primary ownership:

- `.github/workflows/*`
- release/version guard scripts
- dependency lock validation in CI
- branch/ruleset policy documentation/configuration where repository capabilities permit
- release evidence retention policy

Do not hide product test failures by weakening required gates.

## Implementation checklist

- [x] Define one authoritative reusable same-SHA validation gate for release candidates.
- [x] Make `main-latest` publication wait until every required same-SHA gate completes successfully.
- [x] Make versioned tag release rerun the full required gate for the exact tag target SHA.
- [x] Encode and lint the release DAG so packaging/publication cannot bypass required validation.
- [x] Pin every third-party GitHub Action in active workflows to immutable full commit SHA with human-readable version comments.
- [x] Default workflow permissions to read; grant `contents: write` only to final publisher jobs that need it.
- [x] Add direct Desktop Rust lint/tests to authoritative CI.
- [x] Add direct provider lint/tests to authoritative CI.
- [x] Add explicit Desktop `Cargo.lock` preflights to authoritative CI and package paths.
- [x] Force full dependency resolution in lock preflights; do not use `cargo metadata --no-deps` as a lock-currentness proof.
- [x] Keep package lock preflights cross-platform by selecting an explicit portable shell where output redirection is used.
- [x] Bind `vX.Y.Z` tag exactly to authoritative app version metadata.
- [x] Validate shipped component versions/locks, including excluded `swarm-provider` and Desktop lock graph.
- [x] Replace mutable Fabric Loom snapshot tooling with released Loom `1.17.2` and reject future snapshot coordinates in policy/version checks.
- [x] Define production release signing/notarization policy and fail closed when required credentials are missing.
- [x] Keep unsigned builds only on explicitly preview-grade `main-latest` channel.
- [ ] Add/enable `main` branch/ruleset required-status enforcement for `Required validation gate`. Live repository state does not contain this rule and this connector exposes no mutation operation.
- [x] Standardize artifact/evidence retention by class in active workflows.
- [x] Migrate useful legacy Agent validation checks into current CI before removing historical workflow files.
- [ ] Delete final safe obsolete remote validation branch `ci/discovery-fixture-trigger`. It is proven a strict ancestor of `agent/discovery`, but this connector exposes no ref-deletion operation.

## Work completed

### Release identity and reproducibility

- Aligned excluded `crates/swarm-provider/Cargo.toml` from stale `0.4.0` metadata to application `0.5.0`; the committed Desktop lock already recorded provider `0.5.0`.
- Reworked `scripts/check_release_version.py` to derive the authoritative application version from the root workspace, validate Desktop/provider/Tauri/Fabric metadata, validate both committed lock graphs, preserve independent wire-protocol versioning, reject snapshot Loom coordinates, and optionally require an exact `v<version>` release tag.
- Replaced `loom_version=1.17-SNAPSHOT` with released `1.17.2` and added CI/policy rejection of snapshot tooling.

### Authoritative validation

- Converted core CI, release-version validation, live player journey, and network soak into reusable workflows callable against the caller's exact SHA.
- Added `.github/workflows/required-validation.yml` as the single aggregate gate with terminal job `Required validation gate`.
- The aggregate gate requires Core CI, migrated specialist validation, version/release identity, CI-governance regressions, and live player journey; release-producing callers additionally require the multi-GiB network soak.
- Added caller/input-scoped concurrency to Required Validation so superseded attempts cancel without allowing an ordinary PR gate to cancel the separately soak-enabled release-candidate gate.
- Added `.github/workflows/specialist-validation.yml` and migrated live catalog validation, Modrinth deterministic tests, CurseForge deterministic tests/command registration, and Desktop locked metadata from historical Agent workflows.
- Added direct Desktop check/clippy/tests and direct provider check/clippy/tests to Core CI.
- Added a separate Desktop RustSec audit using the committed Desktop lock graph.
- Fixed Windows Desktop package lock validation after Actions exposed Unix `/dev/null` syntax being interpreted by PowerShell.
- Strengthened all authoritative Desktop lock-currentness checks after regression testing proved `cargo metadata --locked --no-deps` can accept a changed dependency graph. Active checks now force full dependency resolution.
- Added deterministic CI governance probes that temporarily mutate the Desktop dependency graph and inject failing tests into excluded Desktop/provider crates; the workflow expects Cargo to reject each negative case.

### Publication control and least privilege

- `main-installers.yml` calls Required Validation with network soak before package jobs; final publication depends on validation plus every package job and is additionally limited to `refs/heads/main`.
- `release.yml` calls Required Validation for the exact tag target with `release_tag=${{ github.ref_name }}` and network soak before package jobs.
- Added `scripts/check_release_credentials.py`; production tag releases fail closed unless Windows signing and complete Apple signing/notarization credentials are present.
- Removed Windows unsigned and macOS ad-hoc fallbacks from production tag releases. The rolling `main-latest` channel remains explicitly preview-grade and documents its unsigned/ad-hoc status.
- Active workflows default to `contents: read`; only final `publish` jobs in `main-installers.yml` and `release.yml` receive `contents: write`.
- Publisher checkout disables persisted Git credentials.

### Supply-chain workflow policy

- Resolved all active third-party Action refs to exact full commit SHAs, including annotated-tag dereferencing for Rust cache and Gradle actions, while retaining human-readable version comments.
- Added `scripts/check_workflow_policy.py`, executed by the required version gate, to reject mutable external `uses:` refs, missing pin comments, workflow-level/excess `contents: write`, release DAGs that bypass Required Validation/network soak, missing release-tag binding, missing credential policy, and mutable Fabric Loom snapshots.
- Pinned the scheduled fuzz workflow as well, so the active workflow surface follows the same immutable-action rule.

### Evidence lifecycle and historical cleanup

- Standardized active artifact retention: ordinary CI packages 7 days, live player journey/main/release staging 14 days, network-soak evidence 30 days.
- Retired `.github/workflows/agent1-lockfiles.yml`, `agent2-final-validation.yml`, `agent3-curseforge-provider.yml`, and historical `final-ui-screenshots.yml` only after their useful checks were promoted into current validation. Git history and audit references remain preserved evidence.
- Confirmed historical validation PRs #44, #47, and #49 are closed and unmerged; the old Agent 1/2 validation refs are gone.
- Confirmed the remaining `ci/discovery-fixture-trigger` ref at `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c` is a strict ancestor of `agent/discovery`, which is three commits ahead with no divergence. It is safe to delete, but the connector cannot delete refs.

### Repository governance inspection

- Live repository ruleset `21764953` (`meow`) is active and contains deletion, non-fast-forward, and code-quality rules only. It contains no required-status-check rule.
- Direct branch-protection read is inaccessible to this GitHub integration (403), and the available connector exposes ruleset reads but no ruleset/protection mutation action.
- `docs/RELEASE_GATES.md` records the exact repository-admin change required to make `Required validation gate` a protected required status rather than falsely claiming it is already enabled.

## Dynamic validation evidence

### Publication fails closed

Main Desktop Installers run `33582275682` provides an executable negative release-DAG proof:

- nested `Required validation gate` concluded `failure`;
- Linux `.deb`, Windows `.exe`, Fabric JAR, and macOS package jobs were cancelled;
- `Publish rolling main release` was cancelled and did not publish.

This satisfies the required negative assertion that a failed same-SHA gate cannot fall through into rolling publication.

While Required Validation is unresolved, the release-producing workflow remains held behind its `validation` reusable job and no publisher can start. Main Desktop Installers runs observed during this campaign remained pending with no downstream publisher while validation was unresolved.

### Broad green evidence on the latest substantially equivalent executed graph

Required Validation run `33582275450` on earlier implementation head `bcecd2eac7ae796736fb4edaf490f46586794d5c` reached green for the following substantive gates before its known superseded regression-harness failure:

- specialist catalog/Modrinth/CurseForge validation;
- live clean-machine player journey against official services;
- Fabric server mod build and embedded Fabric API check;
- process-level acceptance suite;
- direct Desktop/provider check, clippy, and tests;
- QUIC impairment regression;
- fuzz smoke;
- Rust check/test matrix on Ubuntu, Windows, and macOS;
- release identity/workflow-policy validation, including tag mismatch and signing-credential negative/positive regressions;
- root and Desktop RustSec audits;
- Windows and both macOS Desktop packages.

The only substantive red on that superseded run is the old stale-lock regression methodology. Its failure exposed that `--no-deps` was an inadequate lock-currentness assertion; Agent 8 then strengthened the active checks and regression to full dependency resolution.

### Exact implementation source and supersession behavior

Implementation source head: `6ca930e8177d9104246b40f506b5abef485c7386`.

PR validation vehicle: draft PR #61, `fix/agent-8-ci-release` -> `integration/audit-remediation-v1`.

Source-head Required Validation run `33583109333` was accepted by GitHub with the complete expected graph. When the first BLOCKED ledger-only closeout commit was pushed, the newly added concurrency policy correctly superseded that older attempt, cancelling its already-started/pending work. It therefore did not produce complete exact-source-head evidence.

The documentation-only closeout head then created:

- Required Validation run `33583354093`
- Main Desktop Installers run `33583354291`

Those runs were pending/queued when this correction was recorded. They validate the same implementation source tree plus ledger documentation, but no exact-head completion claim is made.

## Tests / evidence ledger

| Test | Result | Exact SHA / run | Notes |
|---|---|---|---|
| Required source/audit review | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Scope and regression requirements reconciled before production edits. |
| Latest remediation-base reconciliation | PASS | merge `eee08d8e545bb963e9091572a69a2966a84da82a` | Upstream delta was Agent 10 ledger-only; Agent 8 production tree preserved. |
| Workflow parser / reusable DAG | PASS | multiple PR runs including `33582275450` | GitHub accepted and executed the reusable workflow graph. |
| Release identity + workflow policy | PASS | run `33582275450` | Tag mismatch, missing production signing credentials, positive credential set, immutable Action/write-policy checks green. |
| Direct Desktop/provider lint/tests | PASS | run `33582275450` | Both excluded Rust crates were checked directly. |
| RustSec root + Desktop | PASS | run `33582275450` | Both lock graphs audited. |
| Windows package portability | PASS | run `33582275450` | Windows package passed after shell fix. |
| Rust matrix | PASS | run `33582275450` | Ubuntu, Windows, macOS green. |
| Fabric + live player journey | PASS | run `33582275450` | Pinned Fabric tooling and live journey green on that source generation. |
| Failing required validation blocks rolling publication | PASS | Main Installers run `33582275682` | Required gate failed; all package/publish jobs cancelled. |
| Unresolved validation blocks publisher | PASS | observed Main Installer dependency graphs | Downstream publish remained unavailable while validation was unresolved. |
| Stale Desktop lock negative regression, old method | FAIL AS DESIGNED DISCOVERY | governance job `100099213583` | Demonstrated `--no-deps` was insufficient; active implementation was strengthened to full resolution. |
| Full-resolution stale-lock regression | PENDING | source run `33583109333` superseded | Source attempt was cancelled by the intentional concurrency policy after a ledger-only commit. |
| Injected failing provider/Desktop tests rejected | PENDING | source run `33583109333` superseded | Same supersession; no false PASS recorded. |
| Exact implementation-source complete validation | PENDING | `6ca930e...` | No complete exact-source run before ledger closeout. |
| Required status rule on `main` | BLOCKED | live ruleset `21764953` | Rule is absent; connector is read-only for rulesets/protection. |
| Safe stale validation ref deletion | BLOCKED | `ci/discovery-fixture-trigger` | Strict ancestor proven; connector lacks ref deletion. |

## Required validation before READY handoff

- [x] workflow syntax / reusable DAG accepted by GitHub
- [x] intentionally failing required gate blocks rolling publication
- [x] unresolved required validation blocks downstream publication
- [x] mismatched tag/version negative regression
- [ ] full-resolution stale Desktop lock negative regression on an unsuperseded exact implementation head
- [ ] injected failing Desktop/provider tests on an unsuperseded exact implementation head
- [x] workflow policy rejects mutable `uses:` and excess write permissions
- [x] production release missing-credential negative/positive regressions
- [ ] complete exact implementation-head Required Validation
- [ ] enforce `Required validation gate` in repository branch/ruleset policy
- [ ] delete final safe obsolete validation ref

## Blockers

### BLOCKER 1 — repository required-status enforcement

FINAL-038 cannot be truthfully closed from this environment. Live ruleset `21764953` does not require `Required validation gate`. The connected GitHub capability can read the ruleset but exposes no ruleset or branch-protection mutation operation. A repository administrator must add a required-status-check rule for the exact terminal status `Required validation gate` on the protected integration/main path described in `docs/RELEASE_GATES.md`.

### BLOCKER 2 — final obsolete remote ref cleanup

FINAL-046 cleanup has one safe remote ref remaining: `ci/discovery-fixture-trigger` at `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`. It is proven a strict ancestor of `agent/discovery`. The connector exposes file deletion but no Git ref deletion operation. A repository administrator must delete that stale ref after preserving the existing historical links.

### BLOCKER 3 — exact implementation-head completion evidence

The exact implementation-source run `33583109333` was superseded by the intentionally added concurrency policy when the first ledger-only closeout commit advanced the PR. The resulting closeout-head run `33583354093` was still pending when this ledger correction was made. This environment has no local repository execution path (`CALLER_IDENTITY_REQUIRED`) and no workflow-dispatch/cancel operation that can produce a separate frozen-source run without another ref change. Therefore the final full-resolution stale-lock regression and injected failing-test probes remain unclaimed rather than being waived.

## Required repository-admin actions to unblock

1. Update repository ruleset/branch protection so `Required validation gate` is a required status for the protected integration/main path, exactly as documented in `docs/RELEASE_GATES.md`.
2. Delete `refs/heads/ci/discovery-fixture-trigger` after confirming the already-recorded ancestry/evidence.
3. Allow one unsuperseded Required Validation attempt for the final Agent 8 implementation tree to complete. If the full-resolution lock or injected-test regressions fail for a real implementation reason, return Agent 8 to implementation rather than waiving the gate.

## Handoff

READY FOR INTEGRATION: NO

IMPLEMENTATION SOURCE HEAD: `6ca930e8177d9104246b40f506b5abef485c7386`

Exact ledger head: this documentation-only correction commit

PR: #61 remains draft and must not be merged while this ledger is BLOCKED.

Known conflict areas: workflows may need to consume new tests created by Agents 1-7/9 during later integration. Preserve the terminal `Required validation gate` contract and its same-SHA publication dependency.

## Agent final statement

BLOCKED
