# Optional Fabric client reconnect (#7)

The client-only SwarmCraft Reconnect mod is a separate, opt-in local
convenience component for Minecraft Java 26.1.2, Fabric Loader 0.19.3,
Java 25 and the supported Fabric API 0.155.2+26.1.2. It does not replace
the server lifecycle bridge or participate in consensus, route signing,
hosting consent or Minecraft login. The normal manual gateway reconnect
remains available without it.

## Build and local installation

From the repository root, with the compatible Gradle and Java toolchain:

```sh
cd minecraft/fabric
gradle --offline clientReconnectCheck clientReconnectJar build
```

The optional artifact is
`build/client-reconnect/swarmcraft-fabric-reconnect-0.5.1.jar`. The ordinary
server build remains in `build/libs/swarmcraft-fabric-0.5.1.jar`.
The client artifact is not included in the ordinary server artifact,
automatic runtime installer, update feed or release workflow.

1. Install the optional client JAR and the matching Fabric API in your
   deliberately selected Minecraft client instance's `mods` directory.
   Do not put this client artifact into a managed server's mods directory.
2. In SwarmCraft, select the existing private world and obtain its exact
   verified loopback gateway address. Use the ordinary Join current host
   action; do not infer the address from the world ID or old host's IP.
3. Create `config/swarmcraft-reconnect.json` inside that Minecraft instance.
   The following is an example only; replace every placeholder with your
   trusted local SwarmCraft executable, existing profile, world ID and
   verified gateway address.

```json
{
  "cli": "/absolute/path/to/swarmcraft",
  "data_dir": "/absolute/path/to/swarmcraft-profile",
  "world_id": "scworld:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "local_gateway": "127.0.0.1:31842",
  "auto_reconnect": false
}
```

Use the actual trusted native CLI executable (not a command string, shell
script from a server, download or PATH search). Windows users supply their
absolute executable/profile paths with JSON backslashes escaped. The
configuration is read once at client startup. Without a valid file, the
mod is disabled. Only literal `127.0.0.1:<port>` is supported; the vanilla
default port 25565 is explicitly refused because its resolver can perform
DNS SRV redirection. The configured port must match the daemon's current
verified gateway.

When `auto_reconnect` is false or omitted, a successful play session at
that exact loopback socket may expose a **Reconnect with SwarmCraft**
button on its disconnect screen. Clicking it is the one-shot opt-in.
Setting `auto_reconnect` to the JSON boolean `true` explicitly enables
automatic reconnect after an eligible disconnection; the client displays
a cancelable screen with a two-second grace period before its first
probe. Ordinary logout, unrelated servers, integrated singleplayer,
Realms and failed initial logins do not arm a reconnect. The operation
never persists a new opt-in or creates a new server-list entry.

## Safety and failure behavior

- Each attempt invokes only
  `<trusted-cli> --data-dir <profile> world game-route-status <world> --json`
  as a fixed argument vector. It does not select worlds, issue permits,
  change consent, authenticate as a new player or contact a raw host.
- The daemon must return a newly checked positive route for the exact
  pinned world, exact gateway, authority peer, nonzero generation/fence,
  checkpoint, fresh proof and unexpired lifetime, with all quorum,
  consent, runtime and route verification flags true. Strict bounded JSON,
  type checks, duplicate-key denial and a second expiry check immediately
  before dispatch protect the local handoff. No cached status is used.
- Three serial probes maximum, a 45-second monotonic overall window,
  two seconds between failed probes, and a 22-second per-probe ceiling.
  Output is bounded at 4096 bytes; timeout, nonzero exit or malformed
  output are refusals. The query runs off the Minecraft UI thread.
- Cancel, Escape, or navigation away invalidates the pending request and
  terminates its local query process. A late completion cannot start
  a connection. An accepted proof starts **one** fresh vanilla login to
  the same numeric loopback address. Failed login does not arm another
  automatic cycle; a fresh successful play session is required. The
  existing gateway independently re-verifies every game connection.
- The trusted local executable, profile, client instance and operating
  system remain the local trust boundary; this is not a defense against
  malicious same-user programs, another mod replacing vanilla connection
  behavior, or OS process stalls. The query-to-connect interval is not
  a distributed lease. The mod cannot promise preservation of unsaved
  game state, permission to join, or a successful successor login.

## Validation and remaining field gate

The local policy runner checks default-off and explicit automatic opt-in,
fixed command arguments, exact joined socket, strict fresh-proof decoding,
retry and deadline bounds, cancellation and one-shot dispatch. It launches
real disposable subprocesses for output limits, timeout, failure and
cancellation. Set `SWARMCRAFT_RECONNECT_TEST_CLI` to the locally built
native binary to additionally exercise an isolated profile without a
live daemon. The standalone client JAR compiles against the exact
Minecraft/Fabric libraries; it contains no server bridge or test code.

The 26 November 2026 local offline Gradle run completed 89 assertions,
including the real no-daemon negative query. The client JAR SHA-256 was
`9bcf72fe09527aec7e334359e520eae97acac594d4ed27ffdebfccfd15acd591`.
The server JAR retained its exact pre-change SHA-256
`86e639de3dcb6f88143fdf9988f50a653f17c3ac2d4887317d0b1beea4795cd6`.
Archive metadata and contents confirmed client/server isolation and no
packaged test classes. An initial offline test classpath unnecessarily
required the development launcher; the pure runner now uses the cached
compile classpath. A later cancellation test exposed a race reading its
PID marker before writing completed; atomic test marker publication
fixed that race. The final build, workflow policy, release version and
diff checks passed locally. No hosted CI, release publication, client
installation or authenticated game session was performed.

Before release, manually install and exercise this exact client artifact
in a disposable Minecraft instance with a real authenticated player. In
addition to normal manual reconnect, check opt-out, explicit automatic
opt-in, cancellation during the grace period and during an active query,
unrelated server and failed-login refusal, actual gateway replacement,
planned handoff and crash recovery, quorum loss, expired proof, old-host
stream closure and the saved world/inventory marker at the same stable
loopback port. Check readable screen layout, keyboard navigation and
small windows. Repeat on representative native client OSes and independent
networks. Local policy/build tests alone are not field acceptance or a
signed release of this optional component.

### Native gateway process gate (test-only)

To exercise the actual compiled reconnect policy and native daemon
before installing a licensed client, build the separate test probe:

```sh
cd minecraft/fabric
gradle --offline clientReconnectProbeClasspath runtimeChallengeCheck
cd ../..
export SWARMCRAFT_RECONNECT_PROBE_CLASSPATH="$(cat minecraft/fabric/build/client-reconnect-test/classpath.txt)"
export SWARMCRAFT_FABRIC_MOD_JAR="$PWD/minecraft/fabric/build/libs/swarmcraft-fabric-0.5.1.jar"
python3 scripts/e2e/sabathon_local_gateway_smoke.py --cli target/debug/swarmcraft
cargo test --locked --offline -p swarm-cli --test manual_transfer_process_gate \
  hosted_live::real_minecraft_gateway_handoff_and_recovery -- --ignored --nocapture --test-threads=1
```

The latter gate uses official runtime downloads and two disposable
managed server installations, plus a third player-only peer. It checks
the actual reconnect JAR's native subprocess and strict proof decoder
at each accepted authority epoch, then opens a new literal-loopback
Minecraft status socket. The sleep boundary must refuse another
connection. The separately packaged test probe is not a mod, a login
client or a release asset. The negative smoke additionally checks
absence of an unauthorized session attempt before and after daemon
restart. A Minecraft status response does not prove authenticated
game login, saved inventory, or independent physical-device acceptance.

The first positive lab attempt on 26 November installed both real
runtimes and established authenticated Fabric IPC, but failed before
the first positive route. The server thread was approximately 5892ms
behind, exceeding the unchanged Rust readiness RPC budget; controller
invalidation correctly stopped the old runtime. The server bridge now
permits just one queued readiness observation, with a 500ms terminal
correlated `RUNTIME_UNAVAILABLE` refusal if the game thread is stalled.
A timed-out slot remains occupied until its queued task drains; late
success cannot produce another response, and repeated challenges do
not accumulate work. A separate deadline executor leaves the controller
heartbeat watchdog independent. A responsive server thread must still
produce every positive reply with the existing permit/identity/port
checks, and genuine IPC timeout or malformed replies still invalidate
the authenticated Rust session. Neither the controller lease nor the
canonical route proof deadlines were extended.

The 113-assertion Java runtime-slot regression covers real timer/game
thread interleaving, duplicate/late reply refusal, bounded queueing,
and fresh recovery after drain. All eight Rust IPC tests pass, including
explicit correlated refusal followed by a new challenge and save
barrier, actual timeout invalidation and mismatched reply invalidation.
All 89 original client-policy assertions pass. The actual Java probe
refused all four no-daemon/unconsented/stopped/restarted scenarios,
without opening an unauthorized socket. The corrected local real-server run passed the first two positive stages:
initial authority epoch 1 and planned handoff to epoch 2, each with the
actual Java decoder and native live proof followed by a fresh real
Minecraft status exchange. The same permanent loopback address and
saved file marker were verified. After the deliberate successor crash,
the surviving majority certified epoch 3 and the player replica verified
the recovered checkpoint. The remaining epoch-3 status and safe-stop
checks are separate gates; no authenticated player login is claimed.
The exact rebuilt server SHA-256 was
`f57b0ed3a6101da742304aa0b9efd6ff57f65732588a7d9b814029291a34b073`;
the optional client JAR was byte-identical to the preceding build.
The test-only probe SHA-256 was
`4c42e4c0ced992a42f115eb8b7e44397a5bdbbabb5acee9fd173275fdb091aa0`.
