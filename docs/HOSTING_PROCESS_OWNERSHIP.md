# Hosting admission across local data roots

A local world must still have explicit current hosting consent, enough
physical/available memory and the accepted authority generation, fencing and
quorum. The process reservation adds a **denial-only** gate; it grants none
of those permissions.

## Linux

One SwarmCraft authority JVM reservation is shared by all supported launch
paths in the same OS account, even when different `--data-dir`,
`SWARMCRAFT_DATA_DIR`, HOME or XDG values are used. Replica-only profiles
do not consume that account-wide reservation. The kernel file lock is held
from admission, before canonical epoch mutation, until Java has exited and
the supervisor has finished its checkpoint lifecycle.

The reservation lives in the account's passwd-resolved home, in a private
`.swarmcraft-machine-hosting-v1` directory. The stable lock file must never
be deleted to resolve contention. The home and reservation must be on
recognized persistent local filesystems (ext, XFS, Btrfs, ZFS, F2FS, JFS,
ReiserFS or UBIFS); remote, temporary, unsupported or untrusted storage
fails closed. The reservation is not a global coordination mechanism
between multiple OS users, independently mounted homes, containers or
hosts.

A durable pending runtime record is written both in the shared reservation
and the selected world's control directory before Java launch. After spawn,
the controller records the actual child PID and Linux birth identity: local
machine, boot ID, PID namespace and kernel process start ticks. Both copies
are synced and atomically replaced. A live or ambiguous previous record
denies another launch, even if the previous controller was killed and its
kernel lock released. A recycled PID with different birth time is not
mistaken for the previous JVM; a mismatched machine or PID namespace is
not proof that the original process is dead. A definitely completed earlier
boot of the same machine proves all processes from that boot are gone.
Legacy version-1 records are still read conservatively: a live PID or
unresolved pre-spawn record blocks admission.

The account reservation is a conservative one-JVM policy, not an RSS
limit or a claim about arbitrary independently launched software. Use
one local machine/account and local data roots. For upgrades from an older
binary, safely stop all previous SwarmCraft hosts before using the new
reservation; older binaries do not participate in the new account lock.
Do not mix versions that use different local coordination rules.

## Recovery

- A normal successful shutdown retires the runtime records after process
  liveness is checked and keeps the stable account lock file.
- After a controller crash, do not delete the shared or per-world records,
  switch data roots to bypass them, or kill a process just because a PID
  is recorded. The runtime is intentionally blocked until the recorded
  process is proven gone. A PID alone is not an instruction to terminate.
- A same-boot pending record without a recorded child is ambiguous.
  Safely investigate the existing runtime and retain its save data. A
  controlled reboot of the same Linux machine proves that its old processes
  no longer execute; it does not certify uncheckpointed saves as canonical.
- On Windows and macOS the existing per-data-root lock and durable
  conservative liveness check remain. Windows uses a kernel process-handle
  wait, not an exit-code sentinel; account-wide coordination and platform
  process-birth tracking are not yet claimed there.

## Automated evidence

Linux tests use disposable roots and separate child-process handles to
prove real file-lock exclusion, hard controller termination with a surviving
JVM fixture, denial from an empty second root, and safe admission after the
test-owned surviving process exits. Additional tests cover PID reuse,
machine/PID-namespace mismatch, pending interruption and reboot, malformed
and symlinked records, and normal record retirement. The mock Fabric
migration and host-process gates exercise the actual production admission
path without modifying authority or consent rules.

This verifies local lifecycle coordination. It is not real Minecraft
gameplay, physical multi-machine acceptance, or cross-platform process-cap
certification.
