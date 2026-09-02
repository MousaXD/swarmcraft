# Auditor 4 — Networking, Invites, Discovery, and Privacy

## Audit identity

- Repository: `MousaXD/swarmcraft`
- Audit branch: `audit/network-discovery`
- Audited baseline: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Authoritative baseline branch: `main`
- Baseline gate: **PASS**. Live remote `main` was verified at the exact required SHA before the audit branch was created.
- Production changes: **none**
- Audit method: hostile-peer source review plus review of existing unit, process-level, automatic-invite, and network-soak tests. Local test execution was not available through the attached terminal connector, so this report does not claim fresh local runtime execution.

## Executive verdict

SwarmCraft has several good network controls: libp2p Noise/QUIC authentication at the transport layer, signed application records, bounded wire payloads, strict invite multiaddress parsing, explicit public/unlisted/private publication rules, short-lived signed discovery records, and a substantial interrupted-transfer soak.

However, the application-identity handshake is replayable because `PeerHelloV1` is only self-signed and is not bound to a receiver challenge, the current connection, or the libp2p transport identity. A malicious peer can first connect to a legitimate member, capture that member's automatically transmitted signed hello, then replay the unchanged hello over the attacker's own transport connection to another node. The receiving node maps the attacker's transport peer to the victim application peer. The daemon then proactively pushes canonical world material to connections whose mapped application peer is a member, including the snapshot manifest; an attacker can acknowledge the manifest and receive its blobs. This is a confirmed private-world confidentiality break.

Separate authorization gaps also allow any self-authenticated peer that knows a world ID to query `WorldDescriptor`, `WorldStatus`, and `HostCapability` without proving membership. Discovery result signatures prove only the announcer's self-identity, not that the announcer is the canonical authority for the claimed world, so malicious DHT providers can poison public browse and exact resolution. The network stack also has semantic payload bounds but no explicit peer/global request-rate or connection admission control even though `PROTOCOL.md` calls for rate limits.

**VERDICT: FAIL**

---

## Scope reviewed

Primary implementation:

- `crates/swarm-network/src/lib.rs`
- `crates/swarm-network/src/node.rs`
- `crates/swarm-network/src/discovery.rs`
- `crates/swarm-network/src/invite_connectivity.rs`
- `crates/swarm-network/src/wire.rs`
- `crates/swarm-core/src/discovery.rs`
- `crates/swarm-protocol/src/discovery.rs`
- `crates/swarm-protocol/src/lib.rs`
- `crates/swarm-cli/src/discovery.rs`
- `crates/swarm-cli/src/invite.rs`
- `crates/swarm-cli/src/daemon.rs`
- `PROTOCOL.md`

Tests/workflows reviewed:

- `crates/swarm-network/tests/handshake_hardening.rs`
- `crates/swarm-network/tests/input_hardening.rs`
- `crates/swarm-network/tests/network_transfer_soak.rs`
- `crates/swarm-network/tests/peer_networking_acceptance.rs`
- `crates/swarm-cli/tests/automatic_invite_join.rs`
- `.github/workflows/network-soak.yml`

---

# Findings

## N4-001 — HIGH — Replayable `PeerHelloV1` permits application-peer impersonation and private snapshot disclosure

**Files / functions**

- `crates/swarm-protocol/src/lib.rs` — `PeerHelloV1::signing_bytes`
- `crates/swarm-network/src/lib.rs` — `verify_peer_hello`
- `crates/swarm-network/src/node.rs` — `SwarmNode::next_event`
- `crates/swarm-network/src/discovery.rs` — `DiscoveryNode::next_event`
- `crates/swarm-cli/src/daemon.rs` — authenticated-event handling, `push_known_worlds`, `handle_response`

**Invariant**

An authenticated application peer must demonstrate current possession of its application private key on the specific live connection. A signed identity record captured on one connection must not authenticate a different transport connection later.

**Evidence**

`PeerHelloV1` contains the application peer ID, application public key, protocol list, capabilities, a sender-selected nonce, and a signature. The signature is only over those fields. There is no receiver challenge, receiver identity, libp2p transport peer ID, connection identifier, timestamp, or freshness proof in the signed transcript.

`verify_peer_hello` validates that the peer ID derives from the public key, that the current protocol version is present, and that the signature is valid. It does not bind the hello to the libp2p `TransportPeerId` that delivered it and it keeps no replay cache.

Both `SwarmNode` and `DiscoveryNode` store one `local_hello` and automatically send `self.local_hello.clone()` on connection establishment. The same signed application hello is therefore exposed to every peer that connects during that process lifetime and is deliberately resent on reconnect paths.

When a remote hello verifies, `SwarmNode` executes the equivalent of:

`authenticated[transport_peer] = hello.peer_id`

The daemon trusts that mapping. On `NetworkEvent::Authenticated`, it invokes `push_known_worlds`. That function iterates local worlds and, if `descriptor.member(application_peer)` exists, proactively sends canonical world configuration, epoch/recovery material, membership, transfer/sleep state, and the latest snapshot manifest. When the remote responds `ManifestAccepted` with missing blobs, `handle_response` sends those blob bytes.

**Attack / failure scenario**

1. Bob is a member of Alice's private world.
2. Mallory establishes an ordinary libp2p connection to Bob.
3. Bob automatically sends his valid signed `PeerHelloV1` to Mallory.
4. Mallory saves the unchanged hello. Bob's application private key is never learned.
5. Mallory connects to Alice using Mallory's own libp2p transport key.
6. Mallory sends the captured Bob hello.
7. Alice verifies Bob's self-signature and maps Mallory's transport peer to Bob's application peer ID.
8. Alice's daemon sees an authenticated world member and invokes `push_known_worlds` toward Mallory.
9. Mallory receives the snapshot manifest, responds that all referenced blobs are missing, and receives the world blob contents.

The cryptographic signature remains valid because nothing in the hello proves that the sender of the current connection possesses Bob's application key now.

**Existing coverage**

`handshake_hardening.rs` verifies protocol downgrade rejection and verifies that mutating a signed nonce invalidates the signature. That does not test replay of the original unchanged signed hello. Normal network tests verify successful mutual hello exchange. The soak intentionally changes the sender nonce when reconstructing test nodes but does not require a receiver challenge or assert that an old hello is rejected.

**Missing test**

A three-peer process/network test:

- B connects to attacker C and exposes B's signed hello.
- C opens a distinct transport identity to A and replays B's unchanged hello.
- A must reject application authentication.
- A must not emit world config, membership, manifest, blob, lease, status, or other member-only data to C.

**Recommended remediation**

Replace the one-message reusable hello with connection-bound proof of possession. A robust handshake should include a fresh receiver-generated cryptographic challenge and bind the signature to both sides of the transport context, for example:

- fresh receiver nonce/challenge;
- claimant application peer ID/public key;
- local and remote libp2p transport peer IDs;
- negotiated protocol/domain;
- optionally a handshake version and expiry.

Do not cache an authenticated application identity across a replacement connection unless the replacement independently completes the challenge. If practical, derive or cryptographically bind application identity to the libp2p transport identity so transport possession itself contributes to the proof.

**Confidence:** HIGH

---

## N4-002 — HIGH — Private-world descriptor and operational metadata are queryable without membership

**Files / functions**

- `crates/swarm-cli/src/daemon.rs` — `handle_request`
- `crates/swarm-protocol/src/lib.rs` — `WorldDescriptorV1`, `WorldMemberV1`, `WorldStatusV1`
- `crates/swarm-network/src/wire.rs` — `HostCapabilityV1`

**Invariant**

Discovery and transport authentication must not be confused with authorization to inspect canonical private-world state. A peer that is not a current world member must not receive the private membership roster or private operational state merely because it knows a world ID.

**Evidence**

After the generic hello gate, these request branches return data without calling `authorize_member`:

- `WireRequest::WorldStatus { world_id }`
- `WireRequest::HostCapability { world_id }`
- `WireRequest::WorldDescriptor { world_id }`

`WorldDescriptorV1` contains the complete `members` vector. Each `WorldMemberV1` exposes peer ID, public key, authority eligibility, and ban state. `WorldStatusV1` exposes epoch/sequence, latest snapshot/state hashes, compatibility fingerprint, and authority eligibility. `HostCapabilityV1` exposes runtime/mod readiness, conflict state, and recovery-quorum capability.

By contrast, blob, snapshot, membership-sensitive, and many authority flows later in `handle_request` explicitly call `authorize_member`, demonstrating that a membership authorization primitive already exists.

**Attack / failure scenario**

A removed member or holder of an expired/stale invite still knows the private world ID and one or more bootstrap addresses. With its own perfectly valid self-signed hello, it reconnects and requests `WorldDescriptor`, `WorldStatus`, and `HostCapability`. It learns the current private membership roster and live world/host state even though it is no longer a member and cannot use the expired invite to rejoin.

This is independent of N4-001. N4-001 impersonates a current member and can disclose full world content. N4-002 requires no impersonation at all.

**Existing coverage**

No reviewed test asserts that a self-authenticated non-member receives a denial for these three requests. Existing process tests exercise legitimate member synchronization and happy-path joining.

**Missing test**

For PRIVATE, UNLISTED, and PUBLIC canonical worlds, authenticate a non-member and issue all canonical metadata requests. Confirm that private canonical metadata endpoints deny the request unless membership or a narrowly scoped invitation bootstrap authorization explicitly permits the exact response.

**Recommended remediation**

Apply `authorize_member(storage, world_id, application_peer)` before returning canonical `WorldDescriptor`, `WorldStatus`, or `HostCapability`. If pre-membership joining needs a subset of data, create a dedicated minimal join-bootstrap response authorized by a valid, unexpired, current-authority invitation rather than weakening the canonical metadata endpoint.

**Confidence:** HIGH

---

## N4-003 — MEDIUM — Discovery announcements are self-authenticating but not authority-authenticating, enabling DHT poisoning and exact-resolution spoofing

**Files / functions**

- `crates/swarm-core/src/discovery.rs` — `verify_world_announcement`, `AnnouncementReplayGuard`
- `crates/swarm-protocol/src/discovery.rs` — `WorldAnnouncementV1`
- `crates/swarm-cli/src/discovery.rs` — `search_public_worlds`, `resolve_world`
- `crates/swarm-network/src/discovery.rs` — Kademlia provider publication/lookup

**Invariant**

A discovery record claiming facts about world W should prove that its signer is authorized by W's canonical identity/authority state. A self-created identity must not be able to impersonate the discovery metadata of an unrelated existing world ID.

**Evidence**

`verify_world_announcement` checks protocol version, rejects `Private`, checks lifetime, and verifies that the announcement signature matches `announcer_peer_id` / `announcer_public_key`. It does not prove that the announcer is a member or current authority of `world_id`, nor does it validate `config_hash`, `config_sequence`, `authority_epoch`, or `fencing_token` against a trusted chain.

The honest publisher only emits announcements when the local identity is the stored authority, but remote verification does not enforce the same property.

Kademlia provider records are untrusted locators. A malicious node can provide the constant public-directory key or a chosen `world_discovery_key(target_world)`, then return a `WorldAnnouncementV1` for a target world ID signed by the attacker's own application identity. `search_public_worlds` accepts such an announcement if it says `Public`, matches the filter, and passes `verify_world_announcement`. `resolve_world` accepts a non-private announcement matching the requested world ID and stops on the first accepted result.

`AnnouncementReplayGuard` only compares `(authority_epoch, announcement_sequence)` for a world in memory. Because an unauthorized signer can choose those values and because exact resolution does not use the guard, it does not establish authority authenticity.

**Attack / failure scenario**

- Public browse poisoning: attacker advertises arbitrary fake worlds or fabricated metadata under the public-directory provider key.
- Existing-world impersonation: attacker advertises `world_discovery_key(known_world_id)` and returns a signed but unauthorized announcement claiming a different name, compatibility, region, visibility, membership policy, or authority generation.
- Exact resolve is first-valid-result wins, so a fast malicious provider can eclipse the legitimate answer for that lookup.

Discovery does not directly mutate canonical state, which limits severity. The canonical join path still verifies its invitation/current authority. The defect remains a trust-boundary and player-misdirection problem.

**Existing coverage**

Tests cover signature mutation, lifetime, private announcement rejection, and monotonic replay behavior. They do not test an announcement that is cryptographically valid for the announcer but unauthorized for the claimed world.

**Missing test**

Construct two identities and a legitimate canonical world owned by A. Have B self-sign a syntactically valid announcement for A's world ID. Public browse and exact resolution must reject B's record unless B can present a verifiable authorization chain from the canonical world state.

**Recommended remediation**

Make announcements carry verifiable world authorization. Options include a compact authority/config proof chain, an authority key anchored in signed genesis/current epoch proof, or another explicit proof that lets a first-contact client bind the announcement signer to the claimed world. For a world already known locally, verify the signer and generation against the local canonical descriptor/epoch/config. Do not treat Kademlia provider identity as world authority.

**Confidence:** HIGH

---

## N4-004 — MEDIUM — No explicit connection/request-rate admission control leaves hostile-peer resource-exhaustion surface

**Files / functions**

- `crates/swarm-network/src/node.rs` — `SwarmNode::new`, `SwarmNode::next_event`
- `crates/swarm-network/src/discovery.rs` — `DiscoveryNode::new`, `DiscoveryNode::next_event`
- `crates/swarm-network/src/wire.rs` — semantic size limits
- `PROTOCOL.md` — Blob transfer section requires rate limits

**Invariant**

A malicious Internet peer should not be able to consume unbounded CPU, connection state, signature verification, Kademlia/identify work, or application handler work by multiplying many individually valid bounded requests or connections.

**Evidence**

Positive controls exist:

- request/response timeouts;
- `with_max_concurrent_streams(128)` for the primary node and 64 for discovery;
- semantic limits on blobs, hash lists, membership size, discovery queries/results, and announcement size;
- the pinned libp2p CBOR codec itself defaults to pre-deserialization maxima of 1 MiB requests and 10 MiB responses.

The audit found no explicit libp2p connection-limits behavior, global connection cap, per-IP/per-transport-peer request budget, token bucket, progressive backoff, or authenticated-peer rate limiter. mDNS discovery also auto-dials discovered peers, and every connection triggers application hello traffic and potentially Ed25519 verification. `PROTOCOL.md` explicitly lists rate limits as a blob-transfer requirement.

Semantic `validate_limits()` runs after CBOR decoding, so it protects application vector sizes but is not a traffic-volume policy. The codec's raw limits prevent unbounded single-frame allocation, but many peers/streams can still multiply bounded work.

**Attack / failure scenario**

A botnet or local-LAN attacker opens many transport identities/connections, repeatedly completes or fails handshakes, and sends valid bounded requests at the maximum stream concurrency. Even without violating any per-message bound, the target spends CPU on Noise/QUIC, hello signature verification, serialization, Kademlia/identify events, storage lookups, and response generation. No reviewed policy explicitly sheds load by peer/IP/global budget.

**Existing coverage**

`input_hardening.rs` proves an oversized pre-auth blob request is rejected and that a vanished response channel does not kill the node. The network soak proves interrupted large transfers and reconnect/resume behavior under impairment. Neither is a many-peer flood/admission test.

**Missing test**

A controlled hostile-load test with hundreds/thousands of synthetic transport identities and sustained bounded requests that asserts connection count, memory, CPU/work queue, and useful-client latency remain capped. Include discovery-query floods and repeated failed/successful hellos.

**Recommended remediation**

Add explicit connection admission and request-rate limits at both primary and discovery nodes, with separate budgets for unauthenticated and authenticated traffic. Apply conservative global and per-peer limits, handshake rate limiting, and backoff for repeated failures. Keep existing semantic payload limits.

**Confidence:** MEDIUM

---

## N4-005 — MEDIUM — Friend presence is globally publishable/probeable to anyone who knows a peer ID

**Files / functions**

- `crates/swarm-cli/src/discovery.rs` — `serve`, `handle_discovery_request`, `friend_presence`
- `crates/swarm-network/src/discovery.rs` — `friend_presence_key`, provider publication/lookup
- `crates/swarm-protocol/src/discovery.rs` — `FriendPresenceV1`

**Invariant**

A feature presented as friend presence should not expose a peer's online status to arbitrary non-friends unless the product explicitly defines presence as public-by-peer-ID.

**Evidence**

The discovery service unconditionally calls `start_providing_friend_presence(identity.peer_id())` when it starts. The DHT key is deterministic from the application peer ID.

On an inbound `FriendPresence` request, the server checks that `requester_peer_id` equals the authenticated application peer, and then returns a challenge-bound signed presence response if `expected_peer_id` is the local identity. It does not check that the requester appears in the local friend store or shares a world.

The client-side `friend_presence` command *does* require the target to be in the local accepted friend store, but a malicious client can speak the discovery wire protocol directly and bypass that UI/CLI precondition.

The response cryptography is otherwise good: it is bound to target peer, requester peer, fresh nonce, validity window, and target signature, so a DHT impostor cannot forge a fresh presence response merely by poisoning provider routing.

**Attack / failure scenario**

An observer who learns a user's stable application peer ID queries the deterministic friend-presence DHT key, connects as any self-authenticated identity, and repeatedly asks for presence. The target returns fresh signed liveness, allowing online/offline tracking without ever being accepted as a friend.

**Existing coverage**

Core tests verify nonce/requester binding. No reviewed test requires the remote requester to be an accepted friend.

**Missing test**

Attempt presence from a cryptographically valid but non-friend requester. Expected behavior should be explicitly specified and tested. If presence is intended to be public-by-ID, document that privacy model instead of calling it friend-private presence.

**Recommended remediation**

If friend presence is intended to be private, authorize the requester against a local accepted-friend relation or another explicit signed relationship before answering and consider whether unconditional DHT provider publication is appropriate. If it is intentionally public-by-ID, rename/document the feature and threat model so users do not infer a stronger privacy boundary.

**Confidence:** MEDIUM

---

## N4-006 — LOW — Invites are reusable bearer capabilities with no nonce consumption/revocation and no maximum lifetime

**Files / functions**

- `crates/swarm-protocol/src/lib.rs` — `InviteV1`
- `crates/swarm-cli/src/invite.rs` — encode/decode and expiry
- `crates/swarm-cli/src/main.rs` — invite creation
- `crates/swarm-cli/src/daemon.rs` — `JoinRequest` handling

**Invariant / design question**

The replay semantics of an invite must be explicit. If an invite is intended for one person or one use, replay must be rejected. If it is intended as a reusable bearer link, the UI/protocol should treat leakage as granting membership until expiry/revocation.

**Evidence**

The invite signature correctly covers world/genesis, inviter identity/key, connection hints, expiry, nonce, and display name. Decode rejects invalid signatures, wrong world/genesis, malformed connection hints, wrong protocol version, and expired tokens. The join authority additionally requires the invite to be signed by the *current* authority and verifies the joining member's own signature.

There is no consumed-invite nonce store, target-member binding, use counter, or revocation list. A single valid token can therefore authorize multiple distinct joining identities until expiry. CLI defaults to 60 minutes, but `expires_minutes` is an unbounded `u64`; saturating arithmetic permits effectively non-expiring tokens when a very large value is supplied.

This may be an intentional group-invite/bearer-link design, so severity is LOW rather than assuming a single-use contract that is not documented.

**Missing test / specification**

Specify whether an invite is single-use, multi-use, target-bound, or freely reusable until expiry. Test that exact policy, including replay by a second unrelated identity and behavior after authority rotation/revocation.

**Recommended remediation**

At minimum document bearer/reuse semantics and expose clear expiry. If one-use or person-specific invites are desired, bind the invite to an intended joining peer or persist consumed nonce IDs. Consider a reasonable maximum lifetime and a revocation mechanism.

**Confidence:** MEDIUM

---

## N4-007 — LOW — DNS invite hints are not reclassified after resolution

**Files / functions**

- `crates/swarm-network/src/invite_connectivity.rs` — `validate_address`
- `crates/swarm-cli/src/daemon.rs` — `dial_pending_invite_bootstraps`

**Invariant**

An invite hint classified as remotely usable should not become loopback/link-local/private only after DNS resolution without an explicit policy decision.

**Evidence**

IP literals receive detailed scope checks. DNS/DNS4/DNS6 hints reject only literal hostnames `localhost` and names ending in `.local`. An arbitrary public-looking DNS name is accepted based on syntax, then libp2p resolves it during dialing. The validator does not resolve and re-check the resulting IP scope.

A malicious or compromised DNS name can therefore resolve to loopback/private/internal destinations after validation. Noise/libp2p protocol negotiation still limits what the remote service can accomplish, so this is not by itself an arbitrary-code finding, but it weakens the claimed remote-usability/address-scope boundary and can create local network probes or surprising failed dials.

**Recommended remediation**

For untrusted/manual invite DNS hints, resolve immediately before dialing and apply the same scope policy to every resolved target, or explicitly document that DNS invite hints are allowed to resolve privately and do not count as proven Internet reachability.

**Confidence:** MEDIUM

---

# Privacy matrix

| Visibility / surface | Public directory provider | Public browse response | Deterministic per-world DHT provider | Exact discovery resolve by world ID | Canonical member/world data | Audit result |
|---|---:|---:|---:|---:|---:|---|
| PRIVATE | No | No | No | No | **Should be members only; metadata query gap exists** | **FAIL** because N4-001/N4-002 bypass confidentiality outside discovery |
| UNLISTED | No | No | Yes | Yes | Members only | Discovery semantics **PASS**; canonical metadata auth gap still applies |
| PUBLIC | Yes | Yes | Yes | Yes | Discovery metadata may be public; canonical state remains membership-gated in principle | Discovery/canonical separation mostly sound, but N4-002/N4-003 apply |

### Privacy semantics confirmed

- `refresh_publications` skips PRIVATE worlds entirely.
- UNLISTED and PUBLIC worlds may publish the deterministic per-world provider key.
- Only PUBLIC worlds cause public-directory provider publication.
- `DiscoveryPublic` only returns PUBLIC announcements.
- `DiscoveryResolve` allows UNLISTED/PUBLIC and filters PRIVATE.
- Search client rechecks PUBLIC before accepting browse results.
- Resolve client requires the requested world ID and rejects PRIVATE.
- Discovery handlers do not directly alter membership, authority, snapshots, or canonical world state.

The intended visibility split is therefore implemented correctly on honest nodes. The failure is in adjacent authentication/authorization and remote-announcement trust.

---

# Hostile-peer threat table

| Threat | Existing control | Remaining problem | Severity |
|---|---|---|---|
| Application peer spoofing | Ed25519-signed hello; peer ID derives from app key | Hello is replayable and not connection/transport/challenge bound | **HIGH** |
| Private world content disclosure | Canonical handlers usually use membership checks | Replayed member hello triggers proactive snapshot push and blob transfer | **HIGH** |
| Private membership metadata leak | Generic hello required | `WorldDescriptor`, `WorldStatus`, `HostCapability` lack membership authorization | **HIGH** |
| Forged invite fields | Entire invite is signed; genesis/world ID checked | Good control | INFO / positive |
| Stale invite | Decode and authority join path check expiry | Boundary comparison differs (`<=` vs `<`); no revocation beyond expiry/authority rotation | LOW |
| Replayed invite | Nonce is signed | Nonce is not consumed; same bearer token can add multiple identities until expiry | LOW |
| Malformed/unsafe multiaddress | Parsing, count/length limits, IP scope checks, relay shape checks | DNS target scope is not revalidated post-resolution | LOW |
| DHT provider poisoning | Returned announcements are signed and time-bounded | Signature proves announcer identity, not world authority | **MEDIUM** |
| Public browse poisoning | Public-only filter and signature verification | Unauthorized signer can claim arbitrary world ID/metadata | **MEDIUM** |
| Unlisted browse leakage | Public response explicitly filters Public only | No leak found on honest implementation | PASS |
| Private discovery leakage | Private publication skipped; verifier rejects Private announcement | No discovery leak found on honest implementation | PASS |
| Friend presence forgery | Requester+nonce+TTL+target signature binding | Fresh forgery resisted | PASS |
| Friend presence tracking | Client UI requires local friend | Server/DHT publication does not authorize requester as friend | **MEDIUM** |
| Oversized single wire request | libp2p codec raw maxima plus semantic limits | Good single-message bound | PASS / positive |
| Request flood / many peers | Timeouts and concurrent-stream caps | No explicit global/per-peer/IP rate/admission control | **MEDIUM** |
| Duplicate/reconnect data transfer | resumable offsets, reconnect tests, lost-ACK soak | Good recovery evidence; not a flood defense | PASS / positive |
| Relay content tampering | Noise/QUIC end-to-end transport; relay peer ID pinning in configured addresses | Relay remains availability/metadata observer; no plaintext trust assumption found | PASS / positive |
| NAT invite false-green | AutoNAT must currently say Public; relay reservation must be concrete | DNS caveat; otherwise fail-closed behavior is strong | PASS with LOW caveat |
| Discovery becoming canonical authority | Join/authority state uses separate signed canonical records | No path found where discovery directly decides canonical history | PASS |

---

# Detailed controls that are working

## Transport

- TCP uses Noise and Yamux.
- QUIC support is enabled.
- Relay client transport also uses Noise/Yamux.
- Relay addresses are tied to libp2p peer IDs where configuration APIs require relay/bootstrap identities.
- DCUtR and AutoNAT integration are present.
- Relay infrastructure events are kept separate from application connection-path accounting.

## Invite connectivity

`validate_invite_addresses` provides strong syntactic and structural hardening:

- maximum 8 addresses;
- maximum 512 characters per address;
- canonical libp2p multiaddress parsing;
- loopback, unspecified, link-local, multicast, documentation, benchmark, and other unusable literal IPs rejected;
- private/CGNAT addresses distinguished as local-only rather than Internet-capable;
- transport must be TCP or UDP+QUIC;
- relay path must name both relay and destination peer around `/p2p-circuit`;
- local/private relay host is not claimed as Internet fallback;
- duplicate addresses are canonicalized/deduplicated.

`InviteConnectivityV1::from_diagnostics` does not trust a historical observed public address unless current AutoNAT state is Public and only advertises a relay path when a concrete active relay listener exists. The failure mode is generally conservative rather than falsely green.

## Invite authenticity

- Invite fields are signed.
- Decode checks protocol version.
- `genesis.world_id()` must equal the invite world ID.
- Join checks invite signature again.
- Join verifies expiry.
- Join verifies joining-member signature and app identity.
- Join only accepts an invite from the current local authority and checks the authority is a current non-banned member.

## Discovery lifetime/replay

- World announcements are short-lived and have maximum lifetime enforcement.
- Future-issued records beyond allowed clock skew are rejected.
- PRIVATE announcements are rejected by the core verifier even if signed.
- Public search uses an in-memory `(authority_epoch, announcement_sequence)` replay guard.
- Friend presence is challenge-bound to requester ID and nonce with a short TTL.

## Wire bounds

Semantic limits include:

- blob chunks: 256 KiB;
- missing blob hashes: 16,384;
- world members: 1,024;
- recovery votes: 1,024;
- world artifacts: 4,096;
- presentation tags: 64;
- discovery results: 64;
- discovery filter tags: 16;
- discovery filter JSON: 512 bytes;
- individual discovery announcement JSON: 16 KiB.

The pinned libp2p request-response CBOR codec also applies raw pre-deserialization size maxima by default, preventing an unbounded single encoded request from allocating arbitrary memory.

---

# DHT / discovery poisoning assessment

Kademlia is used as a provider-location mechanism, not a canonical database. This is the correct architectural direction. The discovery node uses an in-memory Kademlia store and publishes:

- one constant public-directory provider key when at least one PUBLIC world is available;
- a deterministic world key for each PUBLIC/UNLISTED world;
- a deterministic friend-presence key for the local application peer.

The weak point is not Kademlia itself; it is the trust assigned to the application record returned after locating a provider. A provider can be malicious, and `WorldAnnouncementV1` currently proves only that *some* application key signed the record. Because that signer is not anchored to the claimed world, the DHT can steer the client to a convincingly signed lie.

DHT records should remain hints. A result should only become player-visible as authenticated metadata after its signer is proven authorized for that world.

---

# Membership versus discovery boundary

### Correct separations

- Discovery does not write canonical membership.
- Discovery does not elect authority.
- Discovery does not accept snapshots as canonical.
- The canonical join handler verifies a signed `JoinRequestV1` and current-authority invitation.
- Exact unlisted discovery is implemented as a world-ID lookup rather than public browse.

### Boundary failures

- Application transport authentication is treated as member identity despite replayability (N4-001).
- Canonical descriptor/status/capability endpoints treat generic authenticated identity as sufficient to query world metadata (N4-002).
- Discovery treats self-signed announcer identity as sufficient authenticity for a claimed world (N4-003).

---

# NAT traversal / relay / bootstrap assessment

- Bootstrap configuration requires a `/p2p/<peer-id>` component and dials that transport peer.
- Relay configuration similarly requires a relay peer identity.
- Direct paths are attempted before the explicit relay fallback helper when configured that way.
- Relay circuit construction includes the remote peer ID.
- Active connection paths distinguish direct application, relayed application, bootstrap infrastructure, and relay infrastructure.
- AutoNAT state feeds invite diagnostics and avoids advertising stale observed public addresses unless currently classified Public.
- mDNS automatically adds and dials discovered local peers. This is convenient but contributes to the need for connection/rate admission controls.
- Invite bootstrap addresses are dialed from pending joins. The authority/join signatures prevent successful canonical membership from an arbitrary bootstrap peer, but the DNS scope caveat in N4-007 remains.

No evidence was found that a relay or bootstrap provider is allowed to decide canonical world state.

---

# Partition, reconnect, stale-peer, and duplicate-message assessment

## Positive evidence

The network transfer soak exercises large QUIC transfers, repeated hard sender restart using a persistent transport key, lost acknowledgements, reconnect authentication, resume offset negotiation, and continuation from receiver-committed bytes. The scheduled/manual workflow adds loss, delay, and rate impairment and runs a multi-GiB ignored soak explicitly.

`SwarmNode` tracks a canonical active connection per transport peer and has logic to avoid dropping authentication merely because an older superseded connection closes. This directly addresses a real reconnect race.

Canonical daemon handlers commonly reject stale membership/snapshot/authority generations and verify signatures, although authority correctness is primarily Auditor 2's scope.

## Gaps

- No test observed for hostile replay of the same signed hello on a *different* transport identity.
- No many-peer reconnect storm/admission test.
- Discovery exact resolve is first-accepted-result and has no multi-provider consistency comparison.
- The discovery anti-replay guard is in-memory and public-search specific; process restart resets it. TTL still bounds the window.

---

# Test coverage assessment

| Area | Existing evidence | Missing adversarial proof |
|---|---|---|
| Peer hello signature | Downgrade rejection; nonce mutation invalidates signature | Reuse identical captured hello on a different transport connection/identity |
| Pre-auth oversized input | Oversized BlobChunk rejected; node remains usable | Many bounded requests/connections over time |
| Response-channel failure | Dropped requester does not kill node | Repeated churn from many peers |
| QUIC reconnect/resume | Lost ACK + sender restart + multi-GiB soak | Reconnect storm/adversarial identity churn |
| Automatic invite | Real process-level auto-bootstrap join and replication | Same invite reused by second identity; expired/stale/rotated-authority invite on live process |
| Invite address parsing | Loopback, excessive count, malformed override, connectivity snapshot cases | DNS resolving to private/loopback target |
| PRIVATE/UNLISTED browse | Unit test confirms neither matches public browse | Process-level malicious provider trying to force leak |
| Announcement signatures/TTL | Forgery mutation, expiry, overlong lifetime, private rejection, replay guard | Unauthorized but valid signer claiming someone else's world ID |
| Friend presence | Requester and nonce cryptographic binding | Valid non-friend requester privacy denial |
| Canonical metadata authorization | Member sync paths exercise valid calls | Non-member `WorldDescriptor`/`WorldStatus`/`HostCapability` denial |

---

# Remediation order

1. **N4-001 first:** redesign application handshake as live, connection-bound proof of possession. Do not ship wider public testing with replayable member authentication.
2. **N4-002:** membership-gate canonical metadata endpoints or introduce a narrowly scoped invitation bootstrap protocol.
3. **N4-003:** anchor discovery announcement signer authorization to the claimed world's canonical identity/authority proof.
4. **N4-004:** add global/per-peer connection and request admission/rate limits; include hostile-load tests.
5. **N4-005:** decide and enforce the privacy contract for presence.
6. **N4-006:** document/limit invite bearer replay semantics and add revocation/targeting if desired.
7. **N4-007:** resolve/revalidate DNS invite targets if private/local resolution is not intended.

---

# Re-audit requirements

After fixes, Auditor 4 should re-run at minimum:

- captured-hello replay across distinct libp2p transport identities;
- automatic world push/blob exfiltration attempt after failed replay;
- current member vs removed member vs arbitrary non-member metadata queries;
- malicious DHT provider with self-signed unauthorized world announcement;
- PRIVATE/UNLISTED/PUBLIC browse and exact-resolve matrix at process/network level;
- friend-presence probe from non-friend;
- replayed invite by a second joining identity under the documented policy;
- high-cardinality connection/request flood with useful-client latency/resource ceilings;
- interrupted QUIC soak and automatic invite join regression tests.

# Final verdict

**VERDICT: FAIL**
