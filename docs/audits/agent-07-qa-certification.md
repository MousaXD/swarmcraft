# Agent 07 — Independent QA and Release Certification

**Date:** 2026-11-08 (UTC+04)  
**Coordinator:** Agent 00  
**Repository:** `MousaXD/swarmcraft`  
**Assessment:** **BLOCKED for the new multi-agent integrated release**. This is an initial evidence-based QA snapshot, **not** final release certification.

## Scope and immutable references

- `main` at initial audit: `45174310d1433116e64b9b6978cb4f2aa1c22c15`.
- Read `AGENTS.md`, `docs/RELEASE_GATES.md`, current workflows, PRs #77/#79/#88/#89/#90, changed-file lists, current Rust source and regression tests, and exact-run jobs/logs.
- The existing baseline is CI-green; this does not imply that all historically identified safety defects have been reconciled or that future integration is release-ready.
- Do not merge production code, close issues, disable gates, modify the required workflow or treat missing evidence as passing.

## Certification ledger

| Work item | Head SHA / target | Test SHA / evidence | Required validation and installer result | Findings / limitations | Verdict |
| --- | --- | --- | --- | --- | --- |
| Historical PR #77, Rust safety | `f71684c5b4ed9381161e0fb08fa7872ae993451d`, targeting old `finalize/audit-remediation-v1` | Workflow association to that HEAD; [Required Validation 42107678600](https://github.com/MousaXD/swarmcraft/actions/runs/42107678600), [Installers 42107678830](https://github.com/MousaXD/swarmcraft/actions/runs/42107678830) | Both reported success, but PR remains open against old base | Eight-file changes not fully incorporated into current `main`; cannot promote historical SHA without reconciliation | **BLOCKED** |
| Historical PR #79, composition | `461241cc7e09819572836add2e743e1888b00280`, merged | Historical exact-head logs in PR #81 | Historical success reported | Only selected recovery promise fix from #77 in `main`, not all eight changed files | **N/A — merged historical** |
| PR #88, desktop startup and Linux GUI gate | `dd4d37e9932a318e89d3406eac917ee33fd6a1da`, merged to `main` | [Required Validation 42389283521](https://github.com/MousaXD/swarmcraft/actions/runs/42389283521), [Installers 42389283358](https://github.com/MousaXD/swarmcraft/actions/runs/42389283358) | Both success | Linux installed-GUI smoke with four genuine sidecars; Windows/macOS packaged builders are not installed-GUI launch tests | **READY for its already-merged scope only** |
| PR #89, v0.5.1 metadata | HEAD `ef411477b76e0f42cdddb4976373a7dee29a70e2`; base `main` `45174310d1433116e64b9b6978cb4f2aa1c22c15` | **Actual checked-out merge SHA** `ebb45a1dff1793507c99b15c7f4f8aa405aecfc4`, confirmed from checkout logs | [Required Validation 42408519678](https://github.com/MousaXD/swarmcraft/actions/runs/42408519678) success; [Installers 42408519976](https://github.com/MousaXD/swarmcraft/actions/runs/42408519976) success | All metadata/lockfile version edits align and wire protocol stays 1; baseline still contains serious unresolved safety error-handling defects | **BLOCKED for independent release certification pending safety reconciliation** |
| Agent 01 / PR #90 | At inspection: `24bb963a041650be752ff6a8d9b96f5d87b8260b`, draft | [Required Validation 42466225204](https://github.com/MousaXD/swarmcraft/actions/runs/42466225204) in progress; [Installers 42466225250](https://github.com/MousaXD/swarmcraft/actions/runs/42466225250) queued | Not complete; first commit intentionally adds a red regression only | Test proves the unfixed corruption path once it fails as expected; remaining patches, regression cases, exact final SHA required | **BLOCKED — intended red phase** |
| Agent 02, signed hosting policy | Branch `agent-02/hosting-policy-protocol` existed at same tree as `main` | No submitted unique code or PR at inspection | No qualifying run | Signed config, old-world compatibility, local consent/quorum semantics unverified | **BLOCKED — no candidate** |
| Agent 03, hosting runtime budgets | Branch `agent-03/hosting-runtime-budgets` existed at same tree as `main` | No submitted unique code or PR | No qualifying run | Resource admission/withdrawal and OS memory enforcement unverified | **BLOCKED — no candidate** |
| Agent 04, Svelte/TypeScript desktop | Branch `agent-04/svelte-typescript-desktop` existed at same tree as `main` | No submitted unique code or PR | No qualifying run | All-journey feature parity, typed IPC, accessibility and installed-GUI regression remain unverified | **BLOCKED — no candidate** |
| Agent 05, three-machine E2E | No branch / PR found at inspection | None | None | Requires three separately installed GUI/Minecraft systems on distinct networks and real recovery state evidence | **BLOCKED — no candidate** |
| Agent 06, distributed simulation research | No branch / PR found at inspection | None | None | No benchmarks/RFC to audit; must not claim active-active Minecraft simulation scaling | **NOT APPLICABLE until evidence submitted** |
| Combined integration candidate | Not yet constructed | None | None | Individual successful PRs cannot stand in for revalidated combined exact SHA | **BLOCKED** |

This ledger is a dated snapshot. Recheck HEAD, merge-base, actual checked-out commit, job results, artifacts and review threads before a final decision.

## Confirmed critical source-level findings on main

### QA-SAFE-001 — Corrupt membership promise can be replaced (P0)

In `crates/swarm-storage/src/membership.rs`, `promise_membership_proposal` uses `if let Ok(existing) = self.load_membership_promise(...)` and otherwise creates a new durable promise. A record that exists but is malformed or unreadable is therefore treated as absent. This can overwrite a previous signed-state/fencing commitment. PR #77 changes the error distinction to permit **only NotFound** as absence. Its `corrupt_membership_promise_cannot_be_overwritten` regression is not present on audited `main`. Reproduce the red test, then enforce fail-closed behavior and verify the original bytes remain.

### QA-SAFE-002 — Corrupt membership prepare bypasses authority/recovery fence (P0)

In `crates/swarm-cli/src/daemon.rs`, `ensure_no_membership_prepare` returns `Ok(())` unless `load_membership_promise` succeeds, so malformed state is not recognized as a blocking durable prepare. `maintain_authority_leases` likewise branches on `if let Ok(promise)`; an unreadable promise does not automatically fail closed. The historic PR #77 has a corrective path and `corrupt_membership_prepare_blocks_authority_and_recovery_transitions` regression. Agent 01 PR #90 has begun by adding exactly that regression in a draft red phase.

### QA-SAFE-003 — Unreadable world config or epoch can trigger bootstrap fallback (P0)

In `crates/swarm-storage/src/state.rs`, `save_world_config` uses `if let Ok(existing) = self.load_world_config(...)` followed by an `else` branch permitting sequence 1. It separately uses `if let Ok(epoch) = self.load_epoch_record(...)` followed by fallback membership authority. These branches cannot distinguish genuine first-time missing state from corrupt/unreadable signed canonical records. PR #77 adds precise missing-vs-corrupt handling and negative regressions `corrupt_world_config_cannot_be_reinitialized_from_sequence_one` and `corrupt_epoch_cannot_fall_back_to_bootstrap_world_config_authority`; those regressions were absent on `main` when inspected.

### QA-SAFE-004 — Canonical symlink/path read protections absent (P1)

`crates/swarm-storage/src/transaction.rs` on `main` lacks the PR #77 `canonical_file_exists` and `read_canonical_file` protections. Multiple canonical record paths still use `fs::read`, `path.exists`, or `path.try_exists`, which do not make the intended dangling-symlink/nonregular-file/error distinction. Independently test symlinks, dangling symlinks, directories, and I/O failures without unsafe state reset; account for TOCTOU limitations explicitly rather than claiming complete race freedom.

### QA-SAFE-005 — Historical reconciliation was partial

PR #79 was a narrow composite (workflow, two lockfiles, discovery test and recovery-promise storage file). While recovery corruption testing remains in `main`, the other #77 fixes across eight Rust files were not fully brought over. The ongoing Agent 01 PR #90 correctly treats reconciliation as new work on current `main`, not blind merging of the old PR.

**Evidence quality:** Above findings are direct source comparisons, not completed exploit simulations. The strongly indicated unsafe branches justify **BLOCKED** until regression tests and fixes prove they fail closed. Green CI does not disprove an untested negative case.

## Linux installed-GUI / CI evidence audited

- On PR #89, [the Required Validation native GUI job](https://github.com/MousaXD/swarmcraft/actions/runs/42408519678) printed: `PASS: installed Tauri/WebKitGTK + real bundled sidecars: initial state, GUI create, relaunch, identity/world persistence`.
- The same job intentionally disabled the installed `swarmcraft` CLI sidecar, reran normal acceptance, and observed a native WebDriver page-source diagnostic `startup=failed`, `status=Startup failed`, `error=true`, `retryVisible=true`; it then printed `PASS: missing installed sidecar produced a visible failed startup and failed the normal acceptance`. The application's deliberate failure is a **passing** fail-closed negative control.
- The real GUI job depends on the **same-run Linux desktop package**, extracts the Debian package, verifies all four sidecars, uses Xvfb + WebKitWebDriver + `tauri-driver`, and does not use the mocked browser `window.__TAURI__` fixture. Original Chrome/browser tests alone were insufficient.
- PR #89 Required Validation included Linux/Windows/macOS Rust, provider + excluded Desktop gates, root/Desktop RustSec, Fabric, process acceptance, fuzz, network impairment, specialists, version/governance and real GUI. In normal PR validation the optional network soak job was skipped; PR #89 Main Desktop Installers enabled and **passed** interrupted QUIC multi-GiB network soak. Installer publication was correctly **skipped** on a non-`main` pull request.
- [PR #89 native evidence artifact](https://github.com/MousaXD/swarmcraft/actions/runs/42408519678/artifacts/13588696843) names GitHub's checked-out synthetic merge SHA `ebb45a1dff1793507c99b15c7f4f8aa405aecfc4`. The workflow run is associated with source branch HEAD `ef411477b76e0f42cdddb4976373a7dee29a70e2`; **tested checkout is the synthetic PR merge commit**, not the branch HEAD byte-for-byte. This is an important provenance distinction, not a failure, provided the base and head remain unchanged.
- **Scope limit:** Linux installed-GUI acceptance is evidenced. Windows/macOS installers are built, not launched in native interactive installed-GUI sessions. CI cannot substitute for three independent networks with real Minecraft gameplay and failover.

## Future agent-specific independent gates

1. **Agent 01:** Confirm all eight #77 file-level behavioral gaps, including corrupted record, invalid symlink/nonregular path, promise reuse, world bootstrap and recovery safety; require red-before-green cases and exact-head required CI.
2. **Agent 02:** Inspect signed world policy bytes and migration/compatibility, unsigned local revocable opt-in, zero-allocation meaning, authority eligibility vs membership/replica roles; test old-world decode and negative permissions.
3. **Agent 03:** Verify opt-in limits across initial launch, recovery, wake, transfer, restart, crash and withdrawal. `-Xmx` is heap cap, **not** total RSS cap. No opportunistic hosting without consent; require explicit hard-cap semantics and bounded behavior when unavailable.
4. **Agent 04:** Compare desktop journeys against pre-migration `index.html`, `app.js`, `backend-adapter.js`, launcher/provider helpers and Rust IPC registry. Include RTL/keyboard/focus/error/offline states, startup negative control and genuine packaged GUI launch.
5. **Agent 05:** Require separate physical/virtual machines on distinct networks with independently installed GUI, real Minecraft clients/servers, signed world state, accepted state marker and quorum-backed failover/reconnect. Loopback-only tests are insufficient.
6. **Agent 06:** Review standalone RFC/prototype data, benchmark methodology, negative cases, cross-region semantics and failure isolation. No unsupported claim that standby memory pools into one Minecraft JVM or that active-active simulation is implemented.
7. **Agent 00:** Integrate only compatible, reviewed commits; rerun Required Validation **and** Main Desktop Installers on the exact combined candidate, then re-audit changed safety paths and run negative controls. No independent final READY without this evidence.

## Coordinator handoff

**Current recommendation:** HOLD all new integration/release certification. Treat PR #89 as automated-gate passing but **not independently safety-certified for a new public release** until Agent 01 reconciles the remaining known failure paths. Prioritize Agent 01, then protocol/runtime work, frontend parity, real three-machine certification and separate research. Keep historical PRs and issues open unless explicitly reviewed by Agent 00.

This report does **not** claim local Rust tests, native installed GUI, three-network gameplay, or signed release artifacts were executed by Agent 07. It records directly inspected source and GitHub-hosted evidence.

## Live update — newer Agent 01 patch inspection (2026-11-08)

The initial ledger above records PR #90's *first*, intentionally failing commit. Subsequent observation:

- PR #90 advanced to **draft HEAD** `e8092dd499ccaba1b2fc3470f93676c8ced1dac1`.
- Compared all **eight per-file unified patches** returned by GitHub for [PR #77](https://github.com/MousaXD/swarmcraft/pull/77) and [PR #90](https://github.com/MousaXD/swarmcraft/pull/90): each of the eight Rust file patch strings is **identical**; the only additional changed path in #90 is `docs/audits/agent-01-rust-safety-reconciliation.md`. This is a strong port-parity finding, not an integrated or runnable certification.
- Verified the expected **red-first** failure in [job 128686266472](https://github.com/MousaXD/swarmcraft/actions/runs/42466225204/job/128686266472): `corrupt_membership_prepare_blocks_authority_and_recovery_transitions` failed at line 141, asserting `ensure_no_membership_prepare(...).is_err()` on unmodified production code. This confirms the previously identified baseline defect with an executable regression, rather than only source inference.
- New exact-head [Required Validation 42466519745](https://github.com/MousaXD/swarmcraft/actions/runs/42466519745) was queued and [Main Desktop Installers 42466519780](https://github.com/MousaXD/swarmcraft/actions/runs/42466519780) was in progress when observed. Neither had a terminal-success verdict. **PR #90 remains BLOCKED pending successful final SHA checks, full negative paths and coordinator review.**
- Branches `agent-05/three-machine-real-e2e` and `agent-06/distributed-simulation-research` later appeared, but no submitted PRs or testable evidence had been reviewed. Their verdict remains **BLOCKED / NOT APPLICABLE**, respectively.

No merged integration candidate exists from this campaign yet. The baseline safety findings on `main` remain valid until the missing fixes are **merged** and their integrated commit revalidated. No partial read of #90 supersedes final exact-head CI.
