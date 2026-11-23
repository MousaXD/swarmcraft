# Sabathon Agent 4 — Desktop + real Minecraft field runbook

**State as of 23 November 2026: NOT CERTIFIED.** This is a field operator
procedure, not a test result. Existing `run.py verify` and the independent
media reviewer remain the certification gate. A verified route status, a
successful local TCP connection, an installed package, and actual Minecraft
gameplay are different observations; none can stand in for another.

## Before installing on Alice, Bob and Carol

1. Record `git rev-parse HEAD`, `git rev-parse HEAD^{tree}`, exact installer
   SHA-256, installation type, OS/architecture and test binary provenance for
   **each of three genuinely separate machines**. Never use three profiles on
   one workstation as the positive field test. Preserve the old installer for
   rollback, keep secrets and private addresses out of published evidence.
2. Independently launch each player's legitimate Minecraft Java client with
   the **exact signed world Minecraft version**, matching Fabric loader and
   compatible mods. The current P0 reference is Minecraft `26.1.2`, Fabric
   Loader `0.19.3` or compatible and Java 25; verify from each actual client,
   not merely the server install. Do not accept the Minecraft EULA on a
   participant's behalf.
3. Confirm each installed SwarmCraft Desktop starts using real sidecars, loads
   its persistent identity, and shows the right world and **distinct** safety
   and replication states. Do not use a QA mock or synthetic verifier for the
   gameplay verdict. Windows: installed WebView2/msedgedriver path; macOS:
   native Accessibility/XCUITest/approved QA-only native harness, *not*
   unsupported `tauri-driver` WKWebView. Keep the QA-only harness out of shipping
   artifacts and retain the actual package checksums.
4. Create a dedicated signed world on Alice. Invite Bob and Carol; verify all
   three accepted identities and the canonical three-voter head. Alice and Bob
   explicitly opt into hosting with a confirmed heap allocation. Carol remains
   player-only (`0 MiB`), with identity-bound persisted withdrawal/consent.
5. On the serving machine independently inspect the managed Minecraft process
   and `server.properties`: `server-ip=127.0.0.1`, distinct reserved localhost
   game port, explicit reviewed `online-mode` and allowlist policy,
   `enable-rcon=false`. Confirm no public Minecraft listener, inbound router
   forwarding or direct remote Minecraft endpoint is involved. If the runtime
   cannot enforce this, STOP the field run.

## Record independent checkpoints before saying “ready”

At each stage record the world ID, accepted canonical epoch and fence,
authority, accepted snapshot hash, membership quorum, local-host consent and
revision, exact live Fabric instance nonce, route proof outcome, route
generation and local gateway. Preserve original redacted commands/logs and
time-aligned videos, not a manually filled green checkbox.

| Checkpoint | Minimum genuine observation | Not a substitute |
| --- | --- | --- |
| Save present | World lists a stored snapshot | Latest accepted head or successful verification |
| Save verified | CLI local integrity verification for **that exact checkpoint** | A recent snapshot count or unsigned migration report |
| Authority authorized | Fresh intersecting signed canonical majority, membership, no conflict and current fence | A candidate host, visible peer, or `Canonical` display label |
| Hosting consent | Active local identity-bound consent with sufficient heap | A UI toggle or remembered preference |
| Server ready | Fresh nonce-bound authenticated Fabric IPC on the real JVM | A cached `runtime_ready`, stale PID or open TCP port |
| Game route verified | Fresh signed host offer/challenge, current authority proof, reserved localhost gateway and authenticated game stream | Generic NAT/relay connectivity or `game_endpoint` |
| Client actually joined | Minecraft client visibly joins **through that same loopback address** and reads the saved in-game marker/inventory | A listening port, local probe or recorded `ready` status |

## Positive and negative sequence

1. **Baseline:** With Alice hosting, Bob and Carol join by entering the
   approved `127.0.0.1:<reserved-port>` on each **player's own computer**.
   Record each local port and accepted generation. Place an unmistakable
   marker in-game; independently record inventory/player position, time and
   pre-transfer checkpoint. Review actual user sessions in video. A copied
   address is not gameplay evidence.
2. **Planned transfer:** Save and accept the checkpoint, execute the actual
   signed transfer from Alice to Bob with the third voter alive, and observe
   Alice's old game stream close and server become fenced/stopped. Wait for
   the successor's independently verified quorum, consent, Fabric readiness
   and signed game route. Old/cached routes must remain blocked throughout
   the transition. Bob and Alice reconnect with fresh Minecraft sessions at
   their *unchanged per-world localhost ports*. Verify the same marker and
   accepted inventory, and record lost progress separately if any.
3. **Crash recovery:** Record authorized crash approval on Alice's real
   machine using the existing guarded `run.py` workflow. Kill/isolate only
   the approved test-world authority process using its exact observed PID;
   no global kill command. Two surviving voters must authorize exactly one
   replacement. Rejoin via each client's **same local gateway** and verify
   which accepted checkpoint was restored. Restart Alice and show that the
   old authority cannot resume gameplay or write stale history. The first
   post-crash capture may already find Bob's *newly fenced* signed route;
   accept that only if the epoch and fence advanced, the authority is Bob,
   and the same local gateway has a fresh proof. Alice's old route must fail.
4. **Quorum negative:** With only two total voters, isolate one. The surviving
   peer must refuse replacement hosting and new Minecraft sessions even if
   it has a byte-perfect stored copy or the old migration record says Ready.
   Rejoin only after the required legitimate authority recovery.
5. **Consent negative:** Carol's zero-heap profile can join as a player but
   cannot start the JVM or become a successor. Withdraw an active host's
   consent with the current signed revision, observe its safe checkpoint,
   fence and stop/transfer, and verify a restart does not re-enable hosting.
6. **Network negative:** Repeat player reconnect on real separate networks,
   recording direct/hole-punched versus relay **for game streams separately
   from snapshot replication**. Force one approved relay outage and verify
   bounded refusal with no raw-IP fallback. Do not claim WAN readiness from
   LAN, synthetic local QUIC, or SSH connectivity alone.
7. **Desktop negative:** Switch selected worlds during in-flight status
   requests; verify no prior world's route or generation appears. Stop the
   daemon, corrupt the test-only route proof, expire a proof, or withdraw
   consent; Reconnect instructions must disable immediately on refresh and
   after any five-second proof expiry. Desktop must never turn a cached
   migration Ready or configured IP into a trusted game address.

## Verdict

Run the existing `run.py` evidence collection, provide all required media,
produce an independently signed/identified review, and invoke the existing
strict verifier with the exact immutable artifacts. Record **BLOCKED** for
missing physical hardware, missing fresh Fabric/quorum gateway integration,
failed login, absent signed checkpoint, unavailable real WAN path, or
unreviewed media. Unit tests and synthetic network tests are useful
engineering evidence but **cannot** certify this real three-client journey.

Integration dependency: Agent 3 must expose a `world game-route-status
<world> --json` read-only CLI sourced from Agent 1's independent authority
adapter. Agent 4's native command intentionally returns an error until that
production path exists. The schema is in
`apps/desktop/src-tauri/src/game_route_commands.rs`; the WebView may never
manufacture it from `migration-status.json`.

The `node.py` field probe now records only an allowlisted, world-bound
summary of that fresh native CLI response; it never publishes the signed
proof or remote host endpoint. `run.py verify` requires this report on the
checkpoint, recovered, returned and consent-withdrawal stages, and rejects a
verified old route immediately after the observed crash. Each participant's
localhost port must remain unchanged across authority transitions. Missing
CLI support, timeouts and unverifiable route reports yield **BLOCKED**, not a
fallback to the old configured `game_endpoint`.
At stable playable stages, the current host's signed manifest hash must also
match the snapshot hash in the game-route proof; identical but obsolete
checkpoints on several clients are insufficient. Nonhosting replicas may
legitimately omit a migration-status snapshot hash, but still need matching
fresh world/authority/fencing evidence and their independently verified
canonical heads.
Each stage captures its own completion time after the sequential remote
observations. The route-probe observation timestamp must fall within the
bounded three-machine SSH collection window (330 seconds plus 2 seconds
for clock skew), and strictly advance across playable stages. An old field
run's route report cannot be made fresh just by changing the stage label.
