# Final Player-Journey Acceptance

## Scope and evidence rules

This acceptance pass starts from `integration/runtime-player-journey` at exactly:

`3731183ef97e51172ec8e8ff13981503ca55c2ba`

Baseline GitHub Actions evidence: CI run `32112002373`, successful on that integration head before this acceptance branch changed code.

Acceptance branch: `agent/final-player-journey-gates`.

Pull request: #37 into `integration/runtime-player-journey`. `main` is not a target of this branch or PR.

The authoritative final branch SHA is reported in the PR handoff after the final exact-head checks finish. A Git commit cannot embed its own final SHA in a file that participates in that same commit without changing the SHA again, so this document deliberately does not use a self-invalidating SHA placeholder.

Gate colors are evidence-based:

- **GREEN** means the required real or deterministic evidence described by the gate has passed on the accepted candidate.
- **YELLOW** means the implementation remains fail-closed or the required real-world evidence is incomplete.
- **RED** means the player journey is unavailable or violates an acceptance requirement.

No gate is promoted by weakening authority fencing, quorum, runtime verification, mod verification, EULA handling, stop durability, or history-conflict checks.

## Gate summary

| Gate | Candidate status | Reason |
| --- | --- | --- |
| Clean-machine E2E | YELLOW until the live workflow passes the accepted exact head | A real-artifact workflow now exists and exercises managed Java, explicit EULA, real Minecraft/Fabric launch, safe stop, restart and persisted runtime configuration. The deterministic runtime-hardening suite remains always-on. |
| Alice/Bob two-device E2E | YELLOW | A literal two-voter world cannot lose Alice and still form a majority recovery quorum. Bob-alone failover would require weakening the existing quorum rule. Real join/sync and three-member recovery are covered separately. |
| Existing-world import | GREEN once exact-head CI passes | A backend-first, atomic import path now stages and verifies canonical metadata/snapshot state before publication, leaves the source untouched, and imports no EULA or runtime binaries. |
| Multi-member wake | YELLOW | Current sleeping worlds are intentionally excluded from ordinary recovery. Safe wake needs a sleep-bound quorum proposal/selection protocol that does not yet exist. Existing behavior remains fail-closed. |

## Clean-machine E2E

### Deterministic always-on coverage

`crates/swarm-cli/tests/runtime_setup_hardening.rs` remains the always-on deterministic fixture gate for installer locking, EULA refusal/retry, authenticated Fabric compatibility handling and runtime setup failure behavior. It does not masquerade as an external-artifact test.

The normal CI acceptance job also runs the process lifecycle, migration orchestration, Host Readiness, live join, three-daemon recovery, storage failure injection, networking hardening, fuzz smoke and impaired QUIC resume suites.

### Real external artifact test

`.github/workflows/player-journey-live.yml` runs `scripts/acceptance/clean-machine-live.sh` in a fresh `SWARMCRAFT_DATA_DIR`.

The live journey deliberately separates the build JVM from the player runtime. The workflow builds the candidate SwarmCraft Fabric JAR, removes `JAVA_HOME`, resets `PATH` to the base runner paths, and then requires Runtime Installer to report the Java component as `managed: true`. The managed artifact resolver therefore has to supply the compatible Java runtime instead of inheriting the workflow build JVM.

The live journey proves, in order:

1. empty SwarmCraft data directory;
2. node initialization;
3. normal world creation;
4. no accepted EULA and no persisted launch configuration initially;
5. Runtime Wizard/backend plan resolution;
6. an installation attempt without EULA acceptance;
7. EULA remains unaccepted, no launch configuration appears, Minecraft is not Ready, and canonical state does not change;
8. explicit EULA acceptance;
9. official runtime resolution/download plus candidate SwarmCraft Fabric artifact installation;
10. cryptographic/runtime verification and persisted launch configuration;
11. managed Java was actually exercised;
12. real shared Rust authority/runtime launch;
13. authenticated Fabric world-info compatibility handshake;
14. real Minecraft world startup demonstrated by generated `level.dat` plus backend `runtime_ready=true`;
15. a known world-data marker is written through the live runtime directory;
16. safe Stop World requests the Fabric shutdown/save barrier rather than killing Minecraft;
17. a new canonical snapshot is verified and a durable sleep record is present;
18. backend processes are restarted;
19. EULA acceptance and RuntimeLaunchConfig remain persisted from the earlier explicit acceptance;
20. a second real Minecraft launch restores the marker from canonical state without manual Java/server/mod paths;
21. a second safe stop produces the next canonical snapshot with no divergence.

The live workflow uses official Mojang/Fabric/Adoptium resolution paths. `SWARMCRAFT_FABRIC_MOD_JAR` points at the candidate branch's freshly built Fabric artifact because an unpublished candidate SHA cannot yet have a matching GitHub release asset.

### First-host integration defect fixed by this pass

A fresh solo world has no accepted authority epoch yet. Previously the managed Desktop launch only started the daemon and polled migration state, while daemon supervision correctly waited for an already accepted authority. That could strand first Play in `WaitingForAuthority`.

The managed runtime now has a `swarmcraft-runtime launch <world>` command. Desktop owns that process and it enters the same Rust `migration::run_authority_runtime` path used by direct hosting. The shared path safely establishes the initial solo authority generation, restores canonical state, launches Minecraft, verifies Fabric compatibility, publishes Ready, and checkpoints/sleeps on safe stop. Authority behavior remains in Rust, not JavaScript.

## Alice/Bob player journey

### What is already genuinely proven

`live_join_replication.rs` exercises signed invite/join and canonical synchronization over real daemon/network paths.

`three_daemon_recovery.rs` exercises real process networking, loss of the current authority, majority-backed recovery, epoch/fencing advancement and stale-authority rejection with Alice, Bob and Carol. Runtime process behavior inside that deterministic recovery test uses a Fabric IPC fixture, so it is not claimed as a real Minecraft two-device release journey.

Host Capability and Host Readiness are backend-derived. The main positive readiness path requires a current reachable successor, exact canonical snapshot/state, authority eligibility, verified runtime, verified server mods, no conflict and a surviving recovery quorum.

### Why literal two-device Alice/Bob failover remains YELLOW

The consensus quorum function is majority: `member_count / 2 + 1`.

For exactly two voting members, quorum is therefore 2. If Alice disappears, Bob alone has only one vote. Allowing Bob to recover authority at that point would be a one-of-two election and would permit the exact split-brain class the fencing/quorum rules are designed to prevent.

The deterministic Host Readiness test `two_member_successor_requires_explicit_handoff` therefore reports `BlockedByQuorum`, and the product must not show Alice "Safe to shut down" solely because Bob is otherwise runtime/mod/snapshot ready.

A positive crash-failover journey needs a third voting witness/member, or a separately designed membership/lease protocol that safely changes quorum before Alice disappears. This acceptance pass does not weaken majority quorum to force a green result.

A two-member world can still use the existing explicit manual authority-transfer path while Alice is present and can sign/commit the transition. That is not equivalent to crash recovery and is not used to relabel this gate GREEN.

## Negative readiness matrix

| Variant | Required result | Current backend evidence/behavior |
| --- | --- | --- |
| A. Bob runtime missing | `BlockedByRuntime` | Host Readiness rejects a successor whose runtime proof is missing/unverified. |
| B. Bob required mod missing | `BlockedByMods` | Server-mod readiness evaluates the canonical requirement inventory and blocks on missing required mods. |
| C. Bob mod wrong hash/version | `BlockedByMods` | ID/version/environment/hash checks mark the mod inventory incompatible. |
| D. Bob replica stale | `Syncing` | A successor without the exact current canonical snapshot/state cannot be Safe. |
| E. Bob offline | `WorldWillStop` / unsafe | Current reachability is required; a stale historical success is insufficient. |
| F. quorum insufficient after Alice disappears | `BlockedByQuorum` | Two-member Alice/Bob is explicitly fail-closed because Bob alone cannot form majority quorum. |
| G. conflicting history | `Conflict` or `DegradedSafety` | Divergent accepted history is never promoted to Safe. |
| H. Bob runtime artifact changed after verification | `BlockedByRuntime` | Runtime proof is bound to current artifact/configuration hashes and is invalidated by mutation/reconfiguration. |
| I. Bob mod deleted after verification | `BlockedByMods` | Server-mod readiness is re-evaluated from the current inventory; deleted/replaced artifacts invalidate readiness. |

The exact-head CI acceptance job explicitly runs the Host Readiness library tests as a named gate in addition to the complete workspace test matrix.

## Existing-world import semantics

### Backend contract

`world_import::import_world` is the typed Rust API. `swarmcraft-import` is the thin packaged command-line sidecar and Desktop exposes a thin Tauri `import_world` command that delegates to it.

Import treats a Minecraft save as **world data**. Java, Fabric launcher/runtime paths and EULA state remain **machine-local configuration** and are not imported.

Required inputs are:

- a local source directory;
- display name;
- exact Minecraft version;
- exact Fabric Loader version;
- visibility;
- either every required third-party server-mod JAR or an explicit declaration that there are no third-party server-mod requirements.

Unknown Minecraft/Fabric compatibility is rejected. The importer never invents server-mod requirements from a save directory.

### Transaction and publication

The importer:

1. validates the request and a non-empty regular `level.dat`;
2. creates signed SwarmCraft genesis, descriptor, membership and world configuration in hidden staging;
3. snapshots the source through the normal content-addressed storage machinery without moving or mutating source files;
4. signs, commits and verifies the canonical snapshot in staging;
5. verifies/adds any explicitly supplied server-mod profile artifacts as machine-local data;
6. atomically renames the complete staged world directory into the visible worlds namespace;
7. syncs the parent directory on Unix;
8. cleans hidden staging on failure.

A failed import never publishes a visible half-world. RuntimeLaunchConfig is absent after import and EULA remains unaccepted, so the imported world later enters the normal Runtime Wizard + Play flow.

### Failure/restart coverage

The import tests prove:

- successful valid import;
- source bytes remain unchanged;
- restart/reopen verifies and restores the canonical snapshot;
- invalid/missing `level.dat` is rejected;
- unknown compatibility is rejected;
- ambiguous server-mod requirements are rejected;
- interruption after staged snapshot commit exposes no world;
- interruption before publication exposes no world;
- retry after interruption succeeds;
- importing the same source again creates a separate world ID and never overwrites the prior import;
- no EULA acceptance or runtime launch configuration leaks into the imported world.

A real out-of-disk condition is covered by the existing storage failure-injection suite at the storage layer. The importer adds transaction-level failpoints around commit/publication, rather than pretending to control the runner's physical disk.

### Desktop exposure

The import sidecar is bundled into Linux, Windows, macOS ARM64 and macOS x86_64 Desktop packages and a Tauri command is registered. The backend returns a typed/JSON result only after canonical publication succeeds.

A dedicated visible folder-picker/form has not been added in this acceptance branch. The safe backend and packaged bridge exist, but the normal-player Desktop presentation remains a follow-up UI task. This does not change the backend import safety gate, but it is recorded as a player-journey limitation rather than hidden.

## Multi-member wake safety analysis

### Current fail-closed behavior

A sleeping world has a signed sleep record bound to the final canonical snapshot, epoch, fencing token and authority. `request_world_wake` validates that sleep record and local eligibility before recording wake intent.

For more than one non-banned member, supervision refuses to launch and publishes a blocked state explaining that a quorum-backed authority transition is required. Sleeping worlds are also intentionally excluded from ordinary daemon lease/recovery processing.

That combination is deliberate: it prevents the first peer to click Play from becoming authority merely because the old runtime is asleep.

### Missing protocol needed for GREEN

Safe multi-member wake needs a consensus object/transition that is explicitly bound to the durable sleep state. At minimum it must define:

- which signed sleep record/snapshot is being resumed;
- current epoch and fencing token;
- eligible wake initiators and candidates;
- quorum and ballot rules;
- deterministic resolution of simultaneous wake attempts;
- selected host capability/runtime/mod readiness;
- a new fenced authority generation;
- rejection of stale pre-sleep authority;
- retry semantics if the selected host crashes before or after promotion;
- behavior when quorum is unavailable;
- proof that no competing canonical snapshot lineage is accepted.

Ordinary crash recovery cannot simply be reused unchanged because sleeping worlds do not participate in its lease-loss path, and a sleep transition is not itself an authority-loss election. Re-enabling ordinary recovery for sleepers or allowing the first wake click to self-elect would weaken the safety model.

Therefore multi-member wake remains **YELLOW and fail-closed** in this pass. Solo/single-member wake continues to use the existing safe solo semantics.

## Package/platform matrix

The exact-head CI matrix builds and tests:

| Platform | Rust tests | Desktop package | Import sidecar bundled |
| --- | --- | --- | --- |
| Linux | required | `.deb` + AppImage | required |
| Windows | required | NSIS `.exe` | required |
| macOS ARM64 | required | `.dmg` | required |
| macOS x86_64 | required through the macOS matrix/package runner | `.dmg` | required |

The Fabric artifact build/embedded Fabric API verification, dependency audit, fuzz smoke and impaired QUIC resume gates remain enabled.

## CI evidence

Baseline integration CI: `32112002373` at `3731183ef97e51172ec8e8ff13981503ca55c2ba`.

Acceptance exact-head workflows are configured to run on pushes to `agent/final-player-journey-gates` so the release record can distinguish the literal candidate SHA from GitHub's synthetic pull-request merge SHA.

Final exact-head CI and live player-journey run IDs are recorded in the PR/final handoff after the final documentation commit has itself passed those workflows.

## Remaining limitations

1. **YELLOW, Alice/Bob literal crash failover:** two voting members cannot survive one disappearance with majority quorum. A third witness/member or separately designed safe quorum-changing protocol is required for the requested positive crash-failover journey.
2. **YELLOW, multi-member wake:** no sleep-bound quorum wake protocol exists yet; backend remains explicitly fail-closed.
3. **YELLOW, Desktop import presentation:** safe import backend, packaged sidecar and Tauri bridge exist, but this branch does not add a normal-player folder-picker/form.
4. Clean-machine is GREEN only after the real live workflow passes the accepted exact head. External service failure remains distinguishable from the deterministic offline fixture suite.

## Release recommendation rule

Do not recommend release as fully player-journey complete while either the literal Alice/Bob crash-failover gate or multi-member wake gate remains YELLOW. Import safety and clean-machine evidence can become GREEN independently, but those colors do not erase the consensus limitations above.
