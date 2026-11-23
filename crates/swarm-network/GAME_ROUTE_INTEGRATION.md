# Sabathon Agent 2: authenticated Minecraft game route handoff

**Status:** Network crate transport, authenticated route-control request/reply, host-side signed-offer responder, a production-shaped client resolver and local synthetic-QUIC acceptance. **Not yet production-connected.** The daemon does not yet provide a certified canonical authority source or live Fabric runtime readiness adapter, and has not wired the responder/gateway into actual Minecraft play. Do not advertise the gateway as playable until those are integrated and tested.

## Trust boundary / Agent 1 integration contract

### Authenticated route-control channel (new)

The network crate also exports a separate libp2p protocol named
/swarmcraft/minecraft-route-control/1. It is NOT the legacy snapshot protocol
and does not grant canonical authority by itself. The two typed requests are
Offer (with WorldId) and Challenge (with the verifier-issued one-use challenge);
the replies are signed Offer, signed ChallengeResponse, or an explicit Refused
reason. A signed answer must still pass the independent RouteVerifier.

For daemon integration, call SwarmNode.take_game_route_control_events() once,
then poll SwarmNode.next_event() and drain the returned bounded receiver in
the same asynchronous event loop. Inbound events include authenticated
application/transport identity and a typed reply channel. The daemon must
independently validate membership, current canonical quorum, fencing,
hosting consent and live Fabric readiness before it signs either reply.
Use `SwarmNode::game_route_rpc_client()` for the concurrent player-side
async request mailbox (or `send_game_route_request()` on the mutable node)
and `respond_game_route()` on the host. The legacy exhaustive `NetworkEvent`
enum and snapshot wire discriminants are unchanged.
**The existing daemon does NOT subscribe and therefore refuses route control
requests; this new protocol is not production-integrated yet.**

The control protocol uses bounded 1024-byte requests, 4096-byte replies,
5-second request deadline, up to 16 concurrent control streams, 32 outstanding
outbound requests, 8 inbound requests per authenticated peer per 2 seconds,
and a 32-event queue. Late, unsolicited, wrong-world, misbound, stale-connection
and malformed replies cannot be promoted into verified game sessions.

The following Rust APIs are exported by `swarm_network`:

| API | Role |
| --- | --- |
| `game_route::MinecraftRouteOfferV1` | Domain-separated signed authority advertisement; includes exact world, realm, transport identity, generation, fence, quorum hash, runtime nonce, signed local Minecraft port, checkpoint, compatible server fingerprint, consent revision, expiry and offer sequence |
| `game_route::MinecraftRouteChallengeV1` / `MinecraftRouteResponseV1` | Bound, signed requester/application-identity and transport-identity challenge/response |
| `game_route::RouteVerifier` | Mints a non-forgeable `VerifiedGameRoute` only after independent current proof and persistent anti-rollback floor; never infer authority from a signed offer alone |
| `game_route::FreshAuthoritySource` | **Unimplemented production trust adapter**. The daemon/Agent 1 must provide fresh, independently verified canonical quorum, current membership, conflict absence, fencing, consent, and live authenticated Fabric runtime readiness **for the supplied nonce** |
| `game_route_host::AuthenticatedGameRouteHostSigner` | Host-side responder that signs offers and one-use challenge responses only after injected fresh authority evidence proves exact requester membership, current host identity, quorum, fencing, consent and live server readiness |
| `game_route_host_service::BoundedGameRouteHostResponder` | Performs host-side proof signing away from the async network reactor, with four bounded workers and a three-second deadline |
| `game_route_host_controller::GameRouteHostControlService` | Owns bounded authenticated inbound route replies and shutdown lifecycle, handing `GameRouteHostReplyAction` back to the `SwarmNode` owner |
| `game_route_host::DurableHostOfferSequencer` | Monotonic per-world offer sequencing retained across process restarts, with private storage and atomic replacement; malformed or rolled-back state is refused |
| `game_transport::VerifiedForwardRoute` | Ties a verified route capability to the exact signed offer and signed response |
| `game_route_resolver::AuthenticatedGameRouteResolver` | Working player-side authenticated libp2p offer/challenge RPC and strict `RouteVerifier` flow. Requires the daemon's REAL `FreshAuthoritySource` and per-world candidate source to serve users; never trusts the candidate list as authority |
| `game_route_resolver::GameRouteCandidateSource` | Provides current possible transport peer IDs (hints only, capped at eight); actual authenticated identity, world, quorum, consent and readiness are reverified for every join |
| `game_transport::GatewayRouteResolver` | Async trait implemented by `AuthenticatedGameRouteResolver`; called independently on each new local TCP connection |
| `SwarmNode::game_route_rpc_client()` | Bounded cloneable request mailbox for gateway resolver while the daemon continues polling `next_event()` |
| `game_transport::GameStreamReceiver` | Host-only inbound libp2p game stream to its own `127.0.0.1:<managed_server_port>`; independently rechecks membership, quorum, live runtime/consent and persists one-use signed challenges before admission |
| `game_stream_replay::DurableGameStreamReplayLedger` | Private per-world atomic signed-challenge replay ledger; restarts cannot reset one-use admission while the same signed response remains valid; bounded to 256 unexpired entries and fails closed on corrupt storage |
| `game_transport::GameStreamClient` | Player gateway opens `/swarmcraft/minecraft-game/1` to the route's authenticated transport peer |
| `game_transport::StableLoopbackGateway` | Binds one stable per-world localhost TCP port (IPv4 127.0.0.1 only), refuses wrong world or missing proof; requires a fresh resolver result per connection |
| `game_port_registry::GameWorldPortRegistry` | Append-only durable port reservation per world, with a cross-process lock and immutable assignments. Rejects corrupt storage and collisions rather than silently changing the saved client address |
| `SwarmNode::game_stream_control()` | Cloneable libp2p-stream transport control |
| `SwarmNode::game_authenticated_peers()` | Live transport-to-application identity registry, set only by the existing PeerHello proof and revoked on disconnection |

### Agent 1 must supply (without editing network-owned code)

1. A read-only **canonical authority verifier** that checks accepted and *intersecting signed quorum history* for the exact world/realm/epoch/fence, signed membership chain, active host membership, configured server compatibility and accepted checkpoint. It must fail closed if a current majority is unreachable, if accepted history conflicts, if V2 activation cannot be proved, or if an authority is fenced. A two-voter world with one missing voter must not be served.
2. An **authenticated Fabric IPC readiness challenge** on the **current** authority's **actual** server process. Bind the caller-supplied nonce, exact world ID, realm, accepted epoch/fence, current compatibility, runtime instance nonce, **live local listening port**, and unwithdrawn hosting consent. Reusing the unsigned persisted migration `Ready`, `RuntimeLaunchConfig.game_endpoint`, or an unauthenticated ping is forbidden.
3. A `FreshAuthoritySource::verify_fresh(world, requesting_member, nonce)` adapter that returns `FreshAuthorityEvidence` only after both checks pass and sets `verified_at_unix_ms` / `valid_until_unix_ms` from **actual live evidence**. All returned booleans MUST be derived from verified evidence, not caller flags. This method is invoked per session and every second on live connections: it must be **fast and bounded**, or a separately supervised time-limited adapter is required. It is not currently implemented.
4. Wire the host-side authenticated route receiver into the daemon with `GameRouteHostControlService` (see `GAME_HOST_RESPONDER_HANDOFF.md`). Create `Arc::new(AuthenticatedGameRouteHostSigner::new(...))`, wrap it with `Arc::new(BoundedGameRouteHostResponder::new(signer))`, and construct the controller using the one-time `SwarmNode::take_game_route_control_events()` receiver. Run the controller as a daemon-owned task and return its `GameRouteHostReplyAction` values over a bounded channel (capacity at most 16). **Only the network event loop** calls `SwarmNode::respond_game_route(action.channel, action.response)`, after rechecking current world, consent, fence and shutdown. Continue polling `SwarmNode::next_event()` while proof work runs; blocking quorum/Fabric checks or fsync inside that loop can deadlock authenticated traffic. The controller owns and stops its reply tasks; the responder allows four blocking proof workers and refuses requests that exceed its three-second deadline. The signer independently verifies authority, persists a new offer sequence, and refuses expired/fenced/unready requests. Never interpret a refusal as success. Keep its durable private host-sequencer directory outside tempdirs or Minecraft saves. Do not add snapshot wire variants or invent unsigned success.
5. On a new accepted generation or revoked consent, stop the old runtime and notify host receiver and player gateway via their `watch<bool>` shutdown channel. Never promote a successor until the new generation, quorum and actual restored Fabric runtime are independently proved; restart only with a fresh route. Keep the generation floor directory **outside ephemeral/test directories**, retained across restarts.

The signed `game_server_port` is allowed only as a **loopback** destination on the verified host and must equal the actual live Fabric listener port independently returned by the canonical runtime adapter. No remote raw-IP or DNS fallback exists.

## Minimal wiring sequence

1. Start a `SwarmNode` on each peer and **continue polling `next_event()`** while game tasks run. Completion of `NetworkEvent::Authenticated` alone does not guarantee membership.
2. On the hosting peer, use Agent 1's live runtime adapter, construct the signer and bounded responder, then run `GameRouteHostControlService::new(responder, inbound_events).run(reply_tx, shutdown_rx)`. Drain reply actions from the daemon's continuously polled network event loop and call `node.respond_game_route(...)` only while the exact authority is still current. Independently start `GameStreamReceiver::new(world_id, local_transport_id, managed_port, authority_source, node.game_authenticated_peers(), private_durable_host_replay_dir)?.serve(node.game_stream_control(), stop_rx)`. The sixth argument is **mandatory private durable per-host storage** that survives receiver and daemon restarts; a temporary directory is forbidden. No control or game-stream service can authorize traffic from a cached Ready state.
3. On the playing peer, instantiate `AuthenticatedGameRouteResolver::new(local_application_id, local_transport_id, node.game_route_rpc_client(), RouteVerifier::new(private_durable_generation_dir), node.game_authenticated_peers(), real_authority_source, candidate_hosts)`. This already fetches a signed offer from an authenticated transport peer, issues a fresh one-time challenge, checks the signed response and invokes `RouteVerifier::verify` with the daemon's **independently established current authority**. The candidate source supplies only possible authenticated transport peers; the resolver tolerates stale candidates but never accepts an unverified successor. The network driver MUST be continuously polled. Never accept a cached endpoint, offer signature or configured IP as authorization.
4. Open `GameWorldPortRegistry::new(private_durable_user_data_dir)` and call `StableLoopbackGateway::bind_reserved(world_id, preferred_port, &registry)`. The port assignment is immutable across restarts and cannot overlap another registered world. Use a **private, non-group-writable** directory on Unix (e.g. mode 0700); corrupted or shared-writable storage is refused. A busy or corrupted saved port fails closed; never choose a different port without explicit user remediation. Construct `GameStreamClient::new(local_application_id,local_transport_id,node.game_authenticated_peers(),authority_source)`, then pass resolver, client, stream control and a mandatory shutdown watch to `gateway.serve(...)`.
5. Give the Java client **only** `127.0.0.1:<stable_port>` to join. For a transfer/crash, close old sessions, verify the new authority and real server again, then permit a **new Minecraft login** through that *same local port*. There is **no live TCP session migration**, automatic Fabric client reconnect module, or guarantee of uncheckpointed inventory/tick preservation.

The test `tests/game_stream_loopback.rs` illustrates the complete network wiring, but its `SyntheticQuorumAndRuntime` deliberately supplies fake trust evidence. Never copy it into production. The test `tests/game_stream_transport.rs` exercises real libp2p substream isolation without purporting to prove Java gameplay.

## Backpressure, isolation and fail-closed behavior

- **Relay path:** A known relay/bootstrap infrastructure connection is not an authenticated world member and never enters the game peer registry. libp2p's relay-client listener dials the relay for its reservation; issuing a second explicit dial at the same time can invalidate the reservation, so the configured relay listener owns that dial. Players use an authenticated libp2p circuit with no Minecraft raw-IP/port-forwarding fallback. A local integration test exercises a real third-peer relay. Default relay-server circuit budgets (128 KiB and 120 seconds in the tested libp2p revision) are too short for gameplay; any managed relay must explicitly provision **bounded but usable** circuit bytes, concurrency, bandwidth and duration. The network crate does not deploy or operate that infrastructure, and real WAN/NAT/CGNAT behavior remains untested.
- Candidate authority IDs are discovery hints, never signed permission. The resolver probes no more than eight authenticated candidates concurrently; a connected but unresponsive old host does not block a freshly verified successor. Cancelled asynchronous route requests are removed from the pending admission budget on the next dispatch. Only a fresh, independent quorum and authenticated runtime proof can select the successful candidate.
- **Bounded blocking proofs:** The client route resolver supports eight proof workers, the host's `BoundedGameRouteHostResponder` supports four, and each `GameStreamClient` / `GameStreamReceiver` supports eight live-stream proof workers. Each uses a three-second deadline and runs synchronous quorum/Fabric checks off the Tokio reactor. A timed-out blocking function cannot be killed safely, so its semaphore slot stays held until it exits; exhaustion refuses the session without spawning unbounded threads. After any awaited proof, the game transport rechecks signed expiry, authentication, shutdown and the verified route before forwarding. The real external authority adapter must still enforce its own I/O deadlines and never turn a timeout into a Ready/proof success.
- Dedicated `/swarmcraft/minecraft-game/1` libp2p substreams; snapshot `WireRequest`/replication traffic is not used for Minecraft payload bytes.
- At most 16 active streams per gateway/host receiver and 4 per remote host-side transport identity. Inbound stream open is bounded to 4096 bytes with a 4-second handshake timeout; server connection to 127.0.0.1 is capped at 3 seconds. A session has a 4-hour maximum lifetime.
- **Durable one-use stream replay:** The signed response challenge nonce is atomically persisted per world before admitting game traffic, with a cross-process lock and at most 256 unexpired entries. Corrupt state, unsafe/symlinked storage, concurrent reuse and any disk persistence failure block the session. Host process restarts do not reset consumed challenges. Disk I/O runs off the async reactor under a bounded worker/deadline. The independently signed route, quorum, membership and runtime evidence remain mandatory; the replay ledger alone never grants access. Existing streams are checked every second and closed when fresh canonical evidence, consent, runtime readiness or application authentication disappears.
- Gateway and host receiver session tasks are owned by their parent `JoinSet`; cancelling, aborting, or dropping either supervisor also aborts every child game stream. A switched-world task cannot leave a detached old-world TCP session alive.
- TCP copy state is kept **pinned across heartbeat ticks**, preserving backpressure without dropping buffered game bytes.
- The signed route's durable generation floor updates by atomic sibling replacement: Unix rename or Windows `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`. No temporary removal of the old floor and no success if persistence fails.
- Server binds **only loopback**; no public Minecraft listener, DNS endpoint, port forwarding, untrusted successor, or raw TCP proxy fallback.
- Durable generation floors, host offer sequences, and per-world gateway port registries reject writable or symlinked storage ancestors on Unix (except root-owned sticky temporary roots such as /tmp). A private leaf under a replaceable parent is not durable anti-rollback state. Use a dedicated private application-data path, never a group-writable repository or world save.
- Existing saved generation, host-sequence, gateway-port and signed-challenge replay files (including their cross-process lock files) must be regular, owned by the effective process user, non-hard-linked, and not group/world-writable on Unix. A permissive existing file refuses routing instead of allowing another local account to replace, corrupt or roll back durable authorization evidence.
- Unsupported or unproven realm transitions fail closed; V2 rollout needs explicit versioned route transcript review. Production quorum and IPC adapters have not been shipped by Agent 2.

## Verification performed and not performed

Locally available focused checks (run with at most two jobs, never concurrently with another agent's heavy Rust build):

```sh
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --lib --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_stream_loopback --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_authority_handoff --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_network_resolver --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_host_signer_security --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_route_control_quic --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_route_control_security --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_stream_transport --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_stream_relay --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_host_controller_quic --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_authority_handoff_relay --offline
CARGO_BUILD_JOBS=2 cargo clippy -p swarm-network --lib --tests --offline -- -D warnings
```

The synthetic acceptance covers two real encrypted/authenticated QUIC peers, a 1 MiB loopback TCP byte-for-byte round trip across authority heartbeat ticks, active-stream consent withdrawal, denial of new joins, new session on recovered consent, hard-cancelled gateway and hosting receiver supervisors, and restart of the gateway/listener at the **same** per-world local port against the persisted floor. It **does not** establish current real quorum, real signed Fabric IPC readiness or actual Minecraft protocol/world semantics.

The additional synthetic three-peer handoff test uses a real encrypted QUIC connection to each of two distinct hosts and tagged loopback TCP servers: it proves that old-host sessions close when authority changes, an unready new host receives no Minecraft traffic, a ready successor accepts a fresh session through the unchanged gateway address, and the persisted generation floor blocks a rollback to the old authority. The network resolver test exercises **real host-side signed offers and one-time responses over authenticated QUIC, the production-shaped client resolver, loopback gateway, dedicated game stream and local TCP server**, including deliberate quorum withdrawal and a new TCP session through the same port. The host signer tests exercise requester binding, refused quorum/conflict/consent/readiness, one-time challenges and sequence recovery. Their quorum and Fabric sources remain **test-only**; none replace actual majority-backed recovery or an authenticated Fabric server.

**Still required on independent machines:** three installed peers with real signed majority/fencing, real Fabric Java server and 2+ installed Minecraft clients, planned authority transfer, crash recovery, blocked two-voter quorum loss, exact accepted checkpoint/world markers, off-LAN NAT/hole punching/relay and relay outage behavior, actual Windows installer/firewall integration, long-running churn and interruption tests. Record exact SHAs and independently verify saved world state and any uncommitted gameplay loss. Keep P0 networking acceptance open until those tests pass.
