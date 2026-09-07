# Agent 3 — Storage Transactional Integrity

## Status

STATUS: READY FOR INTEGRATION

BRANCH: `fix/agent-3-storage`

CAMPAIGN BASE SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

ACTUAL REMOTE HEAD WHEN AGENT 3 CLOSURE WORK RESUMED: `03948a37d72112f0c17ba4dced89d92d75ca07f1`

AGENT 1 + 2 COMPOSITION ANCESTOR: `f02bb0d54cb44df67e730f01be4c903e25d670ff`

COMPOSED MILESTONE SHA: `3c6ca9bab5a9ee9b0d228a45a267c3fa8e2722a3`

EXACT VALIDATED PRODUCTION SHA: `67962dcb9c3cb2d5b9e67bb7288b2d786fc9e803`

POST-VALIDATION WORKFLOW CLEANUP SHA: `f39f61a12b704a35f5b366e44ecf659920a145b0`

COMPOSITION RUN: `33769105882` — SUCCESS

EXACT-HEAD VALIDATION RUN: `33769288028` — SUCCESS

## Mission

Make canonical storage history immutable, crash-detectable, generation-fenced, portable across supported filesystems, and safe under multiple local processes.

## Findings owned

- FINAL-008 — duplicate local daemons can equivocate recovery/control state
- FINAL-009 — missing newest manifest can silently roll head backward
- FINAL-010 — committed snapshot number can be replaced and commit is not atomically fenced
- FINAL-011 — portable path aliases can collapse on case-insensitive filesystems
- FINAL-027 — general restore/durability/namespace integrity gaps
- FINAL-041 — crash debris and legacy unbounded blob read

Read before editing:

- `audits/FINAL-AUDIT.md` from `audit/final-integration-report`
- `audits/03-storage-recovery.md` from `audit/storage-recovery`
- `implementation/agent-1-consensus.md` and `implementation/agent-2-protocol.md` for the shared authority-generation/direct-parent boundary

## Dependencies consumed

Agent 3 began independently from the campaign storage branch, then composed the authoritative Agent 1 + Agent 2 integration ancestor `f02bb0d54cb44df67e730f01be4c903e25d670ff` before final acceptance. That composition is present in branch history, not merely copied behavior.

The composition milestone is `3c6ca9bab5a9ee9b0d228a45a267c3fa8e2722a3`. The exact accepted tree at `67962dcb9c3cb2d5b9e67bb7288b2d786fc9e803` differs from that milestone only by one validation-trigger comment in `.github/workflows/agent3-final-validation.yml`; no Rust, tests, Cargo metadata, or product behavior changed between them.

## Ownership boundaries

Primary ownership:

- `crates/swarm-storage`
- storage-facing migration/checkpoint call sites as needed
- filesystem/crash/recovery tests

Agent 3 does not redesign authority election policy, discovery freshness policy, runtime quiescence policy, or FINAL-028.

## Implementation checklist

- [x] Introduce durable per-world canonical head record with exact snapshot number and manifest hash.
- [x] Publish blobs, immutable manifest, then atomically update/sync canonical head in documented order.
- [x] Detect missing/mismatched head manifest and fail closed instead of silently rolling back.
- [x] Ensure snapshot numbers cannot be reused after head loss.
- [x] Make committed numbered manifests create-only/immutable; exact duplicates are idempotent.
- [x] Add expected-parent/head comparison to commit path.
- [x] Add OS-backed per-world control lock around recovery/control read-validate-write transactions.
- [x] Ensure duplicate daemons/processes sharing a data root cannot both emit conflicting accepted promises.
- [x] Add a fenced snapshot commit primitive binding expected epoch/fencing generation and expected current head.
- [x] Define portable path collision identity for case-folding, trailing dot/space, reserved names, traversal/root/drive forms, backslashes, and supported canonical character policy.
- [x] Reject path aliases before publication/restore.
- [x] Make general restore persist an explicit incomplete marker/recovery protocol.
- [x] Reject pre-existing symlink components before restore mutation can escape the destination.
- [x] Sync durable metadata deletions consistently in remediated control/restore/replica paths.
- [x] Implement fail-closed namespace detection through durable commit intent + required-head/head-target consistency checks.
- [x] Validate `load_snapshot(world, number)` embedded world namespace and embedded snapshot number.
- [x] Add conservative stale-temp diagnostics without deleting possibly-live writer data.
- [x] Make the legacy blob decompression helper bounded and streaming.
- [x] Compose Agent 1 authority-generation and Agent 2 history/membership semantics into the Agent 3 storage tree.
- [x] Preserve Agent 2 membership promises/certificates under Agent 3 durable OS-backed world transaction locking.
- [x] Validate the composed production tree on Linux and the storage/failure-injection matrix on Windows and macOS.
- [x] Remove temporary Agent 3 remediation composition/repair/validation workflows after the exact accepted SHA was proven green.

## Work completed

- Added `transaction.rs` with a stable per-world kernel-backed exclusive lock, unique temporary files, durable atomic writes, durable create-once, durable delete, and parent sync helpers.
- Added `integrity.rs` with `CanonicalSnapshotHeadV1`, `CanonicalSnapshotRefV1`, exact manifest hashes, a durable required-head marker, a durable commit-intent record, one-time legacy head adoption, fail-closed incomplete-commit recovery, orphan detection, direct-parent/sequence checks, and `SnapshotCommitFence`.
- Snapshot publication verifies blobs and portable paths, takes the GC lock before the world transaction lock, writes durable intent, publishes a create-only manifest, advances the durable head, then releases publication pins.
- Missing/mismatched head targets, orphan manifests above head, conflicting numbered manifests, stale expected heads, and stale epoch/fencing tokens return explicit storage errors instead of silently selecting older state.
- Added conservative cross-platform path identity and alias rejection before publication/restore.
- Restore verifies source blobs before mutation, rejects existing symlinks in the destination tree before mutation, writes `.swarmcraft-restore-incomplete`, reconstructs the destination without following symlinks, verifies the exact resulting file set, and removes the marker only on success. `discard_incomplete_restore` remains the explicit recovery path.
- Recovery promises/reservations, epoch/control records, world descriptor/membership/pending membership state, background seeding, solo-branch writes, recovery certificates, membership promises, and membership certificates serialize durable mutations through the per-world OS lock.
- Replicated blob finalization syncs the blob directory after deletion/rename transitions.
- `load_snapshot` validates embedded world/number identity and canonical namespace; `latest_snapshot` and `next_snapshot_number` are driven by the durable canonical head rather than directory sorting.
- Legacy `read_blob` checks encoded size and streams with a 256 MiB hard maximum instead of unbounded decompression.
- Added stale transaction-temp diagnostics through `storage_temp_debris`.
- Added permanent unit/integration regressions for immutable snapshot slots, direct-parent history, portable aliases, incomplete restore markers, missing head targets, fencing-token changes, prepared stale-writer races, namespace/number mismatch, stale-temp diagnostics, bounded decompression, and concurrent promise/control races.
- Added an integration regression that launches two independent processes against the same storage root and proves conflicting equal-round recovery promises cannot both be accepted.

## Canonical head and rollback guarantee

`CanonicalSnapshotHeadV1` stores the `world_id` plus an optional `CanonicalSnapshotRefV1`. The reference binds the exact `snapshot_number`, `manifest_hash`, `epoch`, and `sequence` of the canonical snapshot. Storage also persists the required-head marker and commit intent needed to distinguish a missing newest canonical target from an intentionally older history.

The canonical head is authoritative. `latest_snapshot` does not silently scan surviving numbered manifests and reinterpret an older one as current. If the required current manifest is missing, mismatched, orphaned, or inconsistent with the durable head/intent state, storage fails closed. Snapshot numbering is not reused after canonical-head loss. Same-number conflicting manifests are rejected while exact duplicates remain idempotent.

`SnapshotCommitFence` additionally binds the exact previously observed head and expected epoch/fencing token while the per-world transaction lock is held. A publication prepared before authority generation/fencing advances cannot commit afterward with stale ownership.

## Cross-process non-equivocation guarantee

Recovery/control promise read-validate-write transactions use the OS-backed per-world world-transaction lock, not only in-process synchronization. The permanent two-process regression opens the same storage root in genuinely separate processes and proves that two conflicting equal-round recovery promises cannot both be accepted. Agent 2 membership promise/certificate persistence is composed under the same durable locking boundary.

## Agent 1 + Agent 2 composition

Authoritative ancestor composed: `f02bb0d54cb44df67e730f01be4c903e25d670ff`.

Composition run: `33769105882` — SUCCESS.

Composed milestone: `3c6ca9bab5a9ee9b0d228a45a267c3fa8e2722a3`.

The exact storage-local conflicts resolved during composition were:

- `crates/swarm-storage/src/control.rs`
- `crates/swarm-storage/src/lib.rs`
- `crates/swarm-storage/src/root.rs`
- `crates/swarm-storage/src/state.rs`
- `crates/swarm-storage/src/streaming.rs`
- `crates/swarm-storage/src/world.rs`
- `crates/swarm-storage/tests/publication_ownership_race.rs`

Resolution preserved Agent 1/2 authority, membership, semantic validation, direct-parent/history, and checked-counter contracts while retaining Agent 3 durable transaction locking, canonical-head/restore behavior, namespace integrity, and fenced commit machinery. The focused composition proof passed storage compile/tests, `membership_history`, `swarm-protocol`, and `swarm-core` before the merge commit was pushed.

## Durable identity available to future Agent 4 work

Agent 3 now exposes a deterministic, durable storage identity that future Agent 4 freshness/discovery work may consume read-only:

- `CanonicalSnapshotHeadV1 { world_id, head }`
- `CanonicalSnapshotRefV1 { snapshot_number, manifest_hash, epoch, sequence }`
- `SnapshotCommitFence { expected_epoch, expected_fencing_token, expected_head }` for commit ownership/fencing at the storage boundary

This is the storage anchor only. Agent 3 did **not** implement FINAL-028, discovery authority certificates, TOFU behavior, network freshness proofs, or the authority-freshness bridge. Agent 4 remains responsible for authenticated discovery/network freshness semantics.

## Exact-head validation evidence

Exact validated production SHA: `67962dcb9c3cb2d5b9e67bb7288b2d786fc9e803`.

Validation run: `33769288028` — SUCCESS.

The exact-head Ubuntu job completed every required gate successfully:

- [x] exact event SHA assertion at start
- [x] clean worktree assertion at start
- [x] root locked metadata
- [x] Desktop locked metadata
- [x] `cargo fmt --all -- --check`
- [x] `cargo check --workspace --all-targets --locked`
- [x] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [x] full `swarm-storage` suite
- [x] missing newest canonical head fails closed without number reuse
- [x] immutable same-number snapshot conflict rejection / exact-repeat idempotence
- [x] direct expected-parent and exact-sequence validation
- [x] stale fencing-generation commit rejection
- [x] prepared stale writer loses the fenced commit race
- [x] cross-process conflicting recovery-promise non-equivocation
- [x] concurrent publication ownership/commit serialization race
- [x] portable path collision matrix
- [x] restore crash/incomplete marker and fail-closed symlink preflight
- [x] embedded snapshot namespace mismatch rejection
- [x] embedded snapshot-number mismatch rejection
- [x] stale transaction-temp diagnostics
- [x] bounded legacy/decompression behavior
- [x] Agent 2 membership and snapshot-history storage semantics
- [x] Agent 2 protocol suite
- [x] Agent 2 core suite
- [x] Agent 1 consensus suite
- [x] Agent 1 CLI library suite
- [x] Agent 1 adversarial membership / 3-peer and 5-peer partition / Solo-loss safety coverage
- [x] Agent 1 live membership replication
- [x] Agent 1 three-daemon recovery
- [x] Agent 1 recovery successor crash/resume safety
- [x] all workspace targets and integration tests compile
- [x] exact final SHA assertion
- [x] clean worktree assertion at end

Cross-platform storage evidence from the same run:

- [x] Windows `windows-latest` storage portability job — SUCCESS
- [x] Windows full storage suite — SUCCESS
- [x] Windows portable identity matrix — SUCCESS
- [x] Windows storage integrity process/namespace regressions — SUCCESS
- [x] Windows failure-injection restore regressions — SUCCESS
- [x] macOS `macos-latest` storage portability job — SUCCESS
- [x] macOS full storage suite — SUCCESS
- [x] macOS portable identity matrix — SUCCESS
- [x] macOS storage integrity process/namespace regressions — SUCCESS
- [x] macOS failure-injection restore regressions — SUCCESS

## Post-validation closure

The validated production SHA `67962dcb9c3cb2d5b9e67bb7288b2d786fc9e803` is the production/test acceptance anchor.

The next branch milestone, `f39f61a12b704a35f5b366e44ecf659920a145b0`, removes only these temporary Agent 3 storage-remediation workflows:

- `.github/workflows/agent3-compose-integration.yml`
- `.github/workflows/agent3-compose-repair.yml`
- `.github/workflows/agent3-final-validation.yml`
- `.github/workflows/agent3-restore-symlink-fix.yml`

`.github/workflows/agent3-curseforge-provider.yml` was inspected and deliberately preserved because it belongs to the separate `agent/curseforge-provider` campaign and is not a helper for this storage remediation branch.

No production Rust, test, Cargo metadata, permanent repository CI logic, or other product file is intentionally changed after the validated production SHA. Final closure is this ledger update only after the workflow cleanup milestone.

## Known integration seams

### Agent 4 — Network / FINAL-028

Agent 4 will need the integrated Agent 1/2 authority semantics plus Agent 3's durable canonical snapshot identity when it designs authenticated discovery freshness. The storage-side seam is read-only access to the canonical head/reference and durable epoch/fencing state. Agent 4 must not replace the canonical-head rules or infer freshness by scanning snapshot history. FINAL-028 and the authority-freshness bridge remain entirely unimplemented by Agent 3.

### Agent 6 — Runtime lifecycle

Agent 6 owns live Minecraft source quiescence, runtime supervisor/controller liveness, process containment, and import-while-running rejection. The known overlap is primarily `crates/swarm-cli/src/migration.rs` plus restore/import/checkpoint call sites that consume Agent 3 storage primitives. Agent 6 should preserve the transactional restore/canonical commit guarantees and layer authoritative Minecraft quiescence around them rather than weakening or replacing the storage boundary.

### Shared Agent 1 / Agent 2 integration surface

Known composition-sensitive areas remain snapshot commit APIs, per-world control locks, membership/recovery durable promise state, and daemon/migration authority checkpoint call sites. These semantics are already composed and validated on `67962dcb9c3cb2d5b9e67bb7288b2d786fc9e803`.

## Blockers

None.

## Remaining work

None in Agent 3 ownership. Integration coordinator should consume the final branch head after verifying the post-validation diff remains restricted to the four temporary workflow deletions plus this ledger file.

Do not start FINAL-028 or Agent 4 work from this ledger closure.

## Handoff

READY FOR INTEGRATION: YES

Exact validated production SHA: `67962dcb9c3cb2d5b9e67bb7288b2d786fc9e803`

Validation run: `33769288028` — SUCCESS

Composition run: `33769105882` — SUCCESS

Production tree accepted: `67962dcb9c3cb2d5b9e67bb7288b2d786fc9e803`

Integration source: final remote head of `fix/agent-3-storage`, provided its compare against the validated production SHA contains only the four temporary workflow deletions and `implementation/agent-3-storage.md`.

## Agent final statement

READY FOR INTEGRATION
