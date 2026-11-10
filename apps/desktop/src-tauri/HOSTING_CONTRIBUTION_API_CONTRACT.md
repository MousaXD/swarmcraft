# Desktop Rust hosting-contribution IPC — Agent 09 contract

**Status: staged only; NOT ACTIVATED.** This branch is deliberately based on `main`. PR #93 (Agent 03's local Rust hosting budget), PR #92 (Agent 02's protocol policy), and Agent 01's daemon enforcement are not integrated. The only application wiring on this branch is `#[cfg(test)] mod hosting_contribution_commands` in `main.rs`: **there is NO registered hosting-contribution Tauri command, no renderer option, and no newly granted hosting permission**. Do not mistake test doubles or this document for a functioning consent UI.

## Exact upstream contracts inspected

Agent 03: `crates/swarm-cli/src/hosting_budget.rs` in `agent-03/hosting-runtime-budgets` (PR #93).

* `hosting_budget::contribution_readiness(&DataPaths, WorldId) -> Result<ContributionReadiness>`: real local memory sample, selected consent heap, OS/client/native/daemon reserves, `may_host_now`, reason, and `hard_process_memory_cap_guaranteed=false`.
* `hosting_budget::load_consent(&DataPaths, WorldId) -> Result<Option<HostingConsentV1>>`: absent record means player-only. Corruption, unreadability, or changed device peer identity cause an **error**, not a successful implicit opt-in.
* `hosting_budget::set_contribution(&DataPaths, WorldId, heap_mib: u64) -> Result<HostingConsentV1>`: an explicit 0 revokes permission; all writes increment a durable revision and invalidate readiness, with serialized updates. Physical memory and bounds are validated on nonzero choices. On active authority, safe save/barrier and fencing are separately responsible for stopping.
* `hosting_budget::check_budget`: rejects nonzero heaps outside 512–65536 MiB and budgets with insufficient **physical or currently available** RAM, accounting for native JVM, Minecraft client, daemon and OS headroom. Actual launch admission independently checks capacity and persists an admission guard. `-Xmx` limits **heap only**, not whole-JVM RSS.
* The backend has **no public CLI subcommand** to read or set contribution today. Desktop has **no `swarm-cli` dependency** yet. Forwarding invented sidecar commands, making local JSON edits, or returning renderer-only values would be incorrect.

Shared protocol parser: `swarm_protocol::WorldId::from_str` accepts canonical `scworld:<64 hex>` or legacy 64-hex form. `hosting_contribution_commands.rs` validates and normalizes these IDs before any potential backend call. No free-form world path is permitted.

## Planned callable Tauri API (NOT YET REGISTERED)

| Tauri command | Input | Successful output after integration |
| --- | --- | --- |
| `hosting_contribution_get` | `world: String` parsed as `WorldId` | Typed Rust-backed local consent/readiness response, with physical and available MiB, chosen heap, estimated native/client/OS/daemon overhead, candidacy status and rejection reason |
| `hosting_contribution_set` | `world: String, heap_mib: u64` (JS `heapMib` under Tauri `rename_all="camelCase"`) | Typed persisted local consent/revision after invoking Agent 03's real `set_contribution`; no success until the write succeeds |

The staged `HostingContributionBackend` trait deliberately has **no production implementation**. It requires serializable output types and explicit `get/set` methods; validation returns errors without calling it for malformed requests. The test-only `BoundarySpy` does not write world data and is **not** an eligibility, persistence or concurrency test of production.

The final response must not leak private data-root paths, tokens, secrets, or device credential material. Use an explicit DTO with a canonical **string** world ID, selected heap, revision, safe eligibility/reason and numerical memory fields; do not serialize an entire `DataPaths` or `PeerIdentity`. Never claim that `may_host_now` is a signed authority grant or that memory is hard-capped.

## Required Agent 00 integration decisions

1. **Approve a base with PR #93** (and a stable exact commit). Do not introduce a production reference to the unmerged `swarm_cli::hosting_budget` module before that approval. Assign permission to edit `apps/desktop/src-tauri/Cargo.toml` if direct Rust linking via `swarm-cli = { path = "../../../crates/swarm-cli" }` is selected; this path is outside Agent 09's exclusive ownership. No Cargo change is made in this PR. Linking directly to the genuine library is preferable to building a new shell command layer.
2. **Bind `HostingContributionBackend` to Agent 03** through an adapter that discovers the same `DataPaths` as the daemon/runtime, opens `Storage` and uses `load_world(world)` before both operations, rejects missing/nonlocal worlds, and calls `hosting_budget::contribution_readiness` / `set_contribution` with the canonical `WorldId`. The backend validates the cryptographic peer identity when loading persisted consent. Avoid fake return values or optimistic frontend acknowledgements.
3. For a get response that includes a separate consent revision, obtain a **consistent** snapshot or retry if it changed: `load_consent` and `contribution_readiness` are distinct reads and can race a concurrent set. Do not blindly merge two different revisions in one response. Agent 03 may need to expose a combined coherent snapshot.
4. Remove `#[cfg(test)]` staging in `main.rs`, register **only actually compiled, backend-supported** Tauri command functions via `tauri::generate_handler!`, retain every existing command/plugin/capability/sidecar/CSP entry. Match Agent 04's frontend IPC argument and response names without introducing automatic consent in create, join, import, invite, EULA, startup or diagnostics.
5. Require Agent 02's signed permission policy and Agent 01's daemon election/fencing hooks before labeling hosting **activated**. Local readiness says only *local opt-in and preflight memory*, not whether the world authorizes hosting or a peer may become authority. Missing policy/consent remains ineligible; members can still be players, replicas and voters as allowed.
6. Live authority revocation is a durable consent change followed by an independent **safe save/barrier + fenced checkpoint or handoff**. Never kill a current authority JVM directly from a settings toggle. Do not report a completed stop until the backend proves it.
7. Agent 04 must keep feature controls hidden/disabled or explain unsupported state until an integrated exact-head packaged candidate proves the registered API. The presence of this PR does not authorize the UI to show an enabled hosting feature.

## Exact Agent 04 IPC DTO proposal (approval-gated, NOT IMPLEMENTED)

The following is the **proposed post-integration** wire contract. It is not an exported or registered command on PR #100. Both commands must be real Rust/Tauri calls, with Tauri `#[tauri::command(rename_all = "camelCase")]` mapping JavaScript's `heapMib` to the Rust `heap_mib` argument. Rust `serde` output fields stay **snake_case** unless both teams explicitly choose a different serialization convention.

```ts
// Add to Agent 04's NativeCommandTable ONLY when the Rust registry actually
// compiles and registers these commands on the approved combined candidate.
type HostingContributionReadiness = {
  world_id: string; // canonical scworld:<64 lowercase hex>
  consented: boolean;
  selected_heap_mib: number;
  physical_mib: number | null;
  available_mib: number | null;
  estimated_native_overhead_mib: number;
  client_headroom_mib: number;
  os_headroom_mib: number;
  daemon_headroom_mib: number;
  may_host_now: boolean; // LOCAL capacity check ONLY, not authority eligibility
  reason: string;
  hard_process_memory_cap_guaranteed: boolean; // currently false
  consent_revision: string | null; // coherent snapshot required; null if absent
};
type HostingContributionConsentResult = {
  world_id: string;
  enabled: boolean;
  heap_mib: number;
  revision: string; // persisted Rust u64 converted to decimal string
};
// Interface keys and args; commands must be present in generate_handler!.
hosting_contribution_get: {
  args: { world: string };
  result: HostingContributionReadiness;
};
hosting_contribution_set: {
  args: { world: string; heapMib: number };
  result: HostingContributionConsentResult;
};
```

**Revision precision matters:** Agent 03's `set_contribution` sets `revision` from a nanosecond-scale `u64` value. Serializing that `u64` directly to an ordinary JavaScript `number` loses precision above `Number.MAX_SAFE_INTEGER`. The Desktop adapter must convert it to a decimal string before crossing native IPC; the frontend should compare revisions as strings (or parse with `BigInt` only where necessary). The Rust wire DTO deliberately omits `peer_id`, `DataPaths`, signing material and private filesystem paths. Return stable public errors without echoing private paths in backend context chains.

**Consistency contract:** Agent 03's current `ContributionReadiness` lacks a revision while `load_consent` is a separate read. The combined get endpoint must obtain `consent_revision` and the readiness fields from a single coherent consent revision, or retry/reject on an intervening concurrent write. Obtain a backend-supported coherent snapshot before implementing the field; never fabricate the value or return a mixed-revision result. The set response acknowledges persisted consent, **not** completion of a live JVM's asynchronous save, handoff, fencing or shutdown.

**Native adapter on approved base:** Use `swarm_core::DataPaths::discover()` so Desktop resolves `SWARMCRAFT_DATA_DIR` and normal OS-local data directory exactly as its packaged CLI sidecars do; use `swarm_storage::Storage::open(paths.root.clone())?.load_world(world)?` before reads **and** mutations. Bind `validated_get/set` to `swarm_cli::hosting_budget::contribution_readiness` and `set_contribution`, without changing their admission or consent logic. File/memory operations should run off the UI thread using a safe Tauri async/blocking boundary; never return optimistic success from the webview. Reject a world missing from local storage, preserve default-deny on missing/corrupt/mismatched consent, and avoid claiming that world membership or a local readiness sample alone confers signed hosting authority. Agent 00 must approve the Desktop `Cargo.toml` dependency and production registration changes first.

**Agent 04 tests:** Add the two commands to the typed command table only after their real Rust registration; run Rust-registry parity, invalid-world/heap native IPC negatives, absent-consent player-only, explicit opt-in/persistence/restart, 0 MiB withdrawal, memory rejection, backend failures, and real installed-GUI invocation. Do not introduce a simulated hosting backend or JavaScript-only persistence. User-visible active-host withdrawal must say checkpoint/handoff may still be pending, rather than claiming an immediate safe stop.

## Validation matrix

| Scenario | Agent 09 staged contract | Required integrated certification |
| --- | --- | --- |
| Missing/malformed/traversal world ID | Rejects before backend call (unit tests) | Rejects through actual registered Tauri command |
| 0 MiB and bounded heap selection | Accepts 0, 512–65536; denies 1, 511, 65537, MAX (unit tests) | Prove real Rust persistence, revision and reader status |
| Absent consent | No implicit opt-in in staged adapter; test double defaults 0 | Actual `load_consent(None)` and player-only state |
| Insufficient physical/free RAM | Backend error propagation covered via spy | Real `check_budget` failure and no host admission |
| Corrupt file/device identity change | No local file access in this PR | Rust fail-closed consent read and command error |
| Revocation during active hosting | 0 forwarded without conversion (spy only) | Live Fabric save barrier / fencing / failure recovery |
| Rapid concurrent settings changes | Delegated to Agent 03's serialized write and revision | Real concurrent reads/writes, no stale success/authority |
| Tauri command registration | **BLOCKED; intentionally absent** | Check `generate_handler!` and real invoke from packaged GUI |
| Linux/Windows/macOS desktop CI | PR GitHub Actions required | Exact-head Rust checks plus native packaging with Agent 04 |
| Signed world policy / election hooks | **NOT IMPLEMENTED here** | Integration with Agents 01/02/11, QA signoff |

No test is waived; unmet acceptance items remain explicit blockers. This PR must remain separate/draft, must not be merged as an activated feature, and must not close issue #84.
