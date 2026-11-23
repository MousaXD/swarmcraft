# Agent 1: live Fabric authority evidence boundary

## Implemented source pieces (not production authorization)

`minecraft/fabric/.../SwarmCraftMod.java` now accepts a three-field
`RUNTIME_CHALLENGE<TAB>request_id<TAB>nonce_hex` command over its existing
token-authenticated loopback controller socket. A server-thread callback
answers with `RUNTIME_READY<TAB>request_id<TAB>nonce_hex<TAB>instance_nonce_hex
<TAB>server_port<TAB>world_directory_hex<TAB>compatibility_fingerprint`.
The 32-byte runtime instance nonce is generated in the live Java process,
not loaded from disk. Refusal is `ERROR<TAB>request_id<TAB>RUNTIME_UNAVAILABLE`.

`live_fabric_readiness::verify_runtime_reply` enforces the exact request ID,
fresh challenge nonce, canonical compatibility fingerprint, managed world
directory and reserved Minecraft loopback listener port. The returned
`ready_proof_hash` identifies the same running instance across independently
challenged sessions. It is **not a signature**, and neither it nor the existing
`MigrationPhase::Ready` establishes canonical authority.
`VerifiedFabricRuntime` has private fields and read-only accessors, so another
CLI module cannot simply construct a fake verified instance from a cached
status. The decoder still requires its caller to prove that the input came
over the currently authenticated `FabricSession` (not an arbitrary text line).

`live_authority::SignedLiveAuthoritySource` implements the network crate's
`FreshAuthoritySource` trait. It accepts raw signed inputs through the
`LiveAuthorityProofProvider::acquire(world, requester, nonce)` interface and
independently verifies them. Required material is a genesis-anchored,
certificate-backed membership chain; the locally accepted signed epoch,
config and snapshot; fresh, challenge-bound **signed majority votes**; and
`SignedHostFabricAttestation` signed by the authority over the *same nonce*
after actual live authenticated Fabric readiness and current local consent.
Host-side `sign_live_host_fabric_attestation(paths, identity, ...)` reads the
actual current local hosting consent under the same advisory update lock used
by `hosting_budget::set_contribution`. It refuses during a concurrent
consent update, after revocation, on missing/corrupt consent or wrong
identity, and signs the current revision rather than trusting an earlier
consent object supplied by a caller. The caller must still obtain
`VerifiedFabricRuntime` over the SAME live authenticated Fabric IPC session,
and retain the runtime/authority lifecycle and process identity. The
signature does not remain authoritative after later consent withdrawal.
Issuance and three-second expiry are generated from the local system clock
inside the guarded signer, not from a caller-provided timestamp. Tests reject
stale/revoked, missing, corrupt, mismatched-device and concurrently updated
consent records.
The signed attestation additionally includes `RuntimeCanonicalBinding`:
exact current membership/config hashes, accepted snapshot hash/state root,
epoch and fencing token. Agent 3 must derive that binding from independently
accepted canonical records after the managed restore, never a Ready report.
The source also checks the current local canonical state, higher recovery
promises, pending membership, sleep, active authenticated host transport and
consent revision when the host is local. `canonical_quorum_hash` calculates the
stable canonical digest that the provider puts in the quorum challenge's
`announcement_hash` field. It does NOT represent a completed vote by itself.
Private worlds do not need public discovery announcements.

The verifier additionally re-hashes the exact accepted snapshot blob set on
each independent admission/heartbeat proof. A broken or missing checkpoint
never silently becomes a verified world. This is intentionally conservative:
large worlds may exceed the three-second proof budget and **fail closed**. A
future performance optimization must retain independently proved integrity
and explicit immutable checkpoint identity rather than converting this check
into a cached `Ready` shortcut. The scoped positive Rust integration test
builds real signed and persisted V1 records, then verifies consent withdrawal,
blob corruption and majority loss with synthetic live signatures.

The accepted canonical generation is now checked **both before and after**
potentially expensive verification. The final gate re-reads signed membership,
epoch, config, checkpoint head, pending membership, recovery promises and
reservations, committed transfers, sleep, and local hosting consent; it also
rechecks the authenticated host transport. It refuses any surviving signed
solo branch or preserved solo conflict, including malformed conflict records,
instead of returning `conflict_free=true` for an unresolved world. The
`verified_at_unix_ms` timestamp is recorded only after that final gate.
Future daemon wiring must still fail closed and close streams on subsequent
world/consent changes; no synchronous recheck is a perpetual lease.

The signed membership decoder now refuses overlarge private-world histories
using the same limits as discovery, and checks unique, proposal-bound signed
certificate voters plus cryptographic member-key binding on ALL intermediate
and pending membership records. Historical certificates may have unordered
votes because the existing daemon stores HashMap iteration order; duplicates
are rejected, while otherwise valid ordering differences remain accepted.
A fully signed intermediate record containing a
ghost/banned member key does not become acceptable just because the current
voter set is valid. In addition to the signed and wall-clock freshness limits,
the full acquire + snapshot/certificate validation pipeline is bounded by a
monotonic three-second deadline, including the final verification stage.
The provider API now accepts a monotonic `deadline: Instant` on every
`acquire(world, requester, nonce, deadline)` call. Agent 3 must enforce this
deadline on every actual authenticated network/IPC acquisition step; because
the provider is synchronous, the verifier cannot interrupt a blocked call
that ignores its deadline. The source rejects a result returned after the
deadline and never treats blocking or missing proof as an authorized route.

Agent 3 owns the shared CLI module wiring: add
`pub mod live_fabric_readiness;` and `pub mod live_authority;` to
`crates/swarm-cli/src/lib.rs` when
integrating the adapter. Agent 1 intentionally leaves `lib.rs` unchanged to
avoid a shared-file conflict. Until then the scoped
`tests/live_fabric_readiness_contract.rs` compiles the standalone module.
The new Java challenge refuses until a live, changing multi-member authority
permit is observed; no permit is a denial, not a green readiness result.

## Missing integration -- must fail closed until completed

1. The currently shared `swarm-ipc/src/transport.rs::FabricSession` still
   only exposes save/shutdown RPC. Its owner must add a bounded
   `runtime_challenge` method over the **same authenticated live socket**,
   using the exact typed validator above. Set a deadline shorter than the
   network proof worker's three-second budget. Serialize session requests and
   match their IDs; any unexpected response, EOF, old session, or timeout
   refuses readiness.
2. The managed runtime owner must expose the *actual* live FabricSession and
   independently allocated loopback port, with process identity/lifecycle.
   The currently file-based migration status is not interchangeable with a
   session. Do not open another unauthenticated socket or trust the
   configured `game_endpoint`.
3. Agent 3 must implement the live `LiveAuthorityProofProvider` on the
   continuously polled, authenticated `SwarmNode`: solicit NEW signed
   `DiscoveryFreshnessVoteV1` votes for each nonce, including the pending
   joint-membership quorum when applicable. The existing daemon does **not**
   currently route these vote requests; this is a real integration dependency.
   Obtain a fresh signed `SignedHostFabricAttestation` from the actual host,
   not from a remote Ready flag. Follow the strict `SignedLiveAuthoritySource`
   input contract. A two-voter world with one voter, stale lease, revoked
   consent, old runtime or unsupported V2 transition must refuse.
4. Agent 3 owns daemon wiring, supervisor shutdown, route responder and
   gateway; coordinate with that owner before changing `daemon.rs` or
   `main.rs`. Agent 2 owns `migration.rs`, `authority_permit.rs`, and
   `server.properties` / managed-loopback lifecycle.

The scoped Rust tests are local and use synthetic signers to verify
cryptographic refusal behavior; they are not a live three-machine proof.
The Java bridge still needs a real Fabric build and process-level runtime
challenge test. Keep the route disabled until the complete provider and
runtime wiring exist and field acceptance passes.
