# Auditor 5 — Minecraft/Fabric Runtime and Player Lifecycle

## Audit identity

- Repository: `MousaXD/swarmcraft`
- Audit branch: `audit/runtime-minecraft`
- Audited baseline: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Live `main` at audit start: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Production code modified: **no**
- Audit scope: managed Java, Minecraft/Fabric installation, Fabric lifecycle bridge, runtime CLI, Desktop launcher process ownership, checkpoint/stop/wake/migration behavior, import, EULA, and live-player acceptance evidence.

## Executive verdict

**VERDICT: FAIL**

The primary managed `26.1.2` / Fabric `0.19.3` lifecycle is substantially stronger than the historical player-journey reports suggest. The exact audited SHA passed the real `Player journey live acceptance` workflow (`33576322489`, job `100080968215`) on Ubuntu. That workflow builds the candidate Fabric mod and Rust binaries, deliberately removes the compatible build JDK from `PATH`, installs managed Java from a clean data directory, refuses launch before explicit EULA acceptance, launches a real Minecraft/Fabric server, waits for the authenticated Fabric handshake, performs a safe stop, advances and verifies the canonical snapshot, relaunches from persisted state, and repeats the safe checkpoint.

The audit nevertheless fails because three lifecycle boundaries are not safe:

1. **HIGH — Existing-world import can snapshot a Minecraft world while another Minecraft process is actively mutating it.** The import backend validates `level.dat` but never acquires Minecraft's live-world lock or another save/quiescence barrier before streaming `request.source` into a signed canonical snapshot.
2. **HIGH — World creation/import compatibility is broader than the shipped Fabric bridge's actual runtime contract.** The catalog treats every Mojang release/snapshot as `supported`, while the shipped bridge declares `minecraft: ~26.1.2`, `fabricloader: >=0.19.3`, and `java: >=25`. A catalog-valid world outside that contract can be canonicalized and only fail later at Fabric startup/IPC verification.
3. **HIGH — Loss of the Rust runtime supervisor can orphan a still-writable Minecraft authority runtime.** The single-flight lock belongs to the supervisor process, the independent daemon owns the authority-permit heartbeat, and Fabric treats controller IPC EOF as a log event rather than a reason to save/stop. A killed/crashed supervisor can therefore release the launch lock without proving the Java child stopped.

A fourth **MEDIUM** finding is that Desktop consumes and discards sidecar stdout/stderr events, removing the most useful Minecraft/Fabric diagnostics from failed launches.

No evidence was found that the normal managed safe-stop path copies a live world. For managed launches, the implementation performs a Fabric save barrier, waits for process exit, rechecks the authority generation, and only then snapshots the runtime world.

---

## Evidence reviewed

Primary implementation:

- `crates/swarm-cli/src/runtime_installer.rs`
- `crates/swarm-cli/src/runtime_layout.rs`
- `crates/swarm-cli/src/runtime_main.rs`
- `crates/swarm-cli/src/migration.rs`
- `crates/swarm-cli/src/launch_guard.rs`
- `crates/swarm-cli/src/authority_permit.rs`
- `crates/swarm-cli/src/server_mods.rs`
- `crates/swarm-cli/src/world_import.rs`
- `crates/swarm-ipc/src/transport.rs`
- `minecraft/fabric/build.gradle`
- `minecraft/fabric/gradle.properties`
- `minecraft/fabric/src/main/resources/fabric.mod.json`
- `minecraft/fabric/src/main/java/dev/swarmcraft/fabric/SwarmCraftMod.java`
- `apps/desktop/src-tauri/src/runtime.rs`
- `apps/desktop/src-tauri/src/runtime_commands.rs`
- `apps/desktop/src-tauri/src/canonical_world_commands.rs`
- `crates/swarm-catalog/src/lib.rs`

Acceptance/test evidence:

- `.github/workflows/player-journey-live.yml`
- `scripts/acceptance/clean-machine-live.sh`
- `crates/swarm-cli/tests/migration_core.rs`
- import/runtime unit tests adjacent to the implementation
- exact-SHA GitHub Actions run `33576322489`, job `100080968215`, conclusion `success`

The live acceptance gate is valuable evidence for one concrete supported tuple and one platform. It is not evidence for all Minecraft versions exposed by the catalog, runtime-supervisor death, live-source import, cross-platform runtime launch, or multi-peer migration/recovery.

---

## Lifecycle state machine

```text
                     create/import canonical world
                               |
                               v
                    [RUNTIME UNPREPARED]
                               |
                         plan / install
                               |
                 +-------------+--------------+
                 |                            |
         EULA not accepted             EULA accepted
                 |                            |
                 v                            v
           [EULA REQUIRED]            [LAUNCH CONFIGURED]
                 |                            |
          explicit acceptance                 |
                 +-------------+--------------+
                               |
                               v
                    [SELECT CANONICAL SNAPSHOT]
                               |
                       authority proof/fence
                               |
                               v
                     [PREPARE RUNTIME]
                reset runtime/<world> completely
            stage bridge + verified canonical mods
                               |
                               v
                       [RESTORE SNAPSHOT]
                               |
                        recheck authority
                               |
                               v
                         [SPAWN JAVA]
                               |
                    authenticated Fabric IPC
           exact MC / loader / world path / fingerprint
                               |
                               v
                           [READY]
                    players mutate world files
                               |
          +--------------------+--------------------+
          |                    |                    |
      stop intent         transfer intent    spontaneous exit
          |                    |                    |
          v                    v                    v
  PREPARE_SHUTDOWN(1)  PREPARE_SHUTDOWN(2)  success => sleep path
          |                    |              failure => fail, no checkpoint
          +----------+---------+
                     |
           server-thread saveEverything
                     |
              READY_FOR_SHUTDOWN
                     |
               wait child exit
                     |
                     v
                [CHECKPOINTING]
       recheck authority generation before/after
          snapshot/sign/commit stopped world
                     |
          +----------+-----------+
          |                      |
       [SLEEPING]      [AWAIT TRANSFER ACCEPTANCE]
          |                      |
      wake/recovery          successor activation
          +----------+-----------+
                     |
              select exact canonical
                     |
                     +----> PREPARE RUNTIME
```

### Important failure edges

- Fabric handshake timeout or mismatch: Java child is terminated; no post-run snapshot is committed.
- Authority generation changes before readiness: Java child is terminated and runtime is marked superseded.
- Safe-stop save barrier fails: stop intent is cleared and Minecraft is deliberately kept running; no snapshot is taken from a live server.
- Transfer/CLI shutdown barrier or post-barrier exit timeout: the error unwinds and the Java child is killed before checkpointing.
- Unsuccessful spontaneous Minecraft exit: runtime returns an error and does not create a new canonical snapshot.
- **Broken edge:** external import bypasses this state machine and snapshots an arbitrary source directory without proving Minecraft is stopped.
- **Broken edge:** runtime-supervisor/IPC death does not cause the Fabric bridge to stop; the process lock can disappear while Java remains alive.

---

# Findings

## A5-01 — HIGH — Existing-world import can canonize a live, torn Minecraft save

### Exact code

- `crates/swarm-cli/src/world_import.rs`
  - `import_world`
  - `import_world_inner`
  - `stage_and_publish`
  - `validate_request`

### Required invariant

A snapshot that becomes canonical must not be assembled while Minecraft is actively mutating the source files unless a reliable Minecraft save/quiescence boundary is held for the entire copy.

### Failure scenario

`validate_request` verifies that the source is a directory and that `level.dat` is a non-empty regular file. It does **not** prove the source Minecraft world is quiescent. `stage_and_publish` then calls:

```text
staged.snapshot_directory(&request.source, ...)
```

and signs/commits the result as snapshot 1.

If the player imports a single-player/server save that is still open, Minecraft may rewrite `level.dat`, region files, entity/poi files, player data, or session metadata while `snapshot_directory` is streaming the directory. Content-addressed hashing proves the bytes SwarmCraft happened to read, but it does not prove those bytes came from one coherent Minecraft save epoch. A mixture of files from before and after a save can therefore become a validly signed canonical SwarmCraft snapshot.

This is exactly the unsafe live-copy condition called out by this audit's mission.

### Evidence

- No source-world lock is acquired in `validate_request` or held during `snapshot_directory`.
- No Fabric IPC save barrier is available or required for import.
- Existing import tests create static fixture files and verify atomic SwarmCraft publication; they do not keep a real Minecraft process mutating the source during import.
- The import comments correctly state that the source is not mutated, but source immutability by SwarmCraft is different from source quiescence by Minecraft.

### Reproduction

1. Start a real Minecraft server using world directory `SOURCE`.
2. Keep at least one player active or otherwise force periodic chunk/player saves.
3. Invoke SwarmCraft import against `SOURCE`.
4. Current backend has no lock/save-boundary rejection path, so it proceeds to `snapshot_directory`.
5. Repeating with targeted file mutation during the copy can produce a snapshot whose files came from different save points while still passing SwarmCraft hash/signature verification.

### Existing coverage

Positive:

- Atomic staged publication.
- Interrupted import never exposes a half-published SwarmCraft world.
- Unknown compatibility and invalid `level.dat` are rejected.
- EULA/runtime state is not imported.

Missing:

- Real Minecraft process holding the source world open.
- Minecraft `session.lock`/equivalent exclusive-lock contention.
- Mutation of region/player files during import.

### Remediation

Fail closed unless the source can be proven quiescent for the full snapshot operation. For a normal external Minecraft world, acquire and hold the same exclusive session-lock semantics that Minecraft uses, or an equivalent authoritative lock check, before reading any save files. If the lock cannot be acquired, report that Minecraft must be stopped. For a SwarmCraft-managed source, prefer importing/restoring from a canonical checkpoint instead of raw live files.

Do not treat a one-time pre-copy check as sufficient; the proof must remain held until snapshot input has been fully consumed.

### Test required to close

A process-level test that launches a real Minecraft server on a source world and verifies:

- import is rejected while the world lock is held;
- after a proper Minecraft stop/save, the same import succeeds;
- the resulting snapshot verifies and relaunches.

### Confidence

**High.** The missing quiescence mechanism is directly visible in the audited source.

---

## A5-02 — HIGH — Catalog-valid Minecraft/Fabric choices are not constrained to the shipped Fabric bridge contract

### Exact code

- `minecraft/fabric/src/main/resources/fabric.mod.json`
- `minecraft/fabric/gradle.properties`
- `crates/swarm-catalog/src/lib.rs`
  - `parse_minecraft_catalog`
  - `filter_minecraft_versions`
  - `fabric_loader_versions`
  - `validate_fabric_selection`
- `apps/desktop/src-tauri/src/canonical_world_commands.rs`
  - `create_canonical_world`
  - `validate_catalog_selection`
- `crates/swarm-cli/src/runtime_installer.rs`
  - `resolve_runtime`
  - `resolve_minecraft`
  - `resolve_managed_java`
- `crates/swarm-cli/src/migration.rs`
  - `run_authority_runtime_inner`
  - `validate_world_info`
- `crates/swarm-cli/src/world_import.rs`
  - `validate_request`

### Required invariant

Any Minecraft/Fabric tuple that SwarmCraft lets a player make canonical must be launchable by the exact SwarmCraft Fabric adapter identified by that canonical world profile, or creation must fail before canonicalization with an explicit unsupported-runtime error.

### Evidence

The shipped Fabric mod declares hard runtime dependencies:

```json
"fabricloader": ">=0.19.3",
"minecraft": "~26.1.2",
"java": ">=25",
"fabric-api": "*"
```

and the candidate artifact is compiled with `minecraft_version=26.1.2`, loader `0.19.3`, and Java release/target 25.

The catalog does not model this adapter support range. `parse_minecraft_catalog` sets `supported = true` for every Mojang entry whose type is `release` or `snapshot`. `filter_minecraft_versions` then exposes those entries. Desktop `validate_catalog_selection` only checks that the selected Minecraft version appears in the authoritative Mojang catalog and that Fabric Meta reports the selected loader for that Minecraft version.

`RuntimeInstaller::resolve_runtime` then resolves Java from the selected Minecraft version and reuses the current SwarmCraft Fabric adapter artifact. That is not a substitute for validating the adapter's own dependency contract. For an older Minecraft version, the installer may deliberately install that version's required Java major even though the bridge itself requires Java 25. For any Minecraft version outside `~26.1.2`, Fabric Loader can reject the bridge before SwarmCraft receives its authenticated IPC `WORLD_INFO` handshake.

Import is even broader: `world_import::validate_request` requires non-empty Minecraft and Fabric version strings but does not validate them against the adapter support contract.

### Failure scenario

1. Player selects a Mojang release outside the bridge's `~26.1.2` range.
2. Fabric Meta supplies at least one loader for that Minecraft version.
3. `create_canonical_world` accepts the tuple and writes a canonical immutable compatibility profile.
4. Runtime Installer resolves/downloads the selected Minecraft/Fabric/Java artifacts.
5. At launch, Fabric Loader rejects the SwarmCraft bridge's dependency constraints or the bridge fails to load against that Minecraft line.
6. SwarmCraft waits for Fabric IPC and eventually fails startup. The failure happens after world creation, not at selection time.

The same mismatch class exists for a Fabric Loader selection below the bridge's declared `>=0.19.3` floor.

### Existing coverage

The exact-SHA live gate proves one tuple only:

- Minecraft `26.1.2`
- Fabric Loader `0.19.3`
- Java 25
- Ubuntu runner

That tuple matches the bridge metadata and passes.

Missing:

- negative creation tests for catalog-valid but adapter-incompatible Minecraft versions;
- loader-floor tests;
- a generated adapter support matrix shared by catalog/create/import/install;
- live launch matrix for every advertised supported tuple.

### Remediation

Create one authoritative runtime support matrix derived from the shipped adapter artifacts and enforce it **before canonical world creation/import**. At minimum it must intersect:

- Minecraft version/range supported by the adapter;
- minimum/maximum Fabric Loader range;
- Java constraint;
- any Fabric API compatibility requirement.

If SwarmCraft intends to support multiple Minecraft lines, publish distinct tested adapter artifacts or an explicitly compatible adapter range, encode the chosen adapter requirement canonically, and make Runtime Installer acquire the exact matching artifact. Do not label all Mojang releases as player-selectable merely because Fabric Meta has loaders for them.

### Test required to close

1. Unit/contract test that every Minecraft/Fabric tuple exposed by Desktop satisfies the selected adapter metadata.
2. Creation/import test that an authoritative-provider-valid but adapter-incompatible tuple fails before any canonical world is published.
3. Live process matrix for each intentionally supported Minecraft line.

### Confidence

**High.** The contradiction is explicit between `fabric.mod.json` and the catalog/create validation rules.

---

## A5-03 — HIGH — Runtime-supervisor or IPC death can leave a writable Minecraft process outside checkpoint ownership

### Exact code

- `crates/swarm-cli/src/migration.rs`
  - `AuthorityRuntimeGuard`
  - `run_authority_runtime`
  - `run_authority_runtime_inner`
  - `launch_server`
  - `reset_runtime_directory`
- `crates/swarm-cli/src/authority_permit.rs`
  - `refresh_permit`
  - `PermitWatch`
- `crates/swarm-cli/src/daemon.rs`
  - authority permit refresh path
- `minecraft/fabric/src/main/java/dev/swarmcraft/fabric/SwarmCraftMod.java`
  - `Bridge.readerLoop`
  - `PermitGuard`
- `apps/desktop/src-tauri/src/runtime.rs`
  - `RuntimeProcesses`
  - `spawn`
  - `Drop`

### Required invariant

There must never be a writable Minecraft authority runtime that has lost the process/controller responsible for save barriers and canonical checkpointing while another launcher is allowed to claim the same local runtime slot.

### Failure scenario

The implementation has two separate liveness mechanisms:

1. `AuthorityRuntimeGuard` is an OS file lock held by the Rust runtime supervisor process.
2. `authority.permit` is refreshed by the separate SwarmCraft daemon and consumed by the Fabric `PermitGuard`.

`launch_server` uses `std::process::Command::spawn` to create Java. No audited code establishes an OS parent-death guarantee for the Java process. If the Rust runtime supervisor is killed/crashes, its file descriptor closes and `AuthorityRuntimeGuard` is released, but the Java child is not proven dead.

The Fabric bridge does not fail closed when its controller IPC disappears. `Bridge.readerLoop` exits on EOF and only logs an `IOException` when one is thrown; it does not initiate `saveEverything` + server stop merely because the authenticated control connection is gone.

Meanwhile the daemon can continue refreshing `authority.permit`, so the Fabric `PermitGuard` can continue considering the orphan server authorized. The result can be a live Minecraft authority server that players continue changing even though no process owns its checkpoint session.

A later `swarmcraft-runtime launch` can acquire the now-free `AuthorityRuntimeGuard` and immediately call `reset_runtime_directory(runtime)`, which removes and recreates `runtime/<world>` before restore. At minimum, post-supervisor player progress on the orphan runtime can be discarded rather than checkpointed. Depending on OS file semantics and server-port timing, the old process can also interfere with the new runtime reset/launch.

### Why normal stop fencing does not cover this

The normal stop path is good: it asks the live Fabric session to save/stop, waits for child exit, then checkpoints. The problem is that hard supervisor death bypasses the Rust code that owns that sequence.

The authority permit does not substitute for supervisor liveness because the daemon, not the runtime supervisor, writes the permit heartbeat.

### Reproduction

A closure test should explicitly exercise the mechanism:

1. Start daemon and managed authority runtime; wait for `Ready`.
2. Record the Java PID and runtime-supervisor PID.
3. Kill only the runtime supervisor with an uncatchable termination while leaving daemon running.
4. Verify whether Java remains alive beyond the permit timeout and continues accepting world mutations.
5. Start a new managed runtime for the same world.
6. Current file-lock ownership model allows the new supervisor to acquire the authority-runtime lock because the original lock owner died; it then reaches `reset_runtime_directory` without an old-Java death proof.

This exact process-death scenario is not present in the live acceptance workflow.

### Existing coverage

Positive:

- Single-flight file lock prevents concurrent Desktop/CLI/daemon launches while the owning supervisor is alive.
- Fabric permit freshness fences loss of the authority/quorum heartbeat.
- Startup/handshake errors explicitly terminate the Java child.
- Normal runtime errors after `wait_for_runtime_exit` terminate the child.

Missing:

- hard runtime-supervisor death while Java remains running;
- controller IPC EOF while daemon permit remains fresh;
- Desktop/backend crash while a world is live;
- proof that Java dies before the launch lock becomes reusable.

### Remediation

Controller liveness must be part of the authority runtime fence.

Recommended layered fix:

1. Make Fabric treat authenticated controller IPC EOF/loss as a bounded fail-closed condition: execute a server-thread save barrier and stop/exit unless a new authenticated controller session is explicitly and safely re-established.
2. Add a supervisor heartbeat/lease distinct from the daemon's authority permit. A server must require **both** current authority permission and current controller ownership.
3. Before deleting/recreating `runtime/<world>`, prove that any previous Java runtime is gone. Do not rely solely on the supervisor's advisory file lock.
4. Add platform process containment where practical (Linux parent-death/process-group strategy, Windows job object, macOS equivalent) as defense in depth, while preserving the Fabric save-first shutdown path for recoverable failures.

### Test required to close

A process-level chaos test that kills the runtime supervisor while the daemon remains alive, then proves all of the following before a replacement launch may reset the runtime directory:

- old Java is fenced and exits;
- no new canonical checkpoint is manufactured from a live/torn directory;
- replacement launch cannot race the old process;
- loss/recovery behavior is visible in migration status;
- canonical snapshot/history remains unchanged until a valid subsequent checkpoint.

### Confidence

**High on the source-level mechanism; medium-high on exact OS manifestation.** The separation of lock owner, Java child, IPC behavior, and daemon permit writer is explicit. A dedicated process-chaos reproduction is still required to characterize each supported platform.

---

## A5-04 — MEDIUM — Desktop discards Minecraft/Fabric runtime stdout and stderr

### Exact code

- `apps/desktop/src-tauri/src/runtime.rs`
  - `spawn`
- `crates/swarm-cli/src/migration.rs`
  - `launch_server`

### Required property

A failed player launch should retain enough process output to diagnose Minecraft/Fabric startup, mod-loader, JVM, port, or world-load failures.

### Evidence

`launch_server` spawns Java without redirecting stdout/stderr, so it inherits the `swarmcraft-runtime` sidecar's streams. Desktop `runtime::spawn` receives Tauri shell `CommandEvent`s but the event loop only reacts to `CommandEvent::Terminated(_)`. Stdout/stderr events are otherwise consumed and discarded; they are not persisted to a per-world log and are not emitted to the frontend.

This does not corrupt state, but it turns many actionable failures into opaque runtime termination or migration-status errors. Fabric Loader dependency failures, mod exceptions, port-binding failures, JVM diagnostics, and Minecraft crash context are precisely the information players/support need after a launch fails.

### Existing coverage

The live acceptance workflow redirects `swarmcraft-runtime` stdout/stderr into files, so CI retains evidence. That behavior does not prove Desktop retains equivalent logs.

### Remediation

Persist bounded per-world launch logs (with rotation/size limits) and surface the latest log path/summary through Desktop. Alternatively emit sanitized stdout/stderr events to the frontend while also retaining a bounded local log. Do not log IPC secrets or secret-bearing environment variables.

### Test required to close

Force a deterministic Fabric/JVM launch error from Desktop and assert that:

- the player receives a specific failure state;
- stderr/stdout is retained in a bounded log;
- a usable log reference is exposed to the UI;
- secret values are absent.

### Confidence

**High.** Event handling in the audited Desktop process manager is explicit.

---

# Positive controls confirmed

## Managed Java and platform selection

- Runtime Installer derives the required Java major from Mojang version metadata rather than guessing when authoritative metadata is available.
- A system Java is accepted only when its major exactly matches the Minecraft requirement; otherwise managed Java is resolved.
- Adoptium architecture mapping covers `x86_64`, `aarch64`, and `x86`; unsupported architectures fail closed.
- Managed Java packages carry provider SHA-256 and the installed executable is probed for the required major before publication.
- Managed Java is extracted to staging and then swapped into the managed runtime location; the previous runtime is preserved until the new one is ready.
- Exact-SHA live acceptance intentionally removes the setup JDK from `PATH` and verifies that managed Java was actually used.

## Runtime artifact integrity

- Mojang server SHA-1 is verified and SwarmCraft Fabric release assets are paired with SHA-256 checksums.
- Runtime lock records SHA-256 for acquired artifacts and `inspect` re-hashes source/staged artifacts.
- The staged Minecraft server JAR is rechecked against runtime-lock SHA-256 before it is seeded into Fabric's runtime cache.
- Runtime repair forces reacquisition/overwrite of managed artifacts.

Provider-origin trust and first-acquisition policy for every artifact belong primarily to Auditor 6; this report only credits the lifecycle checks actually consumed before launch.

## Runtime directory and mod staging

- Every authority launch calls `reset_runtime_directory` before restore, removing stale runtime files.
- The SwarmCraft bridge is staged fresh.
- User server mods are evaluated against the canonical compatibility manifest before launch.
- `install_verified_user_mods` stages only required, exact-hash user mods into the fresh runtime directory.
- Unexpected mods in the machine-local profile are detected and make readiness fail.

## EULA

- Runtime configuration refuses to become launchable without explicit `accept_eula`.
- The managed launch path independently rejects `accept_eula == false`.
- `eula.txt` is written only into the freshly prepared machine-local runtime after that explicit acceptance.
- Import deliberately does not carry EULA state.
- Live acceptance proves refusal, no launch after refusal, later acceptance, and successful retry.

## Launch and live handshake

- Runtime is restored from a verified signed canonical snapshot before Java launch.
- Java/Fabric starts only after authority checks.
- Fabric IPC is bound locally and authenticated with a launch token.
- Readiness is withheld until Fabric reports exact Minecraft version, Fabric loader version, world directory, and compatibility fingerprint.
- Startup timeout, handshake mismatch, or authority supersession terminates Java before returning an error.

## Safe stop and checkpoint ordering

For managed runtime stop/transfer/CLI shutdown:

1. Fabric receives `PREPARE_SHUTDOWN` on authenticated IPC.
2. `SwarmCraftMod` schedules `saveEverything` on the server thread.
3. Fabric reports `READY_FOR_SHUTDOWN` only after the save call succeeds.
4. Rust waits for Minecraft process exit.
5. Rust rechecks authority generation.
6. Only then does storage snapshot the runtime world, sign it, and commit it.

If the normal safe-stop save barrier fails, the stop intent is cleared and Minecraft is kept running. Desktop does not force-kill it and returns an explicit timeout/failure instead. That is the correct fail-closed direction for data integrity.

## Concurrent launch while supervisor is healthy

- `AuthorityRuntimeGuard` uses an exclusive file lock per world.
- Explicit concurrent launches fail fast.
- Background supervisor contention retries silently rather than resetting a live runtime.

This is good protection while the lock-owning supervisor process remains alive; A5-03 covers the hard-death gap.

---

# Failure table

| Scenario | Current behavior | Data-safety assessment | Verdict |
|---|---|---|---|
| No compatible system Java | Resolve managed Adoptium Java for Mojang-required major; verify checksum and major | Fail closed on resolution/extraction/probe failure | PASS |
| Wrong/corrupt managed Java | Probe major; repair can force reinstall | Launch is not marked ready | PASS |
| Corrupt managed Minecraft/Fabric/bridge artifact | Runtime-lock hash checks detect corruption for managed staged artifacts | Repair/reacquire required | PASS |
| EULA not accepted | No launch config; launch path rejects | Explicit player acceptance required | PASS |
| Normal create/install/launch on 26.1.2 + loader 0.19.3 | Exact-SHA live workflow succeeds | Real Minecraft/Fabric proof exists on Ubuntu | PASS |
| Catalog-valid Minecraft outside adapter `~26.1.2` | Canonical creation can succeed before bridge incompatibility is detected at launch | Player can create a canonically stranded/unlaunchable runtime profile | **FAIL — A5-02** |
| Fabric loader below bridge `>=0.19.3` but provider-valid | Catalog validation does not enforce bridge floor | Fabric can reject bridge after world creation | **FAIL — A5-02** |
| Stale runtime files from prior launch | Entire `runtime/<world>` directory is reset before restore | Stale runtime JAR/data not intentionally reused | PASS |
| Missing/wrong canonical user server mod | Readiness fails before host launch | Fail closed | PASS |
| Unexpected extra user mod | Marked unexpected and will not be launched; readiness fails | Fail closed | PASS |
| Two launch attempts while first supervisor is alive | Exclusive `AuthorityRuntimeGuard` prevents second owner | Avoids runtime reset race | PASS |
| Runtime supervisor hard-crashes while Java survives | Supervisor lock is released; Fabric IPC EOF does not stop server; daemon permit may stay fresh | Java can remain writable outside checkpoint ownership | **FAIL — A5-03** |
| Fabric controller IPC disconnects while server remains alive | Java bridge reader exits/logs but does not stop solely because controller vanished | No checkpoint controller remains | **FAIL — A5-03** |
| Save barrier fails during normal stop | Minecraft kept running; no checkpoint | Safe but requires recovery/retry | PASS |
| Server fails before/while normal save barrier | IPC/child error prevents new checkpoint | Fail closed with respect to canonical snapshot | PASS |
| Server exits unsuccessfully without requested stop | Runtime returns error; no final checkpoint | Canonical state stays at prior verified snapshot | PASS |
| Server exits successfully on its own | Treated as sleep disposition and stopped files are checkpointed | Process is no longer mutating source | PASS, with process-level behavior worth retaining in tests |
| Authority changes during startup | Child terminated; status superseded/error | Old generation cannot proceed to checkpoint | PASS |
| Authority changes before final checkpoint | `ensure_authority_generation` fails before commit path | Prevents stale-authority checkpoint | PASS |
| Safe manual transfer | Save/stop first; final snapshot becomes transfer base | Correct ordering in source | PASS (process-level multi-peer acceptance belongs partly to Auditors 2/10) |
| Wake from durable sleep | Exact signed sleep record/latest snapshot alignment checked; multi-member wake does not auto-solo | Fail closed on stale/ambiguous state | PASS in reviewed path |
| Existing-world import from stopped source | Staged, signed, verified, atomically published; source untouched | Good once source is actually quiescent | PASS |
| Existing-world import while Minecraft is running | No live-world lock/save barrier before `snapshot_directory` | Can canonize a torn save | **FAIL — A5-01** |
| Runtime repair | Force path reacquires/replaces managed artifacts and re-verifies | Good for managed corruption | PASS |
| Desktop launch failure diagnostics | Sidecar stdout/stderr events are consumed but not retained/surfaced | State safety unaffected; supportability degraded | **FAIL — A5-04** |

---

# Acceptance coverage versus required lifecycle

| Required journey step | Evidence | Classification |
|---|---|---|
| Create | Real CLI creation in live workflow; Desktop canonical path inspected | PROVEN for audited tuple |
| Install | Official-service managed runtime install in clean data dir | PROVEN on Ubuntu for audited tuple |
| Launch | Real Java/Minecraft/Fabric process and authenticated bridge | PROVEN on Ubuntu for audited tuple |
| Play/mutate | Live test verifies `level.dat`; test marker is written into runtime world | PARTIALLY PROVEN; marker mutation is process-level persistence evidence but not a real client session |
| Checkpoint | Safe stop advances canonical snapshot and `world verify` passes | PROVEN for audited tuple |
| Stop | Fabric save barrier + process exit + Sleeping status in live test | PROVEN for audited tuple |
| Restart backend/runtime tooling after a clean stop | Second independent runtime CLI launch restores marker | PROVEN after stop |
| Restart backend **while server remains running** | No exact-SHA process test; source exposes A5-03 supervisor-death gap | **BROKEN/UNPROVEN** |
| Restore | Second real launch restores marker from canonical snapshot | PROVEN for audited tuple |
| Relaunch | Second real Minecraft/Fabric launch succeeds | PROVEN for audited tuple |
| Migrate | Source ordering is strong; no exact-SHA two-peer real-Minecraft migration acceptance found in the live gate | PARTIALLY PROVEN |
| Recover | Authority/wake source and tests exist; no exact-SHA real-Minecraft authority-loss process test in the live gate | PARTIALLY PROVEN |
| Import | Static/import fault tests prove atomic publication; no live-Minecraft source lock | **BROKEN for live source — A5-01** |

---

# Test/evidence blind spots

These are not independently classified as product bugs, but they materially limit confidence beyond the exact happy-path tuple:

1. The real Minecraft live workflow runs only on `ubuntu-latest`.
2. It tests only Minecraft `26.1.2` + Fabric Loader `0.19.3`.
3. It stops the first runtime before the second launch; it does not kill/restart the runtime supervisor while Java remains live.
4. It does not exercise Desktop process ownership itself; it runs CLI/runtime binaries directly.
5. It does not exercise a real two-peer manual migration with Minecraft running on source and successor.
6. It does not exercise automatic recovery after unexpected authority/runtime process death.
7. It does not import a source world held open by real Minecraft.
8. It does not force a server crash during `saveEverything` or immediately after the save barrier.
9. It does not assert persistence/surfacing of Java/Fabric logs through Desktop.
10. Windows and macOS installer success does not, by itself, prove native managed-Java extraction and real Minecraft lifecycle behavior on those platforms.

---

# Recommended remediation order

1. **A5-01 first:** make import refuse live source worlds and hold the quiescence proof throughout snapshot input. This closes the only direct path found that can sign a semantically torn Minecraft save as canonical.
2. **A5-03 second:** bind Java writability to controller liveness as well as authority permit, then add hard supervisor-death chaos coverage before allowing runtime-directory reset/relaunch.
3. **A5-02 third:** create and enforce one adapter support matrix across catalog, canonical create, import, runtime plan/install, and live validation. Do not advertise provider-valid tuples that the shipped bridge refuses.
4. **A5-04 fourth:** preserve bounded Desktop runtime logs and surface actionable launch diagnostics.
5. Expand real-Minecraft acceptance into an intentional support matrix and add process-death/migration scenarios after the three safety fixes.

---

# Closure criteria for this audit

Re-audit Auditor 5 after production fixes only when all of the following exist on the same exact head:

- live-source import rejection while a real Minecraft source holds its session lock;
- successful import after that source is cleanly stopped;
- catalog/create/import tests proving every player-selectable tuple is inside the shipped adapter support contract;
- real launch tests for every intentionally supported Minecraft line;
- a chaos test that kills the runtime supervisor while daemon and Java are live and proves Java cannot remain writable outside controller ownership;
- proof that a replacement launcher cannot reset/reuse a runtime directory until the old Java process is fenced/stopped;
- Desktop launch logs retained/surfaced for a deterministic Fabric/JVM failure;
- exact-head real Minecraft safe stop → checkpoint → restore → relaunch still green.

# Final verdict

**VERDICT: FAIL**
