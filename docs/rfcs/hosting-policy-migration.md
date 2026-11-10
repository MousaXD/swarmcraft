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

## Recommended migration architecture — distinct V2 realm

### 1. Explicit decision: new active realm, immutable V1 source

**RECOMMENDATION FOR AGENT 00: DISTINCT V2 AUTHORITY REALM. Same-ID
migration is NO-GO.** A majority of upgraded V1 nodes cannot force an
offline or partitioned legacy V1 binary to stop serving its original signed
world history. Consequently, an in-place policy commitment under the same
active `WorldId` cannot prove the required global absence of double
authority, even when a new local guard passes. The historical
`PolicyCommitmentIntent` in this PR is only a hypothetical **same-ID
extension shape check**, and MUST NOT be used as an activated V2 commitment.

Agent 02's **draft** `swarm-protocol::hosting_realm_v2` supplies the proposed
protocol vocabulary:

- `SourceHistoryAnchorV2`: accepted V1 world/config sequence and hash,
  membership sequence/hash, complete verified snapshot, epoch and fence.
  An independent signed V1 genesis/config/membership chain and a real V1
  quorum or joint quorum certificate MUST authenticate this claimed anchor.
- `ProposedHostingRealmGenesisV2::realm_id()`: domain-separated new active
  realm/world ID derived from the V1 source anchor, fresh migration nonce
  and mandatory authority feature version 2. The new ID MUST differ from the
  source `WorldId`; never modify V1 genesis/signing bytes, world ID or history.
- `SignedRealmHostingPolicyV2::record_hash()`: canonical **target-realm**
  policy record hash (sequence 1 and no predecessor at genesis). This is
  **not** `SignedWorldHostingPolicyV1` (bound to the V1 source config).
  Future updates must chain the previous full signed target-policy record
  hash and advance the sequence under new signed acceptance.
- `SignedHostingMigrationProposalV2::proposal_hash()`: proposed source
  authority signature binding source ancestry, new realm ID, first V2
  policy record hash and strictly newer activation epoch/fencing. A signed
  *proposal* or collection of votes is not an accepted certificate.
- `SignedHostingMigrationVoteV2`: individually signed voter intent
  for that exact proposal, target realm and authenticated **source** voter
  membership. V1/quorum signatures must be verified using accepted voter
  keys and the canonical old and (if changing membership) new voter sets,
  including joint quorum where required.
- `SignedHostingAuthorityActionV2`: a distinct V2 signature domain for
  recovery votes, lease grants/renewals, transfers, wake and direct start.
  Its exact expected migration/policy/membership/snapshot heads, epoch,
  fence, signer, candidate, nonce, consent revision and heap are required
  inputs, **not** an already verified quorum or authority permit.

These Agent 02 types are currently on **separate draft PR #92**, not the
Agent 11 build baseline. No cross-branch import, schema activation or
unapproved Agent 02 file modification is performed here.

A future **single canonical accepted V2 root** must bind the complete
cryptographically verified V1 source ancestry and source checkpoint
certificate, unique distinct target realm ID, signed migration proposal
hash, initial signed target-policy record hash, mandatory feature version,
authenticated target voter/membership head, activation epoch and fencing,
and a unique anti-replay cutover identity. It must carry independently
verified signed source-quorum/joint-quorum acceptance, not merely voter
IDs or a majority-shaped array. Certificate validation and durable storage
must reject double use of the same source barrier, conflicting bridge
records, source-head rollback and target-realm reuse.

### 2. Monotonic V2-only authority lifecycle

```text
LEGACY_V1 (source world; contribution policy not enforceable)
  -> V2_PROPOSED (new realm and first target policy, no authority)
  -> V2_CERTIFIED (independent V1 source checkpoint + signed migration quorum)
  -> V2_CANONICALLY_ACCEPTED (unique durable V2 head + new fence, no rollback)
  -> V2_ACTIVE (V2-only lease/recovery/transfer and consent-ready authority)
  -> V2_POLICY_UPDATE (monotonic signed target-realm policy chain)
```

Preparing V2 does **not** change the original V1 authority domain or stop
a disconnected old client. A V1 copy may continue independently; we must
never claim the migration can prevent that. New realm state must have
disjoint identity, storage paths, process locks, authority tokens,
signing domains and fencing context. Do not run the imported snapshot in
the V2 realm until the V1 cutover save is fully verified, the target
migration is durably committed, all authority admission gates pass and a
fresh V2 fenced lease exists. The source archive remains immutable and
user-visible, with no automatic rollback from V2 to V1.

The existing consensus `validate_policy_commitment_intent` validates
only a hypothetical V1 same-ID extension's local chain shape; do not
interpret its success as authority authorization or V2 acceptance.
`check_distinct_authority_realm` rejects same-ID targeting, but its
success **also** does not verify source signatures, quorum, source
snapshot, target policy or durable uniqueness.
`CanonicalPolicyView::Active` remains unconstructible by production callers.
No live activation can be permitted until Agent 00 approves the root,
Agent 02/11 agree certificate bytes, Agent 01/storage implement accepted
heads and Agent 07 independently certifies integrated paths.

### 3. Network capability is mandatory, not advertised

Use a distinct negotiated authority stream (proposed
`/swarmcraft/hosting-authority/2`) that a V1 binary cannot accidentally enter.
The connection-bound authenticated challenge response binds the **new target
realm ID**, independently committed target policy and migration hashes,
accepted membership head, verified snapshot hash, epoch, fencing, authenticated
peer key, verifier-pinned local consent revision **and selected JVM heap**,
nonce, transport endpoints and short expiry.
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

**Choose an explicit, user-visible new-realm migration boundary** with a
distinct active world identity and immutable V1 source ancestry; never
silently change identity or describe it as the same active V1 world.
This is the recommended design pending Agent 00 approval, NOT an implemented
security control. If the coordinator insists on keeping the same active
world ID, the migration decision remains **NO-GO** until an independently
verified offline legacy V1 authority-exclusion mechanism exists.

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

## Exact cross-agent certificate contract and remaining blockers

**Agent 02 / Agent 11 agreement proposal (not a frozen shared schema):**
The authenticated **source** record is the V1 ancestry anchor plus an
independently verified signed V1 source checkpoint/membership quorum
certificate. The authenticated **target** root is the *different*
`ProposedHostingRealmGenesisV2::realm_id()` and first
`SignedRealmHostingPolicyV2::record_hash()`. The signed bridge proposal
digest is `SignedHostingMigrationProposalV2::proposal_hash()`; never
substitute the V1 source policy hash. A future accepted migration certificate
must prove signatures from the canonical V1 voters (and joint quorums if
membership changes), authenticate the exact target root and become a
**single unique, atomic durable accepted head** before any V2 authority can
start. Count verified signatures, not caller-provided approver IDs.

Every subsequent `SignedHostingAuthorityActionV2` and V2 negotiated
session must use the accepted migration hash, target policy hash, accepted
membership hash, target realm ID, snapshot hash and epoch/fence. Verifier
pins expected nonce, peer, local consent revision and selected heap;
an authoritative admission requires all proofs under the same freshness
boundary. A signed network transcript alone is NOT a lease, quorum,
runtime-consent proof or OS-enforced memory cap.

**Agent 01 requirements:** atomic accepted-root write and crash recovery,
unique migration anti-replay/used-source barrier, new-realm storage/lock
separation, reject legacy V1 authority packets on *all* V2 producer and
consumer paths, validate signed V2 transfer/recovery/lease certificates
without using shape-only quorum checks, preserve voter/replica roles where
safe, and revalidate the canonical head, epoch/fence and lease immediately
before admission.

**Agent 03 requirements:** per-device disabled-by-default opt-in, monotonic
consent revisions, independently measured budget and exact selected JVM heap,
challenge/claim freshness, atomic admission lock recheck at launch/transfer/
recovery/wake, safe handoff or fenced stop on revocation, and no implicit
opt-in on migration. A reported `-Xmx` does not constitute a total RSS cap.

**Integration remains BLOCKED** on Agent 00 protocol approval, Agent 02/11
canonical signed quorum certificate and root compatibility, Agent 01's
durable storage/daemon enforcement, Agent 03's runtime integration,
mandatory V2 wire registration and rejection of live legacy authority,
and Agent 07's real mixed-version partition/restart, downgrade and
double-authority negative certification. Neither PR #92 nor #101 can
claim that policy-enforced hosting is active.

## Validation matrix and scope

**Isolated code added here:**

- Consensus: same-ID target rejection; historical same-ID extension shape only; wrong world/config/membership,
  conflicting previous hash, stale fence, minority quorum, duplicate/unknown
  voter, V1/uncommitted refusal, stale authority session, revoked consent,
  unready snapshot, partition/quorum and fencing rejection.
- Network: V1 stream refusal, forged capability string refusal, signed
  connection-bound challenge, replay, altered policy/migration/membership/
  snapshot heads or selected heap, wrong key, foreign realm, stale
  epoch/fence/revision, bad nonce/transport binding,
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
