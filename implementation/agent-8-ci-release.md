# Agent 8 — CI / Release Governance

## Status

STATUS: NOT STARTED

BRANCH: `fix/agent-8-ci-release`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT HEAD SHA: pending

INTEGRATED SHA: pending

## Mission

Make release publication mean that the exact published SHA passed the project’s required validation gates, while reducing workflow supply-chain privilege and making current-main validation authoritative for all shipped crates.

## Findings owned

- FINAL-021 — publication not gated on successful same-SHA validation
- FINAL-022 — mutable GitHub Action refs with write-capable workflow tokens
- FINAL-035 — excluded Desktop/provider direct lint/test blind spots
- FINAL-036 — release tag/version binding gap
- FINAL-038 — signing/notarization and release-class policy gap
- FINAL-044 — branch protection/required checks governance
- FINAL-046 — evidence/workflow lifecycle and repository cleanup assigned by final audit

Read `audits/FINAL-AUDIT.md` and Auditor 9 CI/Release before editing.

## Dependencies

Required before starting: none.

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

None yet.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| None yet | - | - | - |

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

None at campaign start.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: workflows may need to consume new tests created by Agents 1-7/9. Keep the gate extensible and update after integration.

## Agent final statement

NOT COMPLETE
