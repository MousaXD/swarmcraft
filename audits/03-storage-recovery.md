# Auditor 3 - Storage, Snapshot Integrity, and Crash Recovery

## Audit baseline

- Repository: `MousaXD/swarmcraft`
- Audit branch: `audit/storage-recovery`
- Audited `main` SHA: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Live remote baseline check: PASS. `main` was exactly the required SHA when this audit began.
- Production code changes: none.
- Audit scope: `crates/swarm-storage`, snapshot/blob persistence, manifests, retention/GC, replication persistence, restore behavior, import publication, recovery/control records, migration restore/checkpoint call sites, and existing storage/process recovery tests.

## Executive verdict

SwarmCraft has several strong storage primitives: blobs are content-addressed and verified, streaming decode is bounded on the production snapshot/replication path, snapshot manifests are written only after blob verification, publication pins are durable and coordinated with GC, corrupted newest manifests fail closed rather than being silently skipped, import uses same-filesystem staging, and the authority runtime discards/rebuilds its runtime directory before each restore.

Those controls are not sufficient for a PASS. The storage layer still permits silent rollback when the newest manifest disappears, permits an existing committed snapshot number to be replaced by a different manifest, does not define a portable path-identity rule strong enough for case-insensitive filesystems, and does not serialize read-check-write recovery/control records across processes. Snapshot commit also is not atomically fenced to the current authority generation. These defects can turn filesystem loss, concurrent local processes, or a narrow authority-transition race into durable state that `latest_snapshot()` treats as authoritative local history.

**VERDICT: FAIL**

## Scope and implementation map

The `swarm-storage` library entry point is `crates/swarm-storage/src/root.rs`, which includes `lib.rs` as `base` and enables these production modules:

- `control.rs`
- `recovery_v2.rs`
- `replica.rs`
- `retention.rs`
- `scheduler.rs`
- `state.rs`
- `streaming.rs`
- `world.rs`

`crates/swarm-storage/src/replication.rs` exists in the tree but is not included by `root.rs`; it is not the active production replication implementation. The active implementation is `replica.rs` plus `scheduler.rs`.

Important call sites reviewed outside the crate:

- `crates/swarm-cli/src/migration.rs`
- `crates/swarm-cli/src/launch_guard.rs`
- `crates/swarm-cli/src/world_import.rs`
- `crates/swarm-cli/src/main.rs`
- `crates/swarm-cli/src/daemon.rs`

Existing recovery-oriented tests reviewed include:

- `crates/swarm-storage/tests/failure_injection.rs`
- `crates/swarm-storage/tests/streaming_recovery.rs`
- `crates/swarm-storage/tests/replication_resume.rs`
- `crates/swarm-storage/tests/retention_gc.rs`
- `crates/swarm-storage/tests/publication_ownership_race.rs`
- `crates/swarm-storage/tests/replication_scheduler.rs`
- `crates/swarm-storage/tests/snapshot_swarm_acceptance.rs`
- process-level migration/host tests under `crates/swarm-cli/tests/`

This audit inspected the exact remote source and existing tests. The local command runner available to this auditor could not execute repository commands, so tests were not independently rerun in this session. The exact-head GitHub Actions set was also still in progress during the audit. None of the confirmed findings below depends on a failing test run; they follow directly from reachable persistence code paths.

---

# Positive controls already present

## P-01 - Blob integrity is verified before snapshot restore or replica finalization

`streaming.rs::verify_encoded_blob_streaming` and `replica.rs::verify_encoded_blob` validate encoded length, bounded decompression, uncompressed length, and BLAKE3 content hash. `restore_blob_streaming` writes decoded output to a temporary file and validates length/hash before replacing an existing destination file.

This correctly handles ordinary truncated/corrupt blob cases and prevents a corrupt blob from overwriting an existing destination file.

Existing coverage includes:

- `failure_injection::compressed_blob_larger_than_declared_size_is_rejected`
- `streaming_recovery::corrupt_restore_never_replaces_existing_destination_file`
- `streaming::streaming_snapshot_round_trip_and_truncation_detection`
- replica decompression-expansion tests

## P-02 - Snapshot manifest publication is ordered after complete blob publication

`SnapshotPublicationLease` in `retention.rs` gives each local snapshot publication its own durable pin directory and kernel-held owner lock. Each blob hash is pinned while holding the GC coordination lock. `commit_snapshot_streaming` verifies the manifest and blobs, holds the GC lock while publishing the manifest, and only then releases that publication's pins.

This is a good blobs-before-manifest ordering. A crash before manifest publication can leave extra complete blobs and pins, but it does not create a committed snapshot that references missing data solely because GC raced the publisher.

## P-03 - Abandoned publication pins are recovered conservatively

`Storage::open` calls `recover_abandoned_snapshot_publications`. Retention recovery only removes a transaction-owned publication directory after it can acquire the publication owner lock, proving the original process no longer owns it. Malformed publication directories with no ownership proof are retained rather than aggressively deleted.

That is the correct failure bias.

## P-04 - Retention is mark-before-sweep and recovery roots are preserved

`retention.rs` derives live blob hashes from committed manifests and durable pins. It also protects snapshots referenced by transfer, sleep, recovery promise, recovery certificate, solo branch, and preserved solo-conflict records. A missing mandatory recovery snapshot causes pruning to fail rather than silently discarding another root.

## P-05 - Corrupt newest manifest fails closed

`list_snapshots` decodes every `*.postcard` file. If the newest manifest is truncated or otherwise undecodable, the operation returns an error rather than skipping that file and silently selecting an older manifest. `failure_injection::truncated_manifest_is_rejected_instead_of_becoming_canonical` covers this case.

This protection does **not** cover deletion of the newest manifest; see SR-01.

## P-06 - Authority runtime retries restores from a disposable runtime directory

`migration.rs::run_authority_runtime_inner` resets the per-world runtime directory before installing runtime material and restoring the selected snapshot. A process death during restore therefore leaves a partial runtime directory, but the next normal authority-runtime attempt removes that directory and reconstructs it from the snapshot again before launch.

This materially reduces the product impact of SR-06 for the main Minecraft authority launch path.

## P-07 - Import uses hidden staging before making a new world visible

`world_import.rs` assembles a complete imported SwarmCraft world under `.import-staging/<world>`, verifies the signed snapshot, drops the publication lease, and publishes the staged world with a same-filesystem directory rename. On Unix it then synchronizes the parent `worlds` directory. Failure-injection tests cover interruptions before publication.

---

# Findings

## SR-01 - HIGH - Deleting the newest manifest silently rolls the local canonical head backward

**Files/functions**

- `crates/swarm-storage/src/lib.rs`
  - `Storage::list_snapshots`
  - `Storage::latest_snapshot`
  - `Storage::next_snapshot_number`
- `crates/swarm-cli/src/migration.rs`
  - `run_authority_runtime_inner`
  - `prepare_authority_epoch`

**Invariant**

Loss of the newest committed snapshot must be detected as missing canonical state. Storage must not silently reinterpret an older prefix of history as the current head.

**Failure scenario**

1. Snapshot `N-1` and snapshot `N` are both valid and committed.
2. `snapshots/<N>.postcard` is deleted by filesystem corruption, operator accident, or partial storage loss.
3. `list_snapshots` sees only surviving files and returns the older manifests.
4. `latest_snapshot` returns `N-1` with no indication that `N` ever existed.
5. `next_snapshot_number` returns `N`, allowing that number to be reused.
6. In an awake/non-sleeping authority state, `run_authority_runtime_inner` can verify the older signed snapshot and continue from it. A later checkpoint can publish a new snapshot `N` descending from `N-1`, silently discarding the lost progress.

A durable sleep record protects the sleeping path because it stores the exact expected snapshot hash, but the sleep record is cleared when waking and there is no durable always-present head record for the ordinary awake state.

**Evidence**

`latest_snapshot` is only `list_snapshots(world)?.pop()`. `list_snapshots` has no durable expected-head number/hash and no tombstone or maximum-ever-published counter. `next_snapshot_number` is derived from the highest surviving manifest.

**Existing test coverage**

- Truncated/corrupt manifest fail-closed is tested.
- Missing newest manifest is not tested.

**Missing test**

Commit snapshots 1 and 2, durably record head 2, delete manifest 2, reopen storage, and assert that `latest_snapshot`/authority launch returns a missing-head error rather than snapshot 1. A second test must prove snapshot number 2 cannot be reused after loss.

**Recommended remediation**

Introduce a durable per-world canonical-head record containing at least snapshot number and manifest hash. Publish order should be:

1. complete and sync blobs;
2. publish and sync immutable manifest;
3. atomically update and sync the head record.

On startup, the head record must be verified against the exact manifest/hash. A missing or mismatched target must fail closed and trigger explicit recovery from a replica, never automatic rollback. A manifest created before a crash but not referenced by the head record should be treated as an orphan candidate requiring explicit reconciliation.

**Confidence: HIGH**

---

## SR-02 - HIGH - Committed snapshot numbers are mutable and conflicting history can overwrite an existing manifest

**Files/functions**

- `crates/swarm-storage/src/streaming.rs`
  - `Storage::commit_snapshot_streaming`
  - `atomic_write`

**Invariant**

Once snapshot number `N` is committed for a world, that numbered history slot must be immutable. A repeat commit may only be idempotent if it is byte/hash identical.

**Failure scenario**

`commit_snapshot_streaming` constructs the path solely from `manifest.snapshot_number` and calls `atomic_write`. The streaming `atomic_write` implementation removes an existing destination before renaming the temporary file. There is no check that an existing numbered manifest has the same manifest hash, previous hash, epoch, sequence, or state root.

Therefore two distinct, individually well-formed manifests for the same world and same snapshot number can be committed sequentially, and the second replaces the first.

This amplifies SR-01 because a lost newest manifest can be recreated with different content under the same number. It also makes a stale/concurrent publisher capable of replacing a numbered local history record if it reaches the commit API.

**Existing test coverage**

No test was found asserting immutability or idempotency of an already committed snapshot number.

**Missing test**

1. Commit manifest `A` as snapshot 7.
2. Construct distinct valid manifest `B` also numbered 7.
3. Attempt to commit `B`.
4. Assert a conflict error and assert the bytes/hash of snapshot 7 remain exactly `A`.
5. Repeat `A` and assert an explicitly idempotent success path if desired.

**Recommended remediation**

Publish numbered manifests with create-only semantics. If a target already exists, load it and compare the manifest hash:

- identical hash: return idempotent success;
- different hash: return an immutable-history conflict and preserve both evidence objects outside the canonical namespace if forensic retention is desired.

Also validate the expected predecessor/head under the same serialization primitive used to advance the canonical head.

**Confidence: HIGH**

---

## SR-03 - HIGH - Portable-path validation allows filename aliases that collapse silently on case-insensitive filesystems

**Files/functions**

- `crates/swarm-storage/src/streaming.rs`
  - `validate_portable_path`
  - `validate_manifest_shape`
  - `restore_snapshot_streaming`
  - `restore_blob_streaming`

**Invariant**

A valid snapshot must restore to the same set of distinct logical files on every supported filesystem. Two manifest paths must never resolve to the same destination object.

**Failure scenario**

Current validation rejects absolute paths, `..`, backslashes, empty components, and drive-letter prefixes. Duplicate detection compares exact path strings only.

A snapshot created on a case-sensitive filesystem can therefore contain both:

- `region/Foo.dat`
- `region/foo.dat`

The state root correctly commits to two entries. On a case-insensitive destination filesystem, those paths can identify the same file. Restore processes entries sequentially; the second entry replaces the first and restore returns success. The resulting directory contains fewer distinct files than the signed manifest and no final tree verification detects the collapse.

Related platform aliases include Windows trailing-dot/trailing-space normalization and reserved device names. Unicode normalization behavior may create additional aliases depending on filesystem configuration.

**Existing test coverage**

- exact duplicate path rejection is tested;
- `..` traversal is tested;
- Unix symlinked root/parent rejection is tested;
- cross-platform alias collisions are not tested.

**Missing test**

Create a manifest containing case-fold aliases and platform-reserved aliases. Restore on Windows and on a case-insensitive macOS filesystem and require rejection before any destination publication.

**Recommended remediation**

Define a project-level portable filename identity policy, not merely lexical traversal checks. At minimum:

- compute a canonical collision key for every path component;
- reject case-fold collisions if Windows/default macOS are supported;
- reject Windows reserved device names;
- reject components with Windows-significant trailing dots/spaces;
- define Unicode normalization behavior explicitly;
- apply the same validation at snapshot creation and before restore.

For defense in depth, restore to staging and verify that the reconstructed tree maps one-to-one to manifest entries before publication.

**Confidence: HIGH**

---

## SR-04 - HIGH - Recovery/control read-check-write transitions are not serialized across local processes

**Files/functions**

- `crates/swarm-storage/src/state.rs`
  - `Storage::promise_recovery_ballot`
  - `atomic_write`
- `crates/swarm-storage/src/control.rs`
  - recovery reservation / epoch / transfer / sleep save paths
  - `atomic_write`
- `crates/swarm-storage/src/world.rs`
  - membership/control-style metadata saves
- `crates/swarm-storage/src/recovery_v2.rs`
  - recovery certificate save

**Invariant**

A storage decision such as "promise this recovery ballot only if no conflicting equal/newer promise is durable" must be one atomic cross-process transition. Two processes sharing the same data root must not both observe the same old state and both report success for conflicting writes.

**Failure scenario**

`promise_recovery_ballot` performs:

1. load current promise;
2. validate round/base;
3. write replacement.

There is no per-world/process-shared lock around the read-check-write sequence. The helper also uses a fixed temporary path (`recovery-promise.tmp` via `path.with_extension("tmp")`) rather than a unique temporary file.

Two daemon/backend processes can both read the same old promise before either publishes its new promise. They can then race the fixed temp and final rename. A schedule exists where process A publishes and returns `Accepted`, then process B, which made its decision from the stale pre-A read, publishes a conflicting promise and also returns `Accepted`.

The daemon has in-memory serialization inside one process, but no repository-wide single-instance data-root lock was found. The authority-runtime lock in `migration.rs` protects Minecraft runtime ownership, not all storage control transitions.

This is especially dangerous for recovery promises because the return value is used to decide whether to emit a recovery vote. It can therefore escape the filesystem as externally observed behavior before the losing/overwritten local durable state is noticed.

**Existing test coverage**

Tests cover restart persistence and stale-round rejection sequentially. No cross-process or concurrent conflicting-promise test was found.

**Missing test**

Use two independent processes or two `Storage` handles coordinated with barriers:

- both read the same initial promise state;
- submit conflicting ballots;
- assert at most one returns `Accepted` and only that exact ballot is durable;
- repeat for recovery reservations and other monotonic control records.

Run on Linux and Windows.

**Recommended remediation**

Use a durable per-world control lock, held across each read-validate-write transaction. The lock must be OS-backed and shared by all processes using the same data root. Use unique create-new temp files for publication. For monotonic records, re-read and validate while holding the lock immediately before rename. Consider a small journal/transaction record if multiple control files must advance together.

**Confidence: HIGH**

---

## SR-05 - HIGH - Snapshot publication is not atomically fenced to the currently durable authority generation

**Files/functions**

- `crates/swarm-storage/src/streaming.rs`
  - `Storage::commit_snapshot_streaming`
- `crates/swarm-cli/src/migration.rs`
  - final checkpoint sequence in `run_authority_runtime_inner`
  - `ensure_authority_generation`

**Invariant**

After authority generation N+1 is durable, a generation-N writer must not be able to publish a snapshot that local storage later treats as the newest committed snapshot.

**Failure scenario**

`commit_snapshot_streaming` validates manifest shape and blob integrity, but it does not read the current epoch record, verify the manifest signature, check authority identity, or check a fencing token. `SnapshotManifestV1` carries epoch but not a fencing token.

The main checkpoint path does important checks:

1. verify authority generation before snapshotting;
2. create snapshot;
3. verify authority generation again;
4. sign snapshot;
5. commit snapshot.

There is no lock or compare-and-swap tying step 3 to step 5. If a new authority epoch becomes durable after the second check but before the manifest rename, the stale writer can still publish its snapshot. Because `latest_snapshot` selects by surviving snapshot number rather than by a separately fenced head record, that stale snapshot can become the highest local numbered manifest.

Higher-level authority checks can prevent that peer from launching once it notices the epoch change, so this is not by itself proof that the network accepts split brain. It is nevertheless a storage invariant failure and creates a dangerous local-history artifact that later code must distinguish correctly.

**Existing test coverage**

Authority migration tests cover many sequential generation checks, but no failure-injection test was found that pauses exactly after the final authority check and advances the epoch before snapshot commit.

**Missing test**

Inject a barrier between the last `ensure_authority_generation` and `commit_snapshot`. Advance the durable epoch/fencing generation from another process. Resume the old checkpoint and require the commit to fail without publishing a canonical manifest.

**Recommended remediation**

Provide a fenced snapshot-commit primitive that atomically checks an expected authority generation under the same per-world control lock used to update the epoch/head state. The commit should bind:

- world;
- expected epoch;
- expected fencing token;
- expected current head hash/number;
- new manifest hash/number.

A post-commit recheck is useful for detection but is not a substitute for serialization because it cannot undo an already externally visible canonical publication safely.

**Confidence: HIGH for the storage race; MEDIUM for whole-network exploitability**

---

## SR-06 - MEDIUM - General restore is file-atomic but not directory-transactional

**Files/functions**

- `crates/swarm-storage/src/streaming.rs`
  - `restore_snapshot_streaming`
  - `restore_blob_streaming`

**Invariant**

A restore operation should either publish a complete verified snapshot or leave the previous destination intact/clearly marked incomplete.

**Failure scenario**

Restore iterates manifest entries and publishes each destination file immediately after validating that blob. There is no restore journal, staging tree, completion marker, or final tree transaction.

If the process dies after file `k` of `n`:

- a previously empty destination contains a partial snapshot;
- a preexisting destination can contain a mixture of old and new files;
- stale `.restore-<pid>-<counter>.tmp` files may remain;
- the storage layer has no persisted indication that the directory is incomplete.

For an individual existing output file, restore removes the old file before renaming the verified temporary file. A rename failure or crash in that narrow interval can lose the previous file.

**Mitigation in primary runtime path**

`migration.rs::run_authority_runtime_inner` deletes/recreates the entire runtime directory before each normal authority restore. Therefore a crashed partial runtime restore is discarded on retry before Minecraft is launched. CLI Recover/Export also require an empty or missing destination at command start, making an interrupted partial export visible to the operator, but requiring manual cleanup before retry.

**Existing test coverage**

- corruption-before-replacement preserves the old file;
- traversal and symlink checks are covered;
- process death after some successful file publications is not covered.

**Missing test**

Failure-inject after each file rename, reopen, and assert one of two explicit policies:

- old destination remains complete; or
- an incomplete restore marker forces fail-closed cleanup/retry.

**Recommended remediation**

For general restore/export, reconstruct beneath a sibling staging directory on the same filesystem, sync files/directories, verify the reconstructed tree, then publish with a directory-level transaction/rename. If replacing an existing destination is supported, use a two-name swap/backup protocol with a recovery journal rather than per-file destructive replacement.

**Confidence: HIGH**

---

## SR-07 - MEDIUM - Several successful metadata deletions are not directory-synced on Unix

**Files/functions**

- `crates/swarm-storage/src/control.rs`
  - `remove_if_present`
  - `clear_recovery_reservation`
  - `clear_sleep_record`
- `crates/swarm-storage/src/world.rs`
  - `remove_protocol_file`
  - pending join/leave and local membership cleanup

**Invariant**

If a deletion is reported durable, a power loss should not resurrect the removed control file because its parent directory entry was never synchronized.

**Failure scenario**

The atomic-write helpers generally fsync a file, rename it, and sync the parent directory on Unix. By contrast, the deletion helpers above call `fs::remove_file` and return success without syncing the parent directory.

After an acknowledged clear followed by sudden power loss, a filesystem is permitted to replay the previous directory state. A stale sleep record can reappear and block launch fail-closed; a stale recovery reservation or pending membership request can reappear and interfere with recovery/join semantics.

This is primarily an availability and stale-control-state risk because downstream code tends to validate these records rather than blindly trust corruption.

**Existing test coverage**

Sequential remove/reload behavior is tested, not power-loss durability.

**Missing test**

Use a crash-consistency harness or filesystem fault injector to crash after `remove_file` but before any later directory sync. On restart, the cleared record must not reappear.

**Recommended remediation**

After every durable metadata deletion, sync the parent directory on platforms where directory syncing is supported. Consolidate persistence helpers so delete durability is not implemented inconsistently across modules.

**Confidence: HIGH for the missing sync; MEDIUM for observed resurrection frequency on real hardware/filesystems**

---

## SR-08 - MEDIUM - Windows directory-entry durability is explicitly unproven for rename-based commits

**Files/functions**

- `crates/swarm-storage/src/lib.rs::sync_parent`
- `crates/swarm-storage/src/streaming.rs::sync_parent`
- `crates/swarm-storage/src/control.rs::sync_parent`
- `crates/swarm-storage/src/state.rs::sync_parent`
- `crates/swarm-storage/src/world.rs::sync_parent`
- `crates/swarm-storage/src/recovery_v2.rs::sync_parent`
- `crates/swarm-storage/src/retention.rs::sync_dir`
- `crates/swarm-cli/src/world_import.rs::sync_directory`

**Invariant**

After a commit/import reports success, the rename that made the new file/directory visible must survive power loss on every supported platform.

**Failure scenario**

Most persistence helpers perform file `sync_all`, rename, and then directory sync under `#[cfg(unix)]`. The non-Unix branch is a no-op. Import similarly syncs the published `worlds` directory only on Unix.

Therefore the code has a deliberate durability gap on Windows between "rename returned success" and "directory namespace change is proven durable after sudden power loss." The exact persistence behavior is filesystem/OS dependent, so this audit does not claim a deterministic Windows data-loss reproduction from source alone. It does establish that SwarmCraft itself does not currently issue or verify an equivalent durability barrier there.

The active `replica.rs` final blob rename has an additional issue: it does not call a parent-sync helper at all after `.part -> final` publication.

**Existing test coverage**

Normal restart tests exercise Windows-compatible code paths logically, but normal process restart does not simulate cache loss/power failure.

**Missing test**

A Windows crash/power-loss integration harness covering:

- manifest rename;
- control record rename;
- replicated blob `.part -> final` rename;
- imported world directory publication.

**Recommended remediation**

Define and document platform-specific durability semantics. On Windows, use an OS-supported strategy that gives the required guarantees for file replacement and directory publication, or explicitly weaken the product guarantee and add recovery detection. Do not silently equate successful rename with durable commit.

**Confidence: MEDIUM**

---

## SR-09 - MEDIUM - `load_snapshot(world, number)` does not bind the decoded manifest to the requested namespace

**Files/functions**

- `crates/swarm-storage/src/lib.rs::Storage::load_snapshot`
- `crates/swarm-cli/src/main.rs::WorldCommand::Recover`

**Invariant**

A manifest loaded from `world A / snapshot N` must itself declare world A and snapshot N before it can leave the storage API as that object.

**Failure scenario**

`load_snapshot` builds the path from the requested world and number, reads it, postcard-decodes it, and returns it without checking:

- `manifest.world_id == world`;
- `manifest.snapshot_number == number`.

If a valid manifest is misplaced/cross-copied into another world's numbered path, the storage API returns it under the wrong lookup key. CLI Recover then verifies the manifest and signature according to the manifest's embedded world and restores it, while the command was issued for the requested world path.

`list_snapshots` does filter embedded `world_id`, so behavior is inconsistent between list and direct load.

**Existing test coverage**

No direct namespace-substitution test was found.

**Missing test**

Place a valid signed world-B manifest at world-A snapshot path N and require `load_snapshot(A, N)` to return `WorldMetadataMismatch` or a dedicated namespace mismatch before any blob lookup/restore.

**Recommended remediation**

Validate world ID and snapshot number immediately after decode. Prefer to run basic manifest-shape validation there as well, while keeping expensive blob/signature verification explicit if desired.

**Confidence: HIGH**

---

## SR-10 - LOW - Crash leftovers are ignored but not consistently cleaned

**Files/functions**

- `crates/swarm-storage/src/streaming.rs::create_unique_temp`
- restore temporary files under arbitrary restore destinations
- snapshot/blob temp files
- `Storage::open` recovery scope

**Invariant**

Crash debris should be either automatically reclaimed when ownership is provably stale or explicitly documented as operator-visible debris.

**Failure scenario**

Snapshot publication ownership directories have thoughtful stale-owner recovery. Restore temporary files and ordinary unique atomic-write temps do not have equivalent ownership/recovery metadata. A process death can leave `.restore-*`, `.blob-*`, or `.atomic-*` files. Many are ignored safely, but repeated crashes can accumulate storage and potentially contribute to disk-full conditions.

**Existing test coverage**

`streaming_recovery::stale_temporary_files_are_ignored_after_restart` proves stale temp names do not become canonical snapshots/blobs. It does not prove cleanup.

**Recommended remediation**

Add conservative stale-temp scavenging with name/age/ownership rules that cannot delete live writers, or keep the current safe-ignore behavior and surface debris through diagnostics/maintenance commands.

**Confidence: HIGH**

---

## SR-11 - LOW - Legacy `Storage::read_blob` fully decompresses before enforcing declared uncompressed size

**Files/functions**

- `crates/swarm-storage/src/lib.rs::Storage::read_blob`

**Invariant**

Untrusted compressed input should not force an allocation proportional to decompressed attacker-controlled output before size limits are enforced.

**Failure scenario**

For Zstd, `read_blob` calls `zstd::stream::decode_all` and only after full decompression checks `descriptor.uncompressed_size` and the hash. The streaming production restore and replica verifiers correctly bound reads to one byte beyond the declared size, but this older convenience API does not.

A code search found no production caller outside tests at this SHA, which limits current exploitability.

**Existing test coverage**

The decompression expansion test verifies that `read_blob` eventually rejects an oversized expansion, but it does not assert bounded memory use. The streaming verifier has a dedicated bounded-read test.

**Recommended remediation**

Remove/deprecate the unbounded helper or implement it using the same bounded streaming decoder and an explicit maximum allocation policy.

**Confidence: HIGH for the API behavior; LOW current product reachability**

---

# Crash-consistency matrix

| Failure point | Current behavior | Silent inconsistent-state risk | Assessment | Required hardening |
|---|---|---:|---|---|
| 1. Power loss during snapshot blob write | Blob is first written to a temporary file. Manifest is not yet published. Publication pins/GC coordination prevent a live in-flight blob from being swept. | Low | **PASS with caveats** | Clean stale ordinary temps; retain current pin ownership model. |
| 2. Power loss after complete blobs but before manifest | Complete blobs can remain orphaned. No manifest references them. Abandoned publication ownership is recovered conservatively; later GC can reclaim unreferenced complete blobs. | Low | **PASS** | Keep blobs-before-manifest ordering. |
| 3. Power loss after manifest rename but before publication-pin release | On Unix, manifest file is fsynced and parent directory synced before pins are released, so manifest becomes a GC root first. Stale pins are extra protection. Windows directory durability is not explicitly proven. | Low on Unix; uncertain on Windows | **PASS Unix / UNKNOWN Windows** | Implement/validate Windows namespace durability. |
| 4. Corrupt newest snapshot manifest | `list_snapshots` decode fails and the operation stops. It does not silently skip to an older snapshot. | Low corruption risk, availability loss | **PASS** | Add explicit recovery UX from replica while preserving fail-closed behavior. |
| 5. Delete newest snapshot manifest | No durable head pointer exists. Highest surviving older manifest becomes `latest_snapshot`; number can be reused. | **High** | **FAIL - SR-01** | Durable head number/hash plus missing-head detection and no automatic rollback. |
| 6. Full disk during checkpoint | Blob/temp writes and sync failures propagate. Manifest is published only after blob verification. A failure after rename but during parent sync is commit-ambiguous and returns error. | Low silent corruption; possible ambiguous success/failure boundary | **PARTIAL** | Add fault-injection around every write/sync/rename; define recovery for rename-succeeded/sync-failed state. |
| 7. Process death during restore | Per-file writes are integrity checked, but already-restored files remain. General restore has no staging/journal/completion marker. Main authority runtime deletes/rebuilds runtime dir on retry. | Medium outside disposable runtime | **FAIL - SR-06** | Stage complete restore and publish atomically, or persist an incomplete marker and force cleanup. |
| 8. Stale authority writes while recovery/epoch advances | Migration checks authority before and after snapshot creation, but commit itself is not fenced and there is a TOCTOU interval before manifest publication. | **High local-history risk** | **FAIL - SR-05** | Atomic fenced commit under same per-world authority/head lock. |
| Existing destination file replaced during restore, then crash before rename | Old output is removed before temp rename. A crash/rename failure can leave the file absent. | Medium | **FAIL - SR-06** | Rename-based staged-tree/swap transaction; do not unlink old destination first. |
| Crash after control-record deletion | Some delete helpers do not sync parent directory. | Medium stale-state risk | **FAIL - SR-07** | Sync parent after durable delete. |
| Crash after Windows rename-based commit | File is synced; directory sync helper is a no-op on non-Unix. | Platform-dependent | **UNKNOWN / SR-08** | Prove or implement Windows durability barrier. |
| Crash during import before final world rename | Staging is invisible under `.import-staging`; retry clears stale staging. | Low | **PASS** | Keep staging model. |
| Crash after imported world rename but before parent sync | Unix call returns error if parent sync fails and attempts rollback. Non-Unix sync is a no-op. | Low Unix; uncertain Windows | **PASS Unix / UNKNOWN Windows** | Windows publication durability validation. |
| Concurrent conflicting recovery promise writes | No cross-process transaction lock; both callers can decide from stale prior state. | **High** | **FAIL - SR-04** | OS-backed per-world control lock around read-check-write. |
| Replication interrupted mid-blob | `.part` is length-tracked and resumable; each received chunk is synced. Final blob is verified before rename. | Low | **PASS with Windows durability caveat** | Parent-sync/Windows durability after final rename. |
| Corrupt/missing blob referenced by manifest | Snapshot verification/restore detects missing I/O or hash/size mismatch and fails. | Low silent corruption | **PASS** | Keep verification mandatory before launch/recovery. |
| GC interrupted after deleting some unreferenced blobs | GC only deletes blobs not rooted by manifests/pins; interruption leaks extra garbage rather than deleting rooted data if root calculation was correct. | Low | **PASS** | Keep lock/pin protocol and fault tests. |

---

# Required failure scenarios - direct answers

## 1. Power loss during snapshot write

The current streaming design is safe against publishing a half-written blob as a committed snapshot. Blob data is written to a unique temp, completed and synced, and only then renamed. The snapshot manifest is not committed until all referenced blobs verify. Main concern is debris, not silent canonical corruption.

## 2. Power loss after blobs but before manifest

Safe by construction against false canonical publication. Complete unreferenced blobs may survive. Durable publication pins and conservative stale-owner recovery prevent GC from deleting a blob that a live publisher still owns. After owner death, unreferenced blobs can later be reclaimed.

## 3. Power loss after manifest but before metadata pointer update

There is currently no universal canonical snapshot-head pointer, which is itself SR-01. For the manifest/pin transaction, Unix ordering is sound: manifest temp is synced, renamed, parent synced, then publication pins are released. If a future head pointer is introduced, the correct order is manifest durable first, then head pointer durable.

## 4. Corrupt newest snapshot

Truncated/undecodable manifest fails closed. Corrupt blob fails verification before restore/launch. This is good. Automatic fallback to an older snapshot is not performed for corruption.

## 5. Delete newest snapshot

FAIL. Deletion is indistinguishable from "snapshot never existed" once no sleep/transfer/recovery record happens to retain its hash. `latest_snapshot` silently returns the highest surviving predecessor and `next_snapshot_number` can reuse the missing number.

## 6. Full disk during checkpoint

Most write/sync errors propagate without publishing the manifest early. However, any rename-then-parent-sync sequence has an ambiguous boundary if rename succeeds and the subsequent sync fails. The caller receives failure even though the new path may already be visible and may or may not survive reboot. The system should explicitly reconcile this state on reopen instead of relying on normal listing.

## 7. Process death during restore

General storage restore is not transactional. Partial output remains. The primary authority runtime mitigates this by deleting the disposable runtime directory on the next attempt before restoring again. CLI export/recover to a now-nonempty partial destination requires operator cleanup. A reusable storage API should provide stronger semantics.

## 8. Stale authority writes while recovery is happening

The main checkpoint path checks authority twice, which substantially narrows the race, but storage commit is not fenced. An epoch transition after the final check and before manifest rename can still leave a stale-generation manifest in the canonical numbered snapshot directory. This must be fixed by a lock/CAS transaction, not another unlocked check.

---

# Filesystem and trust-boundary assessment

## Path traversal

Direct `..`, absolute slash/backslash paths, empty components, and drive-letter prefixes are rejected before restore. The restore root and parent components are checked with `symlink_metadata`, and existing output symlinks are rejected. Existing Unix tests demonstrate root and parent symlink rejection in ordinary, non-racing conditions.

## Symlink handling

Snapshot enumeration uses `WalkDir::follow_links(false)` and rejects discovered symlink entries. Restore also rejects symlinked root/parent/output paths.

There remains a local TOCTOU class between path checks and subsequent opens/creates: another process able to mutate the same directory can replace a checked directory with a symlink. Strong hardening would require descriptor-relative/openat-style operations or equivalent platform-safe directory handles. This audit does not rank that above the confirmed findings because exploitation requires concurrent local filesystem mutation by an actor already able to write the target tree.

## Orphan blobs

Orphan complete blobs are safe from a correctness perspective because manifests reference exact hashes and GC is mark-before-sweep. The main risk is disk consumption. Publication ownership cleanup is conservative and well designed.

## Missing blobs

Missing blobs fail verification/restore. Retention refuses to prune when a mandatory recovery-root snapshot is absent. No silent zero-fill or missing-file acceptance was found.

## Duplicate snapshots

The dangerous case is not two filenames for the same number, because the filename is deterministic. It is replacement of the same numbered file by a different manifest, which SR-02 confirms is permitted.

## Import trust boundary

Import is considerably safer than copying directly into a visible world directory. It validates the source directory and nonempty regular `level.dat`, rejects symlinks through snapshot traversal, stages the entire SwarmCraft world under a hidden root, verifies the signed snapshot, and only then renames the staged world into the visible `worlds` namespace.

Import does not create a filesystem snapshot of the source. If another process mutates an ordinary file between enumeration and read, the resulting manifest commits to whatever bytes were actually read. The real player runtime path should continue to ensure imports are taken from quiescent saves; storage itself does not provide a coherent multi-file read transaction for a concurrently mutating source.

---

# Recovery ordering assessment

The strongest ordering in the current design is snapshot publication:

`source read -> temp blob -> file sync -> durable pin -> blob rename -> directory sync (Unix) -> verify all blobs -> manifest temp -> manifest sync -> manifest rename -> directory sync (Unix) -> release publication pins`

That ordering is sound on Unix for the blob/manifest/GC relationship.

The weakest ordering is the relationship among numbered manifests, canonical head selection, and authority generation:

- numbered manifests are mutable;
- canonical head is inferred from the highest surviving number rather than a durable head hash;
- commit is not atomically conditioned on the current epoch/fencing generation;
- recovery/control read-check-write transitions do not share a process-wide/per-world lock.

Those are semantic transaction problems rather than raw checksum problems.

---

# Test gaps required before closing this audit

At minimum, remediation should add deterministic tests for:

1. newest manifest deleted after successful commit;
2. attempted overwrite of an existing snapshot number with a different manifest;
3. same manifest repeat commit idempotency;
4. two processes racing conflicting recovery promises;
5. two processes racing epoch/control writes using the same data root;
6. authority epoch transition injected between final authority check and snapshot commit;
7. case-fold filename collision restored on Windows/case-insensitive macOS;
8. Windows reserved filename/trailing-dot/trailing-space collision handling;
9. process kill after each restored file rename;
10. crash after removing old restore output but before temp rename;
11. crash after metadata deletion and before parent-directory durability barrier;
12. crash after manifest rename but before/while parent sync;
13. Windows power-loss durability for file and directory publication;
14. missing head target recovered from a peer without automatic rollback;
15. direct `load_snapshot` world/number namespace substitution;
16. bounded-memory behavior for `Storage::read_blob` or removal of that API.

A valuable test architecture would expose explicit fault-injection hooks around each meaningful persistence operation: create temp, write, file sync, rename, directory sync, control lock acquisition, head update, pin creation, pin release, delete, and restore publication. Each test can terminate/reopen at every hook and assert the same small set of invariants.

---

# Recommended remediation order

1. **Introduce immutable snapshot-number publication and a durable canonical-head hash/number.** This closes SR-01 and SR-02 and gives recovery a trustworthy missing-head detector.
2. **Introduce a per-world OS-backed storage transaction lock.** Use it for recovery promises, epoch/control transitions, head advancement, and fenced snapshot commit. This closes SR-04 and provides the mechanism needed for SR-05.
3. **Make snapshot commit authority-fenced.** Bind expected epoch/fencing token/current head to the commit transaction.
4. **Define portable path identity and reject cross-platform aliases before snapshot publication/restore.** Close SR-03 before relying on cross-platform replicas.
5. **Make general restore transactional or explicitly journaled.** Preserve the runtime-directory reset mitigation but harden the reusable storage API and export/recover paths.
6. **Centralize durable write/delete helpers and remove fixed shared temp names.** Add parent sync after deletion and unique temp publication.
7. **Prove Windows crash durability or document a weaker supported guarantee and implement recovery detection.**
8. **Harden namespace validation and cleanup.** Bind `load_snapshot` to world/number, scavenge safe stale temps, and eliminate/deprecate the unbounded legacy blob-read API.

---

# Final assessment

SwarmCraft's blob-level integrity mechanisms are significantly better than the typical "copy files and hope" design. Hash verification, durable publication pins, GC serialization, bounded streaming decode, conservative retention roots, import staging, and fail-closed corrupt-manifest behavior are all meaningful defenses.

The remaining failures sit one layer above those primitives. The system does not yet have a crash-proof definition of **which** manifest is the durable canonical head, does not make numbered history immutable, and does not serialize control/head decisions across processes and authority transitions. Those gaps are exactly where a storage system can have every individual file hash correctly while still accepting the wrong history.

**VERDICT: FAIL**
