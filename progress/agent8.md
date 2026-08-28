# Agent 8 — Integration + Acceptance

## Status

`IN PROGRESS — FINAL EXACT-HEAD VALIDATION`

## Branch / exact head

- Branch: `integration/player-launcher-v1`
- Current pre-validation product head: `9c7816dea8371cfab34203acbd0c8a492894e785`
- Final accepted head: `PENDING`

## Mission

Integrate the complete player-launcher cohort into one coherent branch, resolve integration defects, run the full acceptance matrix, audit false greens, and leave a precise final handoff. Never merge or rewrite `main` during this acceptance phase.

## Dependencies consumed

The recovery audit did not trust stale ledger status. Live branch ancestry, code, commits, and Actions evidence were used as source of truth.

- Networking/invites/discovery integration base: `ddc1667eccf871b64e4089992d43f2bbd4a6392f` from `integration/swarmcraft-v1`.
- Canonical/provider implementation was union-integrated into that base at merge commit `b6395101868b064780a4a5fd1520477aab4900e0`.
- The resulting final-integration line contains the catalog, Modrinth, CurseForge, canonical modpack, automatic invites, discovery, runtime, import, and player-launcher bridge work.
- No Agent 7 branch existed live when final recovery began, so the missing player-facing launcher integration was completed directly on the final integration branch rather than fabricating an Agent 7 handoff.

## Work completed

- Audited live GitHub branches, validation PRs, workflows, and stale progress ledgers.
- Preserved the exact green networking/discovery integration while union-integrating canonical/provider work.
- Added the normal Desktop player-launcher controller and Tauri launcher bridge needed to connect authoritative catalogs, providers, canonical world creation, managed provider staging, exact artifact inspection, and discovery to the player UI.
- Wired provider-backed runtime acquisition so a joining player can reacquire exact canonical Modrinth/CurseForge artifacts from frozen provider provenance, while restricted/manual artifacts remain fail-closed with explicit remediation.
- Registered the previously missing Fabric validation command used by catalog selectors.
- Added launcher/controller and provider-runtime acceptance tests.
- Ran repository Rustfmt through the repository's pinned GitHub runner; formatter-only commit `9c7816dea8371cfab34203acbd0c8a492894e785` changed only `apps/desktop/src-tauri/src/launcher_commands.rs`.

## Contracts / APIs added or changed

- Desktop launcher bridge exposes managed provider staging and exact artifact inspection instead of trusting browser-supplied artifact identity.
- Canonical provider provenance remains the source for Bob's exact runtime artifact acquisition; runtime does not resolve a newer compatible release during join preparation.
- Public discovery/search remains separate from membership; exact resolve of an unlisted world does not grant membership; private worlds remain undiscoverable.
- Empty normal-path invite bootstrap input continues to use backend-derived reachability rather than requiring players to type multiaddresses.

## Files changed

The integration line includes product changes across Desktop JavaScript/Tauri bridge, runtime installer/provider acquisition, tests, and progress/validation support. Exact per-file history is preserved in Git commits; no changes were made to `main`.

## Tests and evidence

Evidence before this ledger-trigger commit:

- `integration/swarmcraft-v1` exact head `ddc1667eccf871b64e4089992d43f2bbd4a6392f`: CI GREEN, Network Soak GREEN, Release Guard GREEN.
- Union integration `b6395101868b064780a4a5fd1520477aab4900e0`: Linux strict Clippy GREEN, macOS strict Clippy GREEN, Desktop frontend/bridge validation GREEN, and process-level acceptance advanced through networking/storage reconstruction and failure-injection stages.
- Pre-format launcher integration head passed workspace format/check/strict Clippy/tests and live catalog validation, then failed only the separate Desktop rustfmt gate.
- Formatter-only commit `9c7816dea8371cfab34203acbd0c8a492894e785` applies the repository runner's exact Rustfmt output. PR workflows on that bot-authored commit reported `action_required` without running jobs because GitHub does not automatically execute PR workflows created by `github-actions[bot]`; this is trigger-policy noise, not a test result.

This ledger update is intentionally a normal author commit so the complete exact-head gates run on a non-bot head. Final workflow IDs/results will be recorded after completion.

Required final acceptance still covers at minimum:

1. Fresh Alice install/device setup.
2. Create world using Mojang-backed Minecraft selector.
3. Select compatible Fabric Loader from Fabric source.
4. Browse/select Modrinth mods.
5. Browse/select CurseForge mods where available/permitted.
6. Resolve dependencies and freeze exact canonical mod requirements/hashes.
7. Runtime install, explicit EULA, real Minecraft/Fabric launch.
8. Save/checkpoint canonical world state.
9. Create an invite without manually entering bootstrap addresses in the normal path.
10. Fresh Bob install, paste invite, complete canonical membership.
11. Bob resolves exact Minecraft/Fabric requirements.
12. Bob automatically retrieves permitted required mods; restricted artifacts produce precise remediation and pass after exact local artifact verification.
13. Bob receives/verifies the canonical snapshot and can launch/join.
14. Public/unlisted discovery behavior is correct; private worlds do not leak into public discovery.
15. Friend discovery does not grant membership.
16. Kill/lose Alice; successor election/fencing/runtime restore remain safe.
17. New authority endpoint is published and player reconnection follows the accepted backend endpoint.
18. Wrong hashes, incompatible mods, stale discovery, unreachable invites, provider failures, and interrupted installs fail closed.
19. Existing import, Stop World, Host Readiness, replication, recovery, package builds, and security gates remain green.

## Decisions / invariants

- Do not merge feature branches directly into `main` during this integration phase.
- Do not mark the milestone complete based only on unit tests. Require process-level and real-Minecraft player-journey evidence for the critical path.
- Preserve provider licensing/redistribution boundaries.
- Preserve canonical signed compatibility, authority fencing, quorum behavior, snapshot integrity, Host Readiness, and backend-owned runtime/mod decisions.
- Any intentionally unsupported scenario must be documented rather than hidden behind a green UI state.

## Known issues / blockers

- Final exact-head CI/Network Soak/Release Guard/provider/catalog validation must complete on a non-bot-authored immutable SHA.
- Final cleanup must remove temporary recovery-only workflow/script scaffolding before freezing the accepted SHA.
- The final Alice/Bob whole-product evidence must be reconciled against that same exact SHA before this ledger can say `DONE`.

## Handoff for dependent agents

This is the final integration ledger. Do not consume an acceptance verdict until the exact final SHA and all named green workflow IDs are recorded here.

## Activity log

- 2026-08-24 — ledger created; integration not started.
- 2026-08-29 — live-repository recovery superseded stale Agent 1–7 handoffs; networking/discovery and canonical/provider lines were union-integrated without touching `main`.
- 2026-08-29 — player-launcher bridge/runtime provider acquisition work landed and broad repository validation reduced the remaining failure to Desktop Rustfmt.
- 2026-08-29 — repository runner applied exact Rustfmt at `9c7816dea8371cfab34203acbd0c8a492894e785`; bot-authored PR workflows were blocked by GitHub trigger policy, so this ledger commit intentionally retriggers exact-head validation as a normal author commit.
