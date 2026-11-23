# Agent 3: staged daemon transfer handoff

Branch: `agent/sabathon-daemon-20261123`. This note covers only the daemon-owned
transfer integration. It is not a claim of real Minecraft field acceptance or
production game-route authorization.

## Automatically driven after explicit, safe transfer intent

The **existing** `migration::prepare_manual_transfer` path remains the only
way to initiate this planned transfer. If a Java world is running, its Fabric
save-and-shutdown barrier must first complete; the source then persists the
signed `Prepared` record tied to the final canonical snapshot. A failed save
barrier does not automatically terminate the old runtime or create a transfer.

The daemon advances durable signed records through these steps:

1. **Snapshot delivery:** while `Prepared` is local to the source, push the
   **exact final checkpoint** to every active world member through existing
   authenticated snapshot/blob replication. A recipient must return a
   `ReplicaAck` matching the latest snapshot number, manifest hash, and state
   root; otherwise the source does not distribute its prepared phase. A member
   that disconnects is no longer counted as having acknowledged the checkpoint.
   Missing RPC responses are retried conservatively after 35 seconds. This
   transfer-specific acknowledgment is **not** a signed authority vote.
2. **Prepared:** the source delivers the signed record to other active
   witnesses before the target. Each peer's daemon checks the existing signed
   membership, old epoch, exact checkpoint, generation and fencing constraints
   before returning `TransferAccepted`.
3. **Accepted:** the target invokes the existing consent-guarded manual accept
   implementation in a bounded blocking worker, signs `Accepted`, and sends
   it to the source. The source then relays it to witnesses. The source may
   invoke the existing manual commit implementation only after every witness
   has acknowledged the exact signed accepted record.
4. **Committed:** the source signs and durably saves `Committed` through the
   existing migration implementation. It sends that record to witnesses
   before sending it to the target.
5. **Activate:** the target invokes the existing consent-guarded activation
   only after receiving the signed committed record and verifying the exact
   local final checkpoint. It signs and persists the successor epoch. The
   daemon then announces the signed successor `Epoch` to already-connected
   peers until they acknowledge it. Their existing epoch-transition checks
   must accept the signed ancestor and fencing transition. Their fresh
   majority-backed lease acknowledgments still govern runtime permissions.

The daemon uses at most four concurrent local transfer phase workers. It
never fabricates phase signatures, reuses a cached `Ready`, treats a bare
connection as quorum, or promotes an epoch from a transfer reply alone.
Duplicate delivery of the **identical** signed phase is idempotent, but a
different or stale phase cannot replace the durable record. A cancelled,
crashed or restarted daemon retries from persisted transfer state; in-memory
phase/replica acknowledgments are not treated as durable proof after restart.

The ordinary daemon lease/recovery tick also examines the **persisted and
signed** transfer before refreshing a permit or starting a crash-recovery
ballot. Any still-in-progress phase fences the old authority permit, target
pre-election and witness recovery, including after daemon restart. An unreadable
or conflicting transfer fails closed. Normal lease processing resumes only
once the exact committed successor epoch, or a later independently signed
canonical epoch, has superseded the transfer. This prevents a stopped source
from reacquiring its hosting permit between handoff phases.

That fence also applies to **inbound** signed RecoveryBallot, RecoveryEpoch,
LeaseGrant and Sleep requests. Inbound ordinary Epoch requests are refused
during a prepared/accepted handoff, except for the exact successor epoch
named by the already committed transfer, with the expected next epoch,
fencing token, canonical parent hash and quorum mode. The existing Epoch
handler still independently validates the successor signature, membership,
base state and parent generation. A persisted signed-transfer regression
checks each phase and verifies that only accepting the successor epoch
releases the fence.

This implementation intentionally requires all currently authorized active
members to acknowledge the final checkpoint and relevant transfer stages.
This is **stricter than quorum**: offline non-banned members may delay a
planned handoff, never silently lower its authority requirements.

## Not implemented / explicit integration dependencies

- The daemon now owns the one-time bounded game-route control subscriber and
  drains authenticated inbound `Offer` and `Challenge` events alongside the
  existing polled `SwarmNode` network loop. **Until the real live verifier is
  injected**, valid requests receive `QuorumUnavailable`, malformed requests
  receive `ExpiredOrMalformed`, and lost application authentication receives
  `Unauthorized`. This is an intentional deny-only handler, not a working
  game-route responder; it does not create a signer, receiver or gateway.
- The daemon can now respond to an application-authenticated
  `WireRequest::DiscoveryFreshnessVote` with a one-use, nonce-bound **signed
  canonical voter response**. The vote is tied to the exact accepted membership,
  epoch/fence, signed configuration and current canonical snapshot head using
  Agent 1's canonical live-quorum context domain. It rejects changed or
  expired nonces, unknown/removed requesters, unreadable or mismatched state,
  active handoff, a sleeping world or a newer recovery promise, and bounds
  per-process replay tracking to 1,024 short-lived nonces. It deliberately
  refuses Solo/Recovery modes and pending joint membership for now rather than
  guessing a safe voter set. **This is only a quorum vote**, not a verified
  Fabric runtime session, game route, host consent or proof of a live JVM.
- The real, nonce-bound `FreshAuthoritySource` adapter integrating current
  canonical signed quorum/membership/fencing, local consent and authenticated
  live Fabric IPC is **not** available in this branch. Do not enable game
  offer signing, host game streams, the player resolver or stable gateway from
  a migration status file, configured endpoint, synthetic proof or stale lease.
- Agent 1 owns real authority and Fabric-IPC readiness evidence. Agent 2 owns
  managed loopback Minecraft server binding, permits and runtime lifecycle.
  Agent 3 will connect their **proven live interfaces** to
  `GameRouteHostControlService`, `GameStreamReceiver`,
  `AuthenticatedGameRouteResolver`, `StableLoopbackGateway`, and
  daemon-owned shutdown when the verified source/session and managed port
  become available.
- Automatically establishing the new Minecraft client TCP connection is not
  implemented. Once the verified gateway is integrated, players will need to
  establish a new Minecraft connection through the same local loopback port
  after a transfer; this branch does not yet serve that gateway and does not
  promise live TCP session migration.
- Full production acceptance still needs three independent installed peers
  and real Minecraft clients: checkpoint and inventory markers, handoff,
  crash recovery, consent withdrawal, two-voter loss, stale-host fencing, and
  home-network/WAN/relay behavior. Local daemon/unit tests do not cover those.

## Local verification

Run Cargo checks **serially with the other agents** from this worktree:

```sh
flock -x /tmp/swarmcraft-sabathon-cargo.lock sh -c \
  'CARGO_BUILD_JOBS=2 cargo test -p swarm-cli --bin swarmcraft --offline --jobs 2 && \
   CARGO_BUILD_JOBS=2 cargo clippy -p swarm-cli --bin swarmcraft --offline --jobs 2 -- -D warnings'
```

On 23 Nov 2026 the full `swarmcraft` binary unit suite passed **37/37**
and strict `swarmcraft` binary Clippy passed on the Agent 3 worktree.
These tests include phase ordering, witness gate, replay/redelivery,
checkpoint-ack identity, exact activated successor generation, and the
no-stale-lease/no-recovery fence across every handoff phase, plus disabled
route-control refusal for valid and malformed authenticated requests. The
live-voter contract tests additionally cover exact nonce/context/expiry
matching and a **real signed vote against persisted accepted canonical
records**, including denial to a mismatched requester and after sleep.
They are **source/local tests, not a three-machine Minecraft field pass**.
