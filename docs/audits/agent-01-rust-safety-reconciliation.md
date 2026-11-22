# Agent 01 — Rust Safety & Recovery Reconciliation (PR #77 → main)

## Scope and baseline

- Repository: `MousaXD/swarmcraft`.
- Coordinator: Agent 00; Agent 01 does not merge or close PR #77.
- Current-main base at start: `45174310d1433116e64b9b6978cb4f2aa1c22c15`.
- Original PR: [#77](https://github.com/MousaXD/swarmcraft/pull/77), head `f71684c5b4ed9381161e0fb08fa7872ae993451d`, base `f6834afb5c21dbbabe8f9cedae4e5b0605b57c51`.
- Reconciliation PR: [#90](https://github.com/MousaXD/swarmcraft/pull/90), branch `agent-01/rust-safety-reconciliation`.

Reviewed `AGENTS.md`, `docs/RELEASE_GATES.md`, `docs/RECOVERY_ACCEPTANCE.md`, `docs/AUTHORITY_RECOVERY.md`, `docs/SNAPSHOT_REPLICATION.md` and `docs/FINAL_PLAYER_JOURNEY_ACCEPTANCE.md`. The safety contract requires valid durable promises, monotone epochs/fencing, quorum-backed authority, and safe canonical snapshot lineage. A corrupt canonical record is not new world state.

## Complete PR #77 file-level reconciliation

Each current-main blob matched PR #77's **original base blob exactly**. Each PR #77 head blob differs from it. Thus no PR #77 change to the eight files was incorporated into main and there was no file-level conflict. Porting uses the original PR head blobs, with no changes to any other production files.

| PR #77 file | Disposition | Reconciled safety change |
| --- | --- | --- |
| `crates/swarm-cli/src/daemon.rs` | **Missing → ported** | Distinguish missing from corrupt durable membership preparation in authority permit maintenance, membership transitions, request/response and certificate validation; fail closed on unreadable prepare. |
| `crates/swarm-cli/src/daemon_protocol_tests.rs` | **Missing → ported** | Ensure corrupted membership preparation blocks authority/recovery transition. |
| `crates/swarm-storage/src/control.rs` | **Missing → ported** | Checked epoch/recovery reservation presence; canonical readers reject symlinks and non-files; protect transitions and sleep/transfer records. |
| `crates/swarm-storage/src/integrity.rs` | **Missing → ported** | Checked canonical head/intent/required markers, snapshot directory/manifests and epoch presence; reject dangling symlink instead of legacy/bootstrap fallback. |
| `crates/swarm-storage/src/membership.rs` | **Missing → ported** | Durable membership promise cannot be overwritten after decode/presence failure; guarded certificate reads. |
| `crates/swarm-storage/src/state.rs` | **Missing → ported** | Preserve recovery promises, signed world config sequence and accepted epoch on corruption; canonical record reads. |
| `crates/swarm-storage/src/transaction.rs` | **Missing → ported** | `canonical_file_exists` and `read_canonical_file` use `symlink_metadata` and distinguish actual NotFound from invalid types/read errors. |
| `crates/swarm-storage/src/world.rs` | **Missing → ported** | Membership sequence/bootstrap and authority-epoch validation cannot bypass corruption; checked membership records/certificates/pending joins. |

No PR #77 changes were classified as already incorporated, partially incorporated, conflicting or obsolete **within these eight files**. No cherry-pick of PR #77's unrelated ancestry or merges was performed.

## Red regression evidence before source fix

1. Created branch from current main without changing production code.
2. Added `corrupt_membership_prepare_blocks_authority_and_recovery_transitions` only.
3. Red commit `24bb963a041650be752ff6a8d9b96f5d87b8260b`.
4. The Ubuntu Rust job in [Required Validation run #42466225204](https://github.com/MousaXD/swarmcraft/actions/runs/42466225204/job/128686266472) failed at `daemon_protocol_tests.rs:141` on the expected assertion: `ensure_no_membership_prepare(...).is_err()`. Existing logic treated corrupt preparation as absent. Format and Clippy had passed before the expected failing test.
5. The follow-up reconciliation replaces only the eight source/test files with the pre-reviewed PR #77 versions.

## Regression coverage retained from PR #77

- Corrupt durable membership preparation, recovery promise and membership promise.
- Invalid canonical world configuration and accepted epoch authority.
- Membership extension after invalid accepted epoch.
- Unix dangling symlink at membership promise/record, recovery promise/reservation, epoch, canonical-head required marker and snapshot directory.
- Existing monotonic recovery, majority/quorum, fencing, snapshot validation, restart and process-level tests remain enabled.

Prior source commit `f71684c5b4ed9381161e0fb08fa7872ae993451d` completed [Required Validation #42107678600](https://github.com/MousaXD/swarmcraft/actions/runs/42107678600) and [Main Desktop Installers #42107678830](https://github.com/MousaXD/swarmcraft/actions/runs/42107678830) successfully. These validate the original PR code against the earlier base, **not the newly reconciled main head**. Record green exact-new-head runs on PR #90 before merge recommendation.

## Residual risk / integration boundaries

- The checked `symlink_metadata` and subsequent read are not an `O_NOFOLLOW`, directory-FD-pinned solution against a privileged concurrent local filesystem adversary. Avoid claims of race-free symlink protection.
- Historical discovery/network freshness tests have had intermittent liveness failures (see PR #77). Never remove or weaken fail-closed security assertions to resolve a transient failure.
- A true missing record may follow bootstrap rules; invalid, unreadable, symlinked and non-regular signed control records must not.
- Agent 03 owns new resource enforcement; no new budget/admission logic, protocol changes, migration/host-readiness edits, frontend changes, release metadata changes, or workflow changes here.
- Automated CI is necessary but does not certify every real-world partition, crash, filesystem or hostile-peer condition.

## Merge authority

Agent 01 cannot merge PR #90 or close PR #77. Coordinator Agent 00 decides only after exact-head required validation and independent review.
