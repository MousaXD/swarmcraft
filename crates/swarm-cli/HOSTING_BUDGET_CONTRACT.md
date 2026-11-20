# Agent 03: local hosting admission and integration contract

Status: **INCOMPLETE / BLOCKED for feature-level readiness** until Agents 01, 02 and 04 integrate the hooks below and required CI verifies the exact PR head. This code is a **local runtime safeguard**, not complete issue #84 delivery.

## Safety model and player semantics

* **Player-only is the default.** A missing consent file, `0 MiB`, a malformed consent file, or any unsupported memory probe cannot authorize Java launch. Joining, replication and voting must remain possible independently of server-hosting consent.
* A locally explicit opt-in invokes `swarm_cli::hosting_budget::set_contribution(paths, world, heap_mib)`; `heap_mib=0` revokes consent. This API does **not** run at world create/join or when EULA is accepted. A write increments the persisted revision, including same-value edits. It survives restarts in `control/<world>/hosting-consent.json`.
* `hosting_budget::contribution_readiness(paths, world)` is the typed, machine-local, player-facing read endpoint. It serializes its consent read and memory sample with concurrent opt-in/withdrawal writes using `hosting-consent-write.lock`, and returns `consent_revision: Option<String>` as a **decimal string** (`None` when the record is absent). The response contains no device `peer_id` or private paths. This is only a **preflight sample**, not a future guarantee, signed quorum claim or durable stop acknowledgment.
* `hosting_budget::require_contribution(paths, world)` fails closed on missing, corrupt or withdrawn consent, unavailable OS memory, unsafe device headroom or an excessive limit. It is safe to call at every candidacy gate.
* `hosting_budget::admit(paths, world)` acquires an exclusive **data-root-wide** memory admission lock and revalidates capacity/consent before any restore, epoch mutation or JVM launch. It deliberately permits only one active authority JVM per device/data root. Release the guard only after process exit and checkpointing. Cross-root capacity coordination and hard-killed Rust controllers with orphan JVMs need dedicated process accounting; this lock alone must not be advertised as a global guaranteed allocator.
* `hosting_budget::runtime_withdrawal(paths, world, admitted)` notices an altered revision, opt-out and severe available-memory pressure during service. The authority runtime requests Fabric's safe save/stop barrier and then commits the fenced canonical snapshot and sleep record. On barrier failure, it **does not silently kill** a live authority and lose signed progress: it logs and retries. An explicit signed manual transfer intent is attempted first. This is not a guarantee of immediate revocation if the JVM is unresponsive.

## Memory accounting

Selected heap is 512–65536 MiB (0 = disabled). Per-launch memory admission requires sufficient **physical AND currently available** memory for:

`heap + max(1024 MiB, heap/2) native-JVM overhead + 1024 MiB Minecraft client + 256 MiB SwarmCraft + max(1024 MiB, physical/8) OS`.

A 32 GiB machine with adequate free memory may admit a 4 GiB heap; a low-RAM or memory-pressured device may opt for 0 and remain a player. This formula is conservative but cannot predict every mod, driver or external application's resource usage; present it as an estimated headroom model.

The Java command receives `-Xmx<heap_mib>m` **before** `-jar`. This bounds JVM Java heap, **not** native allocations, memory-mapped storage, thread stacks, child processes or whole-process RSS. No OS-independent process hard cap is claimed.

| Platform | Physical/available probe | Current enforcement | Guaranteed whole-process cap |
| --- | --- | --- | --- |
| Linux | `/proc/meminfo` and cgroup-v2 `memory.max/current` when present | Conservative admission + serialized launch + `-Xmx` + periodic pressure recheck | **No** — delegated cgroup memory.max placement/lifecycle is not yet implemented |
| Windows | `GlobalMemoryStatusEx` | Conservative admission + serialized launch + `-Xmx` | **No** — Job Object process memory enforcement is not yet implemented |
| macOS | `sysctl hw.memsize` and conservative `vm_stat` free/inactive pages | Conservative admission + serialized launch + `-Xmx` | **No** — reliable OS-level process cap not yet implemented |
| Others | Unsupported: probe returns error | Hosting denied | **No** |

No automatic opt-in from a signed world policy; world permission is not device permission. If strict whole-process limiting becomes a product promise, gate it on a tested OS enforcement mechanism (such as delegated Linux cgroups or Windows Job Objects) rather than silently falling back to `-Xmx`.

## Interfaces for coordinating agents

### Agent 01 — daemon, election, recovery and membership

1. **Before** presenting local recovery-ballot candidacy, wake eligibility, renewing a local host capability, or proposing authority promotion: invoke `require_contribution(&paths, world)`; failures mean **not a hosting candidate**, but do **not** remove the member's replica/voting role. Continue to require signed canonical state, quorum and fencing.
2. Revalidate immediately before any daemon-originated promotion/authority change. A check at the earlier capability advertisement is insufficient. The Agent 03 runtime's final admission gate does not undo an already committed (but non-runnable) recovery generation.
3. Any remote successor requires fresh authenticated capability coupled to actual consent and capacity. Treat a stale readiness packet as ineligible, not as a lease. The existing `HostCapabilityV1.runtime=Unverified` is emitted if local consent/capacity cannot pass; do not infer consent from a previously signed world descriptor.
4. Protect against another *data root* or orphan Java process consuming the same physical memory; the file lock only serializes one data root. Do not downgrade signed recovery, quorum or lease rules.
5. On local consent withdrawal while authority, prefer an existing validated transfer target only after proof of its current readiness. Failure to transfer must fall back to a fenced save-and-sleep flow; never discard signed world state.

### Agent 02 — protocol contract

1. Establish an authenticated, versioned **signed world contribution permission policy**, disabled by default on legacy worlds unless explicitly enabled through an authorized signed update. This is *only* world permission; never replace `set_contribution` with it.
2. Distinguish member/replica/voter from authority candidate. Missing consent or insufficient budget makes the latter false while preserving the former.
3. Publish a precise versioned capability structure to bind a per-world consent/budget revision, current readiness sample time and policy version; the current `HostCapabilityV1` cannot carry those assertions. Use explicit compatibility behavior for older peers.
4. Do not expand quorum or permit transfer when a candidate has no fresh budget confirmation. Agent 03 must add the final signed-policy check once contract is merged.

### Agent 09 — Rust/Tauri coherent native API (approved integration base only)

The existing Rust `hosting_budget::contribution_readiness(paths, world)` now returns all the planned response fields plus `consent_revision: Option<String>` in one coherent read. **Do not call `load_consent` separately** to mix an old revision with new readiness; this function acquires the same cross-process write lock as `set_contribution` through the entire read. Every `set_contribution`, including 0 MiB withdrawal and repeated same-value choices, commits a strictly greater revision under that lock.

* `hosting_contribution_get(world: String)`: validate canonical world ID, discover persistent `DataPaths`, validate world exists, then call `contribution_readiness` and serialize the result directly as the snake_case DTO proposed in PR #100. Its revision is already a JSON **string or null**, never a lossy `u64` JavaScript number. `may_host_now` refers to local permission and sampled budget only, not signed world-policy authority.
* `hosting_contribution_set(world: String, heap_mib: u64)`: validate world and explicit user action, call `set_contribution`, and map only `world_id`, `enabled`, `heap_mib`, `revision: record.revision.to_string()` to the public DTO. Never serialize `HostingConsentV1` directly: it also includes `peer_id` and a numeric `revision`.
* Set success means the preference is durable, not that an active authority has completed Fabric save, fencing, transfer or shutdown. There is **no** implicit opt-in at world creation, joining, EULA acceptance, restart, or V2 migration.

The daemon/CLI/desktop command files remain outside Agent 03 ownership and cannot be considered integrated while these Rust module functions are unregistered.

### Agent 04 — Desktop typed bridge and controls

Expose backend commands that call the Rust API, not a mutable JavaScript-only preference:

```text
hosting_contribution_get(world: WorldId) -> ContributionReadiness
hosting_contribution_set(world: WorldId, heap_mib: u64) -> HostingConsentV1
```

The frontend must show an explicit user action to opt in, a `0 MiB / player-only` selection, RAM/probe and headroom estimates, selected heap, estimated native overhead, pending/ready/active/blocked states and human-readable error messages. Never show "hard RAM cap" when `hard_process_memory_cap_guaranteed=false`. Show that withdrawing an active host requires a durable safe checkpoint and may be delayed by an unresponsive JVM. A disabled world policy must not be represented as host-eligible merely because the local API says eligible.

### Agent 00 — release

Do not close #84 until: signed policy + migration/election hooks + actual Desktop commands + cross-platform memory/process evidence + current exact-head required GitHub CI are verified together. Existing process/authority correctness and quorum must remain intact.

## Known integration blockers

* The signed world policy and protocol-capability revision from Agent 02 are not yet merged.
* Daemon `daemon.rs` is Agent 01's area, so its election and ballot gates are not modified here.
* `main.rs`/Tauri/Desktop command routes are out of Agent 03's ownership, so the new typed Rust API is not yet available as a player action.
* No platform advertises a guaranteed whole-JVM memory cap; a Linux cgroup/Windows Job Object enforcement design and real integration tests are still needed if such a cap is promised.
* Existing legacy world/player workflows need an explicit opt-in UX; tests use opt-in fixtures, never implicit production consent.

## CI runner resource-aware acceptance

The actual JVM admission remains fail-closed at every launch and is NEVER bypassed in production or tests. A macOS runner with 7168 MiB physical but ~2970 MiB available fails the conservative minimum 512 MiB JVM budget, which requires 3840 MiB including OS, client, daemon and JVM-native reserves. Tests requiring a successful live host process therefore log this explicit capacity-denial case and exercise their positive lifecycle paths only on hosts with sufficient real available RAM. Numeric safety and consent/revision behavior use deterministic unit fixtures; the Linux/Windows process lanes provide positive host-process coverage when adequately provisioned. This limitation must be visible to Agent 07, not interpreted as macOS positive hosting certification. Real multi-machine acceptance remains a separate requirement.

## Consensus recovery regression fixture

The pre-existing `consensus_partition_safety` tests created members marked `authority_eligible=true` but did not opt any device into hosting. Their prior expectation of successful recovery permits was inconsistent with a new default-deny device policy. The test host-capability fixture now explicitly calls `set_contribution(..., 512)`, preserving production default-deny. Positive live-daemon recovery assertions additionally require measured safe 512 MiB admission and report an explicit capacity-based skip when CI lacks safe RAM; the negative quorum/partition tests still run. A skip on a low-memory runner is **not certification** of positive recovery for that OS. At least one adequately provisioned CI runner must run the positive scenarios and provide exact-head evidence. Test-only deterministic `check_budget` fixtures continue to verify low-memory denial.

Rustfmt differences reported by the Linux `cargo fmt --check` job were applied directly from that job's exact output. CI must confirm formatting for the updated head; neither previous CI failures nor canceled runs count as validation.
