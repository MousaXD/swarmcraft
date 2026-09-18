# Agent 8 — CI / Release Governance

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-8-ci-release`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

BRANCH CREATION SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb`

LATEST INTEGRATION BASE CONSUMED: `efde7ecb996ead8d414378e7876354b731e4a963`

BASE RECONCILIATION MERGE: `43b3361019bd5f3174f27282b4087df95fd43462`

CURRENT CONTINUATION HEAD BEFORE RATE-LIMIT FIX: `59061532ad64410043e127df7553e71dca9714d2`

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

The branch later consumed the completed remediation integration through Agent 9. `integration/audit-remediation-v1` at `efde7ecb996ead8d414378e7876354b731e4a963` was reconciled at merge commit `43b3361019bd5f3174f27282b4087df95fd43462`, followed by cleanup commit `22525d9b27dfa1c96802526925f6294ae58d06cb` and Desktop lockfile restoration commit `59061532ad64410043e127df7553e71dca9714d2`.

## 2026-09-17 continuation

### Current exact-head evidence before this fix

- Required Validation run `34170353439` on PR head `59061532ad64410043e127df7553e71dca9714d2`: **SUCCESS**. The terminal `Required validation gate` passed, including direct Desktop/provider gates, governance regressions, platform CI, package builds, dependency audits, specialist validation, and the live player journey.
- Main Desktop Installers run `34170353502` on the same PR head: **FAILURE**. Every observed release-path component other than the soak-enabled network validation passed. The single release-blocking failure was `Exact-SHA required validation / Network soak / Interrupted QUIC multi-GiB soak`.
- Archived soak evidence from that run records a 2 GiB transfer with 256 MiB forced restarts. The test timed out after 30 seconds waiting for a blob-chunk acknowledgement.

### Release-path regression discovered after integration reconciliation

The network admission hardening consumed from Agent 4 limits an authenticated peer to 128 requests per 10-second admission window and returns `WireResponse::Error { code: "RATE_LIMITED", ... }` when that budget is exceeded. The large-transfer soak and the real daemon replication sender both treated blob streaming as an effectively unbounded burst. The soak ignored the `RATE_LIMITED` response and waited until its acknowledgement timeout; the daemon could similarly overrun the admission budget during real snapshot replication.

The continuation fix therefore:

- makes the soak harness recognize `RATE_LIMITED`, retain the exact uncommitted offset, wait beyond the admission window, and retry without corrupting resume semantics;
- changes daemon replication from an eager all-chunks burst to one acknowledged chunk in flight per `(transport peer, world)`;
- retains exact pending blob/offset state across rate limiting;
- retries the same chunk after an 11-second bounded backoff, which is deliberately longer than the network layer's 10-second authenticated request window;
- clears pending replication state on disconnect/outbound failure and validates acknowledgement hash/offset/snapshot identity before advancing.

### Local validation for the continuation fix

- `cargo fmt --all -- --check` — PASS after formatting.
- `git diff --check` — PASS.
- `cargo test -p swarm-network --test network_transfer_soak interrupted_quic_transfer_resumes_after_lost_ack --locked -- --ignored --nocapture` — PASS; 40 MiB transfer hit the authenticated request limit once at 32 MiB, backed off, re-established the connection-bound proof, completed one forced lost-ack sender restart, and preserved the exact resume offset.
- `cargo test -p swarm-cli --lib --locked` — PASS, 61/61.
- `cargo clippy -p swarm-network --test network_transfer_soak -p swarm-cli --lib --locked -- -D warnings` — PASS.
- `cargo test -p swarm-cli --test live_join_replication --locked` — PASS.
- `cargo test -p swarm-cli --test host_process --locked` — PASS.
- `python3 scripts/check_workflow_policy.py` — PASS, 10 workflow files accepted.
- `python3 scripts/check_release_version.py` — PASS, application metadata `0.5.0`, wire protocol `1`.

### Exact-head validation of `8200ccfba48f5ec3c0c8672b9b6151f3d184aa43`

The first pushed continuation head proved the primary release blocker was fixed but exposed three independent follow-up gates:

- Main Desktop Installers run `35218481394`: the 2 GiB `Interrupted QUIC multi-GiB soak` **PASSED** at job `105192554819`, proving the original release-path timeout was fixed. The overall run still failed because the nested Required Validation inherited non-soak failures below.
- Required Validation run `35218481209`: `Network impairment (QUIC resume)` completed the 40 MiB transfer successfully under netem but failed only because its assertion expected at least one rate-limit event; impairment slowed 256 KiB requests enough that no 10-second admission window was exhausted. The first follow-up reduced chunk size so the admission path was deterministic without impairment.
- Root and Desktop RustSec jobs failed on newly published `RUSTSEC-2026-0285` against `rustls 0.23.43`. The advisory was published 2026-09-14 and marks `rustls >= 0.23.45` patched. Both committed lock graphs are now advanced to `0.23.45`; local `cargo audit` reports zero vulnerabilities for both graphs.
- The standalone macOS workspace test run timed out in `simultaneous_bidirectional_dials_converge_on_one_authenticated_connection`, while the same-SHA macOS job in the nested release validation passed. The symmetric-dial convergence timeout is increased from 10 to 30 seconds to remove runner-load timing sensitivity without weakening the one-connection/authentication assertions.

### Local validation after exact-head follow-up fixes

- `cargo test -p swarm-network --test network_transfer_soak interrupted_quic_transfer_resumes_after_lost_ack --locked -- --ignored --nocapture` — PASS; 8 MiB/32 KiB regression hit `RATE_LIMITED` at 4 MiB, performed the bounded backoff, completed one forced lost-ack restart, and finished with one rate-limit event.
- `cargo test -p swarm-cli --test discovery_network_freshness simultaneous_bidirectional_dials_converge_on_one_authenticated_connection --locked -- --nocapture` — PASS in three consecutive local runs.
- `cargo clippy -p swarm-network --test network_transfer_soak -p swarm-cli --test discovery_network_freshness -p swarm-cli --lib --locked -- -D warnings` — PASS.
- `cargo audit --file Cargo.lock` — PASS with zero vulnerabilities; informational unmaintained warnings remain allowed by the existing audit policy.
- `cargo audit --file apps/desktop/src-tauri/Cargo.lock` — PASS with zero vulnerabilities; existing informational/unmaintained/unsound warnings remain allowed by policy.
- `cargo check --workspace --all-features --locked` — PASS.
- Direct local Desktop `cargo check` is not authoritative in this nested Git worktree because Cargo discovers the outer checkout workspace; exact-head GitHub Desktop gates remain the authority and will re-run after the follow-up commit.
- The local environment requires an interactive sudo password for `tc`, so exact netem reproduction is delegated to the pushed `Network impairment (QUIC resume)` job.

### Exact-head validation of `8a84a26a23f058f2a1e7ef754030bb8070be4f59`

The second pushed continuation head closed the security and macOS failures: root RustSec, Desktop RustSec, and the standalone macOS workspace job all passed. The only observed failing component in Required Validation run `35384614518` was `Core CI / Network impairment (QUIC resume)`.

That job completed the full 8 MiB interrupted transfer under the configured 12 ms / 0.5% loss / 100 Mbit netem profile with one forced restart and correct resume semantics, but recorded `rate_limits=0`. The failure was solely the assertion that the same impaired transfer must also exceed the authenticated request budget. With impairment enabled, request latency can legitimately refresh the 10-second admission window before 128 requests accumulate.

The network gate is therefore split by invariant instead of making one timing profile prove incompatible conditions:

- before netem, `authenticated_request_budget_backoff_preserves_transfer_offset` uses 32 KiB chunks to exceed the authenticated request budget, requires at least one `RATE_LIMITED` response, backs off beyond the admission window, and preserves the exact transfer offset;
- after netem is installed, `interrupted_quic_transfer_resumes_after_lost_ack` uses 256 KiB chunks and proves interrupted QUIC/lost-ack resume without requiring rate limiting to occur.

Local validation of the split gate is green: the admission test hit `RATE_LIMITED` at 4 MiB and completed with `rate_limits=1`; the independent lost-ack test completed 8 MiB with one forced restart; strict clippy, workflow policy, release-version policy, format, and diff checks all pass.

### Administrative blocker re-check

- `ci/discovery-fixture-trigger` is now absent from the remote after `git fetch --prune`; FINAL-046's final stale-ref blocker is resolved.
- Live ruleset `21764953` (`meow`) still contains only deletion, non-fast-forward, and code-quality rules. It still does not require `Required validation gate`.
- A separate required-status ruleset is still needed for `refs/heads/main` and `refs/heads/integration/audit-remediation-v1`. This execution environment did not permit the repository-ruleset mutation operation, so FINAL-038 remains blocked on repository administration after code validation finishes.

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
| Required status rule installed in repository | BLOCKED | live ruleset `21764953` still lacks required-status rule as of 2026-09-18; authenticated repository access is admin-capable, so this will be applied after the next exact-head gate is green |
| Final stale validation ref deleted | PASS | `ci/discovery-fixture-trigger` absent after remote prune on 2026-09-17 |
| Reconciled-head Required Validation | PASS | run `34170353439` at PR head `59061532ad64410043e127df7553e71dca9714d2` |
| Reconciled-head release path with soak | FAIL | run `34170353502`; only multi-GiB soak failed |
| First rate-limit fix release-path multi-GiB soak | PASS | job `105192554819` in run `35218481394` at `8200ccfba48f5ec3c0c8672b9b6151f3d184aa43` |
| First rate-limit fix Required Validation | FAIL | run `35218481209`: deterministic impairment assertion, new rustls advisory, and one same-SHA macOS timing flake; follow-up fixes implemented locally |
| Second follow-up RustSec + macOS gates | PASS | run `35384614518`: root audit, Desktop audit, and macOS workspace job all green at `8a84a26a23f058f2a1e7ef754030bb8070be4f59` |
| Second follow-up network impairment | FAIL | job `105728481447` completed the impaired transfer with `rate_limits=0`; split invariant gate implemented locally |
| Rate-limit-aware lost-ack transfer regression | PASS | local 8 MiB/32 KiB interrupted transfer; rate limit hit at 4 MiB and exact resume completed on 2026-09-18 |
| Root RustSec after RUSTSEC-2026-0285 | PASS | local audit with `rustls 0.23.45`, zero vulnerabilities on 2026-09-18 |
| Desktop RustSec after RUSTSEC-2026-0285 | PASS | local audit with `rustls 0.23.45`, zero vulnerabilities on 2026-09-18 |
| Symmetric discovery dial stability | PASS | three consecutive local targeted runs after 30-second convergence bound on 2026-09-18 |
| Reconciled daemon library tests | PASS | local `swarm-cli --lib`, 61/61 on 2026-09-17 |
| Live join replication after rate-limit fix | PASS | local `live_join_replication` on 2026-09-17 |
| Host process lifecycle after rate-limit fix | PASS | local `host_process` on 2026-09-17 |

## Remaining blockers

### BLOCKER 1 — repository required-status enforcement

FINAL-038 cannot be truthfully closed from this execution environment.

Live ruleset `21764953` (`meow`) remains active with deletion, non-fast-forward, and code-quality rules only. It still does **not** contain a required-status-check rule for `Required validation gate`.

The repository is reachable through an authenticated GitHub CLI session with repository administration permission. Required-status enforcement is still absent at this ledger revision; it will be applied only after the next exact-head Required Validation and release-path runs are green so the protected status refers to a currently validated head.

Required repository-admin action is documented in `docs/RELEASE_GATES.md`: require the exact terminal status `Required validation gate` on the protected integration/main path.

### RESOLVED — final obsolete remote ref cleanup

FINAL-046's final stale ref is no longer present. A 2026-09-17 remote prune and `ls-remote` confirmed that `ci/discovery-fixture-trigger` is absent while `agent/discovery` remains at `0a72380aebbc6f227957cae733de64dc6f85638c`.

## Remaining work

The reconciled branch exposed one release-path defect after the old ledger was written: large snapshot replication could exceed the authenticated request budget and ignore `RATE_LIMITED`. Commit `8200ccfba48f5ec3c0c8672b9b6151f3d184aa43` fixed that primary blocker and passed the 2 GiB release soak. `8a84a26a23f058f2a1e7ef754030bb8070be4f59` closed the new RustSec advisory and macOS timing failure; the remaining network-test coupling is now split into independent admission-backoff and impaired-resume gates and is pending exact-head CI.

To unblock handoff:

1. Commit and push the split network invariant gate with this ledger update.
2. Require exact-head Required Validation and the soak-enabled Main Desktop Installers release path to pass on the new commit.
3. Add repository required-status enforcement for exact status `Required validation gate` on the protected release/integration path.
4. Re-read live repository state and record the exact validated handoff head.

## Handoff

READY FOR INTEGRATION: NO

Latest pushed continuation head before the split-gate commit: `8a84a26a23f058f2a1e7ef754030bb8070be4f59`.

Required Validation on second continuation head: `35384614518` — network impairment job FAILURE after a successful transfer because rate limiting did not occur under netem; split-gate fix pending exact-head CI.

Release-path validation on first continuation head: `35218481394` — overall FAILURE because nested Required Validation failed, but the release-blocking 2 GiB network soak itself is SUCCESS.

Known conflict areas: active workflow files and release/version policy scripts. Integration must preserve the aggregate `Required validation gate` contract and the release DAG dependency on it.

## Agent final statement

IN PROGRESS
