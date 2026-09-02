# Auditor 10 — Adversarial Whole-Product Acceptance

Repository: `MousaXD/swarmcraft`  
Audit branch: `audit/adversarial-e2e`  
Audited baseline: `354be3b1066428ecab6987590b7c7dbd80fe0870`  
Baseline verification: live remote `main` was verified at this exact SHA before the audit began.  
Production-code changes: none.

## Executive verdict

SwarmCraft does **not** satisfy the literal two-player acceptance journey required by this audit.

The strongest exact-head evidence is genuinely good: the main-push `Player journey live acceptance` workflow run `33576322489` completed successfully on the audited SHA and exercised a clean data directory, explicit EULA handling, managed Java installation, official Minecraft/Fabric runtime installation, a real Minecraft/Fabric launch, safe checkpoint/sleep, backend/process restart, restore, relaunch, and a second monotonic checkpoint. The exact-head main CI process-level acceptance job `100080968583` also completed successfully and covered hard reconnect, storage failure injection, live join replication, runtime hardening, three-daemon recovery, successor-death recovery, and solo divergence detection.

However, those green gates do not prove, and in two places the implementation explicitly prevents, the required Alice/Bob failure sequence:

1. After Alice and Bob become the two voting members, hard-killing Alice leaves Bob without a majority. `host_readiness::two_member_successor_requires_explicit_handoff` intentionally returns `BlockedByQuorum`. Bob cannot automatically become canonical authority. This makes required journey step 20 **BROKEN**.
2. A multi-member world that has safely checkpointed into a durable sleeping state has no sleep-bound quorum wake election yet. The CLI and migration path intentionally fail closed. This independently prevents the required post-checkpoint restart/restore/relaunch path for the two-member world.

These are safety-preserving choices, not canonical-history corruption bugs. They are nevertheless product-acceptance failures against this audit contract.

## Evidence model

I credited evidence only at the layer it actually exercises.

- **PROVEN**: an exact-head test/workflow exercises the material behavior at the claimed layer.
- **PARTIALLY PROVEN**: strong component/process evidence exists, but not the complete real-player path required by the step.
- **UNPROVEN**: implementation may exist, but the inspected exact-head evidence does not demonstrate the required behavior.
- **BROKEN**: source/tests establish that the required behavior cannot complete as specified.

Important evidence anchors:

- `.github/workflows/player-journey-live.yml`
- `scripts/acceptance/clean-machine-live.sh`
- exact-head main run `33576322489` — `Player journey live acceptance` — success
- `.github/workflows/ci.yml`
- exact-head main CI run `33576322543`, process-level job `100080968583` — success
- `crates/swarm-cli/tests/automatic_invite_join.rs`
- `crates/swarm-cli/tests/live_join_replication.rs`
- `crates/swarm-cli/src/host_readiness.rs`
- `crates/swarm-cli/tests/three_daemon_recovery.rs`
- `crates/swarm-cli/tests/recovery_successor_dies.rs`
- `crates/swarm-cli/src/launch_guard.rs`
- `crates/swarm-cli/src/migration.rs`
- `crates/swarm-cli/src/provider_runtime.rs`
- `apps/desktop/src-tauri/src/canonical_commands.rs`
- `apps/desktop/tests/catalog-selectors.test.mjs`
- `apps/desktop/tests/launcher-controller.test.mjs`
- `crates/swarm-cli/tests/runtime_setup_hardening.rs`
- `crates/swarm-storage/tests/snapshot_swarm_acceptance.rs`
- `crates/swarm-storage/tests/failure_injection.rs`
- `crates/swarm-consensus/tests/solo_history_acceptance.rs`
- `docs/FINAL_PLAYER_JOURNEY_ACCEPTANCE.md`
- `docs/RELEASE_GATES.md`

## Findings

### A10-01 — HIGH — Required two-player crash failover cannot elect the sole survivor

**Affected journey:** steps 19–21, with downstream impact on steps 22–26.

**Evidence:**

- `crates/swarm-cli/src/host_readiness.rs` contains deterministic test `two_member_successor_requires_explicit_handoff` and asserts `HostReadinessState::BlockedByQuorum` for the surviving peer of a two-member world.
- The exact-head main CI acceptance job explicitly runs `Host Readiness negative matrix and two-member quorum fail-closed`, and that step passed.
- `docs/FINAL_PLAYER_JOURNEY_ACCEPTANCE.md` explicitly records Alice/Bob two-voter crash failover as yellow because quorum is 2 and Bob alone cannot safely elect himself.
- `three_daemon_recovery.rs` proves hard-kill recovery only when a surviving majority exists. That is the correct safety behavior, but it is a different topology from the required two-player journey.

**Adversarial reproduction:**

1. Alice is current authority.
2. Bob joins, producing two non-banned authority-eligible voters.
3. Alice disappears without performing a manual authority transfer.
4. Bob is the only surviving voter.
5. Recovery quorum cannot be formed; Bob remains fenced from canonical writes.

**Product impact:** the literal promise “two real players, kill Player A authority unexpectedly, continue on successor” does not complete. The world fails closed rather than splitting brain, but Player B cannot continue as required.

**Remediation direction:** do **not** weaken quorum to one-of-two. The product contract must either require a third voting replica for automatic crash recovery, or define and implement a separately safe two-party recovery mechanism with an external/failure proof strong enough to preserve single canonical history. If the intended product behavior is explicit handoff only for two voters, the player journey and acceptance contract must say that plainly.

**Closure test:** a real/process-level two-player acceptance test matching the intended safe semantics. If automatic crash failover remains intentionally impossible with two voters, this Auditor 10 journey must be revised rather than falsely marked green.

### A10-02 — HIGH — Multi-member sleeping worlds cannot complete safe restart/wake

**Affected journey:** steps 22–25.

**Evidence:**

- `docs/RELEASE_GATES.md` states that no dedicated sleep-bound quorum wake election exists and multi-member sleeping worlds remain fail-closed.
- `crates/swarm-cli/src/main.rs` describes `world wake` with: multi-member worlds remain blocked until a quorum transition exists.
- `crates/swarm-cli/src/migration.rs` explicitly rejects direct launch of a multi-member sleeping world and requires the migration supervisor; the product documentation records that the corresponding safe quorum wake transition does not yet exist.
- `launch_guard::load_sleep_record_fail_closed` correctly treats signed sleep state as a security boundary, so bypassing it is not an acceptable workaround.

**Product impact:** even if authority continuity were arranged before shutdown, a two-member world that safely checkpoints/sleeps cannot yet execute the required backend restart → restore → relaunch journey through a safe multi-member wake protocol.

**Remediation direction:** implement a sleep-record-bound quorum wake election whose proof commits to the durable sleep record/canonical snapshot, advances authority generation/fencing safely, and rejects stale/competing wake attempts. Do not reuse ordinary crash recovery without binding it to the sleeping canonical state.

**Closure test:** at least three process identities should safely sleep a canonical world, stop all hosts, restart backends, elect exactly one wake authority using a quorum bound to the sleep record, restore the exact snapshot, relaunch, and prove stale peers remain fenced. A two-voter policy must also be explicitly specified.

### A10-03 — MEDIUM — No single gate proves the real two-player Minecraft + provider + recovery journey

This is an evidence gap distinct from A10-01/A10-02.

**Evidence split:**

- `clean-machine-live.sh` is a strong **real Minecraft/Fabric** gate, but it is one machine/one identity and creates a private world with hard-coded Minecraft/Fabric versions and no selected Modrinth/CurseForge package set.
- `automatic_invite_join.rs` and `live_join_replication.rs` are strong **two-peer network/membership/snapshot** gates, but their world payloads are synthetic files rather than a live Minecraft server lifecycle.
- `three_daemon_recovery.rs` is a strong **process/network/authority recovery** gate, but runtime behavior uses a mock Java/Fabric IPC fixture and requires three voters.
- Provider canonicalization/runtime reacquisition is covered by unit/integration code/tests, but no inspected gate composes those exact provider artifacts into Bob’s joined real-Minecraft runtime after replication.

**Product impact:** even after A10-01/A10-02 are fixed or the topology contract changes, the complete product promise could still regress across seams without a composed acceptance gate.

**Remediation direction:** add one release-blocking multi-process acceptance scenario that uses actual CLI/Desktop contracts, two or preferably three clean data roots, one real Minecraft/Fabric server at a time, a deterministic provider fixture or pinned permitted artifacts, signed invite/join, exact artifact reacquisition, authority transition, checkpoint, restart, restore, and relaunch.

## Required journey classification

| Step | Required behavior | Classification | Evidence / reason |
|---:|---|---|---|
| 1 | Player A clean machine/state | **PROVEN** | `clean-machine-live.sh` asserts an empty fresh data directory before initialization. |
| 2 | Create identity | **PROVEN** | The live script executes `swarmcraft init` in the fresh data root. |
| 3 | Choose Minecraft version | **PARTIALLY PROVEN** | Desktop selector tests verify authoritative selector behavior and invalidation, but the real live gate hard-codes `26.1.2` rather than exercising player selection. |
| 4 | Choose compatible Fabric | **PARTIALLY PROVEN** | Desktop catalog tests reject mismatched Minecraft/Fabric responses and prefer stable loaders; the live gate hard-codes `0.19.3`. |
| 5 | Choose Modrinth/CurseForge mods | **PARTIALLY PROVEN** | Desktop/provider code and launcher-controller tests cover exact provider provenance/dependencies; the real live journey does not select provider mods. |
| 6 | Create canonical world | **PARTIALLY PROVEN** | Real CLI world creation and canonical snapshot lifecycle are proven, and canonical modpack construction is tested separately; not proven as one live provider-populated creation flow. |
| 7 | Install runtime | **PROVEN** | Live gate requires managed Java and installs/verifies the runtime. |
| 8 | Explicitly handle EULA | **PROVEN** | Live gate proves refusal does not make runtime ready, then explicitly accepts EULA and verifies readiness. |
| 9 | Launch real Minecraft/Fabric server | **PROVEN** | Exact-main live gate launches the candidate Fabric artifact with real Minecraft and requires a usable `level.dat`. |
| 10 | Checkpoint safely | **PROVEN** | Live gate requests safe stop, waits for sleeping state, verifies world, and requires monotonic snapshot advance. |
| 11 | Generate invite without manual bootstrap | **PROVEN** | `automatic_invite_join.rs` seeds ordinary connectivity diagnostics then calls `invite create` with no `--bootstrap`; token contains the signed address internally. |
| 12 | Player B clean identity/state | **PARTIALLY PROVEN** | Two-peer tests use an independent fresh temp data root/identity, but do not exercise the complete second-player Desktop first-launch path. |
| 13 | Consume invite | **PROVEN** | `automatic_invite_join.rs` calls the real CLI `world join` with the signed token. |
| 14 | Join canonical membership | **PROVEN** | Two real daemon processes converge on membership sequence 2 containing Bob. |
| 15 | Receive exact signed state | **PROVEN** | Join tests require Bob’s manifest hash/state root/sequence to equal the authority snapshot and verify the replica. |
| 16 | Acquire exact permitted provider artifacts | **PARTIALLY PROVEN** | `provider_runtime.rs` reacquires frozen provider identities and verifies canonical hashes/size; no joined-Bob live provider acquisition gate was found. |
| 17 | Install Player B runtime | **PARTIALLY PROVEN** | Runtime installation is real on the solo clean-machine path, not on the joined replica with provider artifacts. |
| 18 | Launch compatible Player B world | **PARTIALLY PROVEN** | Runtime compatibility fail-closed tests and real solo launch exist, but the joined-Bob real Minecraft launch is not composed in one gate. |
| 19 | Kill Player A authority unexpectedly | **PARTIALLY PROVEN** | Hard-kill is exercised in three-daemon recovery, not in the literal two-voter real-Minecraft journey. |
| 20 | Verify migration/recovery | **BROKEN** | Two-voter survivor is intentionally `BlockedByQuorum`; Bob alone cannot recover canonical authority. |
| 21 | Continue on successor | **BROKEN** | No successor can be canonically promoted in the required two-player crash topology. |
| 22 | Checkpoint on successor | **UNPROVEN** | Required successor from step 21 does not exist; three-daemon fixture coverage is not the same product path. |
| 23 | Restart backend | **PARTIALLY PROVEN** | Real solo backend/process restart is proven; required post-failover multi-member path cannot be reached. |
| 24 | Restore | **PARTIALLY PROVEN** | Real solo restore/relaunch and storage replica restoration are proven separately; required two-player successor path cannot be reached. |
| 25 | Relaunch | **BROKEN** | In addition to step-20 failure, safe multi-member sleeping-world wake is explicitly not implemented. |
| 26 | Canonical identity/history unchanged | **PARTIALLY PROVEN** | Three-/five-daemon recovery and solo divergence tests strongly protect history, but the literal required two-player sequence cannot reach this assertion. |

## Adversarial variant matrix

| Variant | Classification | Evidence / observation |
|---|---|---|
| Wrong Minecraft version | **PROVEN** | `runtime_setup_hardening.rs` rejects an incompatible Fabric-reported Minecraft version before Ready and preserves canonical snapshot. |
| Wrong Fabric version | **PARTIALLY PROVEN** | Catalog selection is compatibility-bound and runtime handshake validates compatibility; inspected hardening evidence explicitly demonstrates wrong Minecraft, not a separately named wrong-loader process case. |
| Modified mod JAR | **PROVEN** | `canonical_commands.rs` has `provider_hash_mismatch_fails_closed`; runtime acquisition re-verifies canonical SHA hashes before publication/use. |
| Provider unavailable | **PARTIALLY PROVEN** | Runtime acquisition propagates exact-provider errors and has no silent substitute path; no live outage gate was inspected. |
| Restricted CurseForge artifact | **PARTIALLY PROVEN** | `provider_runtime.rs` treats manual-required and 403/404 exact files as fail-closed with exact-artifact remediation; no credentialed live restricted-file gate was inspected. |
| Corrupt snapshot | **PROVEN** | Exact-main process acceptance runs storage reconstruction/failure injection; corrupt encoded blobs are rejected and clean retry from another replica is exercised. |
| Stale peer | **PROVEN** | Three-daemon recovery restarts stale Alice and requires adoption of epoch/fencing 2 while denying stale authority permit. |
| Duplicate invite | **UNPROVEN** | Signature/expiry and membership convergence are covered, but no explicit one-time nonce/replayed-valid-invite acceptance test was found in this audit. |
| Expired/stale invite | **PROVEN** | Invite consume/daemon paths compare `expires_unix_ms` to current time and reject expired invites; source contains expiry tests. |
| Private world lookup | **PARTIALLY PROVEN** | Core discovery rejects private announcements and protocol tests mark private visibility undiscoverable; no full hostile distributed lookup gate was inspected. |
| Unlisted browse | **PARTIALLY PROVEN** | CLI/core discovery filtering distinguishes public from unlisted/private; no full hostile distributed browse gate was inspected. |
| Network interruption during join | **PARTIALLY PROVEN** | Exact-main network impairment gate proves interrupted QUIC transfer resume under loss, while join replication is separately process-tested; interruption during the membership transition itself is not composed. |
| Authority dies during checkpoint | **UNPROVEN** | Hard authority death and checkpoint/storage failure are independently tested, but no inspected gate kills the active authority inside the real save/checkpoint critical section. |
| Successor dies after election | **PROVEN** for pre-promotion recovery | `recovery_successor_dies.rs` kills the first certified successor before epoch promotion and proves a newer recovery round elects another successor; death after full runtime promotion remains outside that exact test. |
| Runtime install interrupted | **PARTIALLY PROVEN** | Runtime hardening proves partial setup failure can retry cleanly; no inspected process-kill-mid-download/install gate. |
| Disk failure | **PARTIALLY PROVEN** | Storage failure injection covers malformed/truncated data, filesystem-shape IO failures and safe restore boundaries, but not a true ENOSPC device-level run. |
| Backend restart | **PROVEN** for solo | Real live workflow restarts processes using persisted runtime configuration and relaunches successfully. |
| Conflicting local world files/history | **PARTIALLY PROVEN** | `solo_history_acceptance.rs` detects competing canonical solo histories as conflict instead of merging; arbitrary live Minecraft local-file mutation is not a complete equivalent. |

## Positive controls confirmed

The failed product verdict should not obscure controls that behaved correctly:

- Exact-main real Minecraft/Fabric clean-machine acceptance is green.
- EULA refusal is fail-closed and retry-safe.
- Managed Java is actually required by the live gate rather than inherited accidentally from the build JVM.
- Safe stop advances the canonical snapshot and persists data across relaunch.
- Automatic invite creation can use backend diagnostics without exposing a manual bootstrap setup step.
- Signed join/membership and exact snapshot replication work between two real daemon processes.
- Exact provider artifact hashes are frozen and reverified; restricted/manual artifacts do not silently substitute another file.
- Corrupt replica blobs are rejected and reconstruction can continue from surviving replicas.
- Three-voter recovery advances epoch/fencing and fences stale Alice.
- A five-voter successor-death scenario can advance to a later recovery round and fence stale candidates.
- Two-voter recovery fails closed instead of manufacturing quorum.
- Multi-member sleep fails closed instead of using unsafe first-click-wins wake semantics.

## Evidence limitations

This audit did not manufacture a new local two-machine Minecraft environment. It relied on source inspection plus exact-head GitHub Actions evidence that executed the relevant real/process-level gates on the audited SHA. That is sufficient to mark A10-01 and A10-02 **BROKEN** because the code/tests explicitly establish the unsupported behavior. It is not sufficient to upgrade the seam-level Player B/provider/live-Minecraft steps from **PARTIALLY PROVEN** to **PROVEN**.

The overall verdict is **FAIL**, not **INCONCLUSIVE**, because at least one mandatory journey step is demonstrably impossible under the specified two-player topology, and a second mandatory post-checkpoint behavior is explicitly not implemented for multi-member sleeping worlds.

PRODUCT ACCEPTANCE: FAIL