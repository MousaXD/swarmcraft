# Device resource contribution (#57)

This is a local, per-world, device-identity-bound hosting preference. It is not a signed membership grant, regional work assignment, or authority proof. Creating/importing/joining a world, setting up its runtime, voting, and replicating do not opt the device in. Zero remains player-only: it does not remove the member or prevent its use of an independently authorized play route.

## Explicit controls

Both desktop frontends display the requested Java heap (MiB), requested CPU limit (millicores; 1000 = one logical CPU equivalent), measured CPU capacity and memory, and the observed admission/runtime state. A positive choice requires an explicit save confirmation; changing a value requires renewed confirmation. A saved preference may be temporarily or permanently inadmissible.

For CLI operators:

```sh
swarmcraft --data-dir "$DATA" world hosting status "$WORLD" --json
swarmcraft --data-dir "$DATA" world hosting set "$WORLD" --heap-mib 4096 --cpu-millicores 500 --json
swarmcraft --data-dir "$DATA" world hosting set "$WORLD" --heap-mib 4096 --cpu-millicores 750 --json
swarmcraft --data-dir "$DATA" world hosting set "$WORLD" --heap-mib 0 --cpu-millicores 0 --json
```

Nonzero heap: 512–65536 MiB, also subject to physical memory and conservative overhead/headroom checks. Explicit positive CPU: 250–64000 millicores; zero CPU is only valid with zero heap. Omitting CPU on an existing explicit CPU preference preserves its limit. Existing heap-only records remain readable; the legacy CLI heap-only option remains available for consent without an explicit CPU limit. Desktop positive saves always include a CPU limit. No explicit limit silently becomes an uncapped launch. Older strict readers of local consent/ownership records reject the extended records rather than treating a CPU-limited record as a legacy positive consent. Do not edit those records to bypass a limit or ownership check.

## Requested, measured, admitted and active

- `selected_heap_mib` and `selected_cpu_millicores` describe the durable desired preference. An absent CPU field represents no explicit CPU limit (including legacy heap-only hosting); player-only requests no resources.
- `cpu_capacity_millicores` is the observed effective upper bound from process CPU affinity and finite visible ancestor cgroup CPU quotas. It is not measured idle CPU, a reservation, a physical core count, or a throughput guarantee. Unknown capacity is not inferred to be unlimited.
- `physical_mib`, `available_mib`, overhead and reserved headroom are local memory observations. Java heap is distinct from whole-process memory.
- `may_host_now` is a preflight, not a reservation or permission. Current capacity, consent, signed world policy, leases, quorum, ownership and fencing must be rechecked at the relevant operation.
- `process_cpu.mode` is unspecified, configured, reserved, active, or unavailable (the concrete cgroup mode names are `linux_cgroup_configured`, `linux_cgroup_reserved`, and `linux_cgroup_active`). Configured means a verified delegation can support the requested limit. Reserved means the exact live controller has a prepared, verified empty leaf and durable ownership record; no active JVM is inferred. Active means the exact recorded child is observed in the verified capped cgroup. The reservation and active observation carry their admitted heap, CPU and consent revision. Active CPU time is cumulative kernel `cpu.stat` usage, not instantaneous utilization.
- `process_memory` separately identifies heap-only, configured, active, or unavailable memory enforcement. Neither CPU quota nor memory charge limit is a guarantee of process RSS, game ticks per second, or client performance.

A consent change invalidates previous readiness, admission and live authority proof. Explicit CPU proof requires the same observed admission revision, heap and CPU limit. Withdrawing or changing limits can leave the previous admitted process visible while the existing safe checkpoint/stop sequence proceeds; do not report it exited before verification. The new preference never confers a second concurrent reservation.

## Enforcement and platform boundaries

Explicit CPU-limited hosting requires an operator-provided delegated Linux cgroup-v2 root (see [managed memory enforcement](HOST_MEMORY_ENFORCEMENT.md)). Both memory and CPU controllers must be enabled for children; the existing canonical directory, ownership and process checks remain mandatory. SwarmCraft sets the leaf `cpu.max` to requested millicores multiplied by 100 microseconds of quota per 100000 microsecond period; a nonzero CPU burst setting is rejected. It retains the existing memory max/high, zero swap, and group-OOM controls. The child attaches before execution, and descendants share the limits. It verifies the policy, process identity and live admission instead of trusting a renderer or remote claim. Missing delegation, invalid policy, insufficient capacity and unverified ownership fail closed. A saved preference on Windows/macOS or an ordinary undelegated Linux desktop does not authorize an uncapped runtime; CPU enforcement on Windows/macOS is not implemented. Existing heap-only compatibility is unchanged.

Use the operating system service manager for delegation. Never modify host-wide limits, move unrelated processes, or run an unreviewed worker service. See the actual delegated-kernel test and [local verification report](../research/device-resources/REPORT.md). No distributed Minecraft engine, worker placement, pooled memory, or regional authority is activated by this feature. #56 production-runtime adoption remains gated.
