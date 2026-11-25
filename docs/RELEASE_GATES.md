# SwarmCraft 0.5.1 Release Gates

This document is the release-governance contract for SwarmCraft 0.5.1. A package build is not release evidence by itself: the exact commit being published must first pass the required validation graph.

Application version `0.5.1` and wire protocol version `1` are intentionally independent release dimensions.

## Authoritative validation status

The local-first policy supersedes the former automatic full validation graph.
See [CI_LOCAL_FIRST.md](CI_LOCAL_FIRST.md) for local commands and evidence.

`ci.yml` runs one lightweight metadata check on main/PR code changes only.
It is not a test or build certificate. It never emits the protected final
status `Required validation gate`.

`required-validation.yml` is manual or release-called only, after explicit
operator approval. It requires the exact locally validated SHA, checks the
requested ref and release metadata, and then runs final native Windows/macOS
Rust and Desktop checks. Its terminal `Required validation gate` requires both
preflight and native verification to succeed. The SHA input is an operator
attestation, **not** independent proof that the local tests ran. Retain exact
local logs and do not attest a failed, skipped, or unrun required test.

The former Linux workspace/process, excluded Desktop/provider, deterministic
and authorized live catalog, fuzz, impairment/soak, installed Linux GUI,
Fabric, RustSec and clean-machine acceptance requirements remain local release
requirements. Removing their hosted workflows does not certify those gates.

## Release publication DAG

### Commit-pinned preview snapshots

`.github/workflows/main-installers.yml` is a preview channel only.

The order is:

`local validation including network soak -> manual exact-SHA native validation -> platform/Fabric packages -> verify -> upload draft -> verify uploads -> publish`

All builder jobs depend on validation. The read-only package verification job runs only during explicitly dispatched packaging and records the five package groups and their checksums. The publisher depends on successful verification and then checks the same run's artifacts again before staging the draft. Publication is restricted to `refs/heads/main`.

Each preview is published under `main-snapshot-<full commit SHA>`. The publisher checks the byte count and SHA-256 digest of every uploaded asset while the release is still a draft, then binds the tag to that commit and publishes the prerelease. An interrupted upload leaves the previous releases available on the GitHub Releases page. A rerun can resume a draft whose existing assets match the verified files. A conflicting draft or tag requires operator review.

The existing `main-latest` release is retained for legacy clients. To roll back the application, obtain the previous complete snapshot from GitHub Releases, verify the checksums, stop the application and preserve the current profile. Use the current CLI's read-only [profile compatibility and rollback preflight](PROFILE_UPGRADE.md). The historical targets refuse initialized data; never downgrade a live profile or reconnect a copied historical identity. Published historical executables predate this guard, so the operator preflight is mandatory. The historical 0.5.0 release lacks a commit-bound provenance manifest. Preview Windows installers are unsigned and macOS disk images are ad-hoc signed development builds.

### Versioned `vX.Y.Z`

`.github/workflows/release.yml` is the production channel.

Before any package job starts it requires both:

- local validation including network soak, attested for the exact tag target SHA, plus final native Required Validation; and
- production signing/notarization credential policy.

`scripts/check_release_version.py --tag "$GITHUB_REF_NAME"` semantics are exercised through the reusable release-identity gate. The tag must equal `v<workspace.package.version>` exactly, and shipped Rust/Tauri/Fabric metadata plus both committed Rust lock graphs must agree with that application version.

Production tags fail closed when any required Windows Authenticode or Apple Developer ID/notarization credential is missing. There is no unsigned/ad-hoc production fallback.

The final staged Windows installer must pass native Authenticode verification,
timestamp and expected certificate checks. The staged macOS disk image must
pass stapled-ticket validation and its read-only mounted app must pass strict
signature, expected Developer ID team, app ticket, Gatekeeper and architecture
checks. Native jobs issue a JSON receipt bound to the exact source SHA,
platform, file, size and SHA-256; the publisher rejects missing or mismatched
receipts and records their SHA-256 in the provenance manifest. Receipts report
the native job's checks, not a publisher-side independent OS trust attestation.
No mobile or unsupported target is certified. The preview inventory contains
only Debian amd64, Windows x64, macOS arm64/x86_64 and Fabric JAR; production
additionally requires Linux AppImage.

See the [published preview audit and rollback rehearsal](../research/release-verification/REPORT.md).
Actual signed production execution requires authorized native runners and
credentials; the Linux-only rehearsal and offline gate tests do not replace it.

## Signed update delivery

The explicit opt-in updater uses independently pinned channel keys and
offline-signed indexes prepared only from already reviewed release assets.
The [signed update contract](SIGNED_UPDATES.md) is additive to these
release gates. No updater index, key or package is published automatically;
the existing exact-SHA, native signing and release approval gates still
apply. A signed update descriptor does not replace platform OS trust or
profile compatibility and save-first installation procedures.

## Excluded Rust crates are first-class gates

The root Cargo workspace intentionally excludes:

- `apps/desktop/src-tauri`
- `crates/swarm-provider`

Local validation must cover them explicitly rather than assuming `--workspace` includes them. Final native Desktop checks also run on Windows/macOS.

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
- release-producing local validation must include network soak;
- no temporary/agent workflows, schedules or automatic heavy jobs;
- only the five reviewed workflows are permitted; see the local-first policy;
- production release must bind the tag name and invoke the signing credential policy.

Fabric build inputs are explicit released coordinates. Fabric Loom is pinned to released `1.17.2`; snapshot Loom coordinates are rejected by both release-version and workflow policy checks. Minecraft, Fabric Loader, Fabric API, Gradle, and Java versions remain explicit in repository/workflow metadata rather than dynamic `latest` ranges.

## Artifact and evidence retention

Retention is standardized by evidence class:

- local test/build evidence: retain for the applicable review;
- live player-journey evidence: **14 days**;
- preview/versioned release staging artifacts: **14 days**;
- network-soak logs/metadata/qdisc evidence: **30 days**;
- published GitHub Release assets: retained as release artifacts until the release itself is intentionally replaced/deleted under channel policy.

Historical audit reports and Git commits remain durable evidence. Branch-specific Agent 1/2/3 validation workflows and the historical PR 10 screenshot workflow were removed only after useful checks were migrated to current validation.

## Repository ruleset requirement (historical configuration; verify before rollout)

Repository governance is enforced by two active repository rulesets, both with an empty bypass-actor list:

- ruleset `23573424` (`Agent 8 required validation gate`) targets `refs/heads/main` and `refs/heads/integration/audit-remediation-v1`. It requires the exact status context `Required validation gate` and enables strict required-status policy, so the tested commit must be up to date with its target before merge;
- ruleset `23678402` (`Agent 8 main PR safety`) targets `refs/heads/main`. It requires changes to arrive through a pull request, prevents deletion and non-fast-forward updates, preserves code-quality enforcement, and permits the repository's enabled merge, squash, and rebase strategies. It deliberately requires zero approving reviews because the release-governance contract requires a PR path, not an invented reviewer-count policy.

Live effective-rules inspection on 2026-09-18 confirms that `main` receives both the PR/safety rules and the strict `Required validation gate`, while `integration/audit-remediation-v1` receives the same strict required-status check. There is no configured ruleset bypass actor for either protected path.

## Evidence lifecycle and cleanup

Current validation belongs on local machines, not temporary or agent-specific workflows.

The following historical workflow files were retired after migration:

- `agent1-lockfiles.yml`
- `agent2-final-validation.yml`
- `agent3-curseforge-provider.yml`
- `final-ui-screenshots.yml`

Old audit reports, exact commit SHAs, and relevant workflow-run IDs remain historical evidence. Obsolete remote validation branches/PRs may be closed or deleted only after confirming they contain no unique implementation commit that is still needed. Branch deletion is a repository-admin cleanup action when the available connector lacks a safe branch-delete operation.

## What green CI does not claim

A green Required Validation result is strong automated release evidence, not universal deployment certification. It does not by itself certify every residential NAT/CGNAT/mobile/IPv6 network, every disk-failure mode, or infinite-duration hostile-peer behavior. Those broader claims require dedicated field/soak evidence and must remain accurately scoped.

See `docs/NETWORK_VALIDATION.md`, `docs/FINAL_PLAYER_JOURNEY_ACCEPTANCE.md`, and `docs/IMPLEMENTATION_STATUS.md` for the corresponding product-level evidence and limitations.

## Mandatory packaged desktop acceptance

Run installed Linux Tauri/WebKitGTK acceptance locally, using the exact
candidate Debian package. The existing harness at
`scripts/acceptance/packaged-desktop-webdriver.py` checks actual packaged
application startup, signed/native behavior, and restart persistence.
Also retain the missing-CLI-sidecar negative control in an isolated
test installation. For candidate Svelte, exercise the missing-module
negative and viewport checks as documented in the candidate README.

This is Linux GUI/runtime coverage, not Windows or macOS installed GUI
acceptance. Final Windows/macOS package construction is not evidence that
the GUI was opened. Keep actual reviewed local evidence and record skipped
or blocked checks; no hosted workflow success may substitute for them.

The prior CI definitions and command recipes remain available in git
history (the pre-policy integration checkpoint `49ca1f5`); use them as
local execution recipes only, never restore or dispatch those workflows.
