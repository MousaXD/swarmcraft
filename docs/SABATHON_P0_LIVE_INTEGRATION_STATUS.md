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

### Independent proof lifetime and stream admission (24 November)

Required Validation `45398307104` on `5af60851` passed the Rust
suites on Linux, Windows and macOS, all four native desktop package
builds, and process-level acceptance. The real Minecraft gate still reset
the initial connection: Fabric reached ready, the route status verified,
and the player gateway then logged `verified route is stale`.

The signer previously capped the entire offer/challenge negotiation at the
expiry of an earlier short-lived observation. The additional mandatory
admission proofs could therefore exhaust that observation even after a
fresh proof succeeded. The offer and response now retain their existing
bounded transcript maximum ages (15 seconds and 5 seconds), separate from
the live authority lease. The signer checks evidence freshness after
collection, the verifier still consumes a one-use challenge, and every
stream must enter with a fresh verified capability. Admission must prove
all immutable signed fields independently again; the transcript must remain
unexpired. Forwarding is additionally bounded by the latest independently
proven monotonic expiry, with copying paused during renewal and shutdown or
identity loss failing closed. No proof validity window, quorum, membership,
consent or replay protection is removed. The delayed-proof local gateway
regression, host signer security tests (3), and real QUIC resolver test
passed. Actual Minecraft handoff/recovery must still pass the hosted gate.

Fresh invitation audit (not yet implemented): new worlds can lack an
accepted epoch before the first runtime launch; the ordinary Join path
requests storage-only membership rather than a host candidate. The existing
live-join test seeds the accepted epoch manually and does not prove a full
three-device Create/Invite/Join/Play flow. These remain P0 gaps, along with
complete first-contact signed history and epoch synchronization checks.

### Hosted handoff milestone and fresh invitation work (24 November)

Required Validation run `45399615046` on `6577f523` logged
`HOSTED_LIVE_GATEWAY_INITIAL_PASS` and `HOSTED_LIVE_GATEWAY_HANDOFF_PASS`.
The planned transfer restored the saved marker and the same player loopback
endpoint served the successor's actual Minecraft status protocol. After
crash recovery the real Fabric JVM became ready, but the next player
connection reset. This is not yet a full hosted acceptance pass, an
authenticated Minecraft login, inventory proof, or physical-machine field
certification. Linux also failed the automatic invite regression.

The gateway now uses the already independently verified, still-fresh,
single-use route capability for its initial control-only admission step,
instead of immediately spending another live proof before opening the
transport. The host's independent admission and the client's independent
post-ACK/post-local-dial proof remain mandatory before forwarding Minecraft
bytes. No signed transcript deadline or live lease is extended. Added
specific expiry diagnostics and flushed the signed host admission ACK.

Fresh Create/Import now records a signed genesis quorum-of-one at epoch 1
bound to the exact first checkpoint. This intentionally uses the signed
daemon route rather than creating a new standalone writable solo branch.
Existing worlds are not automatically rewritten. A sole signed member's
direct wake preserves a prior quorum-of-one generation and checks the exact
sleeping authority; a corrupted epoch is not a missing epoch.

Ordinary Join remains player-only by default. An explicitly selected
host-candidate role is bound into the signed request and approved through
the same joint membership certificate, without creating hosting consent or
accepting the EULA. Both legacy and Svelte Desktop routes carry the option.
A repeat join cannot overwrite an existing committed descriptor or silently
change a pending role. Already-running daemons retry signed, unexpired
invitations; first-contact proposal/commit messages carry the bounded
genesis-anchored certificate chain. History is archived without advancing
canonical membership; the exact joint commit is still required. The
receiver retries the signed epoch after the exact verified base snapshot
receipt, and interrupted membership descriptor publication can be repaired
without deleting a newer durable prepare.

Local evidence during this change: the focused signed local route/consent
regression passed; the original two-peer invite test passed after binding
both peers to the real fixture LAN interface; 56 Desktop source/contract
tests passed. The expanded fresh Create -> Invite -> two sequential Join
test verified the first join, epoch, config and checkpoint, then exposed a
conflict with the next durable membership prepare. The repair preserves
that prepare. A new run is required to certify the complete three-peer
flow. Typed Svelte tests could not execute in this worktree because its
frontend dependencies are absent; the updated native payload tests and
full hosted validation must run in CI. The LAN fixture seeds diagnostics
with a concrete reachable listener, so it does not prove automatic
interface discovery on independent machines.

### Real Minecraft gateway recovery verified (24 November)

Exact-SHA Required Validation run `45402665550` on `9765afb` passed all
three real Minecraft status exchanges (initial, signed handoff and crash
recovery), restored the saved marker, verified the recovery certificate and
replicated the recovered JVM's signed sleep checkpoint. The Rust hosted
acceptance test completed successfully in 126.71 seconds. The job's separate
exact-line grep failed because libtest printed its test-name prefix on the
initial marker line. Emit standalone markers while retaining the three strict
grep checks and Cargo exit-status enforcement.

That run also passed the multi-GiB interrupted QUIC soak, process acceptance,
Windows Rust, both RustSec audits, Fabric, specialist and governance checks,
all four native Desktop package builds and installed Linux GUI smoke. Linux
and macOS Rust stopped at membership-partition tests: their historical
membership fixtures skipped the newly required joint genesis certificate,
used infinite invite expiry and relied on non-shareable loopback bootstrap
addresses. Replace these with genuinely signed historical certificates,
bounded invitations and explicit application-peer connections. Preserve the
partition assertions: new-side prepares must actually exist, the previous
certificate must remain canonical, and an unavailable old quorum cannot
activate the proposed universe. The older live-join fixture also uses an
explicit loopback application hint instead of an invalid shareable invite.

These are same-runner processes and real server protocol status exchanges,
not authenticated player login, actual inventory continuity, or independent
physical-device field acceptance. The updated complete three-peer public
Create/Invite/Join test had passed locally on `9765afb` in 13.47 seconds.

### First-play quorum-of-one and CI fixture ordering (24 November)

The subsequent exact-SHA Required Validation `45420043497` on
`54603b9` passed the entire hosted live Minecraft workflow, including
the three standalone success-marker assertions. Linux/macOS Rust then
failed during fixture construction: configuration publication preceded
the accepted membership/epoch on which its signer validation depends.
The fixture now writes its signed genesis epoch and membership before
configuration and preserves every partition safety assertion.

Audit of first Play found a distinct product gap. The Fabric guard
previously treated one-member descriptors as exempt from live authority
permits even when the world had a signed quorum-of-one generation.
That prevented the first signed route and left the runtime policy tied to
an unsigned member count. The supervisor now requires a live advancing
permit for signed quorum worlds, including new one-member worlds and
one-member wake generations, and passes the guard policy explicitly to
Fabric. The legacy exclusive solo path remains distinct and rechecks its
sole signed membership while running and before checkpoint publication.
The clean-machine acceptance script now exercises the actual daemon-owned
gateway, verifies signed route evidence and Minecraft status at genesis
and after an explicit wake on a restarted daemon, asserts the stable
gateway address and retains real managed Java installation, EULA,
hosting consent, checkpoint and save-marker checks.

The changed Rust and Fabric code and revised real-player gate require
new exact-SHA CI evidence. Formatting, diff, shell syntax and embedded
Python syntax checks passed locally. The late-join audit also identified
an unresolved gap for worlds beyond their first checkpoint: the initial
replica push currently offers the latest manifest despite direct-parent
validation, and an epoch must be installed at its exact base. A bounded
parent-by-parent history sync (including historical epochs/config where
applicable) is still required. Do not relax snapshot ancestry validation
or count a late join as playable before verified catch-up.

### Late join in the original signed generation (25 November)

The original-generation bootstrap no longer blindly offers the newest
manifest. After committed membership, an authenticated progress query
selects the creator-signed first checkpoint; its exact verified receipt
can trigger delivery of the original signed epoch even if the host has
newer saves. Each further offer must directly extend the recipient's
reported parent by number, sequence and hash in the same generation.
The receiver independently verifies signatures, data, ancestry and epoch.
One outstanding manifest negotiation per connection/world prevents an
overlapping status-driven offer and final checkpoint offer.

The status report is only a replication scheduling hint. A historical
genesis receipt cannot satisfy a current-checkpoint transfer or sleep
barrier. These still require the exact current ReplicaAck from every
required authenticated member. Original-generation final-save replication
can query a lagging peer's progress without renewing a fenced lease.

Local three-process Create/Invite/Join acceptance now publishes two
distinct signed saves after the first peer joins, then admits a third
player-only peer. All three replicas must verify all three checkpoints,
their exact parent hashes, signatures, data, current config and signed
epoch; no hosting consent is granted. The expanded regression and
existing live join both passed. Six membership/partition regressions
passed in the preceding local run. Full hosting/transfer process cases
were not exercised by the local low-memory runner, whose existing
capacity check refused hosting; use the exact-SHA hosted validation.

This is deliberately limited to a retained original creator-signed quorum
generation. Late join across a host transfer, recovery or wake generation
still requires immutable historical epoch, transition certificate and
config records and a verified replay protocol. Pruned history, actual
Minecraft client login/inventory, and independent-device field acceptance
are not certified by this change.

### Historical authority replay foundations (25 November)

Canonical control publication now preserves the exact replaced epoch and
world-config records, staged transfer phases, and cleared sleep boundaries
in an immutable local authority-history namespace. Before replacing a
recovery epoch, it also preserves the matching accepted recovery certificate
if present, not an unaccepted pending candidate. Archive publication is
serialized by the same world transaction lock as the live record. Conflicting
or corrupt archive entries and a symlinked history directory fail closed.
Live control reads never fall back to this history.

Read-only epoch/config history collectors verify each record signature,
exact predecessor hashes and counters, original creator identity, and
the genesis checkpoint bytes or compatibility fingerprint respectively.
Both collectors refuse histories beyond 256 records; missing older legacy
records are not reconstructed or silently skipped. They return signed
historical records, NOT a certified transition or a live authority grant.
Before cross-generation replication can be enabled, a receiver must still
verify the exact historical membership and transfer/recovery/sleep proofs
against each checkpoint boundary, and persist them through a replay-specific
state machine that cannot activate a retired generation. No wire replay,
late join after migration, or physical gameplay certification is claimed
by the archive implementation.

### Certified replay proof exchange (25 November)

The bounded V1 historical proof now includes the complete signed membership
and configuration chains, all epoch records, exact checkpoint references,
and one transition proof per successor. Validation requires all three
signed transfer phases, or the recovery certificate with unique valid
historical voter signatures and a majority, or an exact creator-signed
sole-member sleep boundary. Checkpoint number, sequence, state root and
promotion continuity must match. Solo/divergent histories and incomplete
or pruned ancestry fail closed. Each actual checkpoint manifest still
requires independent signature, parent and blob verification.

The daemon now automatically requests the proof for newly committed
members of migrated worlds. Only a connection-authenticated current
member may request it from the accepted authority. Bounded workers
collect/verify the proof off the lease event loop; the authority rechecks
the requester, membership and generation before sending. The receiver
binds the envelope to its exact committed membership and durably stages
it without creating a live epoch, config, snapshot or replica receipt.
Ordinary first-contact/live snapshot and epoch admission is blocked
until the signed join and any required replay complete.

Regression: a real two-daemon invitation after a signed sole-member
sleep/wake stages the verified two-epoch proof, survives reopening
storage, and never grants live authority or hosting consent. Separate
fixtures verify a manual transfer followed by majority recovery, rejecting
wrong transfer bases, insufficient recovery votes, and missing joint
membership certificates. This is proof exchange, not full cross-generation
checkpoint replication: the remaining step is authenticated resumable
manifest/blob replay and crash-safe publication of the final verified
canonical head and control state, with a fresh live quorum still required
for routing. No field certification is claimed.

### Resumable cross-generation checkpoint replay (25 November)

First-contact replay now requests the next exact checkpoint from the
durably staged proof. The current authenticated authority verifies the
committed membership and checkpoint hash before serving data. The
receiver checks every signed manifest against the pinned proof, enforces
direct ancestry, and resumes durable partial blobs after a restart,
including the fully written partial-file boundary. Historical checkpoints
do not produce live replica acknowledgements or advertise a recovery
candidate while the final epoch is absent.

Final publication checks the complete checkpoint chain and blob bytes
under the world/GC locks, archives the historical control records, and
publishes the final accepted epoch last. Conflicting metadata, pending
authority decisions, or missing evidence fail closed. Only completion
produces a replica receipt; live routing still requires fresh quorum,
fencing, consent and authenticated Fabric evidence. The recovery proof
continues to use its original ballot roster: later admissions require
each exact signed joint-membership certificate. Exact recovery retries
cannot consume a newer signed sleep record.

The two-daemon test covers a newly admitted replica after a signed wake,
restart with partial checkpoint data, verified resume, and final epoch
publication. Signed fixtures cover transfer followed by majority recovery,
later joint admission, interrupted publication, and missing/corrupt proof
rejection. This is retained-history first-contact replay; already accepted
stale replicas, authority changes during a pinned replay, pruned history,
and oversized histories are not certified by this test. Independent-device
Minecraft login, gameplay, inventory, and hosting acceptance remain
separate field gates.

### Restart reconnect and route-admission regressions (25 November)

Required validation run 45504966244 on de1f665 passed the Linux and
Windows Rust tests, all desktop packages and the QUIC resume test, but
failed macOS automatic-join restart and the hosted Minecraft recovery
journey. macOS lost its connection after restart while multicast discovery
reported no route to host. The hosted gateway refused an expired signed
response after successful initial connection and planned handoff.

The daemon now retains the completed, signed join request separately from
pending admission. Its addresses are only reconnect hints: both peers must
remain in signed committed membership, the joining identity must match,
and ordinary connection-bound authentication and address scope validation
still apply. Completed invitations are never resent as join requests and
local membership removal deletes the hint.

A gateway connection may resolve one completely new signed route if an
earlier attempt expires before any application payload can be copied.
There is no retry after copying begins, no cached route/IP fallback, and no
extension of signed deadlines. Recovery verification avoids redundant
joint-history signature validation only when the fully verified ballot
roster has an exact signed promotion with no intervening admissions.
The first local reconnect and transport suite passed; exact-SHA hosted
validation is required before marking the macOS and Minecraft gates green.

The corrective commit b1dd6b9 passed required validation run 45507603759:
all 20 selected jobs succeeded, including macOS Rust, the hosted real
Minecraft initial connection/handoff/recovery, and installed Linux GUI.
The optional network soak was skipped. The expanded local tests and strict
Clippy also succeeded.

### Stale-replica successor proof discovery and anchoring (25 November)

Existing non-authority replicas (including player-only members) request a
signed historical replay proof when an authenticated known host candidate
reports a newer generation. The status itself is only a discovery hint.
Status polling and consumption use the one-second lease tick, rather than
the five-second invitation tick that would outlive the three-second
status freshness bound for player-only peers.
The complete envelope must authenticate its final authority, include the
recipient in its committed membership, and verify every transition and
joint certificate. The replica captures an exact signed local epoch,
membership, config, descriptor and canonical checkpoint anchor before
the request. It verifies all retained local manifests and blob bytes
against the returned history. Genesis-valid forks, different local
checkpoint anchors, substituted certificates and rollback are refused.

The candidate is durably stored separately from live canonical state.
The storage transaction rechecks the entire anchor; a normal replication
or authority change racing the verification invalidates it. A later
authority may extend a pinned candidate only by preserving its complete
epoch, membership, certificate, config, checkpoint and transition prefix.
Neither pinning nor replacing a candidate creates a replica receipt,
installs a live epoch, changes membership, removes a durable fence, or
permits hosting. Source selection currently requires a known locally
eligible member, and excludes a replica that is itself the old authority.

Tests exercise signed transfer followed by recovery and joint admission,
rejecting a validly signed divergent genesis epoch and an older envelope.
The daemon integration extends the ordinary invite/replication scenario
with a player that misses a checkpoint and transfer, then receives the
newer authority's proof over an authenticated connection. Isolated
checkpoint downloads for this stale-replica path and crash-safe final
publication over existing accepted control state remain open. The
first-contact replay path is unchanged.

### Isolated stale-replica checkpoint downloads (25 November)

The accepted replica requests the next exact missing checkpoint from
the verified successor envelope. Its suffix manifests and new blob bytes
are staged in a separate namespace keyed by the exact local anchor, not
the canonical snapshot directory. Existing complete local blobs may be
reused after verification. Each manifest negotiation validates the signed
proof and historical signer, and storage repeats the pinned source,
membership, checkpoint and parent checks. Staging is ordered; neither a
future manifest nor a superseded authority's negotiation may advance it.

Durable partial encoded offsets and finalized staged manifests survive
reopening storage, including the interrupted EOF-promotion boundary.
Later verified proofs may extend the same staged prefix without copying
it again. An anchor change invalidates the old negotiation. Disconnects
remove the in-memory historical cursor so it cannot become an ordinary
canonical blob write; no historical download sends a ReplicaAck.

The acceptance scenarios cover an already admitted player missing a
checkpoint and transfer, real authenticated transfer of new blob bytes,
restart with a retained isolated suffix, and the old epoch, membership,
config and canonical head remaining unchanged. Signed fixtures also
exercise a superseded source, out-of-order checkpoints, partial byte
resume and an extended proof through recovery and later admission.
The final existing-state publication transaction and its crash fencing
are still required. Pruned histories and missing or corrupt staged data
must not be treated as a complete canonical replica.

### Fenced stale-replica publication and restart recovery (25 November)

An existing non-authority replica with a complete isolated suffix can now
publish the exact verified successor history automatically. Publication is
separate from first-contact replay. The application rechecks the signed
genesis/epoch/membership/config/transition envelope and every manifest
signature. The storage transaction independently checks the exact pinned
anchor, full local prefix, staged suffix and blob contents under the GC
and world locks. A durable intent commits to the pinned anchor/proof
before any canonical writes; ordinary control reads, transaction writes,
snapshot access and retention fail closed until the intent is removed.

Publication streams missing blobs into the canonical store without
overwriting existing blob bytes, installs immutable manifests and signed
history, and writes the final config, membership, descriptor, canonical
head and epoch. The intent remains the visibility fence even after the
new epoch is on disk. Restart accepts only exact old or intended target
records and exact copied manifest slots; the same proof and all bytes
must be verified again before clearing the intent. No intermediate
authority, hosting consent, freshness quorum, or lease is manufactured.
The daemon clears obsolete per-world runtime observations and only sends
a complete replica receipt after publication.

The real daemon integration now completes the stale player's replay
after missing a checkpoint and signed transfer, including new blob
bytes, exact final membership/epoch/head and restart persistence.
Signed fixtures exercise transfer, recovery and joint admission,
partial blob resume, a conflicting durable promise before publication,
mixed canonical writes after an intent, conflicting control record
rejection, and recovery with the final epoch already written but still
fenced. This is a simulated interruption at explicit durable boundaries,
not a physical power-loss campaign.

The automatic publisher deliberately excludes the retired local
authority and refuses existing sleep/transfer/recovery/membership
promises, pending admission/removal, solo conflicts, unresolved ordinary
snapshot commits, and incomplete/pruned history. It does not silently
clear local decisions. Compatible consumption of those exact fences and
first-contact re-targeting across authority changes remain outstanding;
independent three-machine field acceptance is still required.

Prior required validation run 45511308191 on 7b5b4fe initially failed
before the installed Linux GUI test: the hosted runner package index had
no supported native WebKitGTK WebDriver. A failed-job retry passed all
20 selected jobs (optional network soak skipped); no gate was relaxed.

### First-contact replay retargeting and discovery authentication (25 November)

Required validation run 45515921964 on a7f2326 failed the Windows
simultaneous bidirectional discovery dial convergence test. Inspection
found a plausible connection-election race: a proof on a superseded
connection could consume the current receiver challenge, and a challenge
rejected during remote duplicate cleanup was not retried. Commit
b35125c preserves the current nonce on stale/wrong-connection proofs
and permits at most three fresh challenges within the existing
ten-second authentication deadline and request admission budget.
Local discovery network tests and strict Clippy passed. The failed-job
rerun on a7f2326 subsequently passed all 20 selected jobs; this confirms
the original failure was intermittent, not that the new fix is already
certified on Windows. Exact-SHA validation of the new commits is required.

First-contact history is now pinned separately from the accepted
admission membership. The receiver may use authenticated status hints
to request a newer generation from a known eligible member, but checks
the full signed history, exact original admission, local recipient,
source, retained membership certificates and checkpoint bytes before
changing the pin. The new proof must extend the complete pinned history
and target a strictly later certified membership; checkpoint-only growth
does not move the original finish line. A superseded source cannot
negotiate or continue the old download. No epoch or final membership is
installed during this data-only phase.

The publication transaction binds a stable record and exact intent
before writing target metadata; the original signed admission survives
restart even if the final descriptor or membership has already reached
disk. Ordinary epoch, membership, config and membership-promise writers
cannot bypass a pinned first-contact replay. Existing unrelated promises,
pending admission/removal, conflict records and nonmatching canonical
config/certificates are refused. The final epoch is still the last
canonical write, with live routing subject to its independent freshness,
quorum, consent and IPC requirements.

The daemon regression uses three real processes: a late player has a
pinned proof and durable partial blobs from the original authority,
then that authority stops and a known candidate supplies a signed
successor transfer history. The player must resume the prefix, reject
the retired epoch and publish the exact final generation. Signed
fixtures exercise rollback rejection, recovery plus joint admission,
and simulated restart during retargeted publication.

Limitations: discovery currently requires the new authority to have
been an eligible member in the original admission; a previously
unknown successor, an unreachable eligible member, pruned history or
conflicting local decisions still fail closed. Once an exact publication
intent is durable, it must complete before a subsequent catch-up;
retargeting cannot replace an in-progress publication. Same-epoch
membership changes are validated if a new proof is requested, but
automatic retarget discovery is driven by a newer epoch. This is not
independent-machine Minecraft gameplay certification.

### Exact certified local decision reconciliation (25 November)

Required validation 45519415707 on 2d45f97 completed successfully: all 20 selected jobs passed; optional sustained soak skipped.

Existing stale-replica publication now reconciles a local transfer record only if its full canonical encoding matches a signed phase in the verified committed transcript. A recovery or membership promise must match both the exact certified ballot/proposal and an included signed vote. Corrupt, trailing-byte, different or unrepresented records remain blocking. The matched decisions are removed only after exact target history and canonical records are durably installed, with the publication intent still present. The full verified history retains the signed evidence; crash recovery can revalidate and finish the remaining removals.

The signed replay regression covers incompatible and corrupt records, the three matching record types, an interrupted publication with the durable records still present, and successful final consumption. Storage library tests: 67 passed, 1 optional soak ignored. Strict storage/CLI Clippy and formatting passed. Retired local authority, sleep records, recovery reservations, pending decisions and unknown successor discovery still require separate safety work.

### Retired source replay after a planned transfer (25 November)

A former local authority can now discover and stage a known successor's anchored proof after its own exact, signed, committed outbound transfer. The proof must contain that precise immediate transition and the final authority must be a different accepted member. This path cannot stage for a crash-recovered old host with no local transfer. Isolated suffix downloads cannot install intermediate live state.

The publisher holds the same world-specific exclusive runtime lock as the migration supervisor and direct host, denies live or ambiguous persistent JVM records, clears the permit, and invalidates the local runtime verification. Storage independently checks the exact committed transfer at intent creation under its world transaction lock. The publication intent remains active through canonical writes and exact decision consumption. Restart repeats runtime exclusion before resuming.

The real process-level invite regression was extended to reconnect the original source after missing successor checkpoint bytes, observe isolated replay with its old epoch unchanged, then re-enter as an ordinary replica without creating local hosting consent. Both extended runs passed, including direct live-record and held-lock denial assertions (final run: 46.32 seconds). The injected live PID models a persistent orphan record; it is not a physical JVM crash test. Strict CLI/storage Clippy and 135 CLI library tests passed.

Crash-recovery retirement without a locally committed transfer, recovery reservations, signed sleep boundaries and previously unknown successor discovery remain open.
