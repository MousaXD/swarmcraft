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
  exactly 0; if allowed, minimum must be 512–1,048,576 MiB. The minimum is
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
  record. Disabled **requires 0 MiB**. Enabled requires 512–1,048,576 MiB.
  `supersedes` requires a strictly increasing revision on the same
  device identity and world; never accept remote consent as local consent.
- `SignedHostContributionClaimV1` is an **inert, future-negotiated**
  peer freshness claim with `format_version`, world, member peer/public key,
  accepted policy hash, epoch, fencing token, snapshot hash, local consent
  revision, claimed selected heap, sampled/expiration timestamps, a
  **recipient-issued unpredictable 32-byte nonce** and Ed25519 signature.
  `ExpectedHostContributionClaimV1` binds the exact expected canonical
  world/policy/head/nonce and the receiver's current time.
  `verify_for_challenge` checks domain-separated canonical signing bytes,
  signature, identity, exact challenge/head/policy and a maximum **30-second
  lifetime**, rejecting expired/future claims. It does not prove free RAM,
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
