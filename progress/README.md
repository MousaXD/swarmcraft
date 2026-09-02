# SwarmCraft player-launcher integration status

Updated: 2026-08-29

This directory records the live recovery/integration state for the player-launcher cohort. Historical agent notes were reconciled against current GitHub branch ancestry, source code, and exact-head Actions evidence. Stale status text is not treated as authority.

## Final integration line

- Product branch: `integration/player-launcher-v1`
- Product tree before this ledger-only reconciliation: `bbba167df27493a8e90478f2ce177839db91a4b7`
- Final acceptance SHA: the ledger reconciliation commit that contains this file, pending exact-head validation. Its immutable SHA and workflow IDs are recorded on validation PR #57 after the gates complete.
- `main` has not been merged into or rewritten by this recovery effort.

## Live source classification

| Cohort | Live source | Recovery classification |
| --- | --- | --- |
| Agent 1 catalog | `agent/minecraft-fabric-catalog` | INTEGRATED. Functional source is present. The live branch has one extra bot-authored Rustfmt-only tail commit, `a581195bfff3fd3a050e1978910fe77288237cbc`; final integration received its own later Rustfmt over the combined Desktop tree. |
| Agent 2 Modrinth | `355d1f0762fe04391643eabcb802bc4641b1b0a8` | INTEGRATED, exact live head is an ancestor of final integration. |
| Agent 3 CurseForge | `2ec9005591de71fffe8e504607a4ffb3145ff9c8` | INTEGRATED, exact live head is an ancestor of final integration. |
| Agent 4 canonical modpack | `75351794926e1ba183e5615f948a53c7017084bf` | INTEGRATED, exact live head is an ancestor of final integration. |
| Agent 5 automatic invites | `e13a4fd57e3c26121275db0b1628808e2e036a44` | INTEGRATED, exact live head is an ancestor of final integration. |
| Agent 6 discovery | `0a72380aebbc6f227957cae733de64dc6f85638c` | INTEGRATED, exact live head is an ancestor of final integration. |
| Agent 7 player journey | no live Agent 7 branch | RECOVERED DIRECTLY ON FINAL INTEGRATION. Missing launcher/player wiring was completed during repository recovery rather than inventing a nonexistent handoff branch. |
| Agent 8 final acceptance | `integration/player-launcher-v1` | FINAL CANDIDATE, exact-head acceptance in progress. |

## Integrated product capabilities

The final integration line contains authoritative Mojang/Fabric selectors, Modrinth and CurseForge provider integrations, deterministic canonical modpack provenance and hashes, canonical world creation, backend-owned Fabric JAR inspection, managed provider staging, exact provider artifact reacquisition for runtime install/repair, automatic signed invites without ordinary-path multiaddress entry, public/unlisted/private discovery semantics, import, managed Java/Minecraft/Fabric runtime, replication, Host Readiness, stop/checkpoint/wake, migration, authority fencing, and recovery.

Desktop normal-path launcher wiring is active through `app.js` -> `import-flow.js` -> `catalog-selectors.js` and `launcher-controller.js`. Provider runtime acquisition is wired at `swarmcraft-runtime install/repair` before `RuntimeInstaller` verifies the completed canonical mod profile. Restricted/manual provider artifacts fail closed with actionable remediation instead of silently substituting another release.

## Acceptance evidence already established

Exact head `1cca925b44a51aef019f31ada77aaca88fcf4177`, whose only subsequent product-branch changes before this ledger were removal of recovery-only scaffolding, established:

- Release version guard: GREEN.
- Network Soak: GREEN.
- Linux workspace format, strict Clippy, and tests: GREEN.
- Windows strict Clippy and tests: GREEN.
- macOS strict Clippy and tests: GREEN.
- RustSec dependency audit: GREEN.
- Fabric server mod build and embedded Fabric API check: GREEN.
- Fuzz smoke: GREEN.
- Process-level acceptance: GREEN, including live join replication, Host Readiness fail-closed cases, import, storage failure injection, three-daemon recovery, and successor-death recovery.
- Linux Desktop frontend tests, Tauri bridge validation, runtime sidecar build, and native package build: GREEN at the observed checkpoint; Apple Silicon native Desktop package: GREEN.
- Dedicated catalog validation passed workspace format/check/strict Clippy/tests, deterministic catalog tests, and live Mojang/Fabric source validation before entering its Desktop stages.

Workspace tests also include `automatic_invite_join`, which creates an invite with no manual `--bootstrap`, joins from a second peer, runs two real daemon processes, advances canonical membership, and replicates the exact snapshot. `live_join_replication` independently exercises two real daemon processes and signed join/snapshot replication.

## Final validation vehicles

- PR #57 is validation-only and targets `integration/runtime-player-journey`, a branch pinned to `ddc1667eccf871b64e4089992d43f2bbd4a6392f`. This lets the repository's clean-machine real Minecraft/Fabric player-journey workflow run against the final head without touching `main`.
- PR #58 remains a validation vehicle for the dedicated Agent 1 catalog/Desktop gate.
- Neither validation PR is intended to merge into `main`.

## Completion rule

Do not call the milestone complete merely because code exists or unit tests pass. The final immutable candidate must finish its exact-head repository CI, Network Soak, Release Guard, dedicated catalog/Desktop validation, and clean-machine live Minecraft/Fabric journey. The final acceptance report must also reconcile the two-peer invite/join/replication and exact-provider acquisition evidence against that same source tree.
