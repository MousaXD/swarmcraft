# Minecraft client continuity after authority failover (Agent 06)

## Verified architecture boundary (main `badcbc6e73b42f37a4287c2c2e03e72e50e832ce`)

The `minecraft/fabric` artifact is **server-only** (`fabric.mod.json` has `"environment": "server"`). Its authenticated loopback IPC and authority-permit watchdog provide save barriers and server fencing; they do **not** run in the Minecraft client. Java multiplayer keeps the address with which the client connected. No first-party client-side Fabric entrypoint, persistent game proxy, tunnel, authenticated client-routing ticket or automatic multiplayer reconnect API is deployed.

SwarmCraft's authenticated QUIC/libp2p sessions are **control-plane/replication sessions**, not a Minecraft Java TCP tunnel. `RuntimeLaunchConfig.game_endpoint` and the copied `MigrationStatus.game_endpoint` are machine-local configured strings, not signed canonical connection endpoints. A local `MigrationStatus.phase=ready` can indicate Fabric readiness, not that a remote client can reach that address. The CLI world status may print the configured game endpoint for diagnostics; never automatically feed it into a Minecraft client.

## Narrow safe implementation in this PR

- The Desktop Host handoff panel provides an explicit **Minecraft client connection** outcome instead of conflating runtime readiness with client rejoin.
- Every client-facing outcome has `automaticReconnect=false` and `connectionTarget=null`. The unsafe `game_endpoint` field cannot become a Connect button or suggested destination.
- Ready report display is bound to the currently selected world ID, authority identity, epoch, fencing generation, snapshot identification and `runtime_ready=true`. These fields are **local report context, not independent cryptographic proof**. Even when complete, the player is told that manual reconnection is required and that no trusted client route has been published.
- Conflict, reduced/solo safety, blocked quorum, stale/superseded authority, failed recovery, sleep, unknown status and in-progress migration never show a positive reconnect state.
- Selecting a new world clears the prior migration result immediately, in addition to the existing generation-based asynchronous response guard. The status uses text content and accessible live updates.
- The Desktop migration-summary fallback no longer prints `Minecraft is ready at ${gameEndpoint}`, since that string was never verified for client access.

## What is NOT implemented or certified

This patch is a fail-closed UI and presentation correctness improvement, **not automatic Minecraft reconnection**. It creates no network sockets and changes no consensus or authority transition. No running Minecraft Java/Fabric client was connected and redirected here. No claim of accepted-world-state preservation under a live player session is made from the UI tests.

Automatic continuity requires a separately designed and audited transport/routing path, e.g. an authenticated local game proxy or explicitly installed client-side Fabric mod. That subsystem must consume only **signed canonical authority generations**, verify current membership and fencing against trusted backend state, tie the selected route to the world/epoch, account for actual Minecraft TCP reachability and NAT/relay limitations, reject stale or conflicting results, and fail closed on quorum unavailability. A DNS name or copied `game_endpoint` is not a substitute. Client version/modpack/session compatibility also needs validation.

## Real Minecraft acceptance outstanding (Agent 05 remains independent certifier)

On three independently installed peers and real Fabric clients, record pre-failover client play/inventory and a world marker, kill the source authority, establish surviving quorum, observe the restored canonical checkpoint on successor Java/Fabric, prove old host is fenced, reconnect each real Minecraft client to the **verified** successor, and verify post-failover state with logs/media and redacted evidence. Repeat with planned transfer, temporary link loss and unavailable quorum. Until that evidence exists, issue #85 is BLOCKED for seamless client continuity.

## Tests

`node --test apps/desktop/tests/client-continuity.test.mjs` checks fail-closed state and source-level UI wiring. Run the complete Desktop test suite and installed native acceptance on this exact PR head through the existing CI pipelines. These are **not** substitutes for real client reconnection evidence.
