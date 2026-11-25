# #52 local profile upgrade acceptance

**Date:** 2026-11-25. **Scope:** Linux, real published Debian packages and a
locally built candidate; quiescent profile migration, not a live authority
or OS installer upgrade.

## Real save and package chain

A real Minecraft Java Edition 26.1.2 server, using the previously verified
official server JAR and managed Java 25 runtime, generated a disposable
world on loopback with online mode enabled. Existing explicit EULA
acceptance was reused; no new acceptance or user world files were created
or changed. The bounded server (1536 MiB heap) flushed and stopped cleanly.
The save contained 31 files, 2,215,315 bytes and five nonempty region files.
No licensed client login or gameplay claim is made.

The actual published 0.5.0 importer imported that save; actual published
0.5.1 reopened it and recorded absent consent, explicit 512 MiB consent,
then explicit withdrawal. The locally built 0.5.1 candidate preflight
made no writes. A real candidate process was killed waiting on the held
activation lock. A separate simulated partial staging write was left on
disk, never promoted. The candidate then activated the complete floor
and reopened twice. The original identity, checkpoint hash, saved bytes,
canonical records, and withdrawn consent revision remained unchanged.
The historical releases persist the latest consent revision rather than
a complete historical audit log; the test captures the transitions.

Both historical-target preflights rejected initialized data without
changing it; absent/empty roots passed without initialization. Legacy
epoch-0 history correctly refused certified encrypted backup. Verified
legacy snapshot recovery restored all 31 source files exactly. A separate
fresh current identity/world imported that data, preserved its state
root, granted no hosting or EULA consent, and passed the certified
encrypted export/import round trip with byte-exact world payload and
an untouched recipient profile.

Candidate Debian package SHA-256:
`9d6b8525527618be36e9c2ed7ca5ddbe056cb661dacf4b37471ac4222419d594`.
All four extracted sidecars matched the local build outputs byte for byte.

The actual candidate Desktop, in an authenticated private Xvfb session
inside a loopback-only network namespace, reopened the upgraded profile
twice. The UI selected the real world, independent CLI verification
confirmed its signed checkpoint and unchanged withdrawn consent revision,
and the original canonical/consent bytes matched after process shutdown.
A separate future-reader profile visibly failed startup and retry without
mutation.

## Implementation and reproducibility

- `swarm-storage::profile`: bounded read-only compatibility checks, stable
  lock, atomic floor publication and denial of unsupported schema/protocol.
- CLI `profile check` runs before opening storage; world import and Desktop
  canonical creation respect the same guard before identity/staging writes.
- Seven profile unit tests, two real CLI profile tests, 16 world-import tests,
  storage/core/protocol/consensus regressions, runtime setup hardening,
  checkpoint backup, import preview, targeted strict Clippy and native
  Desktop compile/build passed. Two existing ignored tests were not counted
  as passes.
- [Real Java save](evidence/java-save.json), [ten package-chain checks](evidence/package-upgrade.json),
  [native Desktop stages](evidence/native-desktop.json).
- `scripts/acceptance/profile_upgrade.py` consumes the previously verified
  published packages, extracted candidate sidecars, and a real quiescent
  save; use fresh private artifacts outside the save. The companion
  `native_profile_upgrade.py` requires a loopback-only network namespace,
  the completed acceptance artifacts and matching candidate sidecars.
  See its --help for inputs.

An initial small NBT sample was identified as a Bedrock fixture and was
not counted as real Java acceptance. An initial backup assertion compared
the entire restore directory rather than its documented world subdirectory;
the corrected final package run passed all ten checks.

## Limits

This is a metadata-only reader-floor activation, not a wire or storage
schema bump, an epoch/history conversion, or certification of hosting with
an older adapter. The interruption was before activation publication, not
a physical power loss during fsync. The historical binaries themselves
predate the guard: their direct invocation cannot be retroactively
blocked. The supported rollback workflow explicitly preflights with the
current reader and refuses initialized data; never downgrade a live
identity. Windows/macOS native installation replacement, native profile
tests, and any future incompatible schema migration remain separate gates.
No production profile, real authority, main branch, release, or hosted CI
was changed.

[Operator upgrade and recovery procedure](../../docs/PROFILE_UPGRADE.md).
