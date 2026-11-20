# RFC: authenticated Minecraft client route after SwarmCraft authority failover

**Status:** Design input for Agent 00/01/06/09/10; NOT deployed, NOT protocol authority.
**Scope:** Minecraft Java/Fabric clients reconnecting after an already-authorized canonical authority transition.
**Issue:** #85. Candidate UX-only starting point: PR #126.

## Current confirmed boundaries

- `minecraft/fabric` contains a **server-only** Fabric entrypoint. Its authenticated loopback IPC controls runtime save/readiness/stop; it cannot migrate a remote Minecraft player's TCP connection.
- `swarm-cli::migration` launches and verifies a successor Fabric runtime after Rust accepts authority. `MigrationStatus` is a locally persisted, unsigned read model; it may survive a hard crash.
- The optional `RuntimeLaunchConfig.game_endpoint` is machine-local configuration, **not** cryptographically authenticated, reachable or fresh. It must never be used to redirect a client, even if `phase=ready`.
- libp2p/QUIC authenticates SwarmCraft replication/control traffic; Minecraft Java speaks its own server protocol over TCP. A QUIC replica address is not a Minecraft TCP address. NAT/relay support for SwarmCraft peers does not automatically provide Minecraft traffic forwarding.
- A normal Java client cannot transfer an established TCP session to another host mid-stream. At best it can **disconnect and rejoin** the same world with an existing player identity, and only if the successor safely restored the accepted checkpoint. Application/game state produced *after* the last accepted checkpoint may not be committed; UI must never promise lossless live ticks or invent inventory continuity.

## Proposed user experience

1. Player chooses a saved world in SwarmCraft; local desktop offers a fixed **loopback** Minecraft connection entry.
2. A separately installed and explicitly enabled first-party Fabric **client** component may rejoin that fixed loopback target after disconnection (bounded retries, clear opt-out). Without that component the user must manually rejoin; never call manual rejoin automatic.
3. Local Rust process acts as an authenticated game gateway. The gateway resolves the current canonical authority with the existing authenticated peer network and **only** forwards TCP to an authority-verified Minecraft listener.
4. On host loss or planned handoff, the gateway closes the old upstream. New Minecraft packets are never sent to a superseded authority. After a fresh quorum-authorized transition and successful route resolution, the client component may reconnect a *new* Minecraft session. It must not replay earlier game packets.
5. If quorum, trusted route, server readiness or network path is missing, stay disconnected and present the exact reason. The client component must not silently fall back to a raw IP, cached DNS record, previous host or unverified advertisement.

## Proposed typed route transcript (requires separate protocol review)

The wire record names below are **proposals only**; Agent 11 owns signed V2 transcript design and Agent 01 owns authority validation.

`MinecraftRouteOfferV1` (signed by the accepted current authority's application identity):

- Domain-separated type/version and canonical realm ID; bind protocol V1/V2 and migration activation as appropriate. No cross-realm replay.
- `WorldId`, signed membership/configuration record hashes, exact server compatibility fingerprint, current accepted authority `PeerId` and public key.
- Accepted epoch number, fencing token and applicable quorum certificate hash.
- Accepted base checkpoint manifest hash and canonical state root; do not promise changes made since this checkpoint are already durable.
- Offer ID, monotonic sequence per generation, issued/expiry bounds, selected Minecraft transport peer, actual TCP listener/service identity and optional game endpoint hints.
- Proof of current **authenticated Fabric readiness** for exactly the same world, compatibility and authority generation. A configured address alone is not readiness.
- Signature over **all** fields; no editable/display-only address fields outside the signature.

`MinecraftRouteChallengeV1` and `MinecraftRouteResponseV1`:

- Client gateway sends unpredictable nonce bound to world, current verified epoch/fence and selected route offer.
- Current accepted authority or a membership-authorized validator must answer over an authenticated SwarmCraft transport, binding the challenge to route, runtime and current generation.
- Resolver must independently verify the latest canonical accepted history, current membership, quorum/fencing, conflict absence, world ID, compatibility, route expiry, replay floor and signed history ancestry **before any TCP forwarding**.
- An old signed offer + valid clock is **not sufficient**: a partitioned former host may still possess a valid signature and an earlier lease. Require fresh verification of current generation against an intersecting canonical quorum or another independently proved current authority state. If unreachable, return `blocked_by_quorum`/`route_unavailable`; never use last-known route.
- Separate provenance of initial source world in V1→V2 activation. A stale V1 client route cannot be honored in a newly activated V2 realm. Unsupported versions fail closed.

Use constant maximum sizes, limits and timeouts for offers/challenges; reject malformed ports, private/loopback/unroutable addresses *when remote*, DNS rebinding and untrusted endpoint hosts. Restrict the user-facing gateway listener to loopback by default and reject unsolicited inbound connections.

## Gateway / reconnection state machine

`unknown -> resolving -> verifying -> forwarding` is the only positive path.

`forwarding -> disconnected` occurs when socket closes, runtime heartbeat expires, accepted authority generation changes or the host is fenced.

`disconnected -> resolving` requires a new independent freshness challenge; never reuse a previously verified target. `blocked_by_quorum`, `conflict`, `superseded`, `invalid_signature`, `route_unavailable`, `runtime_unready` and `offline` are terminal until genuinely new authenticated evidence arrives.

- Bound connect, challenge, TCP dial, reconnect attempts and overall deadlines. No retry amplification or user-invisible infinite spinning.
- New upstream connection only after all proofs pass. Verify expected game server/version/modpack as possible; bare Minecraft status ping does not authenticate SwarmCraft world identity.
- Keep a per-world monotonic accepted generation floor durably. Never downgrade on restart or due to a status-file reset. New V2 realm supersedes V1 route only with independently verified activation.
- Graceful transfer may stage the next route but cannot activate it before signed transfer commit, fencing and runtime verification. Crash recovery requires majority-backed certificate. Exactly two voters losing one must remain `BlockedByQuorum`.
- Withdrawal of hosting consent stops admitting new game streams and triggers existing checkpoint/fence/stop enforcement; Java heap consent is not a global memory cap.
- Host transport relay fallback requires an actual Minecraft TCP forwarding path, not merely a successful libp2p control-plane relay. Authenticate both ends; constrain bandwidth/resource usage and bound TCP framing/queues.

## Ownership and integration sequence

1. **Agent 00 + Agent 11:** approve canonical route transcript and trust model, including V1/V2 realm separation and membership/quorum proof handling. No code bypass of existing `authority_permit` / V2 gates.
2. **Agent 01 + Agent 09:** expose accepted-current-authority and fresh challenge verification through a read-only, authenticated backend API; implement separately tested secure game-stream transport/direct-or-relay path.
3. **Agent 06:** add local Rust loopback game gateway after verification API exists; add optional standalone Fabric *client* artifact with explicit opt-in, bounded reconnect and player-facing outcomes. Do not convert the existing server-only JAR to a client-only or unsafe universal JAR.
4. **Agent 04:** integrate the same canonical client status and loopback entry in shipping Svelte, preserving the fallback in PR #126 until true parity is certified.
5. **Agent 10:** adversarial review of replay, stale leases, DNS rebinding, V1/V2 confusion, split brain, takeover of gateway loopback port, credential abuse and traffic injection.
6. **Agent 05:** independent real installed Java/Fabric client, separate-machine WAN/NAT/relay and game-state certification. This agent may not self-certify that gate.

## Required genuine tests (no synthetic promotion)

- A/B/C independent installed peers, at least two genuine Minecraft Java/Fabric clients, exact world/inventory markers and accepted snapshot hashes before disruption.
- Hard-kill host: majority certifies successor, successor restores accepted checkpoint and starts real Fabric; old host fenced; clients reconnect to authorized replacement via the stable gateway and verify only accepted state is preserved. Capture exact checkpoint boundary and any uncommitted gameplay loss transparently.
- Planned transfer: signed prepare/export/accept/commit/activate/observe, no early route activation and no double-authority forwarding.
- Two-voter crash without majority, corrupt/unsigned route, old-generation replay, conflicting history, forged membership/quorum, V1/V2 activation confusion, stale DNS and retired server: **no client connection**.
- Packet loss, transient transport reconnect, no direct TCP path, authentic relay fallback, unavailable relay, backend restart and gateway process crash: bounded failure/recovery with no inadvertent raw-address fallback.
- Genuine client-specific behavior: Minecraft version/modpack mismatch, player identity, disconnect reason, opt-in/opt-out, bounded retry and clean cancellation. Capture actual gameplay, not only backend state.
- CI unit/process/installed platform gates on exact SHA; field acceptance by Agent 05 uses independent devices and networks. A green unit test cannot close #85.

## Current PR #126 does NOT implement this RFC

PR #126 is a deliberately narrow fail-closed Desktop status fix. It has no route signer, challenge protocol, game gateway, Fabric client artifact or automatic reconnect behavior. No approved endpoint is offered to players; remain blocked until the backend trust and game transport are real.
