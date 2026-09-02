# Agent 8 — CI / Release Governance

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-8-ci-release`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH CREATION SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (campaign planning head; production baseline remains the declared `b4bab085...` tree plus implementation ledgers)

CURRENT HEAD SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` before this ledger-start commit

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

The final audit and Auditor 9 agree on the core failure: installer/release publication is not coupled to completed validation of the exact published SHA. Auditor 9 additionally provides the concrete action-pin, excluded-crate, tag/version, Loom, Desktop lock, signing, retention, and historical-workflow failure scenarios required below.

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

- [ ] Define one authoritative reusable same-SHA validation gate for release candidates.
- [ ] Make `main-latest` publication wait until every required same-SHA gate completes successfully.
- [ ] Make versioned tag release rerun or verify the full required gate for the exact tag target SHA.
- [ ] Prove packaging success cannot publish while required CI is running/failing.
- [ ] Pin every third-party GitHub Action to immutable full commit SHA with human-readable version comment.
- [ ] Default workflow permissions to read; grant `contents: write` only to final publisher jobs that need it.
- [ ] Add direct Desktop Rust lint/tests to authoritative CI.
- [ ] Add direct provider lint/tests to authoritative CI.
- [ ] Enforce Desktop `Cargo.lock` with explicit locked metadata/build preflight.
- [ ] Bind `vX.Y.Z` tag exactly to authoritative app version metadata.
- [ ] Validate all shipped component versions/locks, including excluded crates.
- [ ] Replace mutable Fabric Loom snapshot tooling with immutable build-tool identity or a pinned equivalent and add dependency verification as appropriate.
- [ ] Define production release signing/notarization policy and fail closed when required credentials are missing.
- [ ] Keep unsigned builds only on explicitly preview-grade channels.
- [ ] Add/enable `main` branch protection/ruleset requiring intended checks before update/merge where connector permissions allow; otherwise document exact manual action needed.
- [ ] Standardize artifact/evidence retention by class.
- [ ] Migrate useful legacy Agent validation checks into current CI before archiving/removing historical workflows.

## Work completed

- Verified no pre-existing `fix/agent-8-ci-release` branch existed, so no legitimate Agent 8 implementation work needed preservation.
- Created `fix/agent-8-ci-release` from campaign planning head `a9736b159d9e9618a3ed8515c20e93f92c1453cb` while retaining the declared production baseline `b4bab08562cf0eb53763674407375b023e1d0858`.
- Read all required audit inputs and reconciled the ledger finding labels with the final audit’s exact FINAL-036/038/044/046 definitions.
- Confirmed the current branch still contains the audited legacy workflow surface and therefore all implementation checklist items remain materially open at start.

## Current exact state

Confirmed incomplete at start:

- rolling and versioned publication are independent from the complete same-SHA validation suite;
- release workflows use broad write permission and mutable action refs;
- current authoritative CI does not directly lint/test the excluded Desktop/provider Rust crates;
- Desktop package construction lacks an explicit locked-metadata preflight;
- release tags are not bound to the application version by the tag path itself;
- Fabric Loom uses a mutable snapshot coordinate;
- production version tags may publish without Windows/macOS production signing credentials;
- `main` protection/required checks were absent at audit time;
- Desktop RustSec/evidence retention are incomplete and legacy Agent validation workflows remain active historical machinery.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Required source/audit review | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Scope and regression requirements reconciled before production edits. |
| Branch-preservation check | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | No pre-existing Agent 8 branch was present. |

## Required validation before handoff

- [ ] workflow syntax validation
- [ ] intentionally failing required job blocks rolling publication
- [ ] still-running required job blocks rolling publication
- [ ] mismatched tag/version blocks versioned release
- [ ] stale Desktop lock blocks CI/package
- [ ] failing Desktop/provider unit test fails standard CI
- [ ] workflow lint rejects mutable `uses:` and excess write permissions
- [ ] production release without signing credentials blocks publication according to policy
- [ ] exact-head validation on branch

## Blockers

No implementation blocker at start. The local checkout/terminal connector currently rejects this chat with `CALLER_IDENTITY_REQUIRED`, so implementation is proceeding through the GitHub read/write connector and exact-head GitHub workflow evidence. Repository policy changes that require a ruleset/protection mutation API will be attempted through available connector capabilities and, if unavailable, recorded as an exact manual governance action rather than silently claimed complete.

## Remaining work

All production workflow/script/policy implementation and exact-head regression validation remain.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: workflows may need to consume new tests created by Agents 1-7/9. Keep the gate extensible and update after integration.

## Agent final statement

NOT COMPLETE
