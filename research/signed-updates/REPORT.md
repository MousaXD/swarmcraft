# #54 signed update delivery acceptance

**Date:** 2026-11-25. **Scope:** local Linux packaged CLI, offline
signing and verified staging, not live release publication or OS update
installation.

## Implementation

- Explicit update status/configure/check/stage/verify-staged commands
  run before canonical storage open. Default off; independently
  authenticated Ed25519 pins per stable/preview, no trust-on-first-use,
  pin substitution or shared key. Opt-out retains replay state.
- Strict signatures for the raw manifest and domain-separated package
  descriptor. Expiry, channel, monotonic sequence, same-sequence
  equivocation, version, immutable release tag, OS/architecture/format,
  protocol, storage schema and profile reader generation checks.
  A newer accepted offer fences an older stage even if the transfer fails.
- Bounded HTTPS source/redirect allowlist and offline file delivery,
  byte-count/SHA-256 streaming, private atomic staging, free-space
  reserve, local tamper detection, no installer execution.
- Offline publisher re-verifies exact commit-bound provenance, package
  bytes/checksums, Debian identity/version/architecture, AppImage header
  and production native signing receipts; requires externally reviewed
  key fingerprint. It never performs release publication. No dependency
  version changed; the CLI explicitly reuses workspace Ed25519.

## Evidence

Seven Rust verification tests and the local acceptance harness exercise
real OpenSSL signatures verified by the Rust client, wrong keys, manifest
tampering, duplicate/unsafe descriptors, invalid metadata, expiry, replay,
channel separation, actual package byte corruption/truncation/oversize,
and restricted URLs. No test credentials are shipped.

The actual historical 0.5.1 preview assets (commit
`aaffac22e32547983f1fd558af6eb32c0da4917b`) were reverified and
locally signed with an ephemeral key. The running current candidate
correctly refused that equal-version offer. An inert, valid next-version
0.5.2 Debian archive and synthetic other-platform records were used
for positive acceptance. A signed stable fixture uses the distinct
ephemeral stable key; it is not a real production-signed native release.
No test package was installed or executed. Incorrectly labelled arm64
Debian metadata with updated checksums and provenance was refused.

A new locally built Desktop Debian package was extracted, and all four
sidecars matched the exact local build bytes. Fifteen end-to-end checks
passed against its actual CLI/import sidecars in a loopback-only network
namespace. The offline profile used the real 31-file Java save from
[#52](../profile-upgrade/REPORT.md), imported into a new identity, with an
explicit consent opt-in and withdrawal. All non-update profile bytes,
identity, verified signed checkpoint and withdrawal revision stayed
unchanged. The enabled online check failed without external network
access; trust/replay state remained unchanged. Disabled checks never
attempted online delivery or initialized a new profile.

Exact hashes and per-check results:
[Linux package acceptance](evidence/linux-package-20261125.json).
The initial fixture construction failed on private Debian control
directory modes; the test was corrected to use normal archive member
permissions inside the private disposable lab. No failed run is counted
as a pass.

## Reproduce

Use a newly built candidate extracted from the actual Linux Desktop
package, the previously verified pinned preview inventory, a quiescent
disposable Java save and a new output directory. Run in an isolated
network namespace; no real profile, key, or external service is required.

```sh
unshare --user --map-root-user --net sh -c 'ip link set lo up && exec python3 scripts/acceptance/signed_updates.py "$@"' sh \
  --bin /absolute/candidate/usr/bin \
  --save /absolute/disposable/java-save \
  --published /absolute/verified/pinned-preview-assets \
  --artifacts /absolute/new/update-acceptance
```

See [signed-update operator contract](../../docs/SIGNED_UPDATES.md).

## Remaining qualification

No production channel key or endpoint was provisioned; there is no
automatic Desktop check, install or restart. Native Windows/macOS
signing verification, actual OS upgrade installation/rollback, positive
public HTTPS delivery, key custody and rotation/revocation, and real
next-version release publication remain separate operator/native gates.
The result is not a claim of native OS signature validation from the
Linux client or production readiness. No hosted CI, release or main
branch change was requested or performed.
