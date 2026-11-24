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

### Legacy Desktop Play and process-world isolation follow-up (24 November)

- The original legacy `host_world` Tauri command was still reachable from the
  shipped Play / Host actions and launched an independent `swarmcraft-host`
  sidecar instead of the new signed Desktop Play dispatcher. It now validates
  the canonical world ID and explicit EULA/Java/mod inputs, then delegates to
  the **same** `runtime_launch` path. Desktop no longer exposes an alternate
  native direct-JVM launch via that compatibility command. The signed
  multiplayer route returns no fabricated host PID; both legacy Play and the
  runtime wizard distinguish that result from starting a solo Minecraft JVM.
- A world transition while daemon startup is in progress causes the solo
  dispatcher to **recheck** signed local world membership/epoch before a
  standalone JVM can launch. The Desktop process slot now also remembers the
  exact world of a running direct host: a second world's launch cannot reuse
  an unrelated PID or be misreported as successful. Existing same-world
  process reuse and generation-based termination fencing are retained.
- Runtime diagnostic filenames now accept validated canonical `scworld:` IDs
  using portable, traversal-safe per-world names (including Windows), without
  weakening sanitization of legacy diagnostic keys. Legacy Play suppresses a
  completed result if the player selected another world during the operation.
- These fixes close locally observed Desktop dispatch and reporting gaps. They
  do not assert that any Java 25 Fabric runtime started, connected to a game
  client, or passed installed-machine acceptance.

## Local verification performed

### Automatic Desktop handoff and transport admission (24 November)

- Required Validation run `45381070672` on `9a8a5b4` passed 19 jobs,
  including macOS/Linux Rust, all four Desktop package builds, installed Linux
  GUI acceptance, live Minecraft/Fabric startup and the 2 GiB interrupted
  QUIC soak. Windows Rust failed a discovery-fixture authentication deadline;
  the dependent aggregate gate also failed. This was not a fully green run.
- The shipping Desktop wizard now calls `world transfer-start <world> <target>
  <source_epoch>` through its native command, ensures the daemon is running,
  and polls `world transfer-status <world>` for durable signed-stage progress.
  The manual signed-token workflow remains an advanced compatibility option.
  Automatic requests are bound to the displayed epoch and serialized; a
  conflicting target cannot overwrite an outstanding intent. Duplicate
  requests read the same durable stage. Reopening the wizard or stale
  asynchronous responses cannot create a second transfer. An observed
  successor is **not** a live route, runtime readiness, or shutdown proof.
- Discovery reconnection tests now re-drive authentication with the same
  single-flight dial path used by discovery. A real repeated-disconnect run
  also exposed phantom request-response connection state: a composite
  libp2p behaviour registered a handler before its later limits behaviour
  rejected the connection. Admission limits are now first in both swarms;
  a deterministic rejection regression and all five real discovery tests
  passed. No connection limit or authentication rule was weakened.
- Fresh private data and control directories are created with owner-only
  Unix permissions even under umask `0002`; previously that normal umask
  could make a newly created data root fail the authority-permit guard.
  Existing unsafe directories remain subject to the fail-closed guards.
  No recursive permissions repair of existing user data is performed.
- All **133 Desktop JS tests passed**, including three real Chrome
  viewport smoke checks and the exact native IPC command table. The
  process-level transfer gate exercises both separate manual CLI stages
  and automatic exchange over three independent daemon processes. These
  tests are not three-machine Minecraft client gameplay.

### Current implementation-plan residuals

The original `SABATHON_IMPLEMENTATION_PLAN.md` is an historical audit and
its status labels are not automatically updated by these code changes.
Items 2, 3, 12, 19, 20 and the baseline of 26 have integrated code and
automated checks; item 4 now has the default Desktop automation path.
Items 1, 5 and 6 still need the installed multi-device player journey.
Item 28 now has the shipping Desktop checkpoint and replica diagnostics:
the local accepted manifest, logical save size, last exact authenticated
replica receipt in the daemon session, elapsed receipt age, checkpoint-number
gap, and explicit progress-at-risk warnings. The local integrity check binds
the manifest hash returned by the verifier, rather than a cached snapshot
number. Receipt age is not the original save time or proof of current
recoverability; actual unsaved gameplay loss still needs field measurement.
Item 13 is the independent field-evidence
gate, not a unit-test checkbox. Item 49 needs reconciliation against main
after an authorized merge. Release-gate items 37 and 50 and the P1/P2
backlog are separate from the Sabathon P0 implementation slice. No field
acceptance or production release is claimed from this branch report.

### Automatic handoff and checkpoint verification follow-up (24 November)

- Exact `ca83afc` local three-daemon process gate passed both scenarios:
  signed manual transfer and automatic transfer waiting for an independent
  witness and explicit target hosting consent. The three replicas converged
  without manual token exchange. Strict targeted CLI Clippy also passed.
- A receipt retained for diagnostics after disconnect cannot replace the
  independently cleared authenticated transfer acknowledgments or quorum
  evidence. Stale/wrong-world reports never show current replica receipts.
  The sleep readiness path now requires a signature-verified sleep record,
  signed accepted epoch, eligible authority, matching fence, exact verified
  saved manifest and conflict-free history before displaying Sleeping.
- Required Validation `45387692037` found an intermittent Ubuntu discovery
  failure where the explicit exact-resolve provider received zero application
  requests. The query now re-drives reserved explicit dial attempts within
  its existing deadline and treats provider error responses as incomplete
  discovery. A handshake-not-yet-complete refusal uses the same existing
  two-retry per-request budget, with exact request/peer binding. No
  authentication, quorum verification or request-count limit is relaxed.

### Final macOS Rust regressions from `ece4b1f` (24 November 2026)

- GitHub Required Validation run `45277228605` completed with **18 successful
  jobs**, including Linux and Windows full Rust suites, process-level
  acceptance, installed Linux GUI smoke, all four desktop packaging targets,
  dependency audits, and the clean-machine Minecraft/Fabric journey. Only
  the macOS Rust job failed; consequently the aggregate required validation
  gate also failed. The optional sustained network soak was skipped.
- Both failures were in `swarm-network`: the foreign-owner ancestry test
  called the internal UID-aware validator directly, bypassing the previously
  added macOS alias handling, and the stalled route-verifier capacity test
  depended on an unreliable fixed sleep before asserting worker recovery.
- Moved Apple's narrowly scoped alias handling into the shared UID-aware
  validator. All real ancestry ownership, mode, and symlink checks remain
  enforced. Reject parent traversal in the complete lexical path even if a
  nonexistent ancestor would otherwise terminate the filesystem walk.
- Replaced the capacity test's fixed sleep with an explicit verifier release
  and observation of the recovered semaphore permit. The stalled request
  still times out and the occupied slot still refuses further requests; the
  next worker independently refuses absent quorum proof. No production
  deadline, worker limit, or authority requirement was relaxed.
- Local verification: **111/111** network library tests passed, strict
  network all-target/all-feature Clippy passed, and workspace formatting and
  diff checks passed. The updated macOS behavior requires an independent
  exact-SHA GitHub result.

### CI cross-platform regressions from `5287081` (24 November 2026)

- Required Validation run `45270169979` passed process-level acceptance,
  real clean-machine Minecraft/Fabric journey, packaging on all four desktop
  targets, network impairment, fuzz, and dependency/security checks. The
  separate all-workspace Rust matrix caught three further issues:
  **macOS** Apple-owned `/var` temporary-path alias rejection in private world
  port storage; **Windows** transient sharing / duplicate-writer errors during
  atomic permit and migration-status publication; **Linux** the adversarial
  discovery test reusing an exhausted QUIC blackhole session across two
  separate queries.
- macOS durable-path validation now recognizes only root-owned, canonical
  Apple `/var` and `/tmp` aliases targeting `/private/var` and `/private/tmp`.
  It validates every resolved ancestor under the original strict UID/mode/
  symlink rules. No user-owned symlink or group-writable ancestor is allowed.
- Windows status writers now serialize via a private cross-process publication
  lock. Existing destination replacements retry bounded transient Windows
  sharing and replacement errors, keeping the old complete value on failure.
  A Windows read interrupted by file replacement is explicitly treated as
  **unavailable, never as a valid heartbeat or Ready status**; successful
  reads must decode complete generation-scoped records.
- The discovery test now uses distinct malicious providers for browse and
  exact resolve. Each must independently receive 1–3 requests, reject false
  success, and terminate inside a bounded deadline, rather than relying on a
  prior QUIC connection that the browse half intentionally churned down.
- The above changes need a **new GitHub exact-SHA run** before being counted
  as independently verified cross-platform fixes; a Linux test does not
  certify Windows/macOS behavior.

### GitHub-hosted integration follow-up: application peer connectivity and Windows publication

- GitHub Required Validation run `45267243725` on commit `0db4a66`
  independently passed the clean-machine Java 25 Minecraft/Fabric build and
  player-journey job, provider/Desktop checks, package jobs, network impairment,
  and security audits. It **failed** later in the genuine three-daemon and
  partition test harnesses: test peers were configured through
  `SWARMCRAFT_BOOTSTRAP`, which deliberately classifies them as infrastructure
  rather than authenticated application members, so no signed majority lease
  acknowledgements could form. Infrastructure was **not** promoted to voters.
- A bounded `swarmcraft daemon --peer <transport-multiaddress>` path now dials
  explicit known application peers. It requires a valid terminal transport
  PeerId, safe addresses and explicit QUIC listener ports, and still requires
  connection-bound signed PeerHello plus ordinary canonical membership/quorum,
  consent and fencing checks after dialing. Exactly formed operator-owned
  `127.0.0.1` QUIC application hints may support real multi-process loopback
  tests; publicly signed invites continue to reject loopback addresses.
  The recovery and partition tests now use this path rather than treating
  bootstrap infrastructure as authenticated voting members.
- The same GitHub run found Windows `Access is denied`/`NotFound` during
  concurrent visible permit and migration-status publication. For an existing
  Windows destination, publication now uses `ReplaceFileW` with bounded
  transient sharing retries rather than `MoveFileExW` alone. No delete-then-
  rename gap, stale permit promotion, or weakened status assertion was added.
  **The new Windows implementation still needs an independent passing CI run**;
  a local Linux compilation cannot certify Windows kernel semantics.

### GitHub-hosted CI follow-up (24 November)

- Manually dispatched **Required Validation** against exact integrated
  commit `2d75b9e` (GitHub Actions run `45262291031`). Unlike a successful
  local Desktop-only test, CI exposed actual cross-platform and workspace
  regressions: official Minecraft `26.1.2` names the running server listener
  getter `getPort`, not the older Yarn `getServerPort`; integration tests had
  stale `migration::supervise` signatures; strict all-target Clippy caught
  test-fixture lints and Windows-only mutable OpenOptions; and the live Fabric
  request counter needed a non-deprecated, MSRV-compatible overflow-checked
  atomic increment.
- Updated the native Fabric bridge, runtime registry, test harnesses and
  platform-specific lints **without relaxing authorization, freshness,
  lifecycle checks, or fail-closed assertions**. Upgraded only the Desktop
  `cookie` lockfile entry from `0.18.2` to `0.18.3` to address the new
  `RUSTSEC-2026-0594` advisory reported by GitHub's dependency audit.
- The three-machine evidence workflow now runs its negative-control harness
  automatically on each P0 integration branch push. Live remote-machine
  preflight requires a **separate explicit manual opt-in and provisioned
  pinned SSH hosts**; a green negative-control job can never be reported as
  real three-machine Minecraft certification.
- The original CI run tested the **prior** exact SHA and is not a verdict
  on the source fixes listed above. Obtain an independent GitHub-hosted
  Required Validation result against the new pushed commit before using it
  as acceptance evidence.

| Check | Result | Scope |
| --- | --- | --- |
| `cargo clippy -p swarm-cli --bin swarmcraft -- -D warnings` | PASS | Core Rust daemon and CLI wiring |
| `cargo clippy -p swarm-network --lib -- -D warnings` | PASS | Protocol, resolver and stream boundaries |
| `cargo test -p swarm-ipc` | 9 PASS | Authenticated bridge challenges and failure paths |
| `cargo test -p swarm-cli --lib` | 126 PASS | Supervisor, permit, signed authority, checkpoint, runtime registry and local IPC |
| `cargo test -p swarm-cli --bin swarmcraft` | 44 PASS | Signed transfer, recovery, fencing, quorum, gateway selection and failure paths |
| `cargo test -p swarm-network --lib -- --test-threads=1` | 111 PASS | Signed routes, durable generation floors, real localhost mock byte forwarding and consent loss |
| `node --test apps/desktop/tests/*.test.mjs` | **130 PASS** after legacy Play compatibility follow-up | Includes real Chrome 157 / matching temporary ChromeDriver smoke at three viewport sizes |
| Three-machine Python `test_*.py` unittest discovery | 112 PASS | Field evidence and acceptance-policy mocks only |
| Native Linux Desktop `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --locked` | PASS again after Desktop Play follow-up | Compiles native Tauri app with real staged sidecars, **not** a signed installer or GUI login |
| Native Linux Desktop `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked -- --test-threads=1` | **58 PASS** after legacy Play compatibility follow-up | Full native Tauri suite; includes signed dispatch, world-scoped host PID isolation, canonical diagnostics, and forged-membership refusal tests |
| Native Linux Desktop `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --bin swarmcraft-desktop --locked -- -D warnings` | **PASS again** | Strict native lint after legacy Play process ownership fix |
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

### Recovered-authority routing and real gateway gate (24 November)

- Required Validation `45389218911` passed **21/21 jobs** on exact baseline
  `27229b4aea6edbe7115faa4d3adb86c02ee0c978`, including the 2 GiB interrupted
  network soak, Windows/Linux/macOS, Desktop packages and installed Linux GUI,
  and official Minecraft/Fabric startup.
- The successor route now independently verifies the durable recovery
  certificate against the exact archived pre-promotion signed membership.
  Newly accepted authority metadata inherits and re-signs the existing world
  configuration without changing policy. Recovery is not permission to bypass
  a fresh canonical quorum or authenticated Fabric proof. Old or corrupt
  history and incomplete handoffs remain fail-closed.
- Exact-generation lease acknowledgments drive bounded retries of successor
  configuration, membership, and direct checkpoint extensions. Signed sleep
  replication requires the exact verified checkpoint receipt before sending
  the sleep record; the source does not report shutdown safety while an active
  member has not acknowledged that boundary.
- Local signed-recovery regression, both transfer process tests and the
  hard-kill three-daemon recovery/resynchronization test passed. Strict
  all-target/all-feature CLI and storage Clippy passed. The process fixtures
  explicitly create private temporary ancestors; the production registry
  correctly refused the previous group-writable fixture under umask 0002.
- The required hosted player-journey workflow now explicitly executes a new
  three-daemon test with official Minecraft, the candidate Fabric artifact,
  real authenticated IPC and real Minecraft status packets via the persistent
  player gateway. It asserts the initial route, signed handoff, restored save
  marker, crash recovery, and replicated sleep boundary. Its compilation is
  checked locally; the live test is deliberately run in CI, not claimed as
  passed here. The test seeds a canonical three-member genesis; it does not
  establish the fresh invitation/bootstrap journey, real Microsoft account
  login, independent machines/WAN, or in-game inventory continuity. Those
  gates remain open. No security or memory-admission rule was relaxed.

### First real gateway gate findings (24 November)

Required Validation `45396688375` on `226c4751` passed Linux Rust,
process-level acceptance and the package builds, but the new real Minecraft
status exchange reset its initial connection. Windows import tests also
failed on long membership-history paths. The raw Windows write-through rename
now uses extended-length paths derived from canonical existing parents;
replacement and removal have a dedicated deep-path Windows regression.

The host signer, route verifier and incoming stream admission had reused one
signed transcript nonce for separate independent quorum collections. The
production collector correctly rejects duplicate live nonces; the old
synthetic source did not. The verifier and host stream admission now use
separate unpredictable fresh evidence nonces, while preserving exact signed
transcript binding, single-use verification and the durable replay ledger.
The real QUIC resolver-to-gateway regression now rejects every repeated
proof nonce across the signer, verifier and stream endpoints, and passed
locally. No freshness deadline or quorum requirement was relaxed. The hosted
gate prints bounded daemon diagnostics on failure. Windows and actual
Minecraft verification of these repairs require the new CI run.
