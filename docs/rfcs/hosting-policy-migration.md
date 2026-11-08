# RFC — Hosting-policy canonical migration and mandatory authority capability

**Status:** PROPOSED / BLOCKED — November 8, 2026  
**Agent:** 11 · **Tracking:** #84  
**Implementation branch:** `agent-11/hosting-authority-policy-migration`  
**Approval:** Agent 00 + independent Agent 07 required **before activation**

## Decision

**NO-GO for in-place live V1 activation.** The current code does not have a
canonical policy-commitment record, a V2 authority transport, a durable
policy-bound recovery/lease/transfer protocol, or a way to exclude
policy-unaware V1 authority binaries. A signed sidecar is not acceptance.

The additions under `swarm-consensus::hosting_policy_migration` and
`swarm-network::authority_capability` are **isolated negative gates and testable
contracts only**. The current daemon, storage, wire message variants, world
genesis, canonical config signatures, and operational authority behavior are
unchanged. Compiling these modules does not grant authority or activate a
policy.

## Investigation: actual V1 trust boundaries

| Surface | Current V1 trust anchor | Migration failure if used alone |
| --- | --- | --- |
| `WorldGenesisV1` | Domain-separated postcard hash determines `WorldId` | Editing fields or encoding changes existing world identity |
| `WorldConfigV1` | Fixed unsigned canonical tuple and signed config hash; `previous_config_hash` | Appending hosting policy changes signed/hash bytes and breaks accepted history |
| `MembershipRecordV1` | Canonical signed members, `authority_eligible`, voting membership | Eligibility is not local consent or runtime suitability |
| Joint membership certificate | Old + new majority shape; signatures verified by core | Concurrent membership/policy updates need both quorum sets or serialization |
| `RecoveryBallotV1` | Base epoch, fencing, snapshot/state and membership hash, signed candidate | No policy hash, feature version or consent revision; old binary may vote for V1 successor |
| `AuthorityLeaseGrantV1` / transfer / sleep | Signed authority generation and existing fencing behavior | No mandatory policy field or mandatory upgraded authority capability |
| `PeerHelloV1` | Signed capabilities and connection challenge-bound proof | Capability string is optional; old peers ignore new semantics |
| `HostCapabilityV1` | Ephemeral runtime readiness metadata | No consent, policy, authenticated V2 support, resource proof or canonical commitment |
| `/swarmcraft/1` | Only currently instantiated request/response stream | V1-only peers can still take authority through legacy wire paths |

`WorldMemberV1.authority_eligible` must remain separate from voting and
snapshot replication. One Minecraft JVM simulates at a time; additional
eligible members offer failover, **not pooled RAM or parallel ticks**.

## Recommended migration architecture

### 1. Preserve legacy history and identity

Do not alter `WorldGenesisV1`, `WorldConfigV1` serialization/signing,
canonical V1 hashes or old message discriminants. Introduce a separately
versioned **canonical extension family** accepted by the existing world
authority system, not an untrusted sidecar. Each record must carry:

- World ID and the last accepted V1 config hash/sequence.
- Current canonical membership hash/sequence, including a verified genesis-to-
  current membership history.
- Epoch, fencing token, previous accepted canonical extension head and
  **policy sequence + exact previous signed-policy-record hash**.
- Domain-separated signed policy hash, signer identity and signature.
- Mandatory authority feature version plus an activation state, and signed
  old/new voter quorum certificate if a joint membership change is in flight.
- A unique digest binding all the above, which **every** lease, recovery,
  transfer, authority advertisement, write and snapshot acceptance checks.

These fields must be cryptographically signed, verified, durably stored
atomically with the canonical acceptance decision, and checked during recovery.
The existing signed Agent 02 `SignedWorldHostingPolicyV1` is a **proposal**
input only; `verify_for_config` proves its signature and config binding, not
commit. The canonical extension must separately commit its **full signed record
hash** and refuse a second conflicting committed extension.

### 2. Monotonic activation states

```text
LEGACY_V1 (no canonical policy, contributions off)
  -> PREPARED (signed proposal and capability readiness only; not active)
  -> CANONICALLY_COMMITTED (signed quorum certificate + durable head + new fence)
  -> ACTIVE_POLICY_V2 (all authoritative paths require V2 policy support)
  -> ACTIVE_POLICY_V2_UPDATE (monotonic policy chain, fenced update)

A restart must reload the highest accepted canonical head.
PREPARED never authorizes hosting.
ACTIVE never downgrades to LEGACY or a lower policy version/hash.
Conflicts suspend promotion and require canonical reconciliation.
```

This chain is a *design*, not a state persisted in this PR. The helper
`validate_policy_commitment_intent` checks local predecessor, hash, base,
sequence and increasing fencing but is deliberately **not** a cryptographic
certificate validator. No activation is permitted until Agent 00 supplies
the canonical storage/acceptance implementation.

A policy disable/update cannot be an unauthenticated rollback to legacy V1.
Resource contribution can be disabled by a new policy or revoked locally,
without changing the monotonic required authority protocol version.

### 3. Network capability is mandatory, not advertised

Use a distinct negotiated authority stream (proposed
`/swarmcraft/hosting-authority/2`) that a V1 binary cannot accidentally enter.
The connection-bound authenticated challenge response binds world,
**committed** policy hash, epoch, fencing, authenticated peer key,
current local consent revision, nonce, transport endpoints and short expiry.
Replay challenges are consumed **atomically exactly once**.

`swarm-network::authority_capability` includes a version-2 transcript and
Ed25519 validator for this prospective stream. It does not register the new
stream, append a `WireRequest` variant, or claim that a forged string under
`PeerHelloV1.capabilities` is proof of negotiation.

A signed claim proves only possession of the peer's signing key and a
transcript matching the claimed context, **not measured remote RAM**, that
the remote is honestly executing the enforcement code, or that consent
persists. Current readiness is a local process measurement; candidate
advertisements are only hints until verified. The Agent 02 signed 30-second
host-freshness claim remains an additional component, not a lease.

### 4. Consensus admission and authority routes

Once the canonical activation implementation is proven, require a shared
fail-closed predicate on **all** authority pathways:

1. Direct server process start and continued serving/renewal.
2. Automatic recovery ballot, vote, certificate, epoch promotion and grant.
3. Manual transfer prepare/accept/commit.
4. Wake, solo advancement, recovery after partition and prior-authority return.
5. Inbound authority records, lease refresh, snapshot/write accept and
   discovery/announcements purporting to describe canonical authority.

For any active-policy world the predicate must require the exact durable
committed policy, authenticated V2 authority session, current consenting
identity/revision, local resource readiness, matching complete canonical
snapshot, real signed quorum and unexpired fenced lease. Epoch/fence and
policy checks must be repeated under the launch/admission lock immediately
before JVM start. Revocation while serving must trigger checkpoint/hand-off
or safe stop; it must not silently revoke votes or forge a newer fence.

`check_policy_authority_gate` is an **additional pure negative guard**, not
an authority creator. Its `Active` view and input facts must be populated
**only by verified canonical storage, real V2 session verifier and existing
signed authority validators**. The present daemon **must not** supply Active.

Do not change the old `elect_authority` signature or make a global V1 behavior
change before this machinery exists. Retain existing quorum/fencing and
fail-closed recovery semantics.

### 5. Legacy coexistence and unavoidable migration boundary

A majority of upgraded nodes can refuse to accept V1 leadership, but cannot
retroactively rewrite the old binary's view of history or prevent an offline
old authority from running an old-world JVM. Old V1 binaries cannot be made
to obey the policy by an optional capability or sidecar.

**Activation rule:** A policy-enabled world must not accept V1 authority,
recovery, grants, transfer or lease acknowledgements. Any quorum certificate
used to activate policy must be signed with the new commitment digest by the
required canonical voters (joint quorums if membership is changing). V1
acknowledgements cannot count toward that activation certificate.

Old peers may remain players or receive replicas *only where the existing
protocol can enforce read-only/non-authority semantics*. Existing V1
`authority_eligible` records cannot be assumed harmless: if any legacy
client can still successfully serve/renew authority or form a competing
canonical history under the same world identity, **do not activate in-place**.

If a safe dual-era authority boundary cannot be proven end to end, choose
an explicit, user-visible **new-world/protocol migration boundary** with a
distinct world identity, while preserving the old world as read-only/history;
never silently change identity or claim that the old world was upgraded.
Choosing a distinct identity is a fallback design requiring Agent 00 sign-off,
not a change implemented here.

## Required integration handoffs

| Owner | Minimal contract / patches needed |
| --- | --- |
| **Agent 02 — protocol** | Versioned canonical extension/commit/vote/certificate formats; full hash-chain and config/membership binding; new domain-separated signed recovery, transfer, lease and authority records; bounded V2 negotiation messages. Do not mutate V1 tuple or discriminants. Align with `SignedWorldHostingPolicyV1` and `SignedHostContributionClaimV1`. |
| **Agent 01 — daemon + storage safety coordination** | Persist accepted policy head and quorum/fence certificate atomically, reject two conflicting heads, recover without rollback; invoke gate on every authority producer/consumer and expiry/renewal; fail closed on unknown version, missing proof or V1 downgrade; ensure no implicit authority admission on world creation/wake. Storage changes require Agent 00 assignment to the storage owner. |
| **Agent 03 — hosting runtime** | Revocable local per-world consent, monotonic revisions and readiness; lock-atomic recheck at JVM launch, recovery, transfer/wake; enforce selected `-Xmx`, headroom and OS limits where feasible; safe serving revocation barrier. |
| **Agent 09 — Desktop Rust IPC** | Reflect the actual committed policy and backend consent/readiness; cannot synthesize activation by presenting a toggle. |
| **Agent 10 — CLI live journey** | Explicit consent in clean-machine acceptance, and negative V1/unsupported-peer authority admission scenarios. |
| **Agent 00 — coordinator** | Approve serialized policy/membership migration, bridge and rollback semantics, coordinate cross-owner changes, sequence integration and exact-head CI. |
| **Agent 07 — QA** | Independent negative tests and legacy-binary mixed-version review, canonical history preservation, partition/fencing and real integrated field certification. |

## Validation matrix and scope

**Isolated code added here:**

- Consensus: first bridge / policy successor, wrong world/config/membership,
  conflicting previous hash, stale fence, minority quorum, duplicate/unknown
  voter, V1/uncommitted refusal, stale authority session, revoked consent,
  unready snapshot, partition/quorum and fencing rejection.
- Network: V1 stream refusal, forged capability string refusal, signed
  connection-bound challenge, replay, altered signed policy, wrong key,
  foreign world, stale epoch/fence/revision, bad nonce/transport binding,
  expired/future timestamps and unsupported version.

**Not covered by these unit tests (must not be claimed):**

- Actual cryptographic acceptance of a canonical policy certificate,
  recovery after policy-linked power loss, real older compiled V1 peer
  exclusion, upgraded joint quorum signatures, live daemon V2 transport,
  distributed partition safety, persisted readiness revisions and actual JVM.
- True cross-machine mixed-version and real Minecraft acceptance, which
  depends on Agents 01/02/03/05/09/10 and independent QA.

**Required final gate:** exact-head Required Validation and Main Desktop
Installers, plus real mixed-V1/V2 network authority and durable recovery
acceptance **after integration**. This PR alone is **BLOCKED / NOT ACTIVE**.

No active hosting policy, main merge, release or closure of #84 may be
inferred from this RFC or the tests.
