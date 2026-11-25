# Sabathon #51 - managed runtime resilience and dependency health

Verified locally, 25 November 2026. No hosted CI, release, or main change.

## Problem and implementation

The original installer used only per-world locks while writing machine-wide
Java and JAR paths. It also unlinked its advisory lock after unlocking, moved
the last known-good files/directories away before publishing replacements,
and could adopt an unreceipted Fabric launcher (which has no published
checksum) as if previously verified. An extracted Java runtime was accepted
based only on a version probe.

The installer now holds stable OS-backed per-world and shared publication
locks, plus the existing world authority-runtime slot and residual-process
check. Shared Java and JAR caches use immutable generations under a
source/expected-hash identity. The mutable lookup receipt is atomically
replaced only after verified bytes are prepared and synced. Previously
referenced paths, including old layout files, are not overwritten. A bad
refresh or unavailable source leaves last known-good generations and receipt
intact. Explicit repair creates a separate verified generation after detecting
corruption. Staging directories are disposable; do not delete older runtime
generations until a separate safe reference inventory/cleanup policy exists.

Every managed JAR record identifies its exact source, expected upstream SHA-1
or SHA-256 when available, local SHA-256 and verification classification.
Local bridge overrides are explicitly operator-supplied, not publisher
verification. The official generated Fabric server launcher has no upstream
checksum: require a valid JAR manifest and record the locally computed
SHA-256; label its limited HTTPS/local integrity separately. The selected
managed Java archive has an expected SHA-256 and extracted tree SHA-256
(internal symlink targets, filenames, file contents and Unix permissions).
Validate the extracted tree before executing the version probe and verify
it again before managed runtime launch. System or manual Java is explicitly
external/version-probed, not claimed to have verified distribution provenance.

Downloads are HTTPS-only with bounded retries, time/size limits, no user curl
configuration, exact initial host and final effective host validation.
SwarmCraft bridge metadata must identify the exact official repository,
release tag and exact asset/checksum names; ambiguous asset entries are
rejected. The installer no longer conflates unreadable managed metadata
with absent metadata, or a stale launch configuration with a current
prepared profile. A real host cannot launch a corrupt/mismatched managed
lock, staged JAR or Java tree: the Rust authority runtime checks them
inside the existing world runtime slot before changing the authority epoch.
No quorum, consent, EULA, fencing or signed compatibility rules are waived.

## Actual local acquisition

[Live two-world evidence](evidence/live-two-worlds-20261125.json) was obtained
from two real private worlds in a disposable data root, using the actual
candidate CLI/runtime sidecar. Cold install downloaded current official
Adoptium Java 25, Mojang Minecraft 26.1.2, Fabric Loader 0.19.3 generated
launcher and matching Fabric API; the bridge was the independently
hash-verified published 0.5.1 JAR explicitly supplied as a local override.
The first installer held the real shared OS lock, and a simultaneous second
world's sidecar install was denied. Retry after release succeeded, sharing
the identical Java generation and all four JAR records. Both signed
world checkpoints remained unchanged, and the status was prepared but not
launchable without explicit EULA acceptance. No Minecraft server or
licensed client was started. Initial live attempt correctly rejected a
group-writable ancestor of the temporary data directory; rerun used a
private test directory and retained the safety guard.

The runtime-sidecar smoke preceded the Postcard feature pruning below; the
same previously written world data and cache were subsequently read by the
recompiled dependency graph. [Actual cache outage/corruption evidence](evidence/live-cache-outage-20261125.json)
records five positive checks. With a failing curl stub confined to these
sidecar subprocesses, both installed worlds remained verifiable/reusable;
a forced repair failed and preserved all shared lookup receipts. Altering
the shared generated launcher and separately a native Java library in
the disposable cache caused both worlds to report Corrupt. The test
restored the exact bytes and confirmed unchanged receipts, world locks
and verified signed checkpoints. No system network settings, real user
profiles or live Minecraft runtime were changed. These tests do not claim
that the published 0.5.1 binary contains the candidate changes.

## Dependency health

Both root and Desktop lockfiles previously had nine RustSec heapless 0.7.17
soundness warnings and an atomic-polyfill maintenance warning. Postcard
1.1.3's default heapless API is not used by this repository; disabling that
feature while retaining alloc removes the unused dependencies from both
graphs. No Postcard version or V1 encoding change was made. The lockfile
resolver's unrelated syn choice was reverted; exact locked builds pass.
[Before/after evidence](evidence/dependencies-20261125.json) records every
remaining advisory. Both audits reported zero vulnerability findings
against the locally available 23 November advisory database, not a
guarantee of no newly reported vulnerabilities. Root still has one
unmaintained paste warning; Desktop has seven maintenance warnings and the
glib 0.18.5 soundness warning. They are neither suppressed nor marked
resolved; upstream Tauri/GTK and dependency review remains necessary.

## Scope and remaining gates

- 300 Rust tests passed across protocol, core, storage, consensus, installer
  and runtime-hardening (two existing ignored tests not credited).
  All 11 runtime-hardening tests ran without the low-memory early-return
  marker. Initial run under the workstation's permissive umask correctly
  failed ownership checks; fixture temp directories now use private modes.
- Strict targeted Clippy, locked native Desktop compile, Rust format and
  15 CI artifact-acquisition fixture tests (including 503/504, exact API
  fallback and corrupted inputs) passed. No new hosted workflows.
- A whole-world multi-file local staging interruption can leave the affected
  world unready until repair, but its previous shared generation survives
  and a mismatch is fail-closed. Fresh-world install still requires online
  source metadata; an already prepared world can reuse verified artifacts
  without network. Existing managed Java locks without extracted-tree
  provenance need explicit repair; older binaries and production profile
  migration/downgrade acceptance are tracked separately in #52.
- Final URL is constrained, not a claim of full per-hop redirect confinement;
  archive extraction relies on the platform tar tool and trusted archive
  digest. Cache/receipt checks defend normal failures and corrupted local
  bytes, not a malicious same-account or privileged filesystem actor.
- Actual native Windows/macOS Java archive and shared-lock behavior,
  sustained disk-full/process-kill campaigns, real live Fabric compatibility
  and provenance of Fabric's secondary dependency downloads remain separate
  qualification work. Existing system executables, manual artifacts, and
  generated launchers without publisher checksums are explicitly limited
  rather than declared publisher-signed.
