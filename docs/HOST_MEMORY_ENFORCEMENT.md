# Optional managed JVM memory enforcement

The selected contribution is still the Java heap in MiB. All existing
local consent, signed authority, fencing, quorum, and physical/available
memory admission requirements remain in force. A cgroup is an additional
local resource limit, never an authority grant.

## Linux (cgroup v2)

Without configuration, the managed JVM is launched with its existing
`-Xmx` limit and conservative memory headroom (heap-only mode).

An administrator can explicitly set `SWARMCRAFT_HOST_CGROUP_ROOT` to an
existing delegated cgroup-v2 directory. The hosting controller must
already be running in a descendant of that directory. The delegated
root must be owned by the local OS account, be a canonical real cgroup
directory below /sys/fs/cgroup, have no processes directly in it, and
have the memory controller enabled for children. Configure delegation
using the OS service manager, not by changing the global hierarchy or
moving unrelated processes. SwarmCraft does not enable parent controllers
or require root privileges. If the requested delegation cannot be used,
the launch fails closed rather than falling back to heap-only.

For each launch, SwarmCraft creates an isolated dedicated leaf and sets:

- `memory.max` = selected heap + the existing estimated native reserve
  (max of 1024 MiB and half the heap).
- `memory.high` = 90% of the charge limit, rounded down to a kernel page.
- `memory.swap.max` = 0; `memory.oom.group` = 1.

Java is attached by a pre-exec write to the already opened leaf's
cgroup.procs file. The controller verifies exact membership and settings
before accepting runtime readiness or responding to authenticated live
runtime challenges. Native allocations, descendants and other memory
charged by the kernel (including charged page cache) share the budget.
This is not a per-process RSS meter or a guarantee against a malicious
same-account process modifying its delegation. A different ancestor
limit may be stricter. Kernel OOM can terminate an allocation-heavy
runtime before a checkpoint; the last verified canonical checkpoint is
the recovery boundary, and no unproven save is reported as committed.

The supervisor requests the existing save-first stop when it observes
high-memory pressure or the configured enforcement can no longer be
verified. Both durable process ownership records bind the exact cgroup
device/inode and policy; a live descendant blocks a new launch, and an
unverifiable same-boot group is ambiguous and fails closed. Checkpointing
after Java exit requires an empty recorded group. Ordinary successful
cleanup removes the exact empty leaf only after both ownership records
are retired. Never delete those records to bypass an ambiguous launch.
A controlled reboot can establish that processes from the old boot have
exited, but cannot certify uncheckpointed progress.

The local CLI/native UI reports heap-only, configured but not yet active,
observed active cap and kernel current/peak charges, or unavailable
enforcement. Observations do not confer hosting authority, and the
legacy whole-process RSS guarantee field stays false. A missing peak
counter is shown as unknown.

## Windows and macOS

Windows remains heap-only. A Windows Job Object design would need
pre-execution assignment (for example, suspended creation), verified
job memory limits, nested-job and breakaway handling, and real process
tree lifecycle tests. No such guarantee is claimed until those gates
exist. macOS likewise remains heap-only; no tested per-runtime
descendant-inclusive kernel memory cap is available in this build.
Explicit Linux delegation configuration on either platform is rejected.

## Verification

The Linux acceptance gate runs in a disposable delegated systemd
scope/service, with a separate controller child. It does not modify
the host-wide cgroup policy. A real kernel test measures a parent and
descendant's memory charges together, checks the production high
threshold, then separately removes high throttling in the disposable
test leaf to exercise hard-limit group OOM at 64 MiB. The host-process
fixture verifies the actual launcher attaches before exec and uses
1536 MiB for a 512 MiB heap, then commits and restores its saved marker.
A second kernel test confirms the account-wide and world records block a reset while a descendant survives the parent, then allow recovery once the group is empty. These are real OS enforcement and mock-runtime lifecycle tests, not
field certification with installed Minecraft and mods.

On a suitable Linux workstation, after compiling the test binary:
```bash
systemd-run --user --scope --unit=swarmcraft-memory-cap-test-local-$$ \
  -p Delegate=memory -p RuntimeMaxSec=90 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  /absolute/path/to/swarm_cli_test_binary \
  --exact host_memory_cap::linux::tests::real_cgroup_enforces_native_and_descendant_memory_before_exec \
  --ignored --nocapture
```

Core CI executes the kernel and launcher gates using dedicated transient
services. Results for the exact candidate commit remain necessary.
