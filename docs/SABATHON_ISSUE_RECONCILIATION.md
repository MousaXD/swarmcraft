# Issue and PR reconciliation — Sabathon #49

**Observed 25 November 2026.** Remote `main` is
`aaffac22e32547983f1fd558af6eb32c0da4917b`; inspected integration is
`6fd4e0a92386c54ce5171b4dfa23033cb50d5598` (159 commits ahead, none
behind). The former is the released source baseline; the latter is an
unreleased development branch. No issue, PR, branch, tag, release or
hosted workflow was changed during the audit. Do not close tickets based
on integration-only work.

[Machine-readable evidence](../research/issue-reconciliation/evidence/ledger-20261125.json)
retains all nine open issues, all 17 PR heads and changed-path comparisons,
historical job URLs and statuses, both dependency lockfile versions and
release asset metadata. Same Git tree entry means the path's mode, type
and object agree; ancestry proves incorporation of a head, not that every
line remains unchanged. Differing blobs need review, not a blind merge.

## Issues — closure recommendations and residuals

| Open issue | Landed or integrated evidence | Exact residual / disposition |
| --- | --- | --- |
| [#84](https://github.com/MousaXD/swarmcraft/issues/84) voluntary hosting | Main: `6fddb5c` genuine native IPC, `3ceef9f` hosting budgets, `3168ad3` consent rechecks. Integration: `3a18b31` inert V2 review, `4de797c` Linux cross-root admission, `28401af` optional delegated cgroup memory charges, `8fad57b` fenced unconfirmed shutdown. | **Keep open.** #19–23: real installed direct/recovery/transfer/wake opt-in and withdrawal; optional Linux cgroup is not a universal process hard cap; durable V2 freeze/CAS, distinct transport/live policy remain unimplemented. V1 local heap/consent does not grant authority. |
| [#85](https://github.com/MousaXD/swarmcraft/issues/85) three installations | Integration `0442c40`, `ca83afc`, `226c475`, `82df85d` wire authenticated gateway, automated handoff and real hosted acceptance constraints. [Live status](SABATHON_P0_LIVE_INTEGRATION_STATUS.md). | **Keep open.** #1/3/4/6/12/13/15: three independent installations/networks, actual licensed client login and shared marker/inventory, safe transfer/crash/return and quorum sleep/wake, verified transport, independent review. Local daemons/fixtures and a server launch are not this evidence. |
| [#97](https://github.com/MousaXD/swarmcraft/issues/97) upstream 504 | Main `db9673f` verified pinned tools and [artifact acquisition](../scripts/ci-artifacts/README.md); exact main installer run finished successfully. | **Keep open.** #51: original Java/AppImage/NSIS outage/cache recovery matrix including bounded retry, checksum corruption, cold/warm cache and shared-runtime publication; current preview has no AppImage asset. Distinguish locally hashed generated launcher from publisher-verified bytes. No outage acceptance inferred from one successful run. |
| [#120](https://github.com/MousaXD/swarmcraft/issues/120) withdrawal negatives | Main `5bfb05f`, `bc6113d`, `a266378`, `bff5b93`, `f6d27c1`, `ecd5345` enforce evidence and document procedure. Current harness 112 tests pass. | **Keep open as field blocker.** #13/19/20: observed genuine installed consent withdrawal, checkpoint/fence/exit, denial of subsequent hosting, Carol's zero-budget attempted host refusal with real client still playing, bound/reviewed media. Verifier tests do not substitute. |
| [#135](https://github.com/MousaXD/swarmcraft/issues/135) Quinn RustSec | `0f84a67` on main; both main and integration root + Desktop lockfiles contain only quinn-proto 0.11.19 and quinn-udp 0.5.16. Exact main [root](https://github.com/MousaXD/swarmcraft/actions/runs/45080577660/job/137387602060) and [Desktop](https://github.com/MousaXD/swarmcraft/actions/runs/45080577660/job/137387602116) audit jobs passed. | **Original three fixed-version findings landed; maintainer closure candidate.** Preserve independent security review and fresh advisory DB audits on final SHA; historical success does not establish absence of later vulnerabilities or warnings. |
| [#137](https://github.com/MousaXD/swarmcraft/issues/137) duplicate Quinn | Same three advisory IDs and same fix/evidence as #135. | **Duplicate closure candidate** with canonical #135 link; no remote change. |
| [#138](https://github.com/MousaXD/swarmcraft/issues/138) Linux WebKit and managed Java | Main `aca14d8` renamed driver handling, `d918ebe` isolated managed Java; exact main [GUI](https://github.com/MousaXD/swarmcraft/actions/runs/45080577660/job/137395088509) and [live Minecraft](https://github.com/MousaXD/swarmcraft/actions/runs/45080577660/job/137387602024) passed. | **Original baseline failure closure candidate.** The integration branch now runs local-first; retain real WebKit and managed-Java logs for its eventual exact release SHA. Do not equate earlier green CI with current branch acceptance. |
| [#140](https://github.com/MousaXD/swarmcraft/issues/140) preview safety | `52d138a` on main. Main installer workflow [45080577651](https://github.com/MousaXD/swarmcraft/actions/runs/45080577651) succeeded. Observed published [pinned 0.5.1](https://github.com/MousaXD/swarmcraft/releases/tag/main-snapshot-aaffac22e32547983f1fd558af6eb32c0da4917b) with 11 uploaded assets and preserved [legacy 0.5.0](https://github.com/MousaXD/swarmcraft/releases/tag/main-latest) with ten. Exact pinned tag target equals main. | **Keep open through #50.** Independently download/check every asset and manifest, test previous package retrieval and rollback against isolated state, preserve old releases, validate tag/version and production signing separately. API digests are metadata, not local byte verification. |
| [#141](https://github.com/MousaXD/swarmcraft/issues/141) native platform QA | `7af383f` integration implements platform-specific harness; `695bdc5` scoped Linux idle frontend rollback; several later Linux package journeys. | **Keep open.** #37/38: actual installed Windows WebView2, Apple Silicon/Intel WKWebView, real IPC, restart, consent, signed join, negative separate build/missing sidecar, no QA hooks in shipping package. Mac Accessibility helper not qualified on Linux. |

No stale report is closed here. The older storage issue #102 is no longer
open in the retrieved list; its remaining independent RED-only PRs are
reconciled separately below. Do not reopen or close tickets automatically.

## Pull requests — preserve unique work

Exact heads and all same/different paths are in the evidence JSON.
The counts below compare the changed-path set to main / integration
respectively; they are not counts of commits to cherry-pick.

| PR | Main / integration changed paths identical | Disposition with source or exact residual |
| --- | --- | --- |
| [#142](https://github.com/MousaXD/swarmcraft/pull/142) | 5/8 · 5/8 | Exact head `52d138a` is an ancestor of both. Later workflow/policy/docs supersede original trigger design. Close stale PR only after maintainer review; #140/#50 byte/rollback qualification stays separate. |
| [#110](https://github.com/MousaXD/swarmcraft/pull/110) | 0/2 · 0/2 | Exact head `6fddb5c` is an ancestor of both. Real registered commands retained in evolved native main/hosting files. No redundant merge. #84 live consent residual remains. |
| [#92](https://github.com/MousaXD/swarmcraft/pull/92) | 6/6 · 3/6 | All changed paths match main. Integration's realm/protocol/tests superseded by `3a18b31`; no live V2. |
| [#99](https://github.com/MousaXD/swarmcraft/pull/99) | 4/4 · 4/4 | Every changed path exactly matches both; main `6d1f145` explicit CLI test. Keep genuine real Minecraft acceptance distinct from field. |
| [#101](https://github.com/MousaXD/swarmcraft/pull/101) | 5/5 · 1/5 | Every changed path matches main. Integration evolves inert verifier/network interfaces in `3a18b31`, not an activation. |
| [#114](https://github.com/MousaXD/swarmcraft/pull/114) | 1/1 · 0/1 | Exact main blob matches. Integration retains bounded measured inner-viewport calibration in `desktop-browser-webdriver.test.mjs` and extends scenarios; #48 both variants passed all three window sizes. No redundant merge. |
| [#111](https://github.com/MousaXD/swarmcraft/pull/111) | 11/12 · 4/12 | Main V2 foundation files match except CI. Integration supersedes crypto/replay sources via `3a18b31`; original hosted workflow delta intentionally replaced by `32441e6` local-first policy. Keep V2 inactive. |
| [#93](https://github.com/MousaXD/swarmcraft/pull/93) | 9/13 · 0/13 | Main `3ceef9f` and `3168ad3` incorporate budget and consent checks; integration `4de797c`, `28401af`, `8fad57b` supersede process/memory/withdrawal behavior. Old test helper `initialize_two_peer_world` evolved; do not restore obsolete two-peer crash-failover assumptions. #84 field and V2 policy remain. |
| [#100](https://github.com/MousaXD/swarmcraft/pull/100) | 0/3 · 0/3 | Staged-only contract document superseded by actual main `6fddb5c` typed native commands; validation names retained. No production test-double wiring. Remaining independent installed platform/withdrawal tests under #84/#141. |
| [#96](https://github.com/MousaXD/swarmcraft/pull/96) | 6/14 · 5/14 | Main `badcbc6` field harness, later #120 commits; integration retained 112 passing harness tests and actual gateway runbook updates. Legacy hosted workflow intentionally replaced by explicit local-first preflight. **Real positive three-network field run remains unexecuted**. |
| [#77](https://github.com/MousaXD/swarmcraft/pull/77) | 4/8 · 0/8 | Source reconciliation documented in [historical audit](audits/agent-01-rust-safety-reconciliation.md); main `682c78f` canonical fail-closed roots and `6e4f64f` combined candidate. Current binary test corrupt membership prepare passed; current retention/certificate tests pass. Eight source blobs have evolved; obsolete helper names `dial_pending_invite_bootstraps` and `ensure_recovery_artifacts` are absent from the current daemon after certified replay changes. Do not restore old replay logic. Residual privileged concurrent local filesystem TOCTOU and full current-source independent security review. |
| [#119](https://github.com/MousaXD/swarmcraft/pull/119) | 0/1 · 0/1 | RED-only tests superseded by `4f4b6fc`, `db6c373` and evolved retention tests; 11 current retention tests pass. Do not merge RED-only source. Preserve long churn and hostile filesystem review. |
| [#124](https://github.com/MousaXD/swarmcraft/pull/124) | 0/1 · 0/1 | Exact live-lease regression incorporated in `0638dbc`; current adversarial pin-root test passes. Current file differs after later storage changes; no blind RED merge. |
| [#129](https://github.com/MousaXD/swarmcraft/pull/129) | 0/1 · 0/1 | `2d8a19b` certificate corruption/rollback guard and `4b33c5a` interrupted promotion fix on main. All named head functions retained and 12 current certificate tests pass including pending/accepted roots. Historical independent review and privileged local race caveat remain. |
| [#122](https://github.com/MousaXD/swarmcraft/pull/122) | 0/9 · 0/9 | Reconciled by `3a18b31` in [V2 cutover plan](rfcs/v2-migration-reconciliation.md); old verifier/test names deliberately replaced by common protocol + Ed25519 adapter, new stable/joint quorum and replay tests. Original handoff remains in PR. Residual durable freeze/root CAS, separate live transport and all-runtime activation gates; **not mergeable as a live policy**. |
| [#125](https://github.com/MousaXD/swarmcraft/pull/125) | 0/6 · 0/6 | Same reconciliation checkpoint; named protocol/crypto tests retained in evolved files. Same durable/live residual as #122; both historical PRs need explicit maintainer disposition after integration. |
| [#91](https://github.com/MousaXD/swarmcraft/pull/91) | 0/1 · 0/1 | Independent QA ledger is historical evidence, not replaced by an implementation author's approval. Preserve the PR's original `docs/audits/agent-07-qa-certification.md`; do not close the veto until an independent exact-release and three-device verdict exists. |

## Validation and remaining main-only actions

Read-only historical [Required Validation](https://github.com/MousaXD/swarmcraft/actions/runs/45080577660)
and [Main Desktop Installers](https://github.com/MousaXD/swarmcraft/actions/runs/45080577651)
both completed successfully on the inspected main SHA. The later
[integration run](https://github.com/MousaXD/swarmcraft/actions/runs/45524776685)
passed at `e1f7bce`, not the latest integration head. Current scoped
local checks: 12 recovery-certificate, 11 retention/GC, one adversarial
pin-root, one corrupt-membership-prepare binary regression, and 112
field-verifier tests passed. An initial library-only filter matched zero
daemon tests; rerun against the actual `swarmcraft` binary matched and
passed one. No new hosted CI was dispatched.

The integration branch has the local-first workflow policy, **not** the
historical automatic matrix. [README](../README.md), [Roadmap](../ROADMAP.md),
[Implementation status](IMPLEMENTATION_STATUS.md), and
[Desktop guide](../apps/desktop/README.md) distinguish branch implementation
from a published technical preview. The [execution tracker](SABATHON_FULL_EXECUTION_TRACKER.md)
is the current item-level evidence ledger. The original November 23
engineering plan remains a dated baseline, with this live reconciliation
linked from it.

After an authorized main merge, re-query open issues/PRs and ancestry,
review the residuals, link landed commits and exact evidence, and only
then close the appropriate stale records. Do not merge old PRs simply to
make ancestry green. Keep #84/#85/#97/#120/#140/#141 and any independent
security/field veto open until their actual criteria pass. #50 handles
release retrieval/rollback; #51 handles artifact resilience. No release
or main mutation is authorized by this document.
