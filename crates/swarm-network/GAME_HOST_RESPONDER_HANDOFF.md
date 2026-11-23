# Agent 1 handoff: bounded authenticated game-route responder

The network crate exports game_route_host_service::BoundedGameRouteHostResponder.
It is a fail-closed async wrapper, **not** a quorum verifier or game-server
readiness oracle. Nothing serves Minecraft without a real FreshAuthoritySource.

## Required construction and network wiring

1. Implement a real Arc<dyn FreshAuthoritySource> that independently verifies
   current intersecting signed quorum, membership, consent, fencing, exact
   world/realm and live authenticated Fabric readiness for the supplied nonce.
   Never use migration status, cached Ready, configured IP/port or peer flags.
2. Create Arc<AuthenticatedGameRouteHostSigner> using the daemon's current
   application signing key and actual SwarmNode transport ID. Use a durable
   private per-user sequence directory, not a tempdir or world save.
3. Wrap the signer in BoundedGameRouteHostResponder::new(Arc::clone(&signer)).
4. Call SwarmNode::take_game_route_control_events() once. Continue polling
   SwarmNode::next_event() throughout all route checks and streaming.
5. On GameRouteControlEvent::Inbound, take the application and transport
   identities from that authenticated event (never from a request payload).
   Spawn an async task to call responder.answer(application_peer,
   transport_peer, request).await. Return its response channel and reply
   through a bounded channel to the mutable daemon network event loop;
   only that loop calls node.respond_game_route(channel, response).
6. On any signing/storage failure refuse or drop the request and log it.
   A Refused answer is always a denial, not a usable route.

The service caps host authority checks to **4** and replies within **3
seconds**, before the route-control RPC's 5-second deadline. A timed-out or
permanently hung source holds its worker permit until it actually finishes;
additional requests receive RateLimited, not a bypass. Timeouts, panics and
thread-spawn errors return Unavailable. Bounded detached OS threads prevent
an external synchronous verifier from blocking Tokio's network reactor or
runtime shutdown. The actual verifier should still bound its own remote calls.

## Related runtime enforcement

- Host accepts game streams only for its managed loopback Java listener.
  Player binds a durable, stable, per-world loopback gateway. No public
  raw-TCP fallback is available.
- On accepted-generation change, withdrawn consent or runtime shutdown,
  send shutdown through the existing watch channel to close old host and
  gateway sessions. Host and player verify live evidence independently before
  forwarding and on subsequent heartbeat ticks.
- New sessions repeat signed offer discovery, challenge/response, canonical
  quorum and Fabric readiness checks, and the durable generation floor.
  Reconnect is a new Minecraft TCP session, never a migrated old session.

## Focused verification

Run when the other agent's Rust build is idle, with at most two jobs:

    CARGO_BUILD_JOBS=2 cargo test -p swarm-network --lib game_route_host_service --offline --jobs 2
    CARGO_BUILD_JOBS=2 cargo clippy -p swarm-network --lib --offline --jobs 2 -- -D warnings

The tests use synthetic stalled/unavailable and synthetic ready proof sources.
They prove bounded refusal, worker-cap enforcement, slot recovery, a valid
signed offer/challenge path, one-use replay rejection and readiness withdrawal.
They do not prove real quorum or Java readiness. The daemon has not yet
connected this service to real host runtime authority.
