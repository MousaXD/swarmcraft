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


## Continued independent QA — Agents 01–06 submissions (2026-11-08)

This later snapshot supersedes the initial no-candidate ledger. Every PR below is against main at 45174310d1433116e64b9b6978cb4f2aa1c22c15. All statuses must be rechecked if agents push again. **No combined integrated candidate has been certified.**

| PR | Observed current head | Exact-head runs / evidence | Independent verdict |
| --- | --- | --- | --- |
| [#90 Rust safety](https://github.com/MousaXD/swarmcraft/pull/90) | e8092dd499ccaba1b2fc3470f93676c8ced1dac1 | [Required Validation 42466519745](https://github.com/MousaXD/swarmcraft/actions/runs/42466519745) FAILURE; [Installers 42466519780](https://github.com/MousaXD/swarmcraft/actions/runs/42466519780) FAILURE. Linux AppRun and Windows NSIS downloads HTTP 504, live player journey official managed Java HTTP 504; packaged Linux GUI was skipped. Rust, RustSec, governance and other jobs passed. No external failure waiver. | BLOCKED |
| [#92 Protocol](https://github.com/MousaXD/swarmcraft/pull/92) | 91774224146b2ce3628f7283ac40dca86fadc49d | [Required 42468029357](https://github.com/MousaXD/swarmcraft/actions/runs/42468029357), [Installers 42468029287](https://github.com/MousaXD/swarmcraft/actions/runs/42468029287), pending. Previous head a7bb45332b67ca5e31d8c92d5f94466eb8057470 had compiler E0506 in new negative tests (Windows and macOS Rust). Latest source replaces mutable borrowed variants, but not yet validated by CI. Inert V1 protocol policy cannot force old clients to enforce consent. | BLOCKED |
| [#93 Runtime](https://github.com/MousaXD/swarmcraft/pull/93) | c6c0fa4051d2c07a9ea25f8a07a0820f9286c802 | [Required 42468048050](https://github.com/MousaXD/swarmcraft/actions/runs/42468048050), [Installers 42468048090](https://github.com/MousaXD/swarmcraft/actions/runs/42468048090), pending. Earlier head 11cdf94c752b1194c5b80472d6366c5838a729d6 failed macOS Rust tests: the test machine had about 2970 MiB free but the attempted heap required 3840 MiB including headroom; production admission correctly refused. Latest source still calls the production consent function with 512 MiB and unwraps in the tests; deterministic non-production test memory fixtures are needed. No player-facing action to opt in, no daemon candidate enforcement or signed-policy migration. Merging alone risks disabling existing hosting. | BLOCKED |
| [#94 Frontend](https://github.com/MousaXD/swarmcraft/pull/94) | b6b156b032890456420f77a4c9ba0ac22c2ae867 | [Required 42468070171](https://github.com/MousaXD/swarmcraft/actions/runs/42468070171), [Installers 42468070234](https://github.com/MousaXD/swarmcraft/actions/runs/42468070234), pending. Ten Svelte components exist, but App.svelte still imports legacy app.js after mount. No committed transitive frontend lockfile, no Tauri production asset switch, no native installed Svelte GUI journey. All 171 IDs alone do not prove IPC/UX parity. | BLOCKED |
| [#95 Research](https://github.com/MousaXD/swarmcraft/pull/95) | 598c1a2899c415d520d7a0e7e5d9e8702c37dbb3 | [Required 42467114738](https://github.com/MousaXD/swarmcraft/actions/runs/42467114738) had unfinished jobs and failures when inspected. All six changed files research-only. Independently parsed saved JSON: each 5-sample median agrees with recorded median and checksums match across workers. Local synthetic speedup 2.65x spread, 1.13x clustered, 2.76x boundary. Excludes real Minecraft, networking, consensus, durability and mod compatibility. RFC NO-GO is appropriately supported *as a decision not to claim production feasibility*. | Research provisional; merge BLOCKED pending complete CI |
| [#96 Three-machine E2E](https://github.com/MousaXD/swarmcraft/pull/96) | 0c6b4084ec1decd526ad9e431f88518d3eca1233 | [Required 42467740105](https://github.com/MousaXD/swarmcraft/actions/runs/42467740105), [Installers 42467740143](https://github.com/MousaXD/swarmcraft/actions/runs/42467740143), [manual workflow 42467740114](https://github.com/MousaXD/swarmcraft/actions/runs/42467740114), pending. Earlier [manual workflow 42467712364](https://github.com/MousaXD/swarmcraft/actions/runs/42467712364) succeeded only at harness NEGATIVE assertions: three-remote preflight was SKIPPED. No real three-machine Minecraft gameplay, player reconnection or recovery evidence yet. | BLOCKED |

### Findings requiring follow-up

1. **QA-NEW-001 / protocol #92:** On prior head, Rust test code attempted to mutate borrowed members, yielding real E0506 compile errors on macOS and Windows in new hosting_contribution_policy.rs near lines 245 and 258. Latest branch source changes those test cases to distinct immutable values; re-run required checks before considering the issue closed. This is not an external HTTP failure. The policy is inert by design; caller-supplied activation and proof booleans are not genuine canonical consensus until a future verified migration.
2. **QA-NEW-002 / runtime #93:** Real Mac test failed host_readiness::tests::runtime_proof_detects_changed_artifacts_and_reconfiguration and hosting_budget::tests::opt_out_invalidates_an_active_admission_generation due to insufficient available memory. Preserve conservative real budget enforcement; make test memory injectable/deterministic. More importantly, the new zero-default consent restriction applies to migration/hosting paths while no user-accessible enablement exists in this PR. This causes a high-risk pre-integration functional blocker, independent of CI.
3. **QA-NEW-003 / frontend #94:** Staged Svelte is not the installed app. Production Tauri still uses the old src folder. Legacy build CI, ID parity, or unlaunched component source do not certify an actual installed Svelte UI. Require locked npm resolution, renderer/IPC parity, genuine installed WebKitGTK success + missing-sidecar negative control and real screenshots at 980x760 and 720x560.
4. **QA-NEW-004 / E2E #96:** The pass label of manual workflow applies to harness negative tests, not three independent native installs. Remote preflight is manual-only and skipped on PR. Independently check the human review booleans and attached evidence digests, verify three networks and actual Minecraft client state, and retain SHA identity of final integrated package. No field-ready verdict.
5. **QA-NEW-005 / research #95:** Stored single-host synthetic benchmark data internally consistent; no TPS/MSPT or WAN performance claims. Treat research NO-GO as a valid, conservative interpretation of current missing prerequisites, not proof horizontal simulation is impossible.
6. **QA-NEW-006 / PR #90:** Exact-head mandatory validation is red due independently observed 504 external asset downloads; Linux real installed-GUI was skipped. Revalidate exact current SHA under functioning verified external fetches without relaxing audits, native packaging, or release gates.
7. **QA-NEW-007 / combined work:** Agent 02 protocol consent format and Agent 03 persisted runtime consent differ and require a deliberate compatibility adapter; signed canonical migration, mandatory capability negotiation, daemon candidate checks, tested withdrawal, GUI budget selection and real E2E all remain integration dependencies. Never silently broaden quorum, permit authority on missing proof, or confuse -Xmx with total RSS limit.

**Independent final verdict for this campaign: BLOCKED.** Green subsets, successful framework scaffolding and synthetic benchmarks are not a completed integrated release. Agent 07 has not launched a local app or a real three-network gameplay session. The report remains draft until Agent 00 reviews.


## Late-moving HEAD addendum (2026-11-08)

While this QA review was underway, three agents pushed additional commits. Earlier SHA findings remain historical rather than automatic statements about the latest commit:

- **#93 newer head d86b8da3eb5fd95f96c4e5582f57d91c93743706**, [Required Validation 42468229475](https://github.com/MousaXD/swarmcraft/actions/runs/42468229475), [Installers 42468229340](https://github.com/MousaXD/swarmcraft/actions/runs/42468229340) not terminal success when checked. The opt-out test now directly constructs the admission lock to avoid requiring a second real available-memory check, but STILL calls production set_contribution(512) in setup, as does the host-readiness test; it may remain sensitive to a memory-starved runner. Preserve real-memory production rejection while making the tests deterministic. No frontend consent flow yet.
- **#94 newer head 828915811d1b2919483ede70c4ecbf284d464677**, [Required 42468372771](https://github.com/MousaXD/swarmcraft/actions/runs/42468372771), [Installers 42468372741](https://github.com/MousaXD/swarmcraft/actions/runs/42468372741) pending. Adds a new isolated diagnostic workflow and desktop-session facade/tests. The diagnostic workflow creates a dependency lockfile with npm install --package-lock-only during the run instead of using a committed one; it explicitly asserts production Tauri remains on the legacy frontend. This is NOT locked/reproducible shipping-Svelte acceptance or real installed GUI certification. The separate new workflow needs Agent 00 governance review before merge.
- **#96 newer head a4a1a74b0dd2abb97907604d6292d01328318d2d**, [Required 42468392393](https://github.com/MousaXD/swarmcraft/actions/runs/42468392393), [Installers 42468392402](https://github.com/MousaXD/swarmcraft/actions/runs/42468392402), [harness 42468392607](https://github.com/MousaXD/swarmcraft/actions/runs/42468392607) pending. Adds harder three-process crash expectation, extra packet-loss/reconnect review requirement and tests. Still has no real three-machine field evidence or accepted Minecraft player-journey proof.
- **#92 newer head 91774224146b2ce3628f7283ac40dca86fadc49d** remains unverified by terminal required workflow; code fixes the specific borrowed-record test pattern. Preserve the protocol migration blockade.

**No Agent 07 READY verdict on any of these new heads.** Re-audit every new exact SHA and its actual checked-out PR merge SHA after tests terminate, especially the combined candidate. No production code/workflow/issue/release mutations were made by Agent 07.


## Third independent audit snapshot — after latest new submissions (November 8, 2026)

The following supersedes earlier per-head conclusions where new commits have been pushed. All PRs still target unmodified main 45174310d1433116e64b9b6978cb4f2aa1c22c15. All statuses are observations, not guaranteed future outcomes.

### Moving-head ledger

| Agent / PR | Latest observed source head | Exact source-head CI evidence and result at review | Verdict |
| --- | --- | --- | --- |
| Agent 01 / #90 Rust safety | e8092dd499ccaba1b2fc3470f93676c8ced1dac1 | Required 42466519745 FAILED, Installers 42466519780 FAILED: externally returning 504 for official Java/Tauri assets; core Rust tests and audits passed, installed Linux GUI skipped | BLOCKED |
| Agent 02 / #92 hosting policy | a1e0473442d38e560c7d0844c6169516b6f2ccc8 | [Required 42468726219](https://github.com/MousaXD/swarmcraft/actions/runs/42468726219), [Installers 42468726202](https://github.com/MousaXD/swarmcraft/actions/runs/42468726202) queued during inspection | BLOCKED, not activated |
| Agent 03 / #93 runtime consent | 2efb842ae72a7e8f567c3e4ad761b1257eb9fd70 | [Required 42468786005](https://github.com/MousaXD/swarmcraft/actions/runs/42468786005) in progress with **actual macOS Rust test FAILURES** and **Ubuntu cargo fmt FAILURE**; [Installers 42468786007](https://github.com/MousaXD/swarmcraft/actions/runs/42468786007) not verified complete | BLOCKED — P0 regression evidence |
| Agent 04 / #94 Svelte | 2181ab566d3d1900c4ade7e3d9fc01bb47631951 | [Isolated Svelte workflow 42469433323](https://github.com/MousaXD/swarmcraft/actions/runs/42469433323) SUCCESS on exact branch head, confirms locked npm ci, 4 source tests, Svelte/TS checks, 22 Vitest tests, Vite build. [Required 42469436384](https://github.com/MousaXD/swarmcraft/actions/runs/42469436384) and [Installers 42469436426](https://github.com/MousaXD/swarmcraft/actions/runs/42469436426) queued at snapshot | BLOCKED — not installed native Svelte app |
| Agent 05 / #96 real-network harness | 979dae7a634eecae6927d3d6ef1bc45b2f8773ce | [Manual/nonrelease workflow 42468599857](https://github.com/MousaXD/swarmcraft/actions/runs/42468599857) SUCCESS only in 18 negative assertion tests; remote three-installation preflight SKIPPED. Other mandatory checks not yet complete | BLOCKED — no field evidence |
| Agent 06 / #95 research NO-GO | 598c1a2899c415d520d7a0e7e5d9e8702c37dbb3 | [Required 42467114738](https://github.com/MousaXD/swarmcraft/actions/runs/42467114738) at last view had pending/running jobs; not a recorded full-success run. Research JSON internally consistent with synthetic limits | PROVISIONAL research review only; merge gate incomplete |

### QA-NEW-008 — PR #93 consensus partition-safety failures (release-blocking)

On source HEAD 2efb842ae72a7e8f567c3e4ad761b1257eb9fd70, [macOS Rust job 128695058146](https://github.com/MousaXD/swarmcraft/actions/runs/42468786005/job/128695058146) failed three tests in unchanged crates/swarm-cli/tests/consensus_partition_safety.rs:
- five_peer_membership_churn_partition_cannot_activate_new_voter_universe — timed out waiting for **old committed 2-of-3 recovery quorum**
- five_daemon_unclean_quorum_loss_never_falls_back_to_solo — timed out waiting for **majority recovery permit**
- three_daemon_unclean_quorum_loss_never_falls_back_to_solo — timed out waiting for **majority recovery permit**

Source-level mechanism strongly suggests a newly introduced fixture/behavior mismatch: configure_host_capability in the old partition tests creates a runtime config and records verified runtime readiness but does not opt into local hosting. The new host_readiness::local_runtime_readiness checks hosting_budget::require_contribution and returns Unverified if consent is absent. Thus an otherwise previously eligible test successor may no longer advertise ready; do not declare this conclusively the only cause without reproduction/diagnostics. No test, quorum rule or negative assertion may be deleted, ignored, weakened or bypassed to get green. Fix setup to use explicit consent for intended legitimate successor peers, preserve zero-budget nonhost assertions, then revalidate expected quorum progress and negative partition protections.

Separately, [Ubuntu Rust job 128695057430](https://github.com/MousaXD/swarmcraft/actions/runs/42468786005/job/128695057430) failed cargo fmt --all -- --check, with rustfmt diffs in host_readiness.rs, hosting_budget.rs, migration.rs and tests. This is a genuine gate, not HTTP availability. The same workflow's managed Java live journey also failed external asset download. PR #93 remains BLOCKED independently of external outages.

### QA-NEW-009 — PR #94 candidate build proof is real but nonshipping

The newest Svelte branch includes an actual committed package-lock.json (earlier missing-lockfile concern is **resolved** for the candidate), a typed world-actions layer and staging session work. Isolated push workflow #42469433323 at exact head 2181ab566d3d1900c4ade7e3d9fc01bb47631951 actually ran npm ci from the committed lock, 4 source-parity tests, Svelte check, 22 Vitest tests across four files, and Vite build — all succeeded. But the same workflow asserts Tauri build.frontendDist remains ../src. This proves source/build hygiene, NOT real native GUI launching the Svelte bundle, legacy controller removal, end-to-end feature parity, or provider/transfer/mod/keyboard/offline acceptance. Do not count this as release or installed-GUI certification.

### QA-NEW-010 — PR #92 new challenge-bound claim security scope

Latest source adds a short-lived signed host-contribution claim binding world, peer/key, policy head, epoch/fencing token, snapshot hash, consent revision, heap, timestamp and an unpredictable challenge. verify_for_challenge checks shape, canonical-head expectation, expiration, signature, then requires a consuming challenge callback. This is promising isolated cryptographic contract design; its comments expressly state remote claims cannot prove local available RAM or replace consensus, and no V1 runtime can truthfully activate the policy. The external callback must be authenticated, atomic and single-use. End-to-end capability negotiation, accepted canonical policy history and daemon enforcement remain absent. Do not merge as a user-visible hosting feature.

### QA-NEW-011 — PR #96 harness success is not remotely executed gameplay

Latest manual/nonrelease workflow #42468599857 **successfully ran 18 fail-closed harness negative unit tests**. The remote preflight job was **skipped**, no real three-network installation/recovery has been provided, and the harness still requires independently reviewed signed/gameplay artifacts. This distinction must survive future PR comments, release notes and acceptance gates.

**Overall decision unchanged: BLOCKED.** A passing isolated framework/test harness or Rust subset cannot substitute for the mandatory integrated exact-head gate. Agent 07 has not performed real installed app/Minecraft field execution.


## Fourth snapshot — agent patches responding to QA findings (2026-11-08)

These agent commits supersede earlier SHA descriptions; they do NOT establish green certification:

| PR | New head at inspection | Change since prior observation | Workflow status |
| --- | --- | --- | --- |
| #92 protocol | 1cd9371990f6099a0b6fe4ec913a7d9a347fb372 | Added protocol claim tests/docs and negative coverage, still inert V1 foundation | [Required 42469961234](https://github.com/MousaXD/swarmcraft/actions/runs/42469961234), [Installers 42469961231](https://github.com/MousaXD/swarmcraft/actions/runs/42469961231): queued, no certified result |
| #93 runtime | ca6bc8ed8939f1d8c5276976f5cb104c2a9c650a | Added explicit consent in formerly failing consensus_partition_safety test fixture plus additional checks; this directly targets the probable cause of previous three recovery test timeouts. It changes tests, not the production default-no-consent semantics | [Required 42470107453](https://github.com/MousaXD/swarmcraft/actions/runs/42470107453), [Installers 42470107516](https://github.com/MousaXD/swarmcraft/actions/runs/42470107516): queued |
| #94 frontend | 81a9e7dda180ad2990f7ea8c067734bae1052c81 | Added separate candidate-only **real native WebKitGTK** workflow and temporary Tauri frontendDist overlay. Also adds typed world-state logic and tests. Production Tauri config stays untouched. | [Required 42470245443](https://github.com/MousaXD/swarmcraft/actions/runs/42470245443), [Installers 42470245486](https://github.com/MousaXD/swarmcraft/actions/runs/42470245486): queued/pending; candidate-native workflow not yet observed as successful |
| #96 E2E harness | 1620843a04260d9b905b603412602b40fba391ce | Adds evidence.py with bounded media magic, SHA256, unique per-role/stage evidence and manifest digest checks; still requires independent human review, does not prove photos represent real gameplay or provide remote machine attestation | [Required 42470216483](https://github.com/MousaXD/swarmcraft/actions/runs/42470216483) queued; [manual harness 42470216442](https://github.com/MousaXD/swarmcraft/actions/runs/42470216442) queued; [Installers 42470216616](https://github.com/MousaXD/swarmcraft/actions/runs/42470216616) cancelled at observation |

### QA-NEW-012 — Svelte candidate native GUI validation plan

Agent 04's new workflow .github/workflows/svelte-candidate-native-gui.yml explicitly:
- checks shipping Tauri config still points to ../src and that its candidate config overlay changes *only* frontendDist to ../frontend/dist;
- runs locked npm ci, Svelte check, parity tests, 22+ unit tests and Vite build;
- rebuilds the exact-branch Rust sidecars and a Debian installer with the overlay;
- extracts .deb to an isolated location (important: extraction and execution of packaged binaries is **not** a system-package-manager installation via dpkg -i);
- launches the extracted packaged desktop binary under Xvfb with real WebKitWebDriver/tauri-driver to perform first launch, create, relaunch and persistence; then deliberately removes the CLI sidecar and requires a real GUI visible failure/retry observation;
- uploads exact-commit candidate package hash, logs, WebDriver acceptance and bundle-size evidence.

If this workflow completes successfully on the exact final SHA and its extracted Debian asset is inspected, it would be meaningful **Svelte packaged GUI launch** evidence rather than only legacy/browser source tests. At present it is **unproven**: no completed successful run was observed. It does not replace the eventual merged production Tauri config, cross-OS installer gating, 980x760/720x560 keyboard/accessibility review, all original features, real Minecraft player E2E, or combined release CI. Agent 00 must review this new workflow before integration.

### QA-NEW-013 — Recovery test corrective patch observed, not yet proved

Agent 03 now calls swarm_cli::hosting_budget::set_contribution(&peer.paths, world, 512) in consensus_partition_safety::configure_host_capability. This matches the new local permission contract better than the earlier fixture. The three historical recovery test failures remain real evidence for the **earlier SHA** and cannot be marked resolved until this newer SHA's exact-head Rust and consensus tests pass. Keep explicit revoked and 0-MiB negative scenarios; do not re-enable old unsafe fallback. Shipping still lacks user-facing consent and canonical signed policy activation.

### QA-NEW-014 — Evidence integrity is not player authenticity

Agent 05 evidence.py requires expected player/stage media slots, distinct hashes, size limits and magic bytes, plus a stable digest of the reviewed manifest. This improves file integrity against substitutions, reused images and missing slots. It cannot prove a specific image/video was recorded from a real installed Tauri/Minecraft client, a different ISP, that a server recovered, or that the reviewer is independent. Signed native state, transport traces, real in-world marker and human observation remain separate gates.

**Independent verdict remains BLOCKED for all current engineering integration; research-only NO-GO may be accepted separately after review and required CI.** Any later PR push invalidates earlier tested-SHA certification.
