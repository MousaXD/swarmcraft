# Sabathon Agent 2: authenticated Minecraft game route handoff

**Status:** Network crate implementation and local synthetic-QUIC acceptance. **Not yet production-connected.** The daemon does not yet provide a certified canonical authority source, live Fabric runtime readiness adapter, or production offer/challenge exchange. Do not advertise the Minecraft gateway as playable until those are integrated and tested.

## Trust boundary / Agent 1 integration contract

The following Rust APIs are exported by `swarm_network`:

| API | Role |
| --- | --- |
| `game_route::MinecraftRouteOfferV1` | Domain-separated signed authority advertisement; includes exact world, realm, transport identity, generation, fence, quorum hash, runtime nonce, signed local Minecraft port, checkpoint, compatible server fingerprint, consent revision, expiry and offer sequence |
| `game_route::MinecraftRouteChallengeV1` / `MinecraftRouteResponseV1` | Bound, signed requester/application-identity and transport-identity challenge/response |
| `game_route::RouteVerifier` | Mints a non-forgeable `VerifiedGameRoute` only after independent current proof and persistent anti-rollback floor; never infer authority from a signed offer alone |
| `game_route::FreshAuthoritySource` | **Unimplemented production trust adapter**. The daemon/Agent 1 must provide fresh, independently verified canonical quorum, current membership, conflict absence, fencing, consent, and live authenticated Fabric runtime readiness **for the supplied nonce** |
| `game_transport::VerifiedForwardRoute` | Ties a verified route capability to the exact signed offer and signed response |
| `game_transport::GatewayRouteResolver` | **Unimplemented production control-plane exchange**. Fetch offer/challenge/response over a currently authenticated SwarmCraft connection and call `RouteVerifier::verify` on *every* new join |
| `game_transport::GameStreamReceiver` | Host-only inbound libp2p game stream to its own `127.0.0.1:<managed_server_port>`; independently rechecks membership, quorum, live runtime/consent |
| `game_transport::GameStreamClient` | Player gateway opens `/swarmcraft/minecraft-game/1` to the route's authenticated transport peer |
| `game_transport::StableLoopbackGateway` | Binds one stable per-world localhost TCP port (IPv4 127.0.0.1 only), refuses wrong world or missing proof; requires a fresh resolver result per connection |
| `game_port_registry::GameWorldPortRegistry` | Append-only durable port reservation per world, with a cross-process lock and immutable assignments. Rejects corrupt storage and collisions rather than silently changing the saved client address |
| `SwarmNode::game_stream_control()` | Cloneable libp2p-stream transport control |
| `SwarmNode::game_authenticated_peers()` | Live transport-to-application identity registry, set only by the existing PeerHello proof and revoked on disconnection |

### Agent 1 must supply (without editing network-owned code)

1. A read-only **canonical authority verifier** that checks accepted and *intersecting signed quorum history* for the exact world/realm/epoch/fence, signed membership chain, active host membership, configured server compatibility and accepted checkpoint. It must fail closed if a current majority is unreachable, if accepted history conflicts, if V2 activation cannot be proved, or if an authority is fenced. A two-voter world with one missing voter must not be served.
2. An **authenticated Fabric IPC readiness challenge** on the **current** authority's **actual** server process. Bind the caller-supplied nonce, exact world ID, realm, accepted epoch/fence, current compatibility, runtime instance nonce, **live local listening port**, and unwithdrawn hosting consent. Reusing the unsigned persisted migration `Ready`, `RuntimeLaunchConfig.game_endpoint`, or an unauthenticated ping is forbidden.
3. A `FreshAuthoritySource::verify_fresh(world, requesting_member, nonce)` adapter that returns `FreshAuthorityEvidence` only after both checks pass and sets `verified_at_unix_ms` / `valid_until_unix_ms` from **actual live evidence**. All returned booleans MUST be derived from verified evidence, not caller flags. This method is invoked per session and every second on live connections: it must be **fast and bounded**, or a separately supervised time-limited adapter is required. It is not currently implemented.
4. An authenticated host response handler that signs `MinecraftRouteOfferV1` and `MinecraftRouteResponseV1` **only** while step (1)-(2) holds. The response must echo the gateway's exact one-time `MinecraftRouteChallengeV1`. Build a compatible append-only request/response wire extension in coordination with daemon RPC handlers and preserve old postcard discriminants. The network crate intentionally does **not** invent an unsigned response or a production resolver.
5. On a new accepted generation or revoked consent, stop the old runtime and notify host receiver and player gateway via their `watch<bool>` shutdown channel. Never promote a successor until the new generation, quorum and actual restored Fabric runtime are independently proved; restart only with a fresh route. Keep the generation floor directory **outside ephemeral/test directories**, retained across restarts.

The signed `game_server_port` is allowed only as a **loopback** destination on the verified host and must equal the actual live Fabric listener port independently returned by the canonical runtime adapter. No remote raw-IP or DNS fallback exists.

## Minimal wiring sequence

1. Start a `SwarmNode` on each peer and **continue polling `next_event()`** while game tasks run. Completion of `NetworkEvent::Authenticated` alone does not guarantee membership.
2. On the hosting peer, make the listener's live world/port available through Agent 1's verified runtime adapter, then start `GameStreamReceiver::new(world_id, local_transport_id, managed_port, authority_source, node.game_authenticated_peers())?.serve(node.game_stream_control(), stop_rx)`. This registers the dedicated game-stream protocol.
3. On the playing peer, implement `GatewayRouteResolver::resolve(world_id)`: fetch a signed offer via an authenticated control request; issue a unique `RouteVerifier::issue_challenge(offer, local_application_id, local_transport_id)`; transmit challenge to that authenticated host; verify signed response and **fresh independently established authority** via `RouteVerifier::verify`; construct `VerifiedForwardRoute::new` only after success. Never accept a route by merely matching a cached endpoint or a valid signature.
4. Open `GameWorldPortRegistry::new(private_durable_user_data_dir)` and call `StableLoopbackGateway::bind_reserved(world_id, preferred_port, &registry)`. The port assignment is immutable across restarts and cannot overlap another registered world. A busy or corrupted saved port fails closed; never choose a different port without explicit user remediation. Construct `GameStreamClient::new(local_application_id,local_transport_id,node.game_authenticated_peers(),authority_source)`, then pass resolver, client, stream control and a mandatory shutdown watch to `gateway.serve(...)`.
5. Give the Java client **only** `127.0.0.1:<stable_port>` to join. For a transfer/crash, close old sessions, verify the new authority and real server again, then permit a **new Minecraft login** through that *same local port*. There is **no live TCP session migration**, automatic Fabric client reconnect module, or guarantee of uncheckpointed inventory/tick preservation.

The test `tests/game_stream_loopback.rs` illustrates the complete network wiring, but its `SyntheticQuorumAndRuntime` deliberately supplies fake trust evidence. Never copy it into production. The test `tests/game_stream_transport.rs` exercises real libp2p substream isolation without purporting to prove Java gameplay.

## Backpressure, isolation and fail-closed behavior

- Dedicated `/swarmcraft/minecraft-game/1` libp2p substreams; snapshot `WireRequest`/replication traffic is not used for Minecraft payload bytes.
- At most 16 active streams per gateway/host receiver and 4 per remote host-side transport identity. Inbound stream open is bounded to 4096 bytes with a 4-second handshake timeout; server connection to 127.0.0.1 is capped at 3 seconds. A session has a 4-hour maximum lifetime.
- A one-time random stream nonce cache is bounded (256). Unknown or unauthenticated application peers are denied. Existing streams are checked every second and closed when fresh canonical evidence, consent, runtime readiness or application authentication disappears.
- Gateway and host receiver session tasks are owned by their parent `JoinSet`; cancelling, aborting, or dropping either supervisor also aborts every child game stream. A switched-world task cannot leave a detached old-world TCP session alive.
- TCP copy state is kept **pinned across heartbeat ticks**, preserving backpressure without dropping buffered game bytes.
- The signed route's durable generation floor updates by atomic sibling replacement: Unix rename or Windows `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`. No temporary removal of the old floor and no success if persistence fails.
- Server binds **only loopback**; no public Minecraft listener, DNS endpoint, port forwarding, untrusted successor, or raw TCP proxy fallback.
- Unsupported or unproven realm transitions fail closed; V2 rollout needs explicit versioned route transcript review. Production quorum and IPC adapters have not been shipped by Agent 2.

## Verification performed and not performed

Locally available focused checks (run with at most two jobs, never concurrently with another agent's heavy Rust build):

```sh
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --lib --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_stream_loopback --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_authority_handoff --offline
CARGO_BUILD_JOBS=2 cargo test -p swarm-network --test game_stream_transport --offline
CARGO_BUILD_JOBS=2 cargo clippy -p swarm-network --lib --tests --offline -- -D warnings
```

The synthetic acceptance covers two real encrypted/authenticated QUIC peers, a 1 MiB loopback TCP byte-for-byte round trip across authority heartbeat ticks, active-stream consent withdrawal, denial of new joins, new session on recovered consent, hard-cancelled gateway and hosting receiver supervisors, and restart of the gateway/listener at the **same** per-world local port against the persisted floor. It **does not** establish current real quorum, real signed Fabric IPC readiness or actual Minecraft protocol/world semantics.

The additional synthetic three-peer handoff test uses a real encrypted QUIC connection to each of two distinct hosts and tagged loopback TCP servers: it proves that old-host sessions close when authority changes, an unready new host receives no Minecraft traffic, a ready successor accepts a fresh session through the unchanged gateway address, and the persisted generation floor blocks a rollback to the old authority. Its synthetic accepted-head source is **test-only**; it cannot replace actual majority-backed recovery or an authenticated Fabric server.

**Still required on independent machines:** three installed peers with real signed majority/fencing, real Fabric Java server and 2+ installed Minecraft clients, planned authority transfer, crash recovery, blocked two-voter quorum loss, exact accepted checkpoint/world markers, off-LAN NAT/hole punching/relay and relay outage behavior, actual Windows installer/firewall integration, long-running churn and interruption tests. Record exact SHAs and independently verify saved world state and any uncommitted gameplay loss. Keep P0 networking acceptance open until those tests pass.
