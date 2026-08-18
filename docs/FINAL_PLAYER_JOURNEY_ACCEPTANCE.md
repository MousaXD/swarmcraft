# Final Player-Journey Acceptance

## Scope and evidence rules

This acceptance pass starts from `integration/runtime-player-journey` at exactly:

`3731183ef97e51172ec8e8ff13981503ca55c2ba`

Baseline GitHub Actions evidence: CI run `32112002373`, successful on that integration head before this acceptance branch changed code.

Acceptance branch: `agent/final-player-journey-gates`.

Pull request: #37 into `integration/runtime-player-journey`. `main` is not a target of this branch or PR.

The authoritative final branch SHA and final exact-head run IDs are recorded in the PR/final handoff after the workflows finish. A tracked file cannot embed the SHA of the commit that contains itself without changing that SHA again, so this document deliberately avoids a self-invalidating final-SHA placeholder.

Gate colors are evidence-based:

- **GREEN** means the required real or deterministic evidence described by the gate has passed on the accepted candidate.
- **YELLOW** means the implementation intentionally remains fail-closed or the requested positive protocol does not yet exist.
- **RED** means the player journey is unavailable or violates an acceptance requirement.

No gate is promoted by weakening authority fencing, quorum, runtime verification, mod verification, EULA handling, stop durability, or history-conflict checks.

## Gate summary

| Gate | Candidate status | Reason |
| --- | --- | --- |
| Clean-machine E2E | GREEN only when the final literal exact-head live workflow passes | The live workflow uses a fresh data directory, explicit EULA acceptance, managed Java, official Minecraft/Fabric resolution, the candidate SwarmCraft Fabric artifact, authenticated Fabric readiness, two real launch/stop cycles, persistent runtime configuration, restored world data, monotonically advancing canonical snapshots and no divergence. |
| Existing-world import backend | GREEN | The Rust importer atomically stages, signs, verifies and publishes canonical world state, leaves source data unchanged, imports neither EULA state nor runtime binaries/configuration, and packages `swarmcraft-import` on all Desktop targets. |
| Existing-world import normal Desktop flow | GREEN once exact-head CI passes | Desktop now exposes `Import existing world` alongside Create/Join, validates exact compatibility metadata and explicit server-mod requirements, shows busy/errors honestly, forwards only the Rust/Tauri import contract, refreshes Worlds, and selects the returned `world_id` after success. |
| Corrupt/unreadable sleep state | GREEN once exact-head CI passes | Every authority-start classification uses the shared fail-closed sleep loader. Only true `NotFound` means awake; present records are signature-verified; corrupt/unreadable records block direct launch, standby and migration/runtime startup. |
| Alice/Bob two-voter crash failover | YELLOW | A two-voter world needs quorum 2. Bob alone after Alice disappears cannot safely elect himself. `BlockedByQuorum` is intentional. |
| Multi-member wake | YELLOW | No sleep-bound quorum wake election protocol exists yet. Multi-member wake remains explicitly fail-closed. |

## Clean-machine E2E

### Deterministic always-on coverage

`crates/swarm-cli/tests/runtime_setup_hardening.rs` remains the always-on deterministic fixture gate for installer locking, EULA refusal/retry, authenticated Fabric compatibility handling and runtime setup failure behavior. It does not masquerade as an external-artifact test.

The normal CI acceptance job also runs process lifecycle, migration orchestration, Host Readiness, existing-world import, corrupt-sleep launch regressions, live join, three-daemon recovery, recovery-successor failure, storage publication/failure injection, networking hardening, fuzz smoke and impaired QUIC resume suites.

### Real external artifact test

`.github/workflows/player-journey-live.yml` runs `scripts/acceptance/clean-machine-live.sh` in a fresh `SWARMCRAFT_DATA_DIR`.

The live journey deliberately separates the build JVM from the player runtime. The workflow builds the candidate SwarmCraft Fabric JAR, removes `JAVA_HOME`, resets `PATH` to base runner paths, and requires Runtime Installer to report the Java component as managed. Runtime Installer therefore has to supply the compatible player JVM rather than inheriting the workflow build JVM.

The live journey proves, in order:

1. fresh SwarmCraft data directory;
2. node initialization and normal world creation;
3. EULA initially unaccepted with no persisted launch configuration;
4. install without EULA acceptance does not configure launch and does not advance canonical world state;
5. explicit EULA acceptance;
6. managed Java installation and official Minecraft/Fabric runtime resolution;
7. candidate SwarmCraft Fabric artifact installation;
8. runtime verification and persisted launch configuration;
9. real shared Rust authority/runtime launch;
10. authenticated Fabric compatibility/readiness and a real generated `level.dat`;
11. known world-data marker written in the live runtime;
12. safe Stop World through the Fabric save/checkpoint/shutdown barrier;
13. verified canonical snapshot advancement plus a durable sleep record;
14. backend restart with persisted EULA and RuntimeLaunchConfig;
15. second real launch restoring the known marker from canonical state;
16. second safe stop with another monotonically newer verified canonical snapshot and no divergence.

The live workflow uses official Mojang/Fabric/Adoptium resolution paths. `SWARMCRAFT_FABRIC_MOD_JAR` points at the candidate branch's freshly built Fabric artifact because an unpublished candidate SHA cannot yet have a matching release asset.

### First-host integration

A fresh solo world has no accepted authority epoch yet. The managed runtime exposes `swarmcraft-runtime launch <world>` and enters the same Rust `migration::run_authority_runtime` path used by direct hosting. The shared path safely establishes the initial solo authority generation, restores canonical state, launches Minecraft, verifies Fabric compatibility, publishes Ready, and checkpoints/sleeps on safe stop. Authority behavior remains in Rust, not JavaScript.

## Corrupt/unreadable sleep state

Sleep state is a security boundary, not a best-effort hint. The shared helper `launch_guard::load_sleep_record_fail_closed` establishes the classification used by authority-sensitive code:

- a genuine storage `NotFound` returns `None` and is the only condition interpreted as an awake/no-sleep-record world;
- a present record is signature-verified before it can be treated as a valid sleeping record;
- decode corruption, invalid signatures, permissions/read failures, and other storage errors propagate as errors.

The authority startup paths consume that classification as follows:

- direct `swarmcraft-host` launch calls `ensure_direct_launch_safe`;
- standby `swarmcraft-host --standby` exits its readiness wait on corrupt/unreadable sleep state and never calls the host runtime;
- the migration supervisor publishes `MigrationPhase::Blocked`, marks runtime not ready, sleeps before retrying, and never falls through into ordinary authority handling;
- shared `run_authority_runtime` and `prepare_authority_epoch` classify sleep state before runtime preparation, so corruption cannot become an implicit awake path;
- safe-stop and sleeping manual-transfer classification also use the shared loader instead of `is_ok()` fall-through behavior.

Deterministic regression coverage includes direct corrupt sleep, true missing sleep state, valid solo/multi-member direct-launch rules, an actual standby readiness caller regression, and an actual migration-supervisor regression that uses nonexistent runtime binaries and asserts no runtime directory was created before the supervisor reports `Blocked`.

The supervisor retry path sleeps for one second on a corrupt/unreadable sleep record. It neither spins at high CPU nor deletes/repairs the record silently.

## Alice/Bob player journey

### What is genuinely proven

`live_join_replication.rs` exercises signed invite/join and canonical synchronization over real daemon/network paths.

`three_daemon_recovery.rs` exercises real process networking, loss of the current authority, majority-backed recovery, epoch/fencing advancement and stale-authority rejection with Alice, Bob and Carol. Runtime process behavior inside that deterministic recovery test uses a Fabric IPC fixture, so it is not claimed as a real Minecraft two-device release journey.

Host Capability and Host Readiness are backend-derived. The positive readiness path requires a current reachable successor, exact canonical snapshot/state, authority eligibility, verified runtime, verified server mods, no conflict and a surviving recovery quorum.

### Why literal two-device Alice/Bob crash failover remains YELLOW

The consensus quorum function is majority: `member_count / 2 + 1`.

For exactly two voting members, quorum is 2. If Alice disappears, Bob alone has one vote. Allowing Bob to recover authority would be a one-of-two election and would permit the split-brain class the fencing/quorum rules are designed to prevent.

The deterministic Host Readiness test `two_member_successor_requires_explicit_handoff` therefore reports `BlockedByQuorum`, and the product must not show Alice "Safe to shut down" solely because Bob is otherwise runtime/mod/snapshot ready.

A positive automatic crash-recovery journey is represented by the three-member Alice/Bob/Carol topology. A two-member world may use explicit manual authority transfer while Alice is still present and can sign/commit the transition. That is not equivalent to crash recovery and is not used to relabel this gate GREEN.

## Negative readiness matrix

| Variant | Required result | Current backend evidence/behavior |
| --- | --- | --- |
| Bob runtime missing | `BlockedByRuntime` | Host Readiness rejects a successor whose runtime proof is missing/unverified. |
| Bob required mod missing | `BlockedByMods` | Server-mod readiness blocks on missing canonical required mods. |
| Bob mod wrong hash/version | `BlockedByMods` | ID/version/environment/hash checks mark the inventory incompatible. |
| Bob replica stale | `Syncing` | A successor without the exact current canonical snapshot/state cannot be Safe. |
| Bob offline | `WorldWillStop` / unsafe | Current reachability is required; stale historical success is insufficient. |
| quorum insufficient after Alice disappears | `BlockedByQuorum` | Two-member Alice/Bob is explicitly fail-closed because Bob alone cannot form majority quorum. |
| conflicting history | `Conflict` or `DegradedSafety` | Divergent accepted history is never promoted to Safe. |
| Bob runtime artifact changed after verification | `BlockedByRuntime` | Runtime proof is bound to current artifact/configuration hashes and invalidated by mutation/reconfiguration. |
| Bob mod deleted after verification | `BlockedByMods` | Server-mod readiness is re-evaluated from current inventory. |

## Existing-world import semantics

### Backend contract

`world_import::import_world` is the typed Rust API. `swarmcraft-import` is the packaged sidecar and Desktop's Tauri `import_world` command delegates to it.

Import treats a Minecraft save as **world data**. Java, Fabric launcher/runtime paths and EULA state remain **machine-local configuration** and are not imported.

Required inputs are:

- a local source directory;
- display name;
- exact Minecraft version;
- exact Fabric Loader version;
- visibility;
- either every required third-party server-mod JAR or an explicit declaration that there are no third-party server-mod requirements.

Unknown Minecraft/Fabric compatibility is rejected. The importer does not infer third-party mod requirements from `level.dat`.

### Transaction and publication

The importer:

1. validates the request and a non-empty regular `level.dat`;
2. creates signed SwarmCraft genesis, descriptor, membership and world configuration in hidden staging;
3. snapshots the source through normal content-addressed storage without moving or mutating source files;
4. signs, commits and verifies canonical snapshot state in staging;
5. verifies/adds explicitly supplied server-mod profile artifacts as machine-local data;
6. releases the staging publication lease before the final directory move so Windows can publish atomically;
7. atomically renames the complete staged world directory into the visible worlds namespace;
8. syncs the parent directory on Unix and cleans hidden staging on failure.

A failed import never publishes a visible half-world. RuntimeLaunchConfig is absent after import and EULA remains unaccepted, so the imported world later enters the normal Runtime Wizard + Play flow.

### Backend failure/restart coverage

The import tests prove successful import, unchanged source bytes, reopen/restore verification, invalid or missing `level.dat` rejection, unknown compatibility rejection, ambiguous mod requirements rejection, interruption cleanup, safe retry, unique world IDs for repeat imports, and no EULA/runtime-config leakage.

Storage out-of-disk behavior remains covered by the storage failure-injection suite. The importer adds transaction-level failpoints around commit/publication rather than pretending to control the runner's physical disk.

### Normal Desktop flow

Desktop now has a visible `Import existing world` entry in primary navigation and next to Create/Join in the Worlds view. It presents a launcher-style form for source world folder path, display name, exact Minecraft version, exact Fabric Loader version, visibility, and the explicit server-mod compatibility choice.

The repository did not already contain a Tauri file-dialog plugin. To keep this final acceptance patch surgical and avoid adding a new native dependency, permission surface and packaging variable, this pass uses the explicitly permitted visible path-entry fallback. The player can paste or type the source folder path from the normal file manager; the import command itself remains the same typed Rust/Tauri backend operation.

Frontend validation requires every compatibility field and refuses an ambiguous empty server-mod list unless the player explicitly confirms that no third-party server mods are required. Supplying JAR paths and simultaneously checking the no-mods confirmation is also rejected.

The frontend never adds EULA acceptance or Java/runtime fields to the import payload. The backend adapter whitelists only the import contract fields. On success, the UI parses the backend's structured `world_id`, refreshes Worlds and selects the imported world. Backend or result errors are shown without pretending the import succeeded. The standard action binder supplies disabled/`aria-busy` progress state while the import runs.

`apps/desktop/tests/import-flow.test.mjs` covers visibility, validation, no-mod confirmation, exact JAR passthrough, absence of EULA state, backend bridge payload, success refresh/selection, failure behavior, and preservation of existing Create/Join/Play wiring. CI executes it through `node --test apps/desktop/tests/*.test.mjs`.

## Multi-member wake safety analysis

### Current fail-closed behavior

A sleeping world has a signed sleep record bound to the final canonical snapshot, epoch, fencing token and authority. `request_world_wake` validates that sleep record and local eligibility before recording wake intent.

For more than one non-banned member, supervision refuses to launch and publishes a blocked state explaining that a quorum-backed authority transition is required. Sleeping worlds are intentionally excluded from ordinary daemon lease/recovery processing.

Direct host, managed launch, standby and shared authority preparation now all treat sleep state through fail-closed classification. A valid sleeping single-member world may use existing solo wake semantics; a valid sleeping multi-member world remains blocked; a corrupt/unreadable sleep record is never interpreted as awake.

That prevents first-click-wins wake and does not reuse crash recovery as an unsafe substitute.

### Missing protocol needed for GREEN

Safe multi-member wake needs a consensus transition explicitly bound to the durable sleep record/snapshot. It must define quorum/ballot rules, deterministic simultaneous-wake resolution, selected host capability/readiness, a new fenced authority generation, stale pre-sleep authority rejection, failure/retry semantics, unavailable-quorum behavior, and canonical-lineage proof.

Ordinary crash recovery cannot simply be reused unchanged because sleeping worlds do not participate in its lease-loss path, and sleep is not itself an authority-loss election.

Therefore multi-member wake remains **YELLOW and fail-closed**. Solo/single-member wake continues to use the existing safe solo semantics.

## Package/platform matrix

The exact-head CI matrix builds and tests:

| Platform | Rust tests | Desktop package | Required sidecars |
| --- | --- | --- | --- |
| Linux | required | `.deb` + AppImage | `swarmcraft`, `swarmcraft-host`, `swarmcraft-runtime`, `swarmcraft-import` |
| Windows | required | NSIS `.exe` | same four sidecars |
| macOS ARM64 | required | `.dmg` | same four sidecars |
| macOS x86_64 | package runner plus shared macOS Rust tests | `.dmg` | same four sidecars |

The Fabric server-mod build, embedded Fabric API verification, dependency audit, fuzz smoke and impaired QUIC resume gates remain enabled.

## CI evidence

Baseline integration CI: `32112002373` at `3731183ef97e51172ec8e8ff13981503ca55c2ba`.

Previous exact-head branch CI: `32119659453` at `d545966aa3ce875beebc932ce049646267c16443`, successful before the final audit fixes in this document.

Previous exact-head live player journey: `32119659458` at `d545966aa3ce875beebc932ce049646267c16443`, successful before the final audit fixes in this document.

Acceptance workflows are configured to run on pushes to `agent/final-player-journey-gates` so the final handoff can distinguish the literal candidate SHA from GitHub's synthetic pull-request merge SHA.

The new final exact-head CI and live run IDs are recorded in PR #37 and the final handoff after this documentation commit itself has passed those workflows.

## Remaining intentional YELLOW gates

1. **Alice/Bob two-voter crash failover:** Bob alone cannot form majority quorum after Alice disappears. Keep `BlockedByQuorum`; do not add a one-of-two recovery shortcut.
2. **Multi-member wake:** no sleep-bound quorum wake election exists yet. Keep the world fail-closed; do not restore solo-wake behavior for multi-member worlds.

These are protocol limitations, not permission to weaken fencing or quorum. Positive automatic crash recovery remains covered by the three-member topology, and explicit authority transfer while the source authority is still present remains a separate supported operation.

## Merge-readiness rule

PR #37 is merge-ready into `integration/runtime-player-journey` only when the literal final branch head has both exact-head CI and exact-head Player journey live acceptance green. The two intentional YELLOW gates above remain documented and must not be relabeled GREEN.
