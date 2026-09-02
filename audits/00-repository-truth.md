# Auditor 0 — Repository Truth / Git Archaeology

Repository: `MousaXD/swarmcraft`

Audit branch: `audit/repository-truth`

Observed: 2026-09-02

## 1. Audited baseline SHA

Authoritative baseline: `main`

Expected SHA: `354be3b1066428ecab6987590b7c7dbd80fe0870`

Live remote SHA at audit start: `354be3b1066428ecab6987590b7c7dbd80fe0870`

Baseline gate: **PASS**. The live remote exactly matched the required SHA, so this audit proceeded on the mandated tree.

The audited `main` head is:

- SHA: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Commit: `fix(acceptance): configure desktop IPC argument threshold`
- Parent: `534fabacf75409ade979e751c3e8f6186b90f2e7`

Important repository truth established by this audit:

- `main` and `integration/player-launcher-v1` are **identical**, both at `354be3b1066428ecab6987590b7c7dbd80fe0870`.
- Exact-head product validation exists and is green for that SHA: CI runs `891` and `892`, Network Soak `129`, Release version guard `555` and `556`, Player journey live acceptance `113`, and Agent 1 Catalog Validation `64` all completed successfully.
- No live non-release branch contains meaningful functional code that is absent from `main`.
- The only live specialist-branch commit not reachable from `main` is a known formatting-only Agent 1 tail commit.
- `release/0.5.0` is intentionally ahead of `main` with release/version metadata only. Its exact-head CI was still running when this report was frozen.

## Executive repository-truth verdict

The product ancestry is substantially coherent: the accepted player-launcher integration has been promoted to `main`, and the specialist implementation branches are already contained in that product history.

The repository metadata is **not** fully truthful as a whole, however. The rolling public `main-latest` tag/release is stale by 237 commits, and several progress ledgers still state that final validation is running and that `main` is untouched even though current Git history proves otherwise. Those mismatches create real release and coordination risk.

## 2. Branch map

Relationship counts below are from GitHub compare results against audited `main` at `354be3b1066428ecab6987590b7c7dbd80fe0870`.

| Branch | Head SHA | Relationship to `main` | Unique commits / classification | Recommended disposition |
| --- | --- | --- | --- | --- |
| `main` | `354be3b1066428ecab6987590b7c7dbd80fe0870` | Audited baseline | None | **KEEP** |
| `audit/repository-truth` | created from `354be3b1066428ecab6987590b7c7dbd80fe0870` | Exact baseline before this report commit | Audit report only after creation | **KEEP** |
| `integration/player-launcher-v1` | `354be3b1066428ecab6987590b7c7dbd80fe0870` | **IDENTICAL**, 0 ahead / 0 behind | None | **ARCHIVE** |
| `release/0.5.0` | `8b85e5a5fe52ed6d906f5c3e8ad3f9bc6db528d5` | 8 ahead / 0 behind | 8 release-preparation commits; final-tree delta is version metadata, SwarmCraft lockfile entries, and release guard only | **KEEP** |
| `agent/minecraft-fabric-catalog` | `a581195bfff3fd3a050e1978910fe77288237cbc` | Diverged, 1 ahead / 112 behind; merge base `b7128c83b83208d7c1d8a82df915766fc7abb3ec` | One unique commit: `a581195...`, `style(agent1): satisfy Desktop rustfmt`; **formatting-only** | **ARCHIVE** |
| `agent/canonical-modpack` | `75351794926e1ba183e5615f948a53c7017084bf` | 0 ahead / 94 behind; head is ancestor | None | **SAFE TO DELETE LATER** |
| `agent/curseforge-provider` | `2ec9005591de71fffe8e504607a4ffb3145ff9c8` | 0 ahead / 204 behind; head is ancestor | None | **SAFE TO DELETE LATER** |
| `agent/discovery` | `0a72380aebbc6f227957cae733de64dc6f85638c` | 0 ahead / 199 behind; head is ancestor | None | **SAFE TO DELETE LATER** |
| `agent/modrinth-provider` | `355d1f0762fe04391643eabcb802bc4641b1b0a8` | 0 ahead / 190 behind; head is ancestor | None | **SAFE TO DELETE LATER** |
| `agent/automatic-invites` | `e13a4fd57e3c26121275db0b1628808e2e036a44` | 0 ahead / 214 behind; head is ancestor | None | **SAFE TO DELETE LATER** |
| `backup/local-work-20260824` | `41c9b5b650aac1e320195f6e1855945f2722abc4` | 0 ahead / 235 behind; head is ancestor | None | **ARCHIVE** |
| `ci/agent3-final-validation-v2` | `699e5b038df91d3d1ad46c8afb19016d31004292` | 0 ahead / 206 behind; head is ancestor | None; historical **CI-only** validation pointer | **SAFE TO DELETE LATER** |
| `ci/discovery-fixture-trigger` | `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c` | 0 ahead / 202 behind; head is ancestor | None; historical **CI-only** trigger | **SAFE TO DELETE LATER** |
| `integration/runtime-player-journey` | `ddc1667eccf871b64e4089992d43f2bbd4a6392f` | 0 ahead / 176 behind; head is ancestor | None | **ARCHIVE** |
| `integration/swarmcraft-v1` | `ddc1667eccf871b64e4089992d43f2bbd4a6392f` | 0 ahead / 176 behind; head is ancestor | None | **ARCHIVE** |

### Branch conclusions

**INFO — Final integration really is on `main`.**

`integration/player-launcher-v1` and `main` resolve to the same exact SHA. Statements that the recovery effort left `main` untouched are no longer true as descriptions of the live repository.

**INFO — No hidden functional implementation remains on specialist branches.**

Agents 2 through 6 are exact ancestors of `main`. Agent 1 has only one non-main tail commit, and direct commit inspection shows it is Rustfmt-only. Its touched Desktop Rust files contain formatting/reflow changes rather than behavior changes. The integrated tree later received its own formatting commits, consistent with the ledger explanation that this tail was deliberately superseded rather than omitted.

**INFO — The release branch is the only intentional non-audit source line ahead of `main`.**

`release/0.5.0` is 8 commits ahead and 0 behind. The final tree differs from `main` only in:

- `Cargo.toml`
- `Cargo.lock`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/Cargo.lock`
- `apps/desktop/src-tauri/tauri.conf.json`
- `minecraft/fabric/gradle.properties`
- `scripts/check_release_version.py`

The workspace package version is `0.4.0` on audited `main` and `0.5.0` on the release head. No protocol/runtime/application source file appears in the final compare delta. This supports the PR's claim that the branch is a release-metadata promotion rather than a new feature line.

At report freeze, exact release-head evidence was:

- Release version guard `558`: **SUCCESS**
- Player journey live acceptance `115`: **SUCCESS**
- CI `894`: **IN PROGRESS**

Therefore `release/0.5.0` and PR #59 must remain active until exact-head CI finishes successfully. This audit does not treat the pending release as already shipped.

## 3. PR map

### Active release PR

| PR | State | Head -> Base | Repository-truth assessment | Recommendation |
| --- | --- | --- | --- | --- |
| #59 `Release SwarmCraft 0.5.0` | OPEN, non-draft | `release/0.5.0` `8b85e5a...` -> `main` `354be3b...` | Legitimate active release candidate. Final-tree delta is metadata/lockfile/release guard. Two exact-head gates green; CI `894` still running at audit freeze. | **KEEP OPEN. DO NOT PROMOTE UNTIL CI IS GREEN.** |

### Open validation-only / historical PRs

The following open PRs are validation vehicles or historical workbench PRs, not product work that still needs to be merged into `main`:

| PR | Purpose | Current truth | Recommendation |
| --- | --- | --- | --- |
| #44 | Agent 2 Modrinth validation | Head `355d1f...` is already an ancestor of `main` | Close after audit evidence is preserved; **do not merge** |
| #45 | Agent 6 discovery CI workbench | Head `0a7238...` is already an ancestor of `main` | Close; **do not merge** |
| #46 | Agent 5 automatic invite validation | Head `e13a4f...` is already an ancestor of `main` | Close; **do not merge** |
| #47 | Agent 1 catalog validation | Functional work is already in `main`; live branch has only the known format-only tail | Close after preserving validation evidence; **do not merge** |
| #48 | Agent 3 CurseForge final validation | Head `2ec900...` is already an ancestor of `main` | Close; **do not merge** |
| #52 | Integrated Agents 5/6 validation | Head `ddc166...` is already an ancestor of `main` | Close; **do not merge** |
| #53 | Agent 3 exact-head CI pointer | Head `699e5b...` is already an ancestor of `main` | Close; **do not merge** |
| #55 | Agent 4 canonical modpack validation | Head `753517...` is already an ancestor of `main` | Close; **do not merge** |
| #57 | Final player-launcher acceptance vehicle | Body records final acceptance passed on `354be3...`; head now equals `main` | Preserve as historical evidence, then close; **do not merge** |
| #58 | Catalog/Desktop acceptance vehicle | Body records final acceptance passed on `354be3...`; head now equals `main` | Preserve as historical evidence, then close; **do not merge** |

Keeping these PRs open adds noise to the live work queue and leaves multiple stale branches visibly mergeable even though their intended role was evidence generation only.

### Recently closed / merged integration PRs

- #56 merged canonical/provider work into `integration/player-launcher-v1`. This is legitimate integration history and its source head is now contained in `main`.
- #54 merged Agent 1 catalog work into Agent 4's canonical-modpack line. This is legitimate integration history.
- #51 merged Agent 6 discovery into `integration/swarmcraft-v1`.
- #50 merged Agent 5 automatic invites into `integration/swarmcraft-v1`.
- #49 was a CI-only discovery trigger and closed without merge. Its branch is now a strict ancestor of `main` and has no unique commits.
- #43 was an older Agent 3 validation PR and was closed without merge, superseded by later exact-head validation.

### PR #42 reachability nuance

PR #42 was explicitly described as a validation-only backup PR that should not be merged to `main` without a later decision. GitHub now reports it as closed/merged on 2026-09-02, while its current head `41c9b5...` is simply an ancestor of present `main` and the normalized PR snapshot does not expose a merge commit SHA.

Treat #42 as **historical validation evidence**, not proof that its old branch should be separately promoted now. Its commits becoming reachable from later `main` history is enough for GitHub to consider the PR merged/reachable; the current evidence does not establish the exact ref-update mechanism used for final promotion.

### Prior release/integration context

PR #36 was a real 0.4.0 main release candidate and was merged in August. It already disproves any broad timeless claim that `main` had never consumed integration work. The newer player-launcher recovery ledgers were narrower claims about that later recovery effort, but those claims are now stale because current `main` equals the final player-launcher integration head.

## 4. Release / tag map

### Live tag refs

Only one live tag ref was observed:

| Tag | SHA | Relationship to current `main` | Assessment |
| --- | --- | --- | --- |
| `main-latest` | `105b19ade82be606e5a855df4e82ce18bb7e885a` | 0 ahead / **237 behind** current `main` | Stale rolling tag |

No live `v0.4.0` or `v0.5.0` tag ref was returned by the live tag-ref collection during this audit.

### Published release state

The observed GitHub release collection exposes `main-latest` as a prerelease named `SwarmCraft Main Snapshot`, targeting `105b19ade82be606e5a855df4e82ce18bb7e885a`. Its downloadable assets are version `0.4.0`, including Fabric, Linux, Windows, and macOS artifacts and checksums.

This release is **not** an accurate representation of the live `main` branch on 2026-09-02. The tag is 237 commits behind the audited main head and predates the final player-launcher integration that is now on `main`.

### `release/0.5.0`

The live release branch accurately appears to be a metadata-only release candidate layered on the accepted product SHA:

- base: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- head: `8b85e5a5fe52ed6d906f5c3e8ad3f9bc6db528d5`
- ahead: 8 commits
- behind: 0 commits
- workspace version on base: `0.4.0`
- workspace version on release head: `0.5.0`
- final tree changes: package/application version metadata, lockfile version refreshes, and release-version guard only
- protocol source is not in the compare delta

The candidate accurately represents the accepted product tree plus release metadata, but it is **not yet a completed release** because exact-head CI `894` was still in progress when this audit report was committed.

## 5. Stale-ledger findings

### RT-001 — MEDIUM — Final integration ledgers contradict live Git truth

Affected files:

- `progress/README.md`
- `progress/agent8.md`
- `progress/integration-unfinished.md`

Live GitHub truth:

- `main` = `integration/player-launcher-v1` = `354be3b1066428ecab6987590b7c7dbd80fe0870`
- exact-head final validation has completed successfully on that SHA
- PR #57 explicitly records final acceptance passed and says promotion to `main` was authorized
- PR #58 explicitly records final acceptance passed and says promotion to `main` was authorized

Stale ledger statements include:

- `main` has not been merged into or rewritten by this recovery effort
- Agent 8 status is `FINAL CANDIDATE — EXACT-HEAD VALIDATION RUNNING`
- the integration-unfinished file still lists the final exact-head acceptance gates as remaining

These statements appear to have been valid when authored on 2026-08-29, but they no longer describe the live repository on 2026-09-02.

Impact:

Future agents can incorrectly conclude that the product has not been promoted, that final acceptance is still pending, or that integration work must be repeated. This is coordination and release-state risk, not evidence that the product code is missing.

Recommended remediation:

Update the final progress ledger to record:

- exact promoted `main` SHA `354be3b1066428ecab6987590b7c7dbd80fe0870`
- exact successful workflow IDs/runs
- `integration/player-launcher-v1` now being identical to `main`
- PR #57/#58 status as historical validation-only evidence
- 0.5.0 release candidate as a separate, currently pending release step

### Accurate specialist ledgers

The stale final-state documents should not cause the whole progress directory to be discarded. The specialist ledger entries are materially consistent with Git ancestry:

- Agent 1 accurately records the format-only non-ancestor tail.
- Agent 2 accurately records `355d1f...` as integrated.
- Agent 3 accurately records `2ec900...` as integrated.
- Agent 4 accurately records `753517...` as integrated.
- Agent 5 accurately records `e13a4f...` as integrated.
- Agent 6 accurately records `0a7238...` as integrated.
- Agent 7 accurately records that no live Agent 7 branch exists and that the missing player journey was recovered directly on the final integration line.

The defect is primarily **freshness of the final promotion/acceptance state**, not wholesale fabrication of the historical specialist handoffs.

## 6. Unique-unmerged-work findings

### RT-002 — INFO — No meaningful functional code is stranded off `main`

Every live specialist, backup, CI-only, and older integration branch except Agent 1 is either identical to or a strict ancestor of current `main`.

Agent 1's sole unique commit is:

- `a581195bfff3fd3a050e1978910fe77288237cbc`
- `style(agent1): satisfy Desktop rustfmt`
- authored by `github-actions[bot]`
- parent/merge base: `b7128c83b83208d7c1d8a82df915766fc7abb3ec`
- touched only four Desktop Rust files
- diff inspection shows line wrapping/reflow/trailing-newline formatting rather than catalog/runtime behavior

Classification: **FORMATTING-ONLY**.

Recommendation: do not cherry-pick it into `main`. Archive the Agent 1 branch until its validation PR is closed, then it is safe to delete later.

### RT-003 — INFO — `release/0.5.0` contains intentionally unmerged release metadata

This is the only live non-audit line with commits genuinely ahead of main. Its final-tree changes are release metadata and lockfile/version-guard updates, not a separate product implementation.

Classification: **RELEASE-METADATA / LOCKFILE-ONLY**.

Recommendation: keep until PR #59 completes exact-head CI and the release decision is made.

## 7. Repository cleanup recommendations

### RT-004 — MEDIUM — `main-latest` does not mean current `main`

The public rolling tag/release `main-latest` points to `105b19ade82be606e5a855df4e82ce18bb7e885a`, which is 237 commits behind current `main`. Its published assets are still SwarmCraft `0.4.0` artifacts.

This is more than cosmetic branch clutter. A user or developer selecting the artifact labeled `main-latest` can receive a substantially older product than the repository's actual main branch.

Recommended remediation:

1. Determine why the rolling release was not regenerated after the final main promotion.
2. Re-run the normal main-snapshot release path on the current `main` SHA, including rebuilding and checksumming all artifacts. Do not merely move the tag while leaving old assets behind.
3. If a rolling snapshot is no longer intended to track every accepted main head, rename it to remove the misleading `latest` claim and document its semantics.

### RT-005 — LOW — Obsolete validation-only PRs remain open

PRs #44, #45, #46, #47, #48, #52, #53, #55, #57, and #58 are still open even though their intended source work is already present in `main` or superseded by the accepted final integration.

Recommended remediation:

- preserve their workflow links/descriptions as historical evidence
- close them without merge
- then delete CI-only/specialist branches that are no longer referenced by active work
- retain `integration/player-launcher-v1` as an archive until the audit cohort and 0.5.0 release are complete, then reconsider deletion

### RT-006 — MEDIUM — `main` has no enforced branch protection in the observed branch metadata

The live `main` branch reports `protected: false` and no required status checks at the branch-protection endpoint exposed with the branch object.

Current product acceptance is still well evidenced because the exact accepted SHA has successful workflows. The risk is prospective: future ref updates can bypass the same gates unless enforcement exists elsewhere.

Recommended remediation:

Require an explicit branch/ruleset policy for `main` that enforces the project's actual release gates, or document and mechanically constrain the exceptional direct-promotion path if direct fast-forward promotion is intentional.

### Suggested branch cleanup order

1. **KEEP** `main`.
2. **KEEP** `release/0.5.0` and PR #59 while CI `894` is unresolved.
3. **KEEP** all `audit/*` branches for the current audit cohort.
4. **ARCHIVE** `integration/player-launcher-v1`, `integration/runtime-player-journey`, `integration/swarmcraft-v1`, `backup/local-work-20260824`, and Agent 1 until their referenced validation history is closed/preserved.
5. Close obsolete validation PRs without merge.
6. Delete later: `ci/agent3-final-validation-v2`, `ci/discovery-fixture-trigger`, Agents 2–6 branches, and other strict-ancestor work branches once no open PR/audit references them.
7. Reassess archived integration/backup branches after the final audit and 0.5.0 release are complete.

## 8. Unresolved questions

### UQ-001 — Exact mechanism of final `main` promotion

Git truth proves that `main` now points at the exact final integration head. PR #57 and #58 were explicitly validation-only and remain open, so they were not the promotion mechanism.

The public repository/PR data inspected here does not expose the precise ref-mutation audit trail that changed `main` to `354be3b1066428ecab6987590b7c7dbd80fe0870`.

Possibilities include an authorized fast-forward/direct ref update or another promotion path not represented by #57/#58. This uncertainty does **not** change the ancestry result, but it should be documented if repository governance depends on proving how production refs move.

### UQ-002 — Why `main-latest` stopped at `105b19a...`

The rolling snapshot tag/release did not advance with current `main`. Determine whether:

- the final promotion path bypassed a workflow trigger,
- the release workflow failed or was intentionally disabled,
- or `main-latest` is no longer intended to be a true rolling main snapshot.

Until resolved, its name is misleading.

### UQ-003 — 0.5.0 release completion

At the moment this report was frozen, release-head CI run `894` remained `IN PROGRESS` even though release guard `558` and live acceptance `115` were green. Auditor 0 therefore cannot classify `8b85e5a5...` as a completed, fully validated release head yet.

## 9. Final verdict

### Severity summary

| ID | Severity | Finding |
| --- | --- | --- |
| RT-001 | MEDIUM | Final progress ledgers are stale and contradict live promotion/acceptance truth |
| RT-002 | INFO | No meaningful functional source is stranded off `main` |
| RT-003 | INFO | `release/0.5.0` is intentionally ahead with release metadata only |
| RT-004 | MEDIUM | Public `main-latest` tag/release is 237 commits behind current `main` |
| RT-005 | LOW | Many obsolete validation-only PRs remain open |
| RT-006 | MEDIUM | Observed `main` branch has no enforced protection / required checks |

The repository's **product code ancestry passes reconciliation**: the accepted final player-launcher tree is on `main`, specialist work is contained, and no hidden functional branch needs to be rescued.

The repository's **truth surfaces do not pass as currently presented**: the published rolling release and authoritative-looking progress ledgers materially misstate the live state, while `main` lacks observed enforcement of the validation policy that was manually followed for this accepted head.

**VERDICT: FAIL**
