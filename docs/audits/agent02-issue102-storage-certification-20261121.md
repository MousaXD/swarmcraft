# Agent 02 — Issue #102 recovery and storage certification ledger

**Date:** 2026-11-21 (UAE)  
**Scope:** storage integrity, durable promises, recovery certificates, retention/GC roots.  
**Release decision:** **HOLD**. This ledger is a source-and-test handoff, not an independent certification. Agent 10 has the independent security veto; Agent 00 is integration owner.

## Exact source sequencing

- Already merged **PR #109**, as present in main at `badcbc6e73b42f37a4287c2c2e03e72e50e832ce`, contains the original #102 durable-promise and canonical five-root guards. Do **not** duplicate that work.
- [PR #119](https://github.com/MousaXD/swarmcraft/pull/119) is **tests-only RED** for solo-conflict and in-flight pin-root corruption. Current-main replay head `b632d46095955f2d616738841e3f7dc6af2f7e82`; [Required #44816131304](https://github.com/MousaXD/swarmcraft/actions/runs/44816131304). This latest run was queued at this ledger's creation. Older network-derive failures were unrelated to these tests.
- [PR #123](https://github.com/MousaXD/swarmcraft/pull/123) is the **storage retention/pin-root fix**, now on main `1ef18fcc373d0e61c0c3fbe1bdb92e1ff264a945`. The code at head `fdf123a43cecc9e029ac22d298a5eeec36a2761a` also includes Agent 10's exact live-lease adversarial test from independent [PR #124](https://github.com/MousaXD/swarmcraft/pull/124). An earlier PR #123 Ubuntu job failed *only* on two rustfmt hunks in `state.rs` (now corrected); Windows/macOS jobs hit the superseded `swarm-network` derive issue (new main incorporated). Fresh exact-head Required and Installers are not yet terminal.
- [PR #129](https://github.com/MousaXD/swarmcraft/pull/129) is **certificate RED tests-only**; latest head `1c5f411f9ca56b6597786454fc2fa237de9f6330`. [Required #44745130129](https://github.com/MousaXD/swarmcraft/actions/runs/44745130129) Windows Rust job `136342001118` executed **seven targeted recovery_v2 failures**. Ubuntu stopped at rustfmt; do not call that a target RED.
- [PR #131](https://github.com/MousaXD/swarmcraft/pull/131) is the **certificate-only fix**, head `68b9ff563497ec185017738b0948a5a0ebcd4e4e`, only `recovery_v2.rs` in its diff. [Required #44745133204](https://github.com/MousaXD/swarmcraft/actions/runs/44745133204) Rust jobs Ubuntu `136340130485`, Windows `136340130842`, macOS `136340130622` all **PASS**, including all original negative certificate tests. The broader Required and Installers workflows **FAIL**, so this is not release approval.
- [PR #134](https://github.com/MousaXD/swarmcraft/pull/134) is a **new stacked RED-only** interrupted-promotion test on top of PR #131, head `bdab2f0ec967e2c2be436279524bcfdd8b5cb3e8`. [Required #44815941768](https://github.com/MousaXD/swarmcraft/actions/runs/44815941768) was queued when recorded. This new risk is not fixed by PR #131.

## Classification by required adversarial case

| Case | Current classification | Owner/evidence |
|---|---|---|
| Missing vs unreadable recovery promises | Existing fail-closed guard in main; not newly implemented | #109; `state.rs`, `transaction.rs` |
| In-memory ballot followed by corrupt durable promise | Existing canonical reread + exact ballot hash; valid signatures rechecked | #109; `daemon_protocol_tests.rs` |
| Inbound signed recovery record with unreadable local promise | Existing fail-closed path | #109; `daemon.rs` |
| Accepted-epoch corruption | Existing fail-closed guards; certificate-specific guard added in #131 | #109, #131 |
| Same-round/older certificate rollback and concurrent writers | **Executed RED and GREEN storage tests** | #129 Windows Rust job / #131 three-platform Rust jobs |
| Existing malformed / symlinked recovery certificate | **Executed RED/GREEN**; local filesystem caller is still trusted for nonprivileged path confinement | #129/#131 |
| Five canonical retention roots (transfer, sleep, promise, certificate, solo branch) | Existing fail-closed handling for dangling/nonregular files, both prune and direct GC | #109, `retention_gc.rs` |
| Solo-conflict files, parent directory and idempotent conflict preservation | New tests and source correction; exact current-main GREEN not yet terminal | #119/#123 |
| Replication and snapshot-publication pin-root corruption | Source fix plus exact Agent 10 live-lease reproduction added to fix branch; independent GREEN/RED still pending | #119/#123/#124 |
| Restoring a valid canonical promise after corruption | Test exists; exact new-head execution pending | #123 |
| Two concurrent promise candidates for one recovery round | Storage transaction-lock regression exists; exact new-head execution pending | #123 |
| Interrupted certificate `save` before accepted epoch `save` | **OPEN**. Existing accepted-generation certificate can be replaced by unaccepted future-generation certificate; if interrupted, restart cannot reliably use the original quorum proof | RED-only #134; coordinated storage/daemon protocol needed |
| Privileged filesystem replacement between metadata validation and read/write | **OPEN**. `symlink_metadata` checks are not fd-relative/no-follow race-free confinement | Future storage architecture / Agent 10 |

## Operationally independent release blockers

At certificate PR #131's full [Required #44745133204](https://github.com/MousaXD/swarmcraft/actions/runs/44745133204):

- RustSec root/dependent audits fail on **quinn-proto 0.11.18** (`RUSTSEC-2026-0524`, `-0523`) and **quinn-udp 0.5.15** (`RUSTSEC-2026-0538`); routing to Agent 00 / dependency-security owner, not fixed inside this storage PR.
- Live clean-machine Minecraft journey failed and Linux installed WebKitGTK GUI smoke failed (missing `webkit2gtk-driver` package). The main installer workflow consequently failed even though its multi-GiB network soak passed.
- **Never waive** a required gate based on a passing unit-test subset. The current effective combined PR #123 + #131 source is not yet independently evaluated.

## Required next certification actions

1. Observe targeted RED for #119 and independent #124, then the identical live-lease reproduction GREEN on #123's exact head.
2. Complete combined retention + certificate storage tests on a single **current-main** effective SHA, without discarding Agent 07/10 reviews.
3. Independently review the cross-call certificate/epoch crash window; implement separately with coordination from Agent 03, including both accepted and staged retention roots and post-crash recovery.
4. Obtain Agent 10 security disposition and Agent 00 integration decision after full exact-head Required Validation and soak-enabled Main Desktop Installers.
5. **Keep #102 open**, all RED-only branches unmerged, and no release permission inferred from this ledger.
