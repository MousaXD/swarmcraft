# Agent 01 — Issue #102 fail-closed recovery and retention continuation

**Date:** 2026-11-10  
**Coordinator:** Agent 00  
**Status:** DRAFT / BLOCKED — pending exact-head CI, independent Agent 07 review and approved integrated baseline.  
**PR:** https://github.com/MousaXD/swarmcraft/pull/103  
**Branch:** `agent-01/issue-102-fail-closed-recovery-retention`  
**Base:** `agent-01/rust-safety-reconciliation` (PR #90, safety SHA `e8092dd499ccaba1b2fc3470f93676c8ced1dac1`).  
**CI dependency:** PR #98 (`6966e1aebdf6abe7eb996b3a5809334245276d59`), unmerged as of report creation.

## Separation from PR #90

No existing PR #90 commits, source, tests or audit report were modified. PR #103 is a separate stacked draft with no changes to `main`, release workflows, hosting policy, UI, or migration. Agent 00 controls integration sequencing and any merge/retargeting.

## Source-level findings and narrow remediation

1. **Cached recovery ballot:** `daemon.rs` previously mapped `load_recovery_promise` failure to round 0, allowing an in-memory ballot to remain usable without reading its durable promise. Now only genuine `NotFound` counts as absence; before writing a quorum recovery certificate the daemon rechecks that the exact ballot hash matches the readable durable promise.
2. **Inbound `RecoveryEpoch`:** unreadable canonical promises were skipped by `if let Ok(promise)`. The handler now rejects unreadable promises and higher/same-round conflicts before writing any certificate or accepted epoch, including idempotent handling.
3. **Accepted epoch corruption:** inbound `RecoveryEpoch`, ordinary `Epoch`, membership, world-config, snapshot and transfer preflights must not fall back to bootstrap/no epoch after an I/O or decode error. `load_optional_epoch_record` distinguishes only genuine `NotFound`.
4. **Transfer history corruption:** `validate_transfer` previously considered unreadable previous transfer records absent, allowing a new Prepared transfer to pass the preflight. It now returns an error. `Storage::save_transfer_record` additionally refuses overwriting unreadable present canonical transfer evidence, including a dangling symlink.
5. **Discovery:** positive freshness votes and membership proofs are denied on unreadable local recovery/membership promises. Malformed pending membership state prevents local public publication.
6. **Recovery promise cleanup:** `clear_recovery_promise_after_epoch_advance` previously returned `Ok(false)` on any read error. It now reports corruption or invalid canonical shape and preserves the existing file; only a genuinely absent record is non-error.
7. **Snapshot retention / blob GC:** all five canonical transfer, sleep, recovery promise, recovery certificate and solo-branch roots are validated with `symlink_metadata` as regular files. Invalid roots or required snapshot hashes prevent manifest pruning **and** direct blob garbage collection. Ordinary missing roots preserve previous behavior.

These are reviewable fail-closed source fixes; **a complete external exploit is not claimed**. Some source-level fallback paths may also have downstream write guards, so individual severity requires exact-call-path review rather than counting every `if let Ok` as an exploitable bypass.

## Regression-first evidence

- **First tests-only commit** `a8bc4441c9e6ec37eb81a0a65cdc9d9ba424dbc5` added failing expectations for corrupt promise cleanup and five dangling canonical roots. Its [Required Validation #42846807990](https://github.com/MousaXD/swarmcraft/actions/runs/42846807990) and [Installers #42846807973](https://github.com/MousaXD/swarmcraft/actions/runs/42846807973) were queued when inspected; a terminal red CI run **has not** been captured. Do not claim executed red-before-green.
- Subsequent negative tests cover corrupt and dangling promises, exact cached ballot checks, higher/same-round conflict, accepted epoch corruption, discovery freshness, non-file canonical roots, direct GC, retained snapshot/blob integrity, valid missing-record cases and corrupted previous transfer.
- [Required Validation #42849109243](https://github.com/MousaXD/swarmcraft/actions/runs/42849109243) at `26e533759a978d517285daef37f1ca41ef2b9ac7` showed Windows Rust, process acceptance, live Minecraft, network impairment and Rust dependency audit success, but Ubuntu Rust **failed `cargo fmt --all -- --check`**. Exact rustfmt hunks were fixed in subsequent commits; this superseded SHA does not certify latest changes.
- The most recent full [Required Validation](https://github.com/MousaXD/swarmcraft/actions/runs/42860311517) and [Main Desktop Installers](https://github.com/MousaXD/swarmcraft/actions/runs/42860311450) were pending on `8cbf35c26521255ce9e789f075cca6486bdd9ae2` when the report was drafted; later report edits create a new commit and require new exact-head runs.

## Remaining release and security requirements

- Terminal success of the full **same-commit** Required Validation and soak-enabled Main Desktop Installers (Ubuntu, Windows, macOS, strict Clippy, rustfmt, package/GUI, network/partition and actual Minecraft).
- Red-before-green execution evidence still needs a reproducible failing test run against the unpatched version; the tests-only commit exists.
- Independent Agent 07 review and coordinator assessment of any remaining daemon/storage race windows.
- Preserve conservative guards and partition negatives; never waive CI or claim that symlink metadata checks close privileged local TOCTOU races.
- Hosting policy V2 migration, candidate eligibility, per-device consent/resource budgets and daemon integration remain Agents 02/03/09/10/11 + Agent 00 coordinated **separate** work, not changes in #103.
- PR #90 and historical #77 remain unmerged until Agent 00 approves CI #98 integration and fresh exact integrated SHA evidence; issue #102 stays open.

**No merge, no close, no `main` modification and no release.**
