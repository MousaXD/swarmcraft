# Agent 10 — explicit CLI hosting consent and live acceptance integration contract

**Status: DESIGN ONLY / BLOCKED for code integration.** This document is deliberately safe to merge on the current `main` without referencing unmerged Rust symbols. It does **not** certify issue #84 or represent a working hosting-consent CLI.

**Ownership:** `crates/swarm-cli/src/main.rs`, `scripts/acceptance/clean-machine-live.sh`, dedicated CLI tests/acceptance fixtures and this document. Agent 03 owns `hosting_budget.rs` and migration, Agent 01 owns `daemon.rs`, Agent 09 owns Tauri IPC, Agent 04 owns Svelte, Agent 08 owns shared CI/workflow acquisition. Do not edit their files here.

## Verified upstream foundation and integration prerequisite

On 2026-11-08, `main` exposes `world create`, `world join`, `world host-readiness`, `world migration-status` and `world runtime-configure`, but **does not expose** `swarm_cli::hosting_budget`. Agent 03's **draft** [PR #93](https://github.com/MousaXD/swarmcraft/pull/93), branch `agent-03/hosting-runtime-budgets`, exposes:

- `swarm_cli::hosting_budget::set_contribution(paths: &DataPaths, world: WorldId, heap_mib: u64) -> Result<HostingConsentV1>`. Explicit local action only; 0 revokes; persists device-bound per-world revision and invalidates cached readiness.
- `swarm_cli::hosting_budget::contribution_readiness(paths: &DataPaths, world: WorldId) -> Result<ContributionReadiness>`. Returns `consented`, `selected_heap_mib`, `physical_mib`, `available_mib`, estimated overhead and headroom, `may_host_now`, `reason` and `hard_process_memory_cap_guaranteed`.
- `swarm_cli::hosting_budget::require_contribution(paths: &DataPaths, world: WorldId) -> Result<HostingConsentV1>`. A fail-closed runtime/candidacy check; the CLI must not bypass it.

These signatures need review against an **Agent 00-approved exact integration candidate** before compiled CLI wiring. Do **not** add references to unavailable APIs in a branch based only on `main`, or graft draft PR #93 onto `main` to make this PR compile. Agent 00's issue #84 decision explicitly states that world policy activation and legacy-authority exclusion are **not** approved by an inert protocol record.

## Intended CLI UX and exact command grammar

Expose under the current `world` namespace, using Clap's nested subcommand pattern:

```console
swarmcraft --data-dir <root> world hosting status <scworld:...> [--json]
swarmcraft --data-dir <root> world hosting set <scworld:...> --heap-mib 4096 [--json]
swarmcraft --data-dir <root> world hosting set <scworld:...> --heap-mib 0 [--json]
```

These are **planned commands**, not yet available on `main`.

1. `status` parses `WorldId`, verifies `storage.load_world(world)?`, and displays the **actual** `contribution_readiness` result. Default/missing consent displays **Player only (0 MiB)**. `--json` serializes the typed backend response without invented eligibility.
2. `set` parses `WorldId`, verifies the world exists locally, and invokes `set_contribution(paths, world, heap_mib)` **only from this explicit command**. After the durable update it queries `contribution_readiness`; `--json` exposes current readiness plus persisted consent/revision in a stable documented envelope. No direct JSON/consent-file writes and no silent consent mutation from any other command.
3. `--heap-mib 0` explicitly chooses player-only / withdrawal. `--heap-mib 512` to `65536` selects a *maximum Java heap*, subject to the Rust API's device capacity validation. Negative/overflow/invalid integer input fails parsing; invalid nonzero ranges fail the Rust validation. A nonzero **saved preference is not a promise to host now**: report `may_host_now=false` and the authoritative reason when available RAM is insufficient.
4. Pretty text must distinguish: `local preference selected`, `currently admissible`, `world policy authorized`, `authority elected`, and `Minecraft running`. `contribution_readiness` is **local preflight only**, not signed world policy or an authority grant. Do not print “Hosting enabled” as an unconditional launch promise.
5. Show physical/available memory and native-JVM, Minecraft client, OS and daemon reservations, plus `hard_process_memory_cap_guaranteed=false`. `-Xmx` bounds Java heap **not whole-process RSS**. If the probe fails, show the failure/denial; never substitute arbitrary fixture memory.
6. If an active host withdraws (`set ... --heap-mib 0`), clearly state that the durable preference changed but a safe save/stop or fenced transfer may remain pending; don't claim immediate JVM exit or successful checkpoint before the runtime confirms it.
7. An existing `world create`, `world join`, `world runtime-configure --accept-eula`, import, restart, wake or daemon startup **must not call** `set_contribution`. A signed world permission to contribute is distinct from this device's permission and cannot force it to opt in.
8. Choosing 0 must not remove the peer from authorized membership, storage replication, quorum voting or the client join flow. Player-only peers can play where the existing client/protocol permits; don't claim a full player-client E2E unless it has actually run.

### Unit/CLI regression contract

Add tests once the approved API is present:

- Clap parses `hosting status` and `hosting set --heap-mib 0/512/4096`; rejects missing/negative/overflow values and unknown options; help describes consent as **voluntary**.
- Fresh identity/world `hosting status --json` has `selected_heap_mib=0`, `consented=false`, `may_host_now=false`, nonempty reason. Neither creating a world nor accepting EULA writes a consent record.
- `set --heap-mib 0` changes revision and stays denied; 512/4096 are accepted only when physical RAM validates, and readiness may still report pressure. Corrupt/foreign-identity records must fail closed, not overwrite themselves silently.
- An impossible nonzero heap rejects without replacing previous preference; invalid world identity and nonexistent local world produce actionable errors.
- Positive preferences survive a CLI process restart; revoking while active reports a pending safe barrier, not an immediate stop promise. Ensure restart/recovery still checks live admission at launch.
- JSON output is parsable, uses typed names, distinguishes persisted consent from temporary eligibility and does not report a guaranteed process-RSS cap.

## Real clean-machine script sequence

The current `scripts/acceptance/clean-machine-live.sh` uses a fresh data root, invokes the public `swarmcraft` CLI to create a private world, calls the real runtime's `plan/install/verify/launch`, verifies **managed Java** rather than a preinstalled JVM, and checks Fabric's `level.dat`, canonical save/snapshot increment, marker survival and second launch. Preserve **all** of those positive checks, explicit `--accept-eula`, the actual candidate Fabric mod artifact and the real download/verification mechanisms.

On the approved integrated candidate, amend only this script to add this **public CLI** consent flow:

1. After `init` and creating the acceptance host world, capture its **actual `scworld:...` ID** from the CLI. Before any consent, run `world hosting status "$WORLD" --json`: require zero, no consent, `may_host_now=false`. Do not fabricate a world ID or insert a consent file.
2. For **negative launch checks**, create a *separate* test world within the same clean data root. Invoke real `runtime plan` and `runtime install ... --accept-eula` for this negative world so a launch denial is specifically from missing hosting consent, **not** missing EULA, Java, or Fabric. Attempt real `runtime launch`, require nonzero exit, demand an explicit absent-consent/player-only denial, and verify no JVM/Minecraft world was started. Then explicitly call public CLI `world hosting set "$DENIAL_WORLD" --heap-mib 0` and repeat the denial. Do not use `|| true` to hide failures and do not mistake any unrelated error for a passing negative assertion. Isolate the negative world so its blocked migration phase cannot poison the positive world's readiness.
3. For the **positive world**, explicitly execute `world hosting set "$WORLD" --heap-mib "$SWARMCRAFT_ACCEPTANCE_HEAP_MIB" --json` (default **512 MiB**, unless Agent 00 approves a different safe runner-specific heap). Assert the selected heap, actual consent and current `may_host_now=true` before launching. The Rust API must reject unsustainable physical capacity; current free-memory shortages must fail the acceptance gate with a transparent **capacity BLOCKED** result rather than a fake PASS or silent skip.
4. Keep explicit EULA refusal, `runtime install --accept-eula`, official verified downloads and actual managed Java/Fabric checks. Do not opt in through EULA acceptance. Retain the two real launch → save-barrier → canonical checkpoint → restart cycles and increasing snapshot numbers.
5. Across the second runtime/process launch, re-read `world hosting status --json` and confirm preference survived restart while admission is still checked at **current** resources; distinguish a temporary memory-pressure block from lost consent.
6. If adding post-restart revocation, use a distinct negative world or a verified sleeping state so the test truly fails because of **0 consent**, not because of a pre-existing sleep/fencing failure. A player-only peer should still be able to use existing join/replica behaviors, but do not assert complete real client play without a real client test.

Do not add test-only memory-probe overrides, directly mutate `control/<world>/hosting-consent.json`, bypass `require_contribution/admit`, replace Java with a fake process, skip real install because artifacts are slow, or edit `.github/workflows/**` (Agent 08 owns acquisition workflows).

## Release gates / blocking dependencies

- **Contract approval:** Agent 00 identifies an **exact SHA** integration candidate that contains Agent 03's API, its runtime launch guards, Agent 01's candidacy gates and compatible Agent 02 policy handling, and accepts the CLI shape above. Agent 07 independently reviews permission/authority semantics.
- **Implementation:** Only then wire `main.rs` against the approved symbol, add tests and update the acceptance script on this agent's branch; keep a distinct draft PR with a base appropriate to the integration candidate, rather than a broken dependency against `main`.
- **Evidence:** Report exact branch head and completed GitHub Actions **Required Validation** and **Main Desktop Installers** runs, with job links/results. The live acceptance lane must show actual selected heap, missing-consent/zero-consent denial, real Minecraft/Fabric managed Java, canonical snapshots and restart. A standalone green document PR or tests from Agent 03's separate SHA do **not** certify integrated behavior.
- **Feature safety:** Signed canonical world-policy activation/legacy-authority exclusion remains OFF until a reviewed compatible migration exists. Neither this CLI nor test fixture must silently activate it. #84 stays open until Agent 00 accepts the full integrated proof.

**Handoff verdict:** **BLOCKED** for compiled CLI and live E2E until an approved API-bearing integration SHA is supplied. Documentation is safe to review independently.