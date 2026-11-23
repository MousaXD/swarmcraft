# Sabathon Agent 2 — Minecraft route/network P0 acceptance

**Date:** 2026-11-23

**Branch:** agent/sabathon-game-route-20261123
**Status:** Locally tested network functionality; **NOT production-playable yet.**

Every successful route-authority fixture in the current integration tests uses
deliberately **synthetic quorum and Fabric-readiness evidence**. The production
daemon does not yet inject a real FreshAuthoritySource or connect the gateway
to an actual Minecraft JVM. Network test success is not release certification.

## Validated network functionality and test boundaries

| Sabathon milestone | Focused test | What is actually established |
| --- | --- | --- |
| #2: Signed offers, exact World ID, epoch/fence, consent, runtime instance, replay and durable floor | game_route.rs unit tests; game_route_host.rs and game_host_signer_security.rs | Bad signatures, wrong worlds, expired or replayed transcripts, downgraded authority, unavailable quorum, revoked consent and unready JVM proof are refused. The quorum source is synthetic. |
| #2: Authenticated separate route-control channel | game_route_control_quic.rs; game_route_control_security.rs; game_route_rpc_mailbox.rs | Real encrypted peer authentication and bound offer/challenge RPC, bounded requests, refused unauthenticated peers, no successful route from a signature alone. The existing snapshot protocol is unchanged. |
| #3: Dedicated game transport | game_stream_transport.rs | Real local QUIC and TCP/Noise game-stream byte exchange, separate from snapshot replication. No actual Minecraft client/server. |
| #3: Stable loopback gateway and bounded forwarding | game_stream_loopback.rs; game_port_registry.rs; game_transport.rs | Real localhost-to-encrypted-QUIC-to-localhost forwarding, backpressure, consent withdrawal, stable per-world port, gateway restart and durable anti-rollback. |
| #3: No router port forwarding through relay | game_stream_relay.rs | A real locally hosted third-peer relay carries approximately 1 MiB **each way**. The relay is never a world member. It remains usable after the ten-second application-authentication deadline; a stopped relay revokes authentication and blocks new game streams. WAN NAT/CGNAT is still untested. |
| #2 + #3: Full signed route and game path over relay | game_network_resolver_relay.rs | The real host signer and production-shaped resolver exchange signed offers and challenges over a local encrypted relay, then forward TCP through the fixed localhost gateway. Simulated quorum withdrawal denies and fresh verified rejoin succeeds. |
| #12: Planned transfer and abrupt host crash | game_authority_handoff_relay.rs | Two distinct hosts and one player use a separate real relay. Planned fencing **and actual loss of original host network/runtime surrogates** close old streams; only the newly verified synthetic successor receives a fresh session through the same local port; rollback is refused. |
| Resource bounds and hostile input | input_hardening.rs; handshake_hardening.rs; game_route_control_security.rs; game_host_controller_quic.rs | Unauthenticated/malformed messages, overload and missing authority never grant world traffic; network-side request/stream/proof worker limits are exercised. |

## Required integration with Agent 1 (no edits to Agent 1 files)

1. Implement a real FreshAuthoritySource::verify_fresh(world, requester, nonce)
   that independently proves the **current accepted intersecting signed quorum
   history**, member roster, exact authority epoch/fencing, active hosting
   consent, compatible checkpoint and **authenticated live Fabric IPC** from
   the actual JVM. Bind world, realm, requester, supplied unpredictable nonce,
   runtime instance and current local Minecraft listener port. Refuse when
   majority is unreachable, history conflicts, runtime is stopped or any
   proof is unavailable/expired. Do **not** infer authority from persisted
   Ready status, configured network addresses or self-reported host flags.
2. Hosting daemon: create AuthenticatedGameRouteHostSigner from its actual
   application signing key, current transport peer and real authority source.
   Persist offer sequences in a private durable directory. Wrap the signer
   in BoundedGameRouteHostResponder, then GameRouteHostControlService using
   SwarmNode.take_game_route_control_events(). Keep SwarmNode.next_event()
   continuously polled in the owner loop. Return bounded reply actions there
   only after rechecking the CURRENT hosting consent/fencing state; never
   block that loop on quorum proofs, JVM IPC or disk fsync.
3. Hosting daemon: run GameStreamReceiver only for the currently authorized
   world's actual loopback Fabric TCP listener, with the same live evidence
   source and SwarmNode's authenticated peer registry. No publicly bound
   Minecraft listener or raw-IP game fallback may be introduced.
4. Player daemon: construct AuthenticatedGameRouteResolver with
   SwarmNode.game_route_rpc_client(), current authenticated peers,
   candidate-host IDs as **hints only**, a real FreshAuthoritySource and a
   private durable RouteVerifier generation floor. Start
   StableLoopbackGateway.bind_reserved(world, preferred_port, port_registry)
   using a per-world private durable GameWorldPortRegistry. Every new Minecraft
   connection must complete a NEW authenticated offer/challenge verification.
5. On new authority generation, loss of quorum, consent withdrawal or
   world switch, fence/stop old runtime and old-world receiver/gateway
   sessions. Only a freshly verified successor with an actually ready JVM
   accepts a NEW Minecraft login through the unchanged per-world localhost
   address. There is **no live TCP migration** and no guarantee of retaining
   uncheckpointed ticks or inventory.

Source-of-truth interface details are in GAME_ROUTE_INTEGRATION.md and
GAME_HOST_RESPONDER_HANDOFF.md. Do not edit Agent 1-owned authority_permit.rs,
migration.rs or server/runtime configurations from the Agent 2 branch.

## Local validation commands

Run at most **two Rust compile jobs**, and never overlap another agent's
heavy Rust build:

~~~sh
cargo fmt --all -- --check
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --offline --jobs 2 -- --test-threads=2
CARGO_BUILD_JOBS=2 cargo clippy -p swarm-network --lib --tests --offline --jobs 2 -- -D warnings
CARGO_BUILD_JOBS=2 cargo check -p swarm-cli --offline --jobs 2
~~~

The separate impaired-network and multi-GiB snapshot transfer soak tests are
**ignored by default**; ignored is not passed. The local relay tests configure
bounded 8 MiB/5-minute circuits because upstream libp2p relay-server defaults
of 128 KiB/120 seconds are insufficient for ordinary gameplay. Actual relay
operators must separately provision bounded but useful bandwidth, duration,
concurrency and uptime. They must not expose Minecraft directly to the public.

## Real-device and product integration still OPEN

- Three independently installed peers with actual signed majority, two-voter
  loss of quorum, conflicting histories, accepted authority fencing and
  hosting-consent revocation.
- The real Fabric Java server readiness exchange tied to live runtime,
  restore checkpoint and listening port; verified world markers, player
  inventory/items, committed state and any known uncommitted gameplay loss.
- At least two real Minecraft Java clients rejoining after planned transfer,
  host crash and recovered ready state on the same saved localhost address.
- Different physical networks/router/NAT/CGNAT paths with **no manual router
  port forwarding**, public relay availability, exhausted relay capacity,
  relay outage and recovery, blocked UDP and interrupted network sessions.
- Windows installer, real firewall permissions and private persistent data
  directories; prolonged gameplay, connection churn, bounded resource
  behavior and separately scheduled soak tests.

**Do not deploy, merge to main, or mark the three-peer playable P0 complete**
until Agent 1 provides real authority/Fabric integration and those physical
acceptance tests pass independently.
