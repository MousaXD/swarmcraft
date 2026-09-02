# Agent 8 — CI / Release Governance

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-8-ci-release`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH CREATION SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (campaign planning head; production baseline remains the declared `b4bab085...` tree plus implementation ledgers)

CURRENT HEAD SHA: `1510899c61c32ef2ddfaf009da1ff760527be843` before this progress-ledger commit

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

Do not remove legacy validation workflows until equivalent authoritative gates exist and are proven.

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
- [x] Make versioned tag release rerun or verify the full required gate for the exact tag target SHA.
- [x] Encode and lint the release DAG so packaging/publication cannot bypass required validation.
- [x] Pin every third-party GitHub Action in active workflows to immutable full commit SHA with human-readable version comments.
- [x] Default workflow permissions to read; grant `contents: write` only to final publisher jobs that need it.
- [x] Add direct Desktop Rust lint/tests to authoritative CI.
- [x] Add direct provider lint/tests to authoritative CI.
- [x] Add explicit Desktop `Cargo.lock` metadata preflights to authoritative CI and package paths, using shell-neutral commands on all matrix platforms.
- [x] Bind `vX.Y.Z` tag exactly to authoritative app version metadata.
- [x] Validate shipped component versions/locks, including excluded `swarm-provider` and Desktop lock graph.
- [x] Replace mutable Fabric Loom snapshot tooling with released Loom `1.17.2` and reject future snapshot coordinates in policy/version checks.
- [x] Define production release signing/notarization policy and fail closed when required credentials are missing.
- [x] Keep unsigned builds only on explicitly preview-grade `main-latest` channel.
- [ ] Add/enable `main` branch protection/ruleset requiring the intended check where connector permissions allow. Connector can read but not mutate rulesets; exact manual repository-admin action is documented.
- [x] Standardize artifact/evidence retention by class in active workflows.
- [x] Migrate useful legacy Agent validation checks into current CI before archiving/removing historical workflows.

## Work completed

### Release identity and reproducibility

- Aligned excluded `crates/swarm-provider/Cargo.toml` from stale `0.4.0` metadata to application `0.5.0`; the committed Desktop lock already recorded provider `0.5.0`.
- Reworked `scripts/check_release_version.py` to derive the authoritative application version from the root workspace, validate Desktop/provider/Tauri/Fabric metadata, validate both committed lock graphs, preserve independent wire-protocol versioning, reject snapshot Loom coordinates, and optionally require an exact `v<version>` release tag.
- Replaced `loom_version=1.17-SNAPSHOT` with released `1.17.2` and added CI/policy rejection of snapshot tooling.

### Authoritative validation

- Converted core CI, release-version validation, live player journey, and network soak into reusable workflows callable against the caller's exact SHA.
- Added `.github/workflows/required-validation.yml` as the single aggregate gate with terminal job `Required validation gate`.
- The aggregate gate requires Core CI, migrated specialist validation, version/release identity, and live player journey; release-producing callers additionally require the multi-GiB network soak.
- Added `.github/workflows/specialist-validation.yml` and migrated live catalog validation, Modrinth deterministic tests, CurseForge deterministic tests/command registration, and Desktop locked metadata from historical Agent workflows.
- Added direct Desktop check/clippy/tests and direct provider check/clippy/tests to Core CI.
- Added a separate Desktop RustSec audit using the committed Desktop lock graph.
- Fixed the Desktop package matrix lockfile preflight after exact-head Actions exposed Unix `/dev/null` redirection under Windows PowerShell. The gate now invokes `cargo metadata ... --locked` directly on every platform.

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
- Retired `.github/workflows/agent1-lockfiles.yml`, `agent2-final-validation.yml`, `agent3-curseforge-provider.yml`, and historical `final-ui-screenshots.yml` only after their still-useful checks were promoted into current validation. Their Git history/audit references remain evidence.

### Repository governance inspection

- Live repository ruleset `21764953` (`meow`) is active and blocks deletion/non-fast-forward plus code-quality errors, but contains no required-status-check rule.
- Direct branch-protection read is inaccessible to this GitHub integration (403), and the available connector exposes ruleset reads but no ruleset/protection mutation action.
- `docs/RELEASE_GATES.md` records the exact repository-admin change required to make `Required validation gate` a protected required status rather than falsely claiming it is already enabled.

## Current exact state

Implemented in source/workflow policy:

- exact-SHA reusable validation DAG;
- release-candidate network soak requirement;
- tag/version binding;
- direct excluded-crate tests/lints and Desktop RustSec;
- immutable Action pins and least-privilege publication permissions;
- locked Desktop metadata preflights, including the Windows package path;
- released Loom build-tool coordinate;
- production signing/notarization fail-closed policy;
- preview-only unsigned rolling channel;
- migrated current specialist gates and removal of obsolete historical workflow files;
- deterministic stale-lock and excluded-crate failure regressions.

Exact-head validation vehicle: draft PR #61, `fix/agent-8-ci-release` -> `integration/audit-remediation-v1`.

Current exact-head run: Required Validation run `33582184553` for commit `1510899c61c32ef2ddfaf009da1ff760527be843`.

Observed on that run so far:

- workflow parser accepted the full reusable graph and scheduled 19 jobs;
- `Release identity / Verify release metadata` is GREEN, including workflow policy, tag mismatch, missing signing credentials, and positive credential-set regressions;
- Desktop Rust lock metadata succeeds in the dedicated dependency-audit path;
- Windows Desktop package has progressed beyond workflow setup on the corrected workflow and is still running;
- specialist, direct excluded-crate, package, Rust, Fabric, network impairment, RustSec, and live-player jobs are still running/queued.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Required source/audit review | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Scope and regression requirements reconciled before production edits. |
| Branch-preservation check | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | No pre-existing Agent 8 branch was present. |
| Action pin resolution | PASS | implementation branch | External action tags/channels resolved to concrete commit objects, including annotated tags, before pinning. |
| Repository ruleset inspection | PARTIAL | live repository state | Ruleset exists but does not require Required Validation; connector cannot mutate it. |
| Source-level release DAG review | PASS | implementation branch | Builders/publishers depend on exact-SHA validation; policy lint encodes this invariant. |
| Workflow parser / reusable DAG | PASS | `1510899c...`, run `33582184553` | GitHub accepted and scheduled the full Required Validation graph. |
| Release identity + policy regressions | PASS | `1510899c...`, run `33582184553` | Mutable-action/write-permission policy, tag mismatch, and production-signing policy checks are green. |
| Desktop Rust locked metadata audit | PASS | `1510899c...`, run `33582184553` | Dedicated Desktop RustSec job passed the locked metadata preflight before audit execution. |
| Windows package lockfile shell portability | RUNNING | `1510899c...`, run `33582184553` | Previous exact-head run failed on `/dev/null`; corrected command is under validation. |

## Required validation before handoff

- [x] workflow syntax validation
- [ ] intentionally failing required job blocks rolling publication
- [ ] still-running required job blocks rolling publication
- [x] mismatched tag/version negative regression
- [ ] stale Desktop lock blocks CI/package — regression job is encoded and awaiting exact-head result
- [ ] failing Desktop/provider unit test fails standard CI — regression job is encoded and awaiting exact-head result
- [x] workflow policy lint rejects mutable `uses:` and excess write permissions
- [x] production release missing-credential negative/positive regressions
- [ ] complete exact-head validation on branch

## Blockers

No known production implementation blocker at this checkpoint.

The local checkout/terminal connector rejects this chat with `CALLER_IDENTITY_REQUIRED`, so implementation uses GitHub read/write plus exact-head Actions evidence.

Repository-admin governance limitation: the active ruleset can be inspected but this connector has no ruleset/protection mutation operation. Required-status enforcement therefore remains an external repository-admin action and is not being mislabeled as complete.

## Remaining work

1. Follow Required Validation run `33582184553` to completion and fix any real failing gate without weakening coverage.
2. Capture exact-head results for the stale-lock and excluded-crate negative regressions.
3. Capture practical evidence that publication remains gated while validation is running/failing, to the extent safely testable without publishing from `main` or creating a production tag.
4. Reconcile safe obsolete remote PR/branch cleanup with connector capabilities.
5. Update this ledger to the final exact SHA and only then choose `READY FOR INTEGRATION`, `BLOCKED`, or `NOT COMPLETE`.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: workflows may need to consume new tests created by Agents 1-7/9. Keep the gate extensible and update after integration.

## Agent final statement

NOT COMPLETE
