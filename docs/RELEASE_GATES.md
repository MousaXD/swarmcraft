# SwarmCraft 0.5.0 Release Gates

This document is the release-governance contract for SwarmCraft 0.5.0. A package build is not release evidence by itself: the exact commit being published must first pass the required validation graph.

Application version `0.5.0` and wire protocol version `1` are intentionally independent release dimensions.

## Authoritative validation status

`.github/workflows/required-validation.yml` is the authoritative same-SHA gate. Its terminal status is:

`Required validation gate`

For ordinary pull requests and `main` validation it requires, for the same commit:

1. Core CI: multi-OS workspace lint/tests, process-level acceptance, fuzz smoke, network impairment, Desktop package matrix, Fabric build, root RustSec, direct excluded Desktop/provider checks, and Desktop RustSec.
2. Specialist validation: deterministic/live catalog tests, Modrinth provider tests, CurseForge provider tests and Tauri command registration.
3. CI governance regressions: stale Desktop lock rejection plus deliberate failing Desktop/provider tests that must be rejected.
4. Release identity: shipped-version/lock coherence, immutable workflow/action policy, Fabric build-tool policy, and negative release-policy regressions.
5. Live player journey acceptance against the candidate commit.

Release-producing callers set `run_network_soak: true`, adding the multi-GiB interrupted QUIC soak to the same exact-SHA gate.

The terminal gate uses `if: always()` and inspects every selected reusable-workflow result. Failed, cancelled, or still-running selected validation therefore cannot satisfy the publisher dependency.

## Release publication DAG

### Rolling `main-latest`

`.github/workflows/main-installers.yml` is a preview channel only.

The order is:

`exact-SHA Required Validation + network soak -> platform/Fabric packages -> publish`

All builder jobs depend on validation. The final publisher depends on validation and every builder. Publication is additionally restricted to `refs/heads/main`.

`main-latest` may contain unsigned Windows and ad-hoc macOS development artifacts, but release notes must call them preview/development artifacts. It must never be represented as the production-signed channel.

### Versioned `vX.Y.Z`

`.github/workflows/release.yml` is the production channel.

Before any package job starts it requires both:

- Required Validation for the exact tag target SHA, including network soak; and
- production signing/notarization credential policy.

`scripts/check_release_version.py --tag "$GITHUB_REF_NAME"` semantics are exercised through the reusable release-identity gate. The tag must equal `v<workspace.package.version>` exactly, and shipped Rust/Tauri/Fabric metadata plus both committed Rust lock graphs must agree with that application version.

Production tags fail closed when any required Windows Authenticode or Apple Developer ID/notarization credential is missing. There is no unsigned/ad-hoc production fallback.

## Excluded Rust crates are first-class gates

The root Cargo workspace intentionally excludes:

- `apps/desktop/src-tauri`
- `crates/swarm-provider`

Authoritative CI therefore validates them explicitly rather than assuming `--workspace` includes them.

Desktop requirements include:

- committed `apps/desktop/src-tauri/Cargo.lock` metadata with `--locked`;
- format/check/strict Clippy/tests;
- native package construction on Linux, Windows, Apple Silicon macOS, and Intel macOS;
- RustSec audit of the Desktop lock graph.

Provider requirements include direct format/check/strict Clippy/tests. The provider is shipped through the Desktop graph, so release identity also verifies its package version against the Desktop lock graph.

## Dependency and workflow supply-chain policy

`scripts/check_workflow_policy.py` is part of Required Validation.

Active workflow policy is:

- external `uses:` actions must be pinned to a full 40-character commit SHA;
- every pin must retain a human-readable version/ref comment;
- workflow permissions default to `contents: read`;
- `contents: write` is allowed only on the final publisher jobs in `main-installers.yml` and `release.yml`;
- release builders and publishers must depend on exact-SHA Required Validation;
- release-producing validation must include network soak;
- production release must bind the tag name and invoke the signing credential policy.

Fabric build inputs are explicit released coordinates. Fabric Loom is pinned to released `1.17.2`; snapshot Loom coordinates are rejected by both release-version and workflow policy checks. Minecraft, Fabric Loader, Fabric API, Gradle, and Java versions remain explicit in repository/workflow metadata rather than dynamic `latest` ranges.

## Artifact and evidence retention

Retention is standardized by evidence class:

- ordinary CI package/build artifacts: **7 days**;
- live player-journey evidence: **14 days**;
- rolling/versioned release staging artifacts: **14 days**;
- network-soak logs/metadata/qdisc evidence: **30 days**;
- published GitHub Release assets: retained as release artifacts until the release itself is intentionally replaced/deleted under channel policy.

Historical audit reports and Git commits remain durable evidence. Branch-specific Agent 1/2/3 validation workflows and the historical PR 10 screenshot workflow were removed only after useful checks were migrated to current validation.

## Repository ruleset requirement

The repository currently has active ruleset `21764953` (`meow`) with deletion, non-fast-forward, and code-quality rules. Live inspection on 2026-09-02 showed that it **does not yet require a status check**. The connected GitHub integration can read rulesets but cannot mutate branch protection/rulesets, so this repository-admin action remains external and must not be described as already enabled.

A repository administrator must edit/create the ruleset targeting `refs/heads/main` and configure all of the following:

1. Require a pull request before update/merge.
2. Require the status produced by workflow `Required Validation`, terminal job `Required validation gate` (select the exact context shown by GitHub after its first successful run).
3. Require the branch to be up to date with its target before merge when that option is available for the selected required check.
4. Preserve the existing deletion and non-fast-forward protections.
5. Do not add a bypass actor that can publish ordinary application changes around the required gate.

Until that repository setting is applied, the workflow DAG still prevents release workflows from publishing without their own exact-SHA validation, but GitHub itself does not yet prevent an administrator from updating `main` without the intended PR status.

## Evidence lifecycle and cleanup

Current validation belongs in current workflows, not branch-named archaeology.

The following historical workflow files were retired after migration:

- `agent1-lockfiles.yml`
- `agent2-final-validation.yml`
- `agent3-curseforge-provider.yml`
- `final-ui-screenshots.yml`

Old audit reports, exact commit SHAs, and relevant workflow-run IDs remain historical evidence. Obsolete remote validation branches/PRs may be closed or deleted only after confirming they contain no unique implementation commit that is still needed. Branch deletion is a repository-admin cleanup action when the available connector lacks a safe branch-delete operation.

## What green CI does not claim

A green Required Validation result is strong automated release evidence, not universal deployment certification. It does not by itself certify every residential NAT/CGNAT/mobile/IPv6 network, every disk-failure mode, or infinite-duration hostile-peer behavior. Those broader claims require dedicated field/soak evidence and must remain accurately scoped.

See `docs/NETWORK_VALIDATION.md`, `docs/FINAL_PLAYER_JOURNEY_ACCEPTANCE.md`, and `docs/IMPLEMENTATION_STATUS.md` for the corresponding product-level evidence and limitations.
