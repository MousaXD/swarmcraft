# Integration unfinished work

## Current state

`NO KNOWN PRODUCT IMPLEMENTATION GAP — FINAL ACCEPTANCE ONLY`

The former integration backlog has been consumed into `integration/player-launcher-v1`. This file now tracks only evidence that must finish before the milestone may be called complete.

Product tree before ledger reconciliation: `bbba167df27493a8e90478f2ce177839db91a4b7`.

## Remaining acceptance gates

- Exact-head repository CI must complete green, including all native Desktop package jobs.
- Exact-head Network Soak must complete green.
- Exact-head Release Guard must complete green.
- Exact-head dedicated catalog/Desktop validation must complete through Desktop format, locked metadata, check, strict Clippy, Rust tests, JavaScript tests, deterministic catalog tests, and live Mojang/Fabric validation.
- Exact-head clean-machine live player journey must complete against real managed Java + Minecraft/Fabric, including explicit EULA, launch, safe checkpoint, backend/process restart, restore, relaunch, and second checkpoint.
- Final acceptance must reconcile the exact-head two-peer automatic invite/join and live snapshot-replication tests plus exact canonical provider acquisition/fail-closed evidence.

## Resolved recovery items

- Provider/canonical and networking/discovery lines are integrated.
- Current Agent 2–6 live heads are exact ancestors of final integration.
- Agent 1's only non-ancestor tail is formatting-only and superseded by integrated Rustfmt.
- Desktop launcher-controller and authoritative catalog selectors are wired into the normal module graph.
- Tauri exposes backend-owned provider staging/JAR inspection/discovery commands.
- Runtime install/repair performs exact provider reacquisition from frozen canonical provenance before final runtime mod readiness verification.
- Automatic invite normal path does not require the player to type a bootstrap multiaddress.
- Recovery-only formatter/marker/patch-script scaffolding has been removed.

If any final gate fails, add only the concrete defect here until repaired. Do not repopulate this file with stale agent-history blockers.
