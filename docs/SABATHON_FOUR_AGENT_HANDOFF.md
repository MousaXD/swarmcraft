# Sabathon P0 - four-agent handoff (23 November 2026)

## Starting point and evidence

All agents branch from `integration/sabathon-p0-20261123`, which merges
`agent/sabathon-runtime-20261123` (the plan baseline) and
`agent/sabathon-game-route-20261123` (37 commits of networking work).
The combined Rust source tree matches commit `99500de` of the networking branch.
At that source tree, the complete `swarm-network` suite passed 140 tests
on a rerun (3 ignored). An earlier full run had a transient connection reset
in `game_network_resolver.rs`; a focused rerun and the full rerun passed.
That is NOT a real Minecraft field pass or a production authorization proof.

## Exclusive ownership to reduce conflicts

| Agent | Branch | Primary files / responsibility |
| --- | --- | --- |
| 1 - Live authority | `agent/sabathon-authority-20261123` | Implement a real, nonce-bound `FreshAuthoritySource` from current signed canonical quorum/membership/fencing, hosting consent and authenticated live Fabric IPC readiness. Own new authority adapter files under `crates/swarm-cli/src/` and necessary `minecraft/fabric/` Java IPC changes; expose a typed interface for Agent 3. Do not edit `daemon.rs`, `migration.rs`, or `authority_permit.rs`. |
| 2 - Safe server runtime | `agent/sabathon-server-20261123` | Own `crates/swarm-cli/src/migration.rs`, `authority_permit.rs`, and new managed `server.properties` helpers/tests. Enforce unique loopback-only Minecraft binds, explicit `online-mode`/RCON policy, atomic private permit updates and safe consent/fence lifecycle. Do not edit `daemon.rs` or the network crate. |
| 3 - Daemon integration and handoff | `agent/sabathon-daemon-20261123` | Own `crates/swarm-cli/src/daemon.rs`, main CLI module/wiring, and transfer orchestration. Connect signed route control, host receiver, player resolver, stable gateway and daemon-owned shutdown to the live authority adapter when available; automate the safe staged transfer over existing authenticated paths. Do not replace a missing authority adapter with cached Ready or synthetic evidence. |
| 4 - Desktop UX and field QA | `agent/sabathon-desktop-20261123` | Own `apps/desktop/src/`, `apps/desktop/src-tauri/src/`, scoped UI tests and `scripts/e2e/three_machine/`. Expose truthful verified checkpoint, quorum, hosting consent, route and reconnect states, and a field runbook. Do not change Rust daemon/core/network authority code. Maintain existing shipped HTML/JS vs Svelte parity policy. |

If a shared file must be changed, do NOT edit it concurrently: provide an
interface/handoff note for the owner, or defer the cross-cutting edit until
integration. Agent 3 may write a fail-closed interface awaiting Agent 1 but
must not claim production authorization or use mock data in production.

## Operating rules

1. Work only in the dedicated checked-out branch and worktree. Never commit
   to the integration branch or another agent's worktree; no force-push,
   hard reset, destructive clean, or blanket conflict resolution.
2. Read `docs/SABATHON_IMPLEMENTATION_PLAN.md`,
   `crates/swarm-network/GAME_ROUTE_INTEGRATION.md`, and
   `crates/swarm-network/GAME_HOST_RESPONDER_HANDOFF.md` before editing.
   Preserve exact-world checks, signed quorum, membership, consent,
   fencing, runtime readiness and fail-closed behavior.
3. Commit and push small verified steps to your own branch. Document blocked
   dependencies and tests honestly; do not invent live Fabric or field results.
4. Disk space was about 17 GiB free, and the host cannot handle concurrent
   Rust builds. Serialize heavy Cargo runs across all four agents with
   `flock -x /tmp/swarmcraft-sabathon-cargo.lock` and `CARGO_BUILD_JOBS=2`,
   `--jobs 2`. Do not run Cargo clean or create four large build caches.
5. Integrate individual branches ONE AT A TIME into
   `integration/sabathon-p0-20261123`, review diffs and affected tests,
   and rerun combined acceptance. Do not merge any branch into `main`
   or `plan/swarmcraft-sabathon` as an automatic side effect.
6. P0 acceptance still requires three independent installed peers and real
   Minecraft clients, quorum/fencing, safe handoff, crash recovery, saved
   marker/inventory verification, consent withdrawal, and real WAN/relay
   behavior. Successful local synthetic network tests do not prove this.
