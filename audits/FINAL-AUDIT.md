# SwarmCraft Final Audit

Repository: `MousaXD/swarmcraft`

Final integration branch: `audit/final-integration-report`

Exact audited SHA: `354be3b1066428ecab6987590b7c7dbd80fe0870`

Audit date: 2026-09-02

## 1. Executive verdict

All eleven required auditor reports were available remotely and all audited the exact same baseline SHA. None was rejected for tree mismatch.

The audited product **fails** the final assessment. The strongest reasons are not cosmetic or evidence-only gaps:

1. two independent consensus paths can create two writable authorities and accepted divergent history;
2. a provider-controlled CurseForge filename can escape the intended staging root and create a new JAR at an attacker-chosen writable path;
3. application-peer authentication is replayable across transport connections and can disclose private-world contents;
4. several canonical state handlers accept semantically unauthorized or non-direct history updates;
5. storage can silently roll back after loss of the newest manifest and can replace a committed snapshot number;
6. the Desktop launcher enhancement layer throws during initialization, disconnecting core Create-with-mods and discovery journeys;
7. real two-player crash failover and multi-member sleep/wake do not satisfy the required product journey;
8. release publication is not gated on the same SHA having passed the validation suite.

The audited SHA is suitable for **continued development** and tightly controlled engineering testing. It is **not suitable for wider public testing or production-like use**. Alpha testing should be restricted to disposable/non-sensitive worlds with explicit awareness that multi-peer authority safety, private-world confidentiality, provider staging, and several recovery paths are not safe.

### Post-audit `main` movement

During final synthesis, `main` advanced to `b4bab08562cf0eb53763674407375b023e1d0858` through PR #59 (`Release SwarmCraft 0.5.0`). The final audit branch remains correctly based on the required audited SHA.

A direct compare from the audited SHA to `b4bab085...` shows only:

- `Cargo.toml`
- `Cargo.lock`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/Cargo.lock`
- `apps/desktop/src-tauri/tauri.conf.json`
- `minecraft/fabric/gradle.properties`
- `scripts/check_release_version.py`

No production source path implicated by the confirmed CRITICAL/HIGH findings changed. Therefore the source-level defects below also apply to the current live `main` at the time this report was finalized, even though the authoritative audited tree remains `354be3b...`.

## 2. Auditor completion matrix

| Auditor | Branch | Baseline | Verdict | Accepted into synthesis |
| --- | --- | --- | --- | --- |
| 0 Repository Truth | `audit/repository-truth` | `354be3b...` | FAIL | YES |
| 1 Protocol/Core | `audit/protocol-core` | `354be3b...` | FAIL | YES |
| 2 Authority/Consensus | `audit/authority-consensus` | `354be3b...` | FAIL | YES |
| 3 Storage/Recovery | `audit/storage-recovery` | `354be3b...` | FAIL | YES |
| 4 Network/Discovery | `audit/network-discovery` | `354be3b...` | FAIL | YES |
| 5 Minecraft Runtime | `audit/runtime-minecraft` | `354be3b...` | FAIL | YES |
| 6 Package Supply Chain | `audit/package-supply-chain` | `354be3b...` | FAIL | YES |
| 7 Security | `audit/security` | `354be3b...` | FAIL | YES |
| 8 Desktop UX | `audit/desktop-ux` | `354be3b...` | FAIL | YES |
| 9 CI/Release | `audit/ci-release` | `354be3b...` | FAIL | YES |
| 10 Adversarial E2E | `audit/adversarial-e2e` | `354be3b...` | PRODUCT ACCEPTANCE: FAIL | YES |

## 3. CRITICAL findings

### FINAL-001 — CRITICAL — Membership changes can create two valid quorum universes

- **Affected:** `crates/swarm-cli/src/daemon.rs`, `crates/swarm-storage/src/world.rs`, `crates/swarm-consensus/src/lib.rs`
- **Auditors:** 2 (AC-01), related protocol evidence from 1 (APC-001)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** membership changes become locally active without a quorum/joint-consensus commit before they redefine the voter set. Recovery quorum size is derived from the local descriptor. Divergent old/new membership views can therefore each contain a local majority.
- **Reproduction:** begin with `{A,B,C}`, let only A learn additions D/E, then partition `A,D,E | B,C`. The first side has 3/5; the stale side has 2/3 and can recover on its stale membership generation.
- **Product impact:** two simultaneously writable canonical histories can be accepted under internally valid but incompatible quorum views.
- **Remediation:** make membership a consensus-controlled configuration generation with durable activation, joint quorum rules for voter-set changes, and canonical committed membership binding for leases/recovery.
- **Test required:** 5-peer membership-churn + 3/2 partition test proving only one side can ever obtain a write permit, including stale removed/banned voter cases.

### FINAL-002 — CRITICAL — Automatic Solo fallback can race majority recovery

- **Affected:** `crates/swarm-cli/src/daemon.rs`, world-config defaults, authority permit path
- **Auditors:** 2 (AC-02)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** when `allow_solo_advancement` is enabled, the current authority can promote locally to Solo after quorum loss and keep a write permit without quorum. The opposite partition can still form a majority on the prior epoch and certify recovery.
- **Reproduction:** synchronized 5-peer world, partition old authority with one peer against three peers. The old authority promotes Solo while the 3-peer majority recovers a different N+1 authority.
- **Product impact:** two writable branches from one base without malicious input.
- **Remediation:** forbid automatic writable Solo after unclean quorum loss in multi-member worlds. Restrict Solo to single-member or clean durable relinquishment cases, or explicitly classify Solo progress as non-canonical and unable to claim unique canonical authority.
- **Test required:** real 3- and 5-daemon partition campaign with Solo enabled, proving no concurrent permits.

### FINAL-003 — CRITICAL — CurseForge provider filename escapes staging root

- **Affected:** `apps/desktop/src/launcher-controller.js`, `apps/desktop/src-tauri/src/curseforge.rs`
- **Auditors:** 6 (SC6-01)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** provider `fileName` is concatenated into `${staging}/curseforge/<project>/<version>/<fileName>` and the resulting path is passed to the native `curseforge_download` command. Backend validation checks the final extension but does not prove containment within server-owned provider staging.
- **Reproduction:** malicious provider metadata uses traversal components ending in `.jar`; backend creates destination parents and publishes the verified provider file at the escaped path if it does not already exist.
- **Product impact:** provider-controlled creation of a JAR outside the intended staging boundary in user-writable filesystem locations.
- **Remediation:** accept only an opaque staging/session ID at the Tauri boundary; construct destination server-side; validate filename as exactly one normal component; reject separators, absolute/prefix/root, `.` and `..`; enforce normalized containment in backend.
- **Test required:** Linux/macOS/Windows traversal matrix including slash/backslash, absolute, drive, UNC, dot components, normalization variants.

## 4. HIGH findings

### FINAL-004 — HIGH — Canonical authorization records are not always bound to the current authority

- **Affected:** `crates/swarm-cli/src/daemon.rs`, membership/config persistence
- **Auditors:** 1 (APC-001, APC-002)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** a previous-epoch authority can advance membership during the designed epoch/membership delivery gap, and any non-banned member can sign the next valid hash-linked `WorldConfigV1` because receipt checks membership rather than accepted-authority identity.
- **Impact:** stale authority can mutate authorization after fencing; ordinary member can change authority/visibility/solo policy fields.
- **Remediation:** central current-epoch/current-authority semantic validator for all canonical control records, with exact direct-parent checks.
- **Test required:** stale old-authority membership replay and non-authority next-config integration tests.

### FINAL-005 — HIGH — Snapshot replication accepts detached/skipped/conflicting history

- **Affected:** `crates/swarm-cli/src/daemon.rs`, `crates/swarm-storage/src/replica.rs`, `streaming.rs`
- **Auditors:** 1 (APC-003), reinforced by 3 (SR-02/SR-05)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** live acceptance rejects only lower epoch/sequence, not non-direct parent, sequence jump, snapshot-number jump, or same-sequence conflicting manifest; final storage commit does not restore that history context.
- **Impact:** validly signed reordered/equivocating authority output can become the local canonical head.
- **Remediation:** idempotent exact duplicate only; otherwise exact direct extension of sequence, snapshot number, previous manifest hash and expected head, rechecked atomically at final commit.
- **Test required:** S5-before-S4, wrong-parent, same-sequence conflict, number overwrite/jump tests.

### FINAL-006 — HIGH — Recovery rounds are not value-preserving after a certificate exists

- **Affected:** `crates/swarm-storage/src/state.rs`, `crates/swarm-consensus/src/recovery.rs`, `crates/swarm-cli/src/daemon.rs`
- **Auditors:** 2 (AC-03)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** a higher recovery round on the same base may select a different candidate even after an earlier quorum certificate exists; the earlier candidate can later resume from its durable certificate and persist a conflicting same-generation epoch locally.
- **Impact:** conflicting accepted recovery generation and stranded replicas; weakens consensus safety/liveness proof.
- **Remediation:** accepted-value/certificate locking across rounds, Paxos/Raft-style value preservation or equivalent.
- **Test required:** pause candidate after certificate persistence, complete later round with another candidate, resume old candidate and prove no conflicting epoch is stored.

### FINAL-007 — HIGH — Recovery may elect a storage-only or runtime-unready host

- **Affected:** recovery candidate construction, host readiness/capability path
- **Auditors:** 2 (AC-04)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** election eligibility uses membership/world status but not the machine's exact runtime/server-mod/conflict readiness even though that capability is separately modeled.
- **Impact:** canonical authority can be installed on a peer unable to run the world, blocking automatic progress.
- **Remediation:** distinguish voter eligibility from host-candidate eligibility and require fresh authenticated exact host capability for automatic authority candidacy.
- **Test required:** deterministic lowest-ID survivor unready while another survivor is host-ready.

### FINAL-008 — HIGH — Duplicate local daemons can equivocate recovery/control state

- **Affected:** `Storage::promise_recovery_ballot`, control-state writes, daemon startup
- **Auditors:** 2 (AC-05), 3 (SR-04)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** read-check-write recovery promises are not serialized across processes and daemon startup lacks a data-root singleton lock. Two processes sharing one identity can both decide conflicting promises are acceptable and emit votes before one file wins.
- **Impact:** same logical voter can appear in conflicting quorum certificates.
- **Remediation:** OS-backed exclusive daemon/data-root lock plus per-world transactional control lock around read/validate/write.
- **Test required:** two real daemon processes sharing one data root receive simultaneous conflicting ballots; at most one may participate.

### FINAL-009 — HIGH — Loss of newest manifest silently rolls storage backward

- **Affected:** `Storage::list_snapshots`, `latest_snapshot`, `next_snapshot_number`, runtime restore
- **Auditors:** 3 (SR-01)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** canonical head is inferred from the highest surviving numbered manifest. Deleting the newest manifest makes the older one silently become latest and allows reuse of the missing number.
- **Impact:** undetected rollback and loss of accepted progress.
- **Remediation:** durable canonical-head record binding expected snapshot number/hash; fail closed on missing/mismatched head and recover from replica explicitly.
- **Test required:** commit N-1/N, delete N, reopen, assert missing-head error and no number reuse.

### FINAL-010 — HIGH — Committed snapshot numbers are mutable and authority commit is not atomically fenced

- **Affected:** `Storage::commit_snapshot_streaming`, checkpoint/authority generation boundary
- **Auditors:** 3 (SR-02, SR-05)
- **Status:** CONFIRMED
- **Confidence:** HIGH for overwrite, MEDIUM-HIGH for whole-network stale-writer race
- **Evidence:** existing numbered manifest can be replaced by a different valid manifest; final publication is not in the same atomic transaction as expected authority generation/head comparison.
- **Impact:** mutable local canonical history and stale-generation publication window.
- **Remediation:** create-only immutable manifest slots plus per-world fenced head CAS binding world, epoch, fencing token, prior head and new manifest.
- **Test required:** same-number conflict and epoch-change barrier immediately before commit.

### FINAL-011 — HIGH — Snapshot paths can alias on case-insensitive filesystems

- **Affected:** `validate_portable_path`, manifest validation/restore
- **Auditors:** 3 (SR-03)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** duplicate detection is exact-string only. Paths such as `Foo.dat` and `foo.dat`, plus Windows trailing-dot/space/device-name cases, can map to one destination on supported filesystems.
- **Impact:** a signed manifest can restore successfully to a different file set than it committed to.
- **Remediation:** cross-platform canonical collision key and reserved-name policy applied at creation and restore; verify staged reconstructed tree before publication.
- **Test required:** Windows and case-insensitive macOS alias matrix.

### FINAL-012 — HIGH — Replayable application hello enables member impersonation and private-world disclosure

- **Affected:** `PeerHelloV1`, `verify_peer_hello`, network authentication mapping, `push_known_worlds`
- **Auditors:** 4 (N4-001)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** hello is self-signed but not bound to a receiver challenge, transport peer, current connection or freshness proof, and the same local hello is resent. A captured valid hello can authenticate an attacker's distinct transport as the victim application peer.
- **Impact:** proactive member synchronization can disclose private snapshot manifests and blob contents to the attacker.
- **Remediation:** connection-bound challenge/response proof of possession binding both transport peers, application identity, fresh receiver nonce and protocol domain.
- **Test required:** three-peer captured-hello replay over a different transport identity must produce no member-only data.

### FINAL-013 — HIGH — Private-world metadata endpoints lack membership authorization

- **Affected:** `WorldDescriptor`, `WorldStatus`, `HostCapability` request handlers
- **Auditors:** 4 (N4-002), 7 (SEC-07-001)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** these world-scoped responses require only global peer authentication, not current unbanned membership.
- **Impact:** strangers/ex-members knowing a private world ID can enumerate current membership/public keys and operational hosting/replication state.
- **Remediation:** central fail-closed authorization matrix for every world-scoped `WireRequest`; dedicated minimal invite-bootstrap response if pre-membership data is necessary.
- **Test required:** stranger, removed, banned and current-member matrix across every request family.

### FINAL-014 — HIGH — Live world import can canonize a torn Minecraft save

- **Affected:** `crates/swarm-cli/src/world_import.rs`
- **Auditors:** 5 (A5-01)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** import validates directory/`level.dat` but does not acquire/hold Minecraft's live-world lock or another save/quiescence barrier while snapshotting.
- **Impact:** mixed pre/post-save files can become a validly signed canonical snapshot.
- **Remediation:** hold authoritative Minecraft session/quiescence proof for entire snapshot input consumption; reject running worlds.
- **Test required:** real Minecraft process holding source lock must block import; stopped/saved source must succeed and relaunch.

### FINAL-015 — HIGH — Catalog supports tuples the shipped Fabric bridge cannot run

- **Affected:** `swarm-catalog`, canonical Create/import validation, `fabric.mod.json`, runtime installer
- **Auditors:** 5 (A5-02)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** shipped bridge declares Minecraft `~26.1.2`, loader `>=0.19.3`, Java `>=25`; catalog marks broad Mojang release/snapshot entries supported and only checks provider compatibility, not adapter compatibility.
- **Impact:** player can create an immutable canonical world that fails later at Fabric startup.
- **Remediation:** single authoritative tested adapter support matrix enforced before canonical creation/import; publish exact adapter provenance for multiple supported lines.
- **Test required:** provider-valid but adapter-incompatible tuple must fail before world publication.

### FINAL-016 — HIGH — Runtime supervisor death can orphan a writable Minecraft authority

- **Affected:** runtime supervisor lock, Fabric IPC reader, authority permit heartbeat
- **Auditors:** 5 (A5-03)
- **Status:** CONFIRMED
- **Confidence:** MEDIUM-HIGH
- **Evidence:** Rust supervisor owns the launch lock, daemon independently refreshes authority permit, and Fabric does not stop merely on controller IPC EOF. Supervisor death can release the lock while Java remains alive and authorized.
- **Impact:** writable server can continue without checkpoint ownership; replacement supervisor may reset runtime directory while old Java still exists.
- **Remediation:** controller liveness lease required in addition to authority permit; Fabric fail-closed save/stop on controller loss; prove prior Java death before runtime reset; platform process containment as defense in depth.
- **Test required:** kill only supervisor while daemon remains alive, then attempt replacement launch.

### FINAL-017 — HIGH — CurseForge API credential can cross origin on redirect

- **Affected:** Desktop CurseForge reqwest client and runtime curl provider path
- **Auditors:** 6 (SC6-02), 7 (SEC-07-002)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** authenticated calls permit HTTPS redirects without same-origin enforcement. Custom `x-api-key` is not guaranteed to be stripped; curl path also places the secret in argv.
- **Impact:** provider credential disclosure to a redirected HTTPS origin; local process-table exposure in curl path.
- **Remediation:** authenticated API client with exact-origin redirect policy; disable/manual-follow redirects; never place secret in argv.
- **Test required:** two-origin HTTPS redirect fixture for Desktop and runtime paths.

### FINAL-018 — HIGH — Canonical CurseForge `provider_download` may be unreproducible

- **Affected:** CurseForge provider hashes, canonical modpack validation, runtime reacquisition
- **Auditors:** 6 (SC6-03)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** initial canonicalization permits MD5-only provider provenance and labels successfully downloaded artifact `provider_download`; runtime reacquisition intentionally refuses MD5-only automatic proof and requires manual artifact.
- **Impact:** clean replica cannot fulfill the canonical retrieval contract even when initial creation succeeded.
- **Remediation:** require strong hash for `ProviderDownload`, classify MD5-only as `ManualRequired`, or define runtime-artifact-hash verification as the explicit final proof consistently across both paths.
- **Test required:** full MD5-only CurseForge canonical package round-trip.

### FINAL-019 — HIGH — Provider metadata bodies are not consistently bounded

- **Affected:** Modrinth curl metadata, Desktop CurseForge JSON, runtime CurseForge JSON
- **Auditors:** 6 (SC6-04)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** metadata responses can be read wholly into disk/memory without explicit byte caps, unlike the catalog subsystem.
- **Impact:** hostile/malfunctioning provider can cause memory/disk exhaustion.
- **Remediation:** streaming response limits, bounded headers, bounded JSON depth/cardinality/string lengths before allocation.
- **Test required:** oversized metadata response fixtures for every provider HTTP stack.

### FINAL-020 — HIGH — Desktop launcher enhancement crashes during initialization

- **Affected:** `apps/desktop/src/launcher-controller.js`, `index.html`
- **Auditors:** 8 (A8-001)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** `installModsUi()` finds a submit button nested under `.form-actions` then calls `form.insertBefore(section, submit)`. The reference node is not a direct child of the form, so browser DOM semantics throw `NotFoundError`, aborting subsequent installation.
- **Impact:** normal-path provider UI, public discovery, import catalog hydration and canonical Create interception never attach; legacy handlers remain active.
- **Remediation:** insert relative to a direct child and add a real browser/module initialization smoke test.
- **Test required:** exact frontend graph mounts with zero uncaught exceptions and proves provider/discovery/canonical-create wiring.

### FINAL-021 — HIGH — Release artifacts can publish before required same-SHA validation finishes

- **Affected:** `main-installers.yml`, `release.yml`, validation workflows, branch protection
- **Auditors:** 9 (A9-001), repository governance support from 0 (RT-006)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** release-producing workflows are independent of CI/live journey/version guard. Auditor 9 observed `main-latest` publish while exact-SHA CI was still in progress. `main` had no enforced protection.
- **Impact:** users can receive artifacts that the project's own complete gate has not accepted.
- **Remediation:** reusable same-SHA validation gate required before publish; required checks/ruleset on `main`; tag release validates target SHA.
- **Test required:** packaging green + one validation failure/running must result in no publication.

### FINAL-022 — HIGH — Mutable third-party Actions run with release-write credentials

- **Affected:** release/main-installer workflows
- **Auditors:** 9 (A9-002)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** workflow-scoped `contents: write` is combined with action tags/channels such as `@v4`, `@v2`, `@stable` rather than immutable SHAs.
- **Impact:** upstream action compromise can become repository/release compromise.
- **Remediation:** pin every action to reviewed full SHA; default `contents: read`; grant write only to publisher job; add provenance/attestation.
- **Test required:** workflow policy lint rejects mutable `uses:` and builder jobs with write permissions.

### FINAL-023 — HIGH — Literal two-player crash failover is intentionally unavailable

- **Affected:** host readiness/recovery product semantics
- **Auditors:** 10 (A10-01)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** after Alice/Bob are the two voting members, hard-killing Alice leaves Bob at 1/2; current tests intentionally classify Bob `BlockedByQuorum`.
- **Impact:** required two-player "kill authority and continue" journey cannot complete. This is fail-closed safety, not a split-brain bug.
- **Remediation:** product must require a third voter for automatic crash recovery or design a separately safe two-party recovery proof. Do not weaken quorum to one-of-two.
- **Test required:** acceptance contract updated to intended topology and proven end-to-end.

### FINAL-024 — HIGH — Multi-member sleeping worlds cannot perform safe quorum wake/relaunch

- **Affected:** sleep/wake migration protocol
- **Auditors:** 10 (A10-02)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Evidence:** multi-member direct wake remains deliberately blocked; no sleep-record-bound quorum wake election exists.
- **Impact:** required checkpoint -> backend restart -> restore -> relaunch journey is broken for multi-member worlds.
- **Remediation:** implement wake election bound to signed sleep record and exact canonical snapshot, advancing epoch/fencing safely.
- **Test required:** multi-peer sleep all hosts, restart, elect one wake authority, restore exact snapshot, stale peers fenced.

## 5. MEDIUM findings

### FINAL-025 — MEDIUM — Unsupported protocol versions are accepted by canonical/control handlers

- **Auditors:** 1 (APC-004)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Impact:** validly signed unknown-version records can be interpreted and persisted using V1 semantics.
- **Remediation/test:** one canonical semantic validator per record family; re-signed unsupported-version negative tests through real handlers.

### FINAL-026 — MEDIUM — Canonicalization semantics are inconsistent for provider hints and set-like ordering

- **Auditors:** 1 (APC-005, APC-006)
- **Status:** CONFIRMED
- **Confidence:** MEDIUM-HIGH
- **Impact:** semantically equivalent state can produce different fingerprints/history identities depending on hints/order, while duplicate records may normalize to the same effective state.
- **Remediation/test:** define canonical/non-canonical fields explicitly; enforce one ordering/uniqueness representation at acceptance boundary.

### FINAL-027 — MEDIUM — General restore lacks directory-level transactionality and portable durability proof

- **Auditors:** 3 (SR-06, SR-07, SR-08, SR-09)
- **Status:** CONFIRMED for restore transaction/namespace/deletion-sync; LIKELY for Windows power-loss manifestation
- **Confidence:** HIGH/MEDIUM
- **Impact:** interrupted general restore can leave mixed destination state; some deletion durability and Windows rename durability are not explicitly fenced; direct snapshot load does not bind embedded namespace.
- **Remediation/test:** staging-directory publish transaction, namespace checks, directory sync strategy per OS, crash/fault-injection matrix.

### FINAL-028 — MEDIUM — Discovery announcement signer is not anchored to world authority

- **Auditors:** 4 (N4-003)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Impact:** malicious DHT provider can self-sign fabricated metadata for another world ID and poison browse/exact resolution without directly mutating canonical state.
- **Remediation/test:** compact verifiable world-authority proof or trusted local chain binding signer to world/generation.

### FINAL-029 — MEDIUM — No explicit peer/global request-rate admission policy

- **Auditors:** 4 (N4-004)
- **Status:** LIKELY
- **Confidence:** MEDIUM
- **Impact:** many individually bounded connections/requests can multiply CPU/memory/storage work.
- **Remediation/test:** unauthenticated/authenticated token buckets, connection caps, backoff, hostile many-peer load test.

### FINAL-030 — MEDIUM — Friend presence privacy is weaker than UI semantics imply

- **Auditors:** 4 (N4-005)
- **Status:** CONFIRMED
- **Confidence:** MEDIUM
- **Impact:** anyone knowing a stable peer ID can probe fresh signed liveness even if not an accepted friend.
- **Remediation/test:** either authorize requester as friend or document/rename presence as public-by-ID.

### FINAL-031 — MEDIUM — Desktop import/canonical-create contracts have masked failure states

- **Auditors:** 8 (A8-002, A8-003)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Impact:** fixing the initialization crash exposes missing required Tauri catalog args; post-create local mod failure can be reported as total creation failure and encourage duplicate worlds.
- **Remediation/test:** correct payloads, preserve original controls on hydration failure, model world creation and local setup as separate committed phases.

### FINAL-032 — MEDIUM — Desktop loses runtime stdout/stderr diagnostics

- **Auditors:** 5 (A5-04)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Impact:** Fabric/JVM/mod/port failures become opaque to players/support.
- **Remediation/test:** bounded sanitized per-world logs and UI reference.

### FINAL-033 — MEDIUM — Tauri standard runtime configuration can select arbitrary executable paths

- **Auditors:** 7 (SEC-07-003)
- **Status:** CONFIRMED primitive; exploitability depends on frontend compromise
- **Confidence:** MEDIUM-HIGH
- **Impact:** a future webview compromise inherits native process-selection authority.
- **Remediation/test:** normal flow uses backend-resolved managed profile; manual executable mode isolated behind explicit privileged path/capability.

### FINAL-034 — MEDIUM — Provider redirect policy can escape host trust boundary

- **Auditors:** 6 (SC6-05), related 7 (SEC-07-002)
- **Status:** CONFIRMED
- **Confidence:** MEDIUM-HIGH
- **Impact:** HTTPS-only redirect validation still permits unintended cross-host request destinations; secret-bearing case is already HIGH in FINAL-017.
- **Remediation/test:** explicit provider allowlist/same-origin policy by request class.

### FINAL-035 — MEDIUM — CI does not make excluded Desktop/provider lint/tests first-class gates

- **Auditors:** 9 (A9-003)
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Impact:** packaging can remain green while excluded-crate unit tests/lints regress.
- **Remediation/test:** move useful Agent validation checks into authoritative main CI.

### FINAL-036 — MEDIUM — Release identity/reproducibility controls are incomplete

- **Auditors:** 9 (A9-004, A9-005, A9-006, A9-007)
- **Status:** CONFIRMED
- **Confidence:** HIGH except locked Tauri invocation MEDIUM-HIGH
- **Impact:** tag/version mismatch can publish, Fabric Loom snapshot is mutable, Desktop package path is not explicitly lock-preflighted, and versioned releases can publish without platform signing/notarization.
- **Remediation/test:** tag-to-version binding, immutable Loom/dependency verification, Desktop locked metadata gate, production-tag fail-closed signing policy.

### FINAL-037 — MEDIUM — No single gate proves the composed real multiplayer/provider/recovery product

- **Auditors:** 10 (A10-03)
- **Status:** CONFIRMED evidence gap
- **Confidence:** HIGH
- **Impact:** real Minecraft, two-peer join, provider reacquisition and recovery are separately tested but not one release-blocking journey.
- **Remediation/test:** one composed multi-process acceptance with clean data roots, real Minecraft/Fabric, deterministic provider artifacts, invite/join, reacquisition, authority transition, checkpoint, restart, restore, relaunch.

### FINAL-038 — MEDIUM — Repository governance truth surfaces are not consistently enforced

- **Auditors:** 0 (RT-001, RT-006), 9
- **Status:** CONFIRMED
- **Confidence:** HIGH
- **Impact:** progress ledgers were stale relative to promoted main and branch protection/required checks were absent, increasing coordination and release-control risk.
- **Remediation/test:** refresh ledger from exact accepted SHA and enforce branch/ruleset policy for required checks.

## 6. LOW findings

### FINAL-039 — LOW — Monotonic generation arithmetic saturates at `u64::MAX`

- **Auditors:** 1 (APC-007), 2 (AC-06)
- **Status:** CONFIRMED
- **Remediation:** `checked_add(1)` and explicit exhaustion errors; MAX-1/MAX tests.

### FINAL-040 — LOW — Invite/DNS semantics need hardening and specification

- **Auditors:** 4 (N4-006, N4-007)
- **Status:** CONFIRMED
- **Impact:** bearer invites are reusable until expiry with no nonce consumption/revocation; DNS hints are not reclassified after resolution.
- **Remediation:** specify invite reuse policy/max lifetime/revocation; re-check resolved address scope before dialing.

### FINAL-041 — LOW — Crash debris and legacy blob read are weaker than primary streaming path

- **Auditors:** 3 (SR-10, SR-11)
- **Status:** CONFIRMED
- **Impact:** stale temp accumulation and an older full-decompression helper can waste resources; current primary streaming path is stronger.
- **Remediation:** conservative temp scavenging; remove/deprecate or bound legacy helper.

### FINAL-042 — LOW — Desktop component/status/copy/visual-evidence polish gaps

- **Auditors:** 8 (A8-004 through A8-007)
- **Status:** CONFIRMED source mismatch; visual impact partly UNPROVEN
- **Impact:** injected controls do not consistently use current component classes, exact lookup feedback targets wrong status, Stop copy understates durable safe-stop behavior, exact-SHA minimum-window rendering is unproven.
- **Remediation:** align component classes/status ownership/copy and add exact module-graph screenshot/keyboard gate.

### FINAL-043 — LOW — Local temp diagnostics can follow predictable symlinked path

- **Auditors:** 7 (SEC-07-004)
- **Status:** CONFIRMED behavior, exploitability platform-dependent
- **Remediation:** private data-root temp, exclusive/no-follow creation and ownership/type checks.

### FINAL-044 — LOW — Desktop dependency audit and evidence lifecycle are incomplete

- **Auditors:** 7 (SEC-07-005), 9 (A9-008)
- **Status:** CONFIRMED
- **Impact:** root RustSec does not directly cover separate Desktop lock graph; evidence retention/workflow lifecycle is inconsistent.
- **Remediation:** separate Desktop RustSec; standard retention; retire historical validation workflows after migrating useful gates.

### FINAL-045 — LOW — Legacy consensus models diverge from production semantics

- **Auditors:** 2 (AC-07)
- **Status:** CONFIRMED
- **Impact:** central simulator/test-only helpers can stay green while distributed production state machine diverges.
- **Remediation:** consolidate invariants into production-used consensus code or make distributed process tests authoritative.

### FINAL-046 — LOW — Obsolete validation PR/branch clutter remains

- **Auditors:** 0 (RT-005)
- **Status:** CONFIRMED at audit time
- **Impact:** work-queue noise and confusing historical mergeability, not direct product safety.
- **Remediation:** preserve evidence links, close without merge, archive/delete strict-ancestor validation branches after final audit/release references are safe.

## 7. Dismissed / disputed / time-resolved findings

### DISMISSED-001 — `main-latest` was 237 commits stale

Auditor 0 correctly observed this earlier in the audit window, but Auditor 9 later observed `main-latest` republished at the exact audited SHA, and final integration independently confirmed the tag moved to `354be3b...`. The stale-tag snapshot was a real transient repository-state finding, but it is **DISMISSED as a current defect**.

The deeper release-control issue is not dismissed: FINAL-021 remains because publication occurred independently of full CI completion.

### DISPUTED-002 — “Normal CI can silently stop compiling the Desktop/provider crates”

A broad version of this claim is **DISMISSED**. Auditor 9 established that current Tauri packaging compiles Desktop on all four supported desktop runner variants and transitively compiles the default provider dependency graph. The real blind spot is direct lint/test coverage, retained as FINAL-035.

### DISMISSED-003 — Hidden functional specialist work remains off `main`

Auditor 0 found no meaningful functional implementation stranded on specialist branches. Agent 1's only unique tail was formatting-only. **DISMISSED** as a product-integrity concern.

## 8. CI/test blind spots

The following missing tests materially contributed to confirmed defects surviving green CI:

1. distributed membership divergence with quorum-set changes;
2. Solo minority versus majority recovery partition;
3. stale-authority membership replay after next epoch;
4. non-authority world-config signing;
5. snapshot wrong-parent/jump/same-sequence conflict/number overwrite;
6. duplicate daemon processes sharing one identity/data root;
7. captured application hello replay over a different transport identity;
8. non-member authorization matrix for every world-scoped request;
9. provider filename traversal and cross-origin secret redirect fixtures;
10. live Minecraft import while source server is running;
11. runtime-supervisor hard death while Java survives;
12. real browser mounting the exact Desktop ES-module graph;
13. two-voter crash topology and multi-member wake acceptance;
14. one composed real multiplayer + provider + recovery release gate;
15. release publication blocked while same-SHA validation is failing/running.

Green CI at the audited SHA remains useful evidence for the paths it actually exercises. It is not evidence against the defects above.

## 9. Repository hygiene findings

- Product ancestry at the audited SHA was coherent: accepted integration reached `main`, no hidden functional specialist branch needed rescue.
- Progress/final-integration ledgers were stale and should be updated from exact Git/workflow truth.
- Historical validation-only PRs and CI branches should be preserved as evidence, then closed/archived rather than merged.
- Branch protection/required status enforcement was absent at audit time.
- During final integration, PR #59 promoted 0.5.0 metadata and moved live `main` to `b4bab085...`; the final audit branch deliberately remains based on `354be3b...`.

## 10. Recommended remediation order

1. **Emergency security/safety fixes:** FINAL-003 provider traversal; FINAL-001 membership consensus; FINAL-002 Solo/recovery split brain; FINAL-012 replayable peer authentication.
2. **Canonical authorization/history:** FINAL-004, FINAL-005, FINAL-006, FINAL-008, FINAL-009, FINAL-010, FINAL-011, FINAL-013.
3. **Runtime data safety:** FINAL-014, FINAL-015, FINAL-016.
4. **Provider trust/reproducibility:** FINAL-017, FINAL-018, FINAL-019, FINAL-034.
5. **Player application:** FINAL-020, FINAL-031, FINAL-023, FINAL-024.
6. **Release controls:** FINAL-021, FINAL-022, FINAL-035, FINAL-036, FINAL-038.
7. **Medium/low hardening:** canonicalization, restore transactionality, discovery authenticity/rate limits, Tauri privilege reduction, diagnostics/evidence hygiene.
8. Only after the above, run the composed E2E gate in FINAL-037 and re-audit all affected domains.

## 11. Proposed agent allocation for fixes

### Fix Agent A — Consensus configuration safety

Own FINAL-001, FINAL-002, FINAL-006, FINAL-039, FINAL-045. Implement committed membership generations/joint consensus, safe Solo policy, value-preserving recovery rounds, strict counter exhaustion.

### Fix Agent B — Protocol authorization/history

Own FINAL-004, FINAL-005, FINAL-025, FINAL-026. Central semantic validators, current-authority binding, direct-parent rules, canonical forms/version rejection.

### Fix Agent C — Storage transactional integrity

Own FINAL-008 through FINAL-011, FINAL-027, FINAL-041. Durable head, immutable manifests, fenced commit transaction, portable path identity, cross-process control locking, restore transaction/durability.

### Fix Agent D — Network authentication/privacy

Own FINAL-012, FINAL-013, FINAL-028 through FINAL-030, FINAL-040. Connection-bound challenge authentication, request authorization matrix, discovery authority proofs, admission limits, privacy/address semantics.

### Fix Agent E — Package/provider security

Own FINAL-003, FINAL-017 through FINAL-019, FINAL-034. Server-owned staging, credential-safe HTTP policy, canonical retrieval consistency, response bounds, redirect allowlists.

### Fix Agent F — Minecraft/runtime lifecycle

Own FINAL-014 through FINAL-016, FINAL-032. Import quiescence, adapter support matrix, supervisor/controller liveness fencing, runtime diagnostics.

### Fix Agent G — Desktop player journey

Own FINAL-020, FINAL-031, FINAL-042 and coordinate FINAL-023/024 UX once backend semantics exist. Add exact browser module-graph smoke/render/keyboard tests.

### Fix Agent H — CI/release governance

Own FINAL-021, FINAL-022, FINAL-035, FINAL-036, FINAL-038, FINAL-044, FINAL-046. Same-SHA gates, immutable actions/minimum permissions, excluded-crate gates, tag/version/signing policy, branch rules, evidence/branch cleanup.

### Fix Agent I — Recovery/wake product completion

Own FINAL-007, FINAL-023, FINAL-024 and integrate with Agents A/F. Specify supported voter topology, host capability candidacy and sleep-record-bound quorum wake.

### Final Acceptance Agent

After A-I are integrated, run FINAL-037 as the release-blocking whole-product acceptance and then trigger the re-audit set below.

## 12. What must be re-audited after fixes

Mandatory re-audits:

- Auditor 1 Protocol/Core
- Auditor 2 Authority/Consensus
- Auditor 3 Storage/Recovery
- Auditor 4 Network/Discovery
- Auditor 5 Minecraft Runtime
- Auditor 6 Supply Chain
- Auditor 7 Security
- Auditor 8 Desktop UX
- Auditor 9 CI/Release
- Auditor 10 Adversarial E2E

Auditor 0 Repository Truth should also be rerun after branch/PR cleanup and after the final fixed release candidate is promoted, so the next final integrator gets one exact repository truth snapshot.

The final re-audit must use one exact fixed SHA across every report. No release/publication should occur before that same SHA has passed the full required gate.

## 13. Safety disposition

| Use | Disposition | Reason |
| --- | --- | --- |
| Continued development | **YES** | Architecture and tests provide a workable base; defects are identifiable and remediable. |
| Controlled internal alpha | **LIMITED** | Only disposable/non-sensitive worlds, no trust in multi-peer canonical safety/private confidentiality/provider staging. |
| Wider public testing | **NO** | CRITICAL split-brain and filesystem-boundary defect plus HIGH authentication/privacy/runtime/UI failures. |
| Production-like use | **NO** | Canonical history, filesystem, private-world and release-control guarantees are not adequate. |

AUDIT VERDICT: FAIL