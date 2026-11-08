# Hosting-contribution policy contract (Agent 02; issue #84)

**Status: INERT FOUNDATION — BLOCKED ON CANONICAL PROTOCOL MIGRATION.**

This document is the API/compatibility handoff for Agent 00, Agent 03 (runtime),
and Agent 04 (Desktop). It does **not** authorize enabling voluntary member
hosting in the current application. Do not close #84 on the basis of this
protocol-only PR.

## Existing architecture and boundaries

- Exactly **one authoritative Minecraft simulation/JVM** runs at a time.
  Replica peers retain compatible snapshots; members can vote or play without
  being eligible for authority. Additional host-capable devices provide
  availability and failover, **not pooled RAM** or simultaneous Minecraft ticks.
- `WorldGenesisV1` determines the stable world ID. Its canonical postcard
  encoding is immutable; never change it to add a hosting option.
- `WorldConfigV1` has a fixed postcard unsigned tuple and a V1 signature
  domain. Never append an optional field with Serde defaults or change its
  existing signature/hash tuple. Existing worlds, current V1 signatures and
  configuration hashes must remain byte-for-byte valid.
- `WorldMemberV1.authority_eligible` is signed membership intent, **not**
  evidence of device consent or current resource readiness. Voting membership,
  replicated snapshot completeness, runtime readiness and authority admission
  are separate states.
- `PeerHelloV1.capabilities` can advertise a string but does not force legacy
  clients to interpret or enforce that string. A signed sidecar policy on its
  own cannot prevent older authority-capable binaries from hosting.
- Keep global `PROTOCOL_VERSION = 1` and application version separate. This
  PR deliberately introduces no new wire message, config field, storage schema,
  connection requirement, consensus rule or runtime entry point.

## New Rust contracts (crates/swarm-protocol/src/hosting_policy.rs)

- `HOSTING_POLICY_FORMAT_VERSION: u16 = 1`: format of the *new*
  standalone records, **not** the SwarmCraft network protocol version.
- `HOSTING_POLICY_CAPABILITY = "swarmcraft/hosting-policy/v1"`: future
  negotiation identifier. The string alone does not prove enforcement.
- `WorldHostingPolicyV1 { format_version, allow_member_contributions,
  minimum_server_heap_mib }`: world intent. If disallowed, minimum must be
  exactly 0; if allowed, minimum must be 512–65,536 MiB. The minimum is
  only JVM `-Xmx`, not an OS-enforced RSS/cgroup limit.
- `SignedWorldHostingPolicyV1 { protocol_version, world_id, config_sequence,
  config_hash, sequence, previous_policy_hash, policy, authority_peer_id,
  authority_public_key, signature }`: **proposal envelope** with a frozen
  domain-separated postcard signing representation. Format version and key
  binding are validated. Signature must be 64-byte Ed25519 and
  `verify_for_config` must be given a real crypto verifier from swarm-core.
  `record_hash` covers the complete signed record.
- `validate_successor(previous)` checks strictly next local sequence,
  previous record hash, world identity and nondecreasing config sequence.
  **It does not independently establish canonical acceptance.** Storage
  atomicity, accepted-head monotonicity, authority fencing, quorum and
  signature checks still belong to integration work.
- `LocalHostingConsentV1 { format_version, world_id, peer_id, revision,
  enabled, max_server_heap_mib }`: machine-local, per-world, per-identity
  record. Disabled **requires 0 MiB**. Enabled requires 512–65,536 MiB.
  `supersedes` requires a strictly increasing revision on the same
  device identity and world; never accept remote consent as local consent.
- `SignedHostContributionClaimV1` is an **inert, future-negotiated**
  peer freshness claim with `format_version`, world, member peer/public key,
  accepted policy hash, epoch, fencing token, snapshot hash, local consent
  revision, claimed selected heap, sampled/expiration timestamps, a
  **recipient-issued unpredictable 32-byte nonce** and Ed25519 signature.
  `ExpectedHostContributionClaimV1` binds the exact expected canonical
  world/policy/head/nonce and the receiver's current time.
  `verify_for_challenge(expected, verify_ed25519, consume_challenge)`
  checks domain-separated canonical signing bytes, signature, identity,
  exact challenge/head/policy and a maximum **30-second lifetime**,
  rejecting expired/future claims. The caller MUST keep a verifier-owned
  outstanding challenge table bound to the authenticated peer/connection
  and use `consume_challenge` to atomically remove the matching challenge
  **exactly once after successful signature verification**. Replayed or
  missing challenges fail closed; an invalid signature does not consume a
  challenge. It does not prove free RAM,
  policy activation, signed quorum, a trusted world-member vote, or an actual
  running JVM. Clock skew can cause rejection; fail closed and re-challenge.
  It is **not** added to `PeerHelloV1`, `HostCapabilityV1` or any network
  message in this PR. Older clients cannot use it to gain authority.
- `HostingPolicyActivationV1::NotEnforced` is the only presently legal
  state. `Enforced { accepted_policy_hash }` is reserved for a future
  canonical migration that commits that exact record and excludes legacy
  authority from policy-enabled worlds.
- `HostingContributionAdmissionV1` and
  `evaluate_hosting_contribution_v1(input, verify_ed25519)`: conservative
  **additional** gate. Requires activation, exact accepted hash and signed
  config, validated membership identity, authority-eligible and not banned,
  local enabled consent at matching revision, heap within both limits,
  negotiated enforcing capability, verified runtime compatibility, complete
  snapshot, budget safety and quorum/fencing evidence. Returns specific
  `HostingAdmissionDenialV1` values. It does not replace consensus and
  does not create leases, tokens or runtime slots.

### Required negative cases

| Scenario | Required result |
| --- | --- |
| Legacy world or no activation record | `PolicyNotActivated`; retain legacy world semantics outside this gate |
| Unknown policy/consent format, invalid heap/minimum, bad signature or signer | Reject; no implicit defaults |
| Missing policy or wrong canonical policy hash | `PolicyMissingOrStale` |
| Binding to wrong world, config hash, config sequence or authority | `PolicyInvalidOrUnverified` |
| Non-member, banned or no authority eligibility | `MemberNotEligible` |
| No local consent or disabled/0 MiB consent | `LocalConsentMissingOrRevoked` |
| Wrong peer/world, invalid or changed consent revision | `ConsentStaleOrInvalid` |
| JVM heap below world minimum or above local cap | `HeapOutsideConsentOrPolicy` |
| Capability advertisement without enforceable negotiation | `CapabilityNotEnforced` |
| Unverified runtime/budget/snapshot/quorum/fencing/membership | `IncompleteAuthorityProof` |

**Important:** A returning `Ok(())` does not grant permission to run Minecraft by
itself. The caller also needs to re-check consent atomically immediately
before process launch or takeover and throughout the serving generation;
ensure current membership/config/policy and unexpired fenced authority lease;
and apply bounded `-Xmx` plus verified headroom and OS limits. Never use
peer-reported RAM as proof of resources available on this machine.

## Agent 03 — runtime/backend handoff

Agent 03 owns persistence and runtime enforcement (not this PR):

1. Store consent per local identity/world with atomic updates and optimistic
   revision checks. Initial state is **disabled**, and joining **never**
   creates/enables consent. A revoked or reduced consent invalidates cached
   readiness and candidate advertisements immediately. Reject rollback and
   stale revisions; recheck under an admission lock immediately before launch.
2. Derive the submitted member from **accepted signed membership**, not
   discovery. Keep player-only/storage-only members in voter/snapshot flows
   as separately permitted by existing protocol rules; do not remove them
   from quorum solely because they do not host.
3. For any future policy-enabled world, invoke this additional gate before
   direct launch, daemon auto-recovery, manual transfer, wake and fresh
   successor promotion. It is not presently valid to set `Enforced` or
   `negotiated_authority_capability_enforced` to true merely because a
   `PeerHelloV1` capability is present.
4. Compute runtime budget from physical and **currently available** RAM,
   Minecraft client consumption, OS/daemon/JVM non-heap overhead, and
   concurrent processes. `max_server_heap_mib` caps the selected `-Xmx`,
   **not RSS**; refuse hard-cap assurances that OS limits cannot enforce.
   Explicit 4 GiB contribution on a 32 GiB machine is allowable if headroom
   is *measured* and sufficient; a zero allocation always means player-only.
5. Consent withdrawal during active authority must block *new* launch/
   takeover, invalidate candidate readiness, and enter the existing
   checkpoint, fenced handoff or safe-stop protocol. Do not drop voting
   membership, skip quorum, discard world state, silently continue serving,
   or bypass fencing to satisfy the withdrawal.
6. Update host status/readiness schemas only in Agent 03-owned files, and
   provide test evidence for OOM/pressure, resource races, recovery, sleep/
   wake, crash, transfer and revoked-consent behavior.

## Agent 04 — UI/backend semantics

World creator chooses whether member contributions are **permitted**.
Each device independently chooses consent and an optional local JVM heap
ceiling. The world option never opts anyone in. Explicit device setting:

- **Player only (0 MiB)** — valid, can still join/play and may replicate
  snapshots/vote under existing membership rules.
- **Contribute** — user selects a heap ceiling, sees world minimum, measured
  available memory, OS and Minecraft-client headroom, JVM overhead, and
  eligibility/rejection reasons. Distinguish eligible standby from actual
  canonical authority and from a ready snapshot replica.
- **Withdraw/reduce** — surface a handoff/stop safety state if this device
  currently serves authority; do not promise immediate shutdown without
  preserving the canonical save barrier and fencing.

The GUI must not call the feature enabled until coordinated migration and
Agent 03 enforcement are complete.

## Migration and activation requirement for Agent 00 (BLOCKER)

A **safe backwards-compatible V1 field extension is not possible**:
changing the WorldConfigV1 tuple breaks existing canonical signatures and
configs; using a standalone sidecar without a mandatory capability gate lets
old clients silently ignore the policy and remain authority-capable.

Before activation, agree on a new signed canonical extension/update family
(or an explicitly versioned WorldConfigV2 successor), containing:

1. A trust-preserving bridge from the last accepted V1 config to a unique
   policy commitment, with world ID/genesis unchanged. The policy hash must
   be included in accepted *canonical state*, not just a signed suggestion.
2. Monotonic policy sequence plus previous hash; authenticated config,
   membership, authority generation and fencing linkage. Resolve conflicting
   proposals without silently choosing a branch. Do not weaken quorum.
3. Protocol capability negotiation whose **required policy feature** is
   mandatory on **every authority-capable path**. Older V1 clients may, where
   safe, remain player-only/storage-only; they must not acquire authority,
   take over, or acknowledge an unsupported authority transition. If legacy
   peers cannot be prevented from serving, require a new world/protocol
   migration boundary and refuse unsafe mixed-version activation.
4. Explicit, consent-respecting initial state for existing worlds:
   contributions OFF, no implicit allocations and no mass eligibility
   promotion. Migration is opt-in with the current canonical authority and
   required approvals. Rollback to V1 may never erase a committed required
   feature or re-enable unsupported authority.
5. Packet/record size limits, malformed/unknown-version rejection,
   protocol-version/support tests across mixed clients, exact-head CI
   evidence and separate runtime/GUI certification.

**Delivery gate:** This PR supplies only inert types, validation, signing-byte
contracts, admission predicates and protocol tests. It deliberately does
not implement required wire/schema/storage/UI/runtime changes. Keep this PR
**draft / BLOCKED** until Agent 00 approves the compatibility migration
and Agents 03/04 integrate and certify it. Never mark issue #84 complete merely
because this PR's protocol tests pass.


## Final continuation — canonical migration and mandatory authority negotiation

**Decision proposal for Agent 00 and Agent 11 (not yet approved or implemented):**
An existing V1 world MUST NOT be silently converted to policy-enforced hosting.
The V1 authority/lease/recovery rules cannot authenticate or enforce a signed
hosting sidecar as a required feature. A V1-capable binary that is offline at
cutover can later ignore the sidecar. Even a sidecar accepted by a quorum of
new clients cannot prove an unmodified V1 binary will reject older V1 authority
credentials. This is a protocol incompatibility, not a UI or test problem.

### Migration contract and lineage-preserving fallback

**Recommended safe route: explicit V2 authority realm + immutable V1 ancestry.**
This is a *new canonical authority domain*, NOT an in-place modification of
`WorldGenesisV1` or `WorldConfigV1`, and not a claim of unchanged active world ID.
The original V1 world/genesis/identity and its signed history remain valid as
historical source records. Its latest certified state can be imported into a
separately identified V2 world, whose mandatory feature and authority domain
cannot be interpreted as V1 votes/leases. If exact same active world ID is a
nonnegotiable requirement, keep contributions OFF and return an explicit
migration NO-GO until a separate protocol and old-client exclusion proof exists.

1. **Inspect / opt in.** Keep V1 operation and policy activation
   `NotEnforced`. Obtain explicit creator/operator migration authorization;
   retain every peer's independent local consent as disabled/0 MiB by default.
   List the last verified V1 genesis/world ID, signed config sequence/hash,
   signed membership/decision certificate, snapshot hash, epoch and fencing
   generation. Disputed heads or incomplete signatures abort.
2. **Prepare a certified source barrier.** The current fenced authority
   checkpoints its save, signs a final source snapshot and upgrade-intent
   descriptor, and obtains the existing V1 quorum/certificate required for
   the source state. The bridge MUST authenticate the relevant source
   membership head and proof; a single authority signature or a lone
   `SignedWorldHostingPolicyV1` is insufficient. Write all source files
   immutably; do not delete/overwrite V1 state or mutate prior signatures.
3. **Create a distinct V2 realm.** A future `WorldGenesisV2` (or equivalent
   explicitly versioned canonical root) uses a *new active authority realm/world
   ID* and anchors the source V1 world ID and genesis hash, accepted source
   config/membership hashes and certificate, snapshot hash, source epoch and
   fencing token, and the committed new policy hash. The V2 canonical signed
   configuration includes protocol/authority generation, mandatory required
   features and an unambiguous first policy sequence. New domains, encoding,
   signature binding and version discrimination MUST be separately reviewed.
   `SignedWorldHostingPolicyV1` remains a proposal; it does not by itself
   create this V2 canonical root.
4. **Verify before promotion.** Verify the source checkpoint and certificate,
   V2 unique identity, new canonical policy commitment, authenticated
   capability negotiation, member eligibility, local consent, measured budget,
   snapshot completeness and a fresh fenced V2 quorum/lease. Admit exactly
   one authoritative Minecraft server to the V2 realm. If any item is absent,
   stay non-authoritative. There is no automatic V1 authority fallback.
5. **Commit/abort boundary.** Before the V2 canonical commitment, abort is
   permitted without modifying V1 signed history. After commitment, do not
   silently downgrade to V1 or reinterpret V2 votes as V1 votes. Preserve the
   V1 archive/read-only source state, refuse reused storage paths/process locks
   and require an explicit, separately reviewed recovery/migration procedure.
   Users must see that a new active world ID was created, linked to the
   historical V1 world; never call it a byte-identical V1-world upgrade.

The bridge/certificate MUST carry or authenticate at minimum:
`source_world_id`, original genesis hash, last accepted V1 config sequence
and hash, last accepted membership head/hash and its quorum certificate,
verified source snapshot hash, source epoch/fencing token, target V2 realm ID,
target protocol/capability requirement, first canonical signed policy record
hash, cutover generation and anti-replay/uniqueness context. Validation must
reject conflicting bridge records, missing V1 certificate, non-monotonic
generations, reused V2 realm IDs, incomplete snapshots and inconsistent
authority membership. Exactly one canonical successor is selected under the
new protocol's quorum/fencing rules; do not choose a policy fork by file order
or timestamp. A new identity/realm isolates authority, not arbitrary
world-data copying: every imported snapshot must be verified.

This recommendation preserves **V1 historical identity and signed history**;
it deliberately does **not** promise continuation under the same active
world ID. Old clients might independently serve an old V1 copy. They must
never be admitted as V2 authority or share V2 storage/process resources.
Avoid marketing the transition as preventing all possible independent V1
forks; it enforces noninterference in the new versioned realm.

### Legacy authority admission matrix (future normative behavior)

| Incoming client | Legacy V1 world | V2 policy-enabled realm |
| --- | --- | --- |
| Unmodified V1 authority-capable client | Existing V1 safety rules only; hosting policy NOT enforced | **DENY** authority, leases, recovery votes, snapshot promotions, handoff, wake and direct start. No V2 authority token or cross-version lease interpretation |
| Updated V2 client with verified mandatory authority feature | Existing V1 rules; no policy activation inferred | Can be authority **only** after canonical commitment, signed capability negotiation, local consent, resource/snapshot verification and current quorum/fencing |
| Legacy or unknown-capability player | V1 participation according to original membership/compatibility | Player-only **only if** an audited protocol gateway can guarantee zero authority messages and correct client compatibility. Otherwise **DENY** connection, not silent authority fallback |
| Updated peer with consent disabled/0 MiB | Voting/playing/replication under existing rules where eligible | Player/replica/voter as permitted, **never** authority merely because world-level contributions are allowed |
| Missing, unrecognized or downgraded feature | No policy-enabled host claim possible | **DENY** all authority-related paths, regardless of optional hello strings |

No V1 protocol-level test can prove that an unmodified binary rejects
future V2 authority policy: it has no such code path. Mandatory enforcement
must be implemented in the separately versioned wire/consensus layer and
demonstrated by mixed-version real-process negative tests. Do NOT represent
the current `CapabilityNotEnforced` unit test as an integration proof.

### Proposed Agent 11 network / consensus APIs (ownership: Agent 11)

- Future canonical `AcceptedHostingPolicyHead` proof must reference a
  verified V2 signed root, exact policy hash and sequence, world/realm ID,
  membership head, quorum certificate, authority epoch and fencing token.
  Reads must be atomic/snapshot-consistent; never infer from a sidecar.
- Future authenticated negotiation result, conceptually
  `AuthorityFeatureProof { realm_id, accepted_policy_hash, required_feature,
  peer_id, peer_public_key, authenticated_session, epoch, fencing_token,
  nonce_or_transcript_hash }`, must be issued only after *both sides*
  authenticate and positively negotiate the canonical mandatory feature.
  A `PeerHelloV1.capabilities` string or local Boolean cannot create it.
  Prevent downgrade, replay, mismatched realm/head or feature omission.
- Reject legacy/unknown client authority capabilities at **every** entry:
  candidacy advertisements, recovery ballots/votes, authority transfer,
  lease/epoch promotion, direct start, auto-recovery and wake. Prevent
  unsupported peers from counting as an eligible *authority* without
  silently deleting separately valid voting/replica membership.
- Wire and storage parsing of mandatory new-version records must be
  length-bounded, strict about unknown versions, domain separated and
  fail closed on corrupt accepted heads. Old V1 messages may not mutate
  a V2 canonical head or be accepted as V2 quorum proof.
- Supply an API that can demonstrate enforcement of a *specific* accepted
  policy hash at a particular fenced authority generation. Agent 01 must
  recheck it before each launch, takeover and recovery.

### Agent 01 daemon / Agent 03 runtime / Agent 09 integration handoffs

**Agent 01 — daemon authority:**
Receive a verified canonical policy-head proof and an Agent-11 authenticated
mandatory-capability result, not a caller-supplied `Enforced` flag.
On all authority routes, consume fresh candidate claim and revalidate
membership, quorum/lease, epoch/fencing, snapshot and current revocable local
consent before granting the runtime permit. Reject a V1 sidecar as evidence
for policy activation; do not evict lawful non-hosting voters or replicas.

**Agent 03 — local runtime (existing draft PR #93):**
`set_contribution(paths, world, heap_mib)` and
`contribution_readiness(paths, world)` remain machine-local sources of truth.
Revision increments, withdraw/reduce and pressure invalidate all previously
issued readiness/challenges. The verifier must pin a trusted consent revision
and selected heap when issuing each claim challenge, independently of the
received claim. Recheck under the same admission lock just before JVM launch,
then throughout a serving generation. Bounds on `-Xmx` do not prove a hard
process RSS cap. Never make join, create, EULA or migration an opt-in.

**Agent 09 — combined integration/acceptance:**
Test old V1 client as candidate and as player (only where an audited gateway
exists); verify denied cross-version handoff/recovery, stale signed policy,
policy forks, rollback, offline legacy restart, revoked consent, reused nonce,
wrong consent revision/heap, wrong epoch/fence/snapshot, and clean-machine
explicit opt-in followed by real Minecraft startup. Certify one integrated
source SHA using the release DAG; separate green specialist PRs do not suffice.

**Agent 00 — approval:**
Must decide whether an explicit new-world V2 lineage migration is acceptable;
approve the canonical format, new realm/world identity, authenticated
capability semantics and integration ownership **before any code activates**
`HostingPolicyActivationV1::Enforced`. Without approval, keep everything
inert and keep #84 open.

### Freshness claim API amendment in this continuation

`ExpectedHostContributionClaimV1` now additionally pins
`expected_consent_revision: u64` and
`expected_selected_heap_mib: u32` as **verifier-supplied**
challenge expectations. `verify_for_challenge` rejects either mismatch
before trying to consume the verifier's single-use nonce, even if the
received Ed25519 claim is otherwise valid. This prevents a response from
substituting a different consent revision or JVM heap under the same
challenge. Tests cover both expected-value mismatch and changed response
fields. Callers must create the expected record from verifier-owned pending
challenge/readiness state, never by copying fields from an untrusted claim.
The signed wire record and existing V1 wire forms remain unchanged.

**Exact-head CI caveat:** old passing Required Validation #42484337003
and Main Desktop Installers #42484337104 certify only original
`326037b53b36abda92a3b2c201209c5ca0ed71fd`. Any subsequent
source changes require new exact-head validation. No claim of certified
V2 authority or real mixed-version legacy exclusion is made here.


## Proposed V2 protocol record inventory — Agent 11 handoff

**Status: INERT PROPOSAL-ONLY TYPES, NOT AN APPROVED WIRE VERSION OR ACTIVATION.**
Agent 02 has added `crates/swarm-protocol/src/hosting_realm_v2.rs` and protocol-only
negative tests to pin the distinct-active-realm migration option for review.
These records are **not** reachable from V1 network, storage, daemon, launch,
recovery or UI paths. `PROTOCOL_VERSION` remains `1`; the V1 signed/hash
tuples and serialized enum discriminants are untouched. Do not use
`HostingPolicyActivationV1::Enforced` on the basis of these records.

The distinct V2 active world ID is derived from the fully serialized
`ProposedHostingRealmGenesisV2` under
`swarmcraft/hosting-realm-id/v2\0`. This binds the original V1 world ID,
accepted config/membership hashes and sequences, source snapshot hash,
epoch/fencing token, a new migration nonce and mandatory authority feature
version 2. V1 genesis and config bytes are not modified or re-signed.
A V2 target is a **new world/authority identity linked to the historic V1
source**, not a claim that a V1 world has changed policy in place.

| Draft record | Exact purpose | Important absent proof |
| --- | --- | --- |
| `SourceHistoryAnchorV2` | Check that source world ID, V1 accepted config/membership hashes and sequence, snapshot hash and authority generation agree with supplied accepted V1 records | These V1 records and snapshot must be authenticated and certified by the real storage/consensus layer |
| `ProposedHostingRealmGenesisV2` | Derive separate, mandatory-feature-version-2 target realm ID; reject missing/unknown feature and zero migration nonce | Preventing two conflicting realm migrations from one source requires an external fenced migration barrier |
| `SignedRealmHostingPolicyV2` | Sign a policy for the **target V2 realm ID**, with its own sequence and predecessor full-record hash | Signature and hash-chain shape do not prove canonical policy acceptance |
| `SignedHostingMigrationProposalV2` | Source authority-signed proposal binding target genesis, first target-policy full hash, source identity and strictly newer epoch/fence | Source quorum certificate, snapshot certification, atomic durable uniqueness and current authority lease are not checked here |
| `SignedHostingMigrationVoteV2` | A voter signs one exact migration proposal hash, source membership hash and target realm ID; membership and signature callbacks reject unknown/banned/forged voters | Majority/joint majority, duplicate-vote detection and genuine history head belong to Agent 11; no canonical certificate constructor exists |
| `SignedHostingAuthorityActionV2` | Separate domain for recovery ballot/vote, grant/renewal, transfer phases, wake and direct start; bind action, realm, committed migration+policy+membership hashes, epoch/fence, snapshot, candidate, consent revision, selected JVM heap, signer and nonce | Never sufficient for authority without real signed quorum/lease, session negotiation, verifier-issued one-use nonce, live local consent and budget checks |

**Crypto signing contract:** Each unsigned `signing_bytes()` representation
is domain-separated from V1 and remains computable *before* a signature is
attached. `validate_shape`, `record_hash` and verification require a
64-byte Ed25519 signature for signed policy/proposal records. Vote/action
verification checks signature length before the external Ed25519 callback.
All hashes of signed policy/proposal records include the signature. The
callback must use real trusted crypto; tests use explicitly labeled
non-cryptographic stubs only to verify context rejection.

**Activation exclusion:** None of these public proposal/record types may be
converted directly into Agent 11's opaque
`VerifiedCanonicalPolicyCommit`. A separate coordinator-approved
canonical root/certificate/acceptance implementation MUST verify all signed
source and target records, unique old/joint quorums, valid snapshot, fenced
single transition, durable accepted head, mandatory V2 session and denial of
every old V1 authority path before any production activation. A correct
signature alone proves neither signer authorization nor vote quorum.

**Verifiers supply trusted expectations:** `ExpectedHostingAuthorityHeadV2`
pins the exact action, target realm, accepted policy/migration/membership
hashes, epoch, fence, snapshot, candidate, signer, nonce, consent revision
and selected JVM heap independently from the incoming signed action.
`SignedHostingAuthorityActionV2::verify_for_head` rejects mismatches
before signature verification. It does not consume the nonce; the future
network/consensus implementation must keep an authenticated
connection-scoped, atomically single-use nonce registry and combine this
check with Agent 11's verified V2 authority session and Agent 03's live
machine-local consent/admission lock. V1 messages never count as V2
authority votes/leases and cannot satisfy this contract.

**Migration-phase ordering:**
1. Read-only validate complete V1 chain and persist a verified snapshot;
   acquire source generation barrier and explicit operator approval.
2. Prepare a distinct target V2 genesis and independently signed initial
   target-realm policy. The source authority signs the migration proposal.
3. Collect authenticated source quorum and, if membership changes, the
   required joint quorum; cryptographically verify and durably commit exactly
   one target canonical root with anti-fork storage guarantees.
4. Initialize isolated target V2 storage, negotiate mandatory separate
   authority capability, and only then use typed V2 records plus fenced
   target quorum/leases to launch an opted-in local host. Keep the original
   V1 history immutable and no cross-realm runtime/lease reuse.
5. On missing proof, power loss or conflict, remain non-authoritative. No
   implicit fallback to old V1 authority and no silent replay or downgrade.

**Agent 11 decision required:** The current #101
`PolicyCommitmentIntent.world_id` is a same-ID extension proposal while
these new Agent 02 V2 records represent a *distinct realm ID*. The two
approaches must NOT be confused or wired together as though they shared
a canonical hash. Agent 00 must approve the chosen strategy and Agent 07
must review mixed-version old-client exclusion before any schema is
declared final. In-place same-ID activation remains NO-GO pending proof.

**Regression test scope:** New tests check V1 serialization preservation,
target-ID separation, source config/membership/snapshot binding, rejected
unknown mandatory features, target policy chain forks, source authority
binding, invalid migration fencing, nonmember/bad-signature votes and
incorrect V2 authority policy/generation/operation/candidate/nonce/revision/
heap fields. These are **protocol-only negative tests**. They do not certify
real V1/V2 interoperability, durable consensus, a working V2 transport, JVM
memory behavior or complete player-only participation.

**Release gate:** Every later source change requires exact-head Required
Validation and Main Desktop Installers again. Preserve PR #92 DRAFT,
issue #84 OPEN and leave the new records inert until a full integrated
candidate is reviewed and certified.
