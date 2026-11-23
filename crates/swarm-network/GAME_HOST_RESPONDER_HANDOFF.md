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
- Create the host GameStreamReceiver with its new mandatory sixth argument:
  a private durable host replay-ledger directory. Keep that directory across
  daemon, gateway and world restarts. It persists signed stream challenge
  consumption before accepting Minecraft bytes, so replayed authorizations
  cannot regain access simply by restarting the host gateway. Do not put
  it in a tempdir or imported world save; corrupt or unsafe storage fails closed.
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

## Optional host event-loop adapter (implemented in network crate)

The network crate now exports GameRouteHostControlService and
GameRouteHostReplyAction from game_route_host_controller. Prefer this
controller over manually spawning signer tasks in daemon.rs:

1. Take the one-time authenticated inbound receiver from
   SwarmNode::take_game_route_control_events(). Construct the controller
   with Arc<BoundedGameRouteHostResponder> and that receiver.
2. Construct an mpsc::channel<GameRouteHostReplyAction> with capacity
   MAX_PENDING_HOST_ROUTE_ACTIONS (16), and run controller.run(sender,
   host_shutdown_rx) as a daemon-owned async task. An oversized reply
   channel is rejected rather than accidentally accumulating signed offers.
3. Keep SwarmNode::next_event() polled. In the same mutable-owner loop,
   select on its normal events and the receiver of GameRouteHostReplyAction.
   Recheck the CURRENT hosting consent/fence/shutdown state immediately
   before calling node.respond_game_route(action.channel, action.response).
   Drop any queued action that became stale during a world switch.
4. On stop, signal the host shutdown watch before stopping or replacing the
   game receiver, and cancel/await the controller task. Its JoinSet owns all
   concurrent async signing attempts; a superseded world cannot retain
   detached authority-response tasks. The lower signer may still finish a
   bounded blocking OS worker but cannot send an answer after shutdown.

This adapter accepts only the authenticated application and transport peer
identities delivered by SwarmNode. It limits in-flight replies to 16 and
individual blocking proofs to the responder's 4 worker slots / 3-second
deadline. Queue saturation, malformed requests and missing canonical
evidence never become an authorization capability.

Integration test (real local authenticated QUIC, deliberately unavailable
authority and no real Minecraft runtime):

    CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_host_controller_quic --offline --jobs 2
