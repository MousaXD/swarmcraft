# #57 — Local device CPU and memory contribution verification

Date: 25 November 2026 UTC / 26 November 2026 UAE for the final local checks.
Scope: actual local consent, single-authority host admission and managed Linux
process enforcement; **not** regional Minecraft scheduling or field certification.

## Implementation

- [Operator contract](../../docs/DEVICE_RESOURCE_CONTRIBUTION.md): device-bound
  per-world requested heap and CPU, effective CPU capacity, explicit revisioned
  confirmation and zero withdrawal, with legacy heap-only compatibility.
- CLI, both Desktop frontends and Tauri validation distinguish desired preference,
  preflight, prepared admission, observed active admission and unavailable
  enforcement. The requested CPU limit cannot disappear by omission in an older
  API call. Live proof requires the exact active CPU/heap admission revision.
- Linux uses the existing managed JVM cgroup leaf and adds the CPU quota, with
  pre-exec attachment and shared descendant accounting. The memory charge cap
  remains distinct from Java heap; neither is a claim of whole-process RSS.
- No regional runtime, signed membership, protocol version, or authority policy
  activation. Player-only consent does not remove membership or replica access.

## Measured local kernel evidence

[Raw kernel receipt](evidence/linux-cpu-kernel.json): actual delegated cgroup v2
with a 512 MiB heap admission, 1536 MiB memory charge limit, and 500 millicore
CPU limit. A parent and descendant busy process used **1,496,600 microseconds
of CPU during 3,000,159 microseconds elapsed**, with 30 throttled periods.
The prepared reservation, actual child/descendant membership, active admission
revision, changed-consent proof rejection, withdrawal with active observation
retained until exit, empty leaf and tampered CPU policy rejection were verified.
These bounded Python fixture processes are not Minecraft; no game performance
or live player capacity is claimed. An initial unsafe group-writable test
parent was corrected to private permissions, not bypassed.

The reproducible kernel case is
`host_memory_cap::linux::tests::real_cpu_cap_applies_to_child_and_descendant_and_detects_policy_change`.
Compile the library test binary, then run it in a disposable delegated scope:
```sh
SWARMCRAFT_TEST_CPU_DELEGATION=1 systemd-run --user --scope --quiet \\
  --unit=swarmcraft-memory-cap-test-cpu-$$ -p 'Delegate=memory cpu' \\
  -p RuntimeMaxSec=40 bash scripts/e2e/linux_cgroup_memory_gate.sh "$TESTBIN" \\
  --exact host_memory_cap::linux::tests::real_cpu_cap_applies_to_child_and_descendant_and_detects_policy_change \\
  --ignored --nocapture
```
No host-wide controller or service configuration was changed.

## Desktop and regressions

Both extracted local Debian packages (legacy shipping frontend and compiled
Svelte candidate), with matching real CLI/host/runtime/import sidecars, exercised
the native WebKitGTK IPC path. Each checks absent default consent, saved 4096
MiB + 500 millicores but denied launch on ordinary undelegated Linux, observed
effective 12000 millicore capacity, 980x760 and 720x560 layout, revised CPU with
new confirmation/revision, zero withdrawal, persisted withdrawal after restart,
and unchanged identity and signed world history without runtime launch.
The browser interaction scenarios additionally cover active-state presentation,
stale response fencing, pending/error handling and duplicate save refusal at
980x760, 720x560 and 1280x900. Native package acceptance uses
`scripts/acceptance/native_resource_contribution.py`, a private Xvfb session,
and independent CLI verification against the same disposable data root.
Both final packages passed all eight checks per frontend (16/16), including
retesting after the helper-text styling refinement. See the [native receipt](evidence/linux-native.json).

Local validation: 208 library and 51 CLI binary unit tests passed (three
library cases ignored by the ordinary suite; the new CPU case explicitly
executed above, two prior delegated memory cases not rerun here), 13 Tauri
boundary tests, 173 shared Desktop JS tests, 84 Svelte tests, 30 source parity
checks, 18 Python native harness tests, strict CLI Clippy, native shell check,
compiled Svelte build, both browser variants, formatting and workflow policy.
Six public CLI hosting-consent regressions passed. The existing three-identity real authenticated invite/replication test passed,
including host-candidate and player joins, verified checkpoint history, no
implicit consent and stale replica recovery. Not a Minecraft client session.

## Remaining scope

Real installed Minecraft/Fabric with an active CPU cap, real client play with
zero contribution, native Windows/macOS CPU enforcement and independent-device
field acceptance remain open. Windows/macOS or an undelegated Linux device
may save a desired CPU limit but cannot silently run without it. The measured
CPU capacity is an upper bound, not idle compute or promised throughput.
#58 admission/placement and #59–63 regional execution require separate evidence
and the #56 adoption gates. No hosted CI or main-branch mutation.
