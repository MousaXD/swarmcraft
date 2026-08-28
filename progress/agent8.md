# Agent 8 — Final integration and acceptance

## Status

`FINAL CANDIDATE — EXACT-HEAD VALIDATION RUNNING`

- Branch: `integration/player-launcher-v1`
- Product tree before ledger reconciliation: `bbba167df27493a8e90478f2ce177839db91a4b7`
- Final validation head: the commit containing this reconciled ledger. Record its immutable SHA and completed workflow IDs on validation PR #57; do not create a post-validation source commit solely to write the SHA into itself.
- `main` remains untouched by this recovery/integration effort.

## Integration result

- Networking/invites/discovery green base `ddc1667eccf871b64e4089992d43f2bbd4a6392f` was preserved.
- Canonical/provider work was union-integrated and all current Agent 2–6 live heads are now verified ancestors of the final branch.
- Agent 1's functional catalog source is integrated; its only live non-ancestor tail is an old bot-authored Desktop Rustfmt-only commit superseded by later integrated Rustfmt.
- No live Agent 7 branch existed, so missing player-launcher integration was recovered directly on this branch.
- Temporary recovery formatter/marker/script scaffolding has been removed from the product branch.

## Whole-product path now present

Alice can select Mojang Minecraft + compatible Fabric Loader, select Modrinth/CurseForge mods, resolve/freeze exact canonical provider requirements, create the canonical world, install/launch the managed runtime, explicitly accept EULA, checkpoint safely, and generate an invite without normal-path multiaddress entry. Bob can stage an invite join, complete canonical membership, receive the signed snapshot, and on runtime install/repair automatically acquire permitted exact provider artifacts from frozen canonical provenance; restricted/manual artifacts fail closed until the exact locally supplied JAR passes canonical identity/hash verification. Public/unlisted/private discovery semantics remain independent from membership.

## Exact-head evidence already green on integrated source

At `1cca925b44a51aef019f31ada77aaca88fcf4177`, before cleanup-only removal of recovery scaffolding:

- Release Guard GREEN.
- Network Soak GREEN.
- Linux workspace format, strict Clippy, tests GREEN.
- Windows strict Clippy/tests GREEN.
- macOS strict Clippy/tests GREEN.
- RustSec dependency audit GREEN.
- Fabric server mod build/embedded Fabric API GREEN.
- Fuzz smoke GREEN.
- Process-level acceptance GREEN: reconnect/hostile input/handshake, snapshot reconstruction, storage failure injection, import, sleep/wake fail-closed, Host Readiness, live join replication, host lifecycle, migration, runtime failure hardening, three-daemon recovery, successor-death recovery, and divergence detection.
- Linux Desktop frontend tests/Tauri bridge/runtime sidecars/native package build GREEN at observed checkpoint.
- macOS ARM64 Desktop native package GREEN.
- Dedicated catalog validation passed workspace format/metadata/check/strict Clippy/tests, deterministic catalog tests, and live official source validation before Desktop stages.

Workspace tests include the two-peer `automatic_invite_join` normal path with no manually supplied bootstrap address, and process acceptance includes `live_join_replication` with exact snapshot verification.

## Final gates

The reconciled ledger commit must complete, on the same exact head:

1. repository CI, including cross-platform Rust, process acceptance, Desktop native packages, Fabric mod, dependency audit, fuzz, and network impairment;
2. Network Soak;
3. Release version guard;
4. dedicated Agent 1 catalog + Desktop format/metadata/check/Clippy/Rust/JavaScript tests and live source validation;
5. clean-machine live Minecraft/Fabric player journey through managed Java, explicit EULA, real server launch, safe stop/checkpoint, process restart, restore, relaunch, and second checkpoint.

PR #57 is the validation-only vehicle for the clean-machine live gate and targets `integration/runtime-player-journey` pinned to `ddc1667eccf871b64e4089992d43f2bbd4a6392f`. PR #58 remains the dedicated catalog/Desktop validation vehicle. Neither should be merged into `main`.

## Verdict rule

Do not mark this ledger `DONE` or report `GOAL REACHED` until every final gate above is green or a genuinely equivalent exact-head proof has been verified. If a final gate finds a real defect, fix the defect on this branch and repeat exact-head validation rather than accepting a partial green.
