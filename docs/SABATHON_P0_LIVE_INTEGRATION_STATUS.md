# SwarmCraft Sabathon P0 — live integration verification

**Updated: 24 November 2026. Status: NOT CERTIFIED / real three-machine gameplay BLOCKED.**

Work ONLY on `integration/sabathon-p0-20261123`. Preserve the original
`docs/SABATHON_SINGLE_AGENT_HANDOFF.md`, the signed integration contracts and
the previous agent branches as recovery history. Do not merge into `main`
without explicit permission. This report records independently observed local
checks, not a completed release or hypothetical Minecraft sessions.

## Implemented and integrated

- The daemon's managed Minecraft Java process retains the original
  token-authenticated Fabric IPC socket. One-use `RUNTIME_CHALLENGE` requests
  return fresh `RUNTIME_READY` nonce, runtime instance, world directory,
  compatibility and reserved localhost-port evidence. Mismatched, stalled or
  malformed replies invalidate the session. The supervisor rechecks JVM
  ownership, advancing majority permit, signed epoch/fence, consent, stop and
  transfer state before and after a challenge.
- Host game-route control now uses the real `SignedLiveAuthoritySource` and
  bounded authenticated fresh-quorum collector. Host-side signed offers and
  challenges are refused without a current canonical membership chain,
  accepted checkpoint bytes, quorum votes, active hosting consent and live
  Fabric IPC. Queued replies are re-fenced before leaving the daemon.
- Remote players request a new signed Host Fabric attestation from the
  application-authenticated authority; the daemon binds its response to the
  requester's exact world, nonce, transport, generation and checkpoint, while
  independently collecting fresh canonical majority votes. A missing peer,
  stale/invalid signature, interrupted RPC or missing quorum does not grant a
  game route.
- The one-selected-world `StableLoopbackGateway` reserves a persistent local
  IPv4 `127.0.0.1` port, never a configured Minecraft IP or LAN fallback.
  Each Minecraft TCP connection must independently resolve a verified signed
  offer/challenge and live proof before using the authenticated P2P game
  stream. A host playing its *own* world uses the same signed route and live
  checks to reach its **own** managed 127.0.0.1 server; self-PeerHello or an
  unverified direct local connection cannot grant access. Sessions recheck
  authority/consent/fence and close on loss.
- The daemon owns a private, short-lived, localhost-only token-authenticated
  control endpoint. `swarmcraft world game-route-select <world>` reserves a
  world-bound gateway but **does not claim authorization**;
  `swarmcraft world game-route-status <world> --json` issues a new proof over
  that live daemon, not a persisted `migration-status.json`. The native Tauri
  `game_route_select` and `game_route_status` commands are wired, and the
  legacy and typed Svelte IPC contracts have been updated. World switches,
  daemon restart, poll overlaps and proof expiry clear or fence stale UI
  reconnect instructions.
- Tauri Desktop retains its own Cargo.lock and now explicitly declares its
  own `[workspace]` so builds work within nested Git worktrees. Its four
  **actual** Linux Rust sidecars were compiled and staged under the existing
  git-ignored `apps/desktop/src-tauri/binaries/` directory for native checks.
  This is not an installed, signed or cross-platform package acceptance.

### Desktop Play process-ownership follow-up (24 November)

- **Closed a runtime-ownership hole in Desktop Play dispatch.** Previously,
  `runtime_launch` always spawned a separate `swarmcraft-runtime launch`
  sidecar, whose Fabric IPC was not owned by the daemon's live runtime
  registry. That standalone process cannot supply fresh authenticated Fabric
  evidence to the multiplayer route signer. The native Desktop now reads the
  current world descriptor and **signature-verified membership**, checks their
  exact member sets, local identity and accepted signed epoch, and reserves
  direct runtime launch only for a verified one-member local solo world.
- For any multi-member or non-solo accepted generation, Desktop Play starts
  only the existing daemon/supervisor and selects the world-local loopback
  gateway; it waits for the daemon's **newly signed, independent, exact-world**
  game-route status rather than a persisted migration Ready. It does **not**
  start another Java host or substitute an unverified process PID. If an
  older Desktop-owned direct host is still running, it refuses with a safe
  checkpoint/stop instruction rather than competing for the authority slot.
  The Player-only "Join current host" control remains independent of the
  server-install/EULA Play wizard.
- A verified daemon route indicates only that a new Minecraft connection may
  be attempted through the local gateway. The Play wizard explicitly avoids
  reporting an independently launched Minecraft JVM or a successful actual
  Minecraft login when the daemon owns the world.
- A single-member solo world retains the original standalone runtime path
  for its existing gameplay behavior; **solo mode cannot produce a quorum-
  verified multiplayer game route**. This deliberately does not permit
  stale solo authority to join a multi-member generation.

## Local verification performed

| Check | Result | Scope |
| --- | --- | --- |
| `cargo clippy -p swarm-cli --bin swarmcraft -- -D warnings` | PASS | Core Rust daemon and CLI wiring |
| `cargo clippy -p swarm-network --lib -- -D warnings` | PASS | Protocol, resolver and stream boundaries |
| `cargo test -p swarm-ipc` | 9 PASS | Authenticated bridge challenges and failure paths |
| `cargo test -p swarm-cli --lib` | 126 PASS | Supervisor, permit, signed authority, checkpoint, runtime registry and local IPC |
| `cargo test -p swarm-cli --bin swarmcraft` | 44 PASS | Signed transfer, recovery, fencing, quorum, gateway selection and failure paths |
| `cargo test -p swarm-network --lib -- --test-threads=1` | 111 PASS | Signed routes, durable generation floors, real localhost mock byte forwarding and consent loss |
| `node --test apps/desktop/tests/*.test.mjs` | 129 PASS after Desktop Play follow-up | Includes real Chrome 157 / matching temporary ChromeDriver smoke at three viewport sizes |
| Three-machine Python `test_*.py` unittest discovery | 112 PASS | Field evidence and acceptance-policy mocks only |
| Native Linux Desktop `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --locked` | PASS again after Desktop Play follow-up | Compiles native Tauri app with real staged sidecars, **not** a signed installer or GUI login |
| Native Linux Desktop `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked -- --test-threads=1` | 57 PASS after Desktop Play follow-up | Full native Tauri suite; includes 3 signed solo/multi-member dispatch and forged-membership refusal tests |
| Native Linux Desktop `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --bin swarmcraft-desktop --locked -- -D warnings` | PASS | Strict native lint; also replaced a behavior-equivalent pre-existing redundant closure in the game-route decoder |
| `python3 scripts/e2e/sabathon_local_gateway_smoke.py` | PASS | Disposable genuine daemon + native CLI, double-daemon ownership refusal, durable localhost port after restart, no-quorum/consent TCP refusal, no cached status |

All Cargo work was serialized via `/tmp/swarmcraft-sabathon-cargo.lock`
with no more than two build jobs, and disk free space was checked throughout.
After verification, approximately 3.3 GiB of disposable Rust incremental
compile caches were removed from this worktree (not the real sidecar binaries,
committed files, or canonical world data), leaving roughly 23 GB free.
The 111-test network run initially saw three `StaleAuthority` positive-fixture
expiries under heavy concurrent host load. The **unchanged** three tests passed
individually, and the complete **unchanged** suite passed serially. No
production freshness lifetime, signature, durable floor or safety predicate
was relaxed. `cargo fmt --all -- --check` and `git diff --check` passed.

## What still requires real machines / certification

**Nothing below has been claimed PASS.** Follow
`scripts/e2e/three_machine/SABATHON_DESKTOP_FIELD_RUNBOOK.md` and the strict
`scripts/e2e/three_machine/run.py verify` evidence gate:

1. **Three genuinely distinct installed machines** (Alice, Bob and Carol),
   actual packaged Desktop sidecars and genuine player-launched Minecraft
   Java `26.1.2` with compatible Fabric `0.19.3` and Java **25**, plus matching
   reviewed mods. The local toolchain currently exposes Java 21, not Java 25.
2. **Real three-voter canonical membership and signed checkpoint**, two
   explicitly consenting host machines, player-only zero-heap Carol, and
   independently measured successful Alice-hosted Minecraft login/gameplay
   through Bob and Carol's own fixed localhost gateways. Capture actual in-
   game inventory, positions and distinctive marker persistence.
3. **Signed planned transfer Alice to Bob** with save/ack/commit, old-JVM
   fencing, fresh post-transfer quorum and Fabric startup, no stale stream,
   and clients rejoining the exact saved world at unchanged per-client ports.
4. **Approved process crash and two-voter recovery**, independently verified
   exact restored checkpoint and unique successor, old authority unable to
   accept gameplay or overwrite state after reconnect.
5. **Real WAN/relay and loss scenarios** including relay outage, no raw-IP
   fallback, quorum loss, consent withdrawal, stale/expired routes, daemon
   restarts and Desktop world switching on actual installed clients.
6. **Independent human evidence/media review**, exact binary/package
   provenance and checksums, three-machine timed observations and an accepted
   strict field-verifier verdict. A locally passing UI/browser or P2P mock is
   not the same as real client login or an approved P0 release.

## Repeatable smoke and operator commands

From the consolidated worktree, build the **real** `swarmcraft` binary first.
Run the disposable negative control with:

```sh
python3 scripts/e2e/sabathon_local_gateway_smoke.py --cli "$PWD/target/debug/swarmcraft"
```

For a genuine selected world on each player's machine, start its daemon,
select its world with `swarmcraft world game-route-select <world>`, then inspect
`swarmcraft world game-route-status <world> --json`. An `unavailable` result is
the correct and safe outcome until live majority, consent, runtime and signed
game-route verification all succeed. The mere presence of a bound localhost
gateway does **not** mean a Minecraft client connected, that a host is ready,
or that the release has passed. Do not replace it with configured
`game_endpoint`, cached migration Ready or synthetic field results.
