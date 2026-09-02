# Auditor 1 — Protocol and Core Invariants

## Audit metadata

- Repository: `MousaXD/swarmcraft`
- Audit branch: `audit/protocol-core`
- Audited baseline: `354be3b1066428ecab6987590b7c7dbd80fe0870`
- Baseline gate: PASS. Live remote `main` matched the required audit SHA before the branch was created.
- Production code changes: none
- Scope reviewed primarily:
  - `crates/swarm-protocol`
  - `crates/swarm-core`
  - protocol-facing acceptance logic in `crates/swarm-cli/src/daemon.rs`
  - canonical-state persistence checks in `crates/swarm-storage`
  - `PROTOCOL.md`
  - relevant portions of `ARCHITECTURE.md`
  - the exact pinned rust-libp2p CBOR codec revision used by the repository, where needed to establish message-size behavior

## Executive assessment

SwarmCraft has several strong cryptographic primitives and some correctly fail-closed transition logic, especially for accepted authority epochs. However, the reviewed tree does **not** preserve all of its claimed core invariants under delayed, reordered, stale, conflicting, or maliciously signed input.

The most important failures are:

1. membership replication can accept a valid record signed by a **previous epoch authority after a newer epoch is already accepted**, during the designed epoch/membership delivery gap;
2. any non-banned world member can author the next hash-linked `WorldConfigV1`, including authority policy and visibility changes, because receipt does not bind the config signer to the accepted epoch authority;
3. replicated snapshots are not required to directly extend the locally accepted snapshot hash/sequence/snapshot number, so detached, skipped, and same-sequence conflicting manifests can be committed if otherwise validly signed by the accepted authority.

There are also fail-open protocol-version checks, canonicalization ambiguities, and integer-boundary defects.

Existing exact-head CI was green at the audited SHA, including repository CI run `33574857732`, process-level acceptance, Rust tests on Linux/Windows/macOS, network hardening tests, and fuzz smoke. Those suites do not exercise the adversarial cases below.

## Adversarial question summary

| Question | Result | Assessment |
| --- | --- | --- |
| Can a valid signature from World A be reused in World B? | No direct reuse found for reviewed world-scoped signed records. | World IDs are included in signing material and record types use distinct signing domains. |
| Can records be reordered without detection? | **Yes.** | Snapshot acceptance permits jumps/detached parents; membership can advance without checking its signed parent hash. |
| Can a stale record be replayed? | **Yes, conditionally.** | A previous-epoch membership record with a higher membership sequence can be accepted after a newer epoch is canonical but before membership catches up. |
| Can an unknown protocol version be silently accepted? | **Yes.** | Multiple canonical record handlers verify signatures but do not require `protocol_version == PROTOCOL_VERSION`. |
| Can noncanonical representations create ambiguous identities/hashes? | **Yes.** | Provider hints and collection ordering can produce different canonical hashes/fingerprints for effectively equivalent state. No cryptographic hash collision was found. |
| Can caller timestamps define canonical ordering? | No such core ordering dependency found. | Discovery/invite timestamps exist, but canonical world history is sequence/epoch based. |
| Can malformed payload sizes cause unbounded network allocation? | No unbounded raw request was established in the reviewed transport. | The exact pinned rust-libp2p CBOR codec caps requests at 1 MiB and responses at 10 MiB before CBOR decode. Application record cardinality checks remain uneven but are transport-bounded. |

## Positive controls observed

The following controls are materially useful and should be preserved:

- Hash/signature domains are separated by record purpose in `swarm-protocol`.
- `swarm_core::verify_signature` binds the claimed `PeerId` to the presented Ed25519 public key before verifying a signature.
- Reviewed world-scoped signed records include `world_id` in their signing material, preventing simple cross-world signature substitution.
- Peer hello authentication requires support for `PROTOCOL_VERSION` and binds the application peer ID to the presented public key.
- Accepted epoch transitions in `daemon.rs` require an exact next epoch, exact next fencing token, and exact `previous_epoch_hash`; recovery epochs additionally require a certificate.
- Recovery durable promises reject stale/same-round conflicting ballots.
- Snapshot blob verification re-hashes decoded bytes and the streaming verifier reads only up to the declared uncompressed size plus one byte, limiting decompression expansion.
- Local snapshot creation sorts paths before constructing the state root.
- Snapshot shape validation rejects unsupported protocol versions, duplicate paths, unsafe paths, and state-root mismatch.
- The pinned rust-libp2p CBOR codec at revision `b4c6d6dcaccbae6c69bc5e579a50478911c6f157` has a 1 MiB request limit and 10 MiB response limit before deserialization.

---

## Findings

### APC-001 — Previous-epoch authority can advance membership after a newer epoch is accepted

- **Severity:** HIGH
- **Exact files:**
  - `crates/swarm-cli/src/daemon.rs`
  - `crates/swarm-protocol/src/lib.rs`
  - `crates/swarm-storage/src/world.rs`
- **Function/type:** `handle_request` / `WireRequest::Membership`, `MembershipRecordV1`, `Storage::save_membership_record`
- **Invariant:** Once epoch `N+1` is accepted, an authority that was valid in epoch `N` must not be able to write newly accepted canonical authorization state. Membership history must also directly extend the accepted membership record.

#### Attack / failure scenario

1. A replica has membership record `(epoch=N, sequence=S)`.
2. It accepts canonical authority epoch `N+1`. The code intentionally allows membership delivery to lag the epoch and comments that the new authority will retransmit membership after the epoch is acknowledged.
3. Before the `N+1` membership record arrives, a delayed or replayed record `(epoch=N, sequence=S+1)` signed by the **old epoch-N authority** arrives.
4. The old authority remains a non-banned world member.
5. The receiver verifies the signature and membership of the signer.
6. Because `record.epoch < accepted_epoch`, the code skips the branch that requires the membership authority to match the accepted epoch authority.
7. Because the incoming membership epoch/sequence is newer than the locally stored membership record, it is not considered stale and is persisted.

The stale authority can therefore change the member set, bans, public keys, and authority-eligibility flags after it has been fenced by a newer canonical epoch.

A related integrity gap is that an accepted membership record is not required to satisfy either:

- `record.sequence == current.sequence + 1`, or
- `record.previous_membership_hash == Some(current.record_hash())`.

A detached higher-sequence record can therefore be accepted even though `previous_membership_hash` is signed and producers populate it.

#### Evidence

`handle_request(WireRequest::Membership)`:

- verifies the membership signature;
- checks that `record.authority_peer_id` is a current non-banned member;
- rejects membership epochs ahead of the accepted authority epoch;
- requires authority peer/key equality with the accepted epoch **only when the record epoch equals the accepted epoch**;
- rejects only records older than the current membership epoch or non-increasing within the same membership epoch;
- never compares `previous_membership_hash` with the current record hash;
- then writes both the membership record and a normalized descriptor.

The same file explicitly documents that request-response delivery may cause the certified epoch to be accepted before its corresponding membership record, making the stale-authority window part of a supported ordering, not a purely hypothetical impossible state.

#### Existing test coverage

Relevant existing coverage includes:

- `crates/swarm-core/tests/signature_hardening.rs` post-signature mutation rejection;
- three-daemon recovery and successor-failure acceptance tests;
- exact epoch/fencing/previous-epoch-hash checks;
- live join replication tests.

No reviewed test injects a validly signed old-epoch membership update after the next epoch has been accepted but before membership catches up. No reviewed test rejects a membership record whose signed `previous_membership_hash` does not reference the local current record.

#### Missing test

Add a process/integration test that:

1. establishes epoch `N` and membership `(N,S)`;
2. advances only the epoch to `N+1`;
3. sends `(N,S+1)` signed by the old authority;
4. verifies rejection and unchanged descriptor/membership;
5. separately sends a current-epoch membership record with a wrong parent hash and a sequence jump and verifies rejection.

#### Recommended remediation

Centralize membership semantic validation before persistence:

- require current protocol version;
- when an accepted epoch exists, require incoming membership to be bound to the accepted epoch and its authority, except for an explicitly specified and separately validated promotion transition;
- require exact direct extension of the current membership sequence and `previous_membership_hash`;
- keep exact duplicates idempotent, but reject same-generation conflicts;
- consider binding membership to fencing token as well as epoch if future transitions can reuse or reinterpret epochs.

- **Confidence:** HIGH

---

### APC-002 — Any non-banned member can author the next canonical world configuration

- **Severity:** HIGH
- **Exact files:**
  - `crates/swarm-cli/src/daemon.rs`
  - `crates/swarm-storage/src/state.rs`
  - `crates/swarm-protocol/src/v2.rs`
- **Function/type:** `handle_request` / `WireRequest::WorldConfig`, `authorize_member`, `Storage::save_world_config`, `WorldConfigV1`, `solo_mode_allowed`
- **Invariant:** Canonical world policy/configuration must be authored by the accepted authority, not merely by any authenticated member.

#### Attack / failure scenario

1. A malicious but non-banned member receives the current `WorldConfigV1`, as normal world synchronization sends configs to known members.
2. The member constructs sequence `S+1` with `previous_config_hash` equal to the current config hash.
3. It preserves the canonical compatibility manifest/fingerprint but changes one or more non-compatibility fields, for example:
   - `visibility`;
   - `authority_policy.allow_solo_advancement`;
   - preferred replication factor;
   - membership policy;
   - presentation metadata.
4. The member sets itself as `authority_peer_id`, signs the config with its own valid key, and sends it.
5. The receiver accepts the signature, confirms the sender/signer is a non-banned member, and persists the config.

`Storage::save_world_config` correctly enforces sequence and `previous_config_hash`, but it has no authority context. The network handler does not require the config signer to equal the accepted epoch authority, nor does it require authority eligibility.

This is not merely metadata. `solo_mode_allowed` loads the signed world config and trusts `authority_policy.allow_solo_advancement`. A non-authority member can therefore author policy that later affects authority behavior.

#### Evidence

The `WireRequest::WorldConfig` path:

- verifies the config signature;
- verifies the compatibility fingerprint still matches genesis/descriptor;
- authorizes the transport sender as a member;
- authorizes the config's claimed authority peer as a member;
- requires only `application_peer == config.authority_peer_id`;
- calls `storage.save_world_config(&config)`.

`authorize_member` checks only that the peer exists in the descriptor and is not banned.

`Storage::save_world_config` provides a good chain guard: it rejects lower sequences, rejects same-sequence conflicts, and for the next config requires exact `+1` and exact `previous_config_hash`. It does **not**, and structurally cannot, prove that the signer is the accepted epoch authority.

#### Existing test coverage

Existing tests cover:

- canonical config signing/hash behavior;
- config persistence and round-trip;
- compatibility fingerprint checks;
- solo history behavior.

No reviewed test attempts to advance a config with a valid signature from a non-authority world member.

#### Missing test

Create a two/three-member integration test where:

- Alice is the accepted authority;
- Bob is a normal non-banned member;
- Bob signs the exact next hash-linked config changing `allow_solo_advancement` and/or visibility;
- every recipient must reject it and preserve the prior config.

Repeat with an authority-eligible but non-current member to prove that eligibility alone is insufficient.

#### Recommended remediation

At the network acceptance boundary:

- load the accepted epoch;
- require `config.authority_peer_id == epoch.authority_peer_id`;
- require `config.authority_public_key == epoch.authority_public_key`;
- require the authenticated sender to equal that authority peer;
- validate the member entry/key/eligibility consistently;
- retain the existing storage sequence/parent-hash guard.

Consider adding epoch/fencing generation directly to `WorldConfigV1` if config authorization must remain unambiguous across authority transitions.

- **Confidence:** HIGH

---

### APC-003 — Snapshot replication accepts non-direct extensions and same-sequence conflicts

- **Severity:** HIGH
- **Exact files:**
  - `crates/swarm-cli/src/daemon.rs`
  - `crates/swarm-storage/src/replica.rs`
  - `crates/swarm-storage/src/streaming.rs`
  - `crates/swarm-protocol/src/lib.rs`
- **Function/type:** `authorize_manifest`, `finalize_and_ack`, `Storage::finalize_replica`, `Storage::commit_snapshot_streaming`, `validate_manifest_shape`, `SnapshotManifestV1`
- **Invariant:** Every accepted snapshot must directly extend the locally accepted canonical snapshot. Reordered, skipped, duplicate, or conflicting manifests must not silently replace/advance canonical history.

#### Attack / failure scenario

**Detached jump:**

- Replica has snapshot `S3`.
- It receives a validly signed `S5` from the accepted authority before `S4`.
- `S5.previous_snapshot_hash` points to `S4`, which the replica does not have.
- Because `S5.sequence > S3.sequence`, `authorize_manifest` accepts it. No parent comparison is performed.
- After blob verification, `finalize_replica` commits it.
- Later `S4` is stale and rejected. The local stored history now contains a link to a parent that was never accepted locally.

**Same-sequence conflict:**

- Replica already has a current manifest at `(epoch=E, sequence=Q)`.
- It receives a different validly signed manifest at the same epoch and sequence, potentially with a larger `snapshot_number`.
- The stale check uses `<`, not `<=`; equality is accepted.
- `snapshot_number` continuity is not validated.
- The storage commit path names files by attacker/sender-controlled signed `snapshot_number` and atomically writes that path. A higher snapshot number can become `latest_snapshot`; an equal number can replace the file.

This permits validly signed conflicting/reordered authority output to become locally accepted instead of being detected as an equivocation or broken chain.

#### Evidence

`authorize_manifest` correctly checks:

- world existence;
- sender membership;
- snapshot authority membership, eligibility, key match;
- snapshot signature;
- accepted epoch equality and authority peer equality.

But its history check rejects only:

- a lower epoch; or
- a lower sequence in the same epoch.

It does not check:

- exact `sequence + 1`;
- exact `snapshot_number + 1`;
- exact `previous_snapshot_hash`;
- same-sequence conflict versus exact duplicate.

`validate_manifest_shape` checks protocol version, path safety/uniqueness, and state root, but has no local-history context. `finalize_replica` delegates to the snapshot commit path after blob completeness and therefore does not add parent/sequence validation.

A separate CLI history-verification path does compare `previous_snapshot_hash` while walking snapshots, showing that direct links are intended to matter, but this check is absent from live replication acceptance.

#### Existing test coverage

Strong existing tests cover:

- blob corruption;
- snapshot reconstruction;
- publication ownership races;
- interrupted transfer/resume;
- network soak;
- signed snapshot verification.

No reviewed acceptance test injects:

- `S5` before `S4`;
- a wrong `previous_snapshot_hash`;
- a same-epoch/same-sequence conflicting manifest;
- a snapshot-number jump or overwrite with otherwise valid authority signature.

#### Missing test

Add replication tests for all four cases above. The receiver should preserve its prior canonical head and return an explicit history-conflict/stale-parent error.

#### Recommended remediation

At the history-aware acceptance boundary, before negotiating blobs:

- allow exact manifest-hash duplicates as idempotent;
- otherwise require an explicit direct-extension rule for `snapshot_number`, epoch/sequence, and `previous_snapshot_hash`;
- reject same-generation/same-sequence conflicts and preserve evidence for diagnosis;
- define the exact expected rule across an epoch transition and test it separately;
- re-check the expected parent before final commit to close a race where local head changes while blobs are transferring.

- **Confidence:** HIGH

---

### APC-004 — Unsupported protocol versions are accepted by several canonical record handlers

- **Severity:** MEDIUM
- **Exact files:**
  - `crates/swarm-core/src/lib.rs`
  - `crates/swarm-core/src/protocol_v2.rs`
  - `crates/swarm-cli/src/daemon.rs`
  - `crates/swarm-storage/src/world.rs`
  - `crates/swarm-storage/src/control.rs`
  - `crates/swarm-storage/src/state.rs`
- **Function/type:** specialized signature verifiers and handlers for `MembershipRecordV1`, `WorldConfigV1`, `EpochRecordV1`, `AuthorityTransferV1`, `AuthorityLeaseGrantV1`, and `SleepRecordV1`
- **Invariant:** Unknown record protocol versions must fail closed before the record is interpreted or persisted using V1 semantics.

#### Attack / failure scenario

A peer that legitimately supports protocol V1 can sign a structurally V1 record whose `protocol_version` field is an unsupported value, such as `65535`.

For several state-bearing request paths, the receiver verifies the signature and semantic relationships but never checks `record.protocol_version == PROTOCOL_VERSION`. Because the version field is itself signed, signature verification merely proves that the signer intentionally supplied the unsupported version; it does not establish compatibility.

A current authority can therefore cause replicas to store and act on an unknown-version membership/epoch/control record using V1 semantics. For `WorldConfigV1`, APC-002 means even a non-authority member can attempt this while preserving the compatibility fingerprint.

#### Evidence

There are explicit version checks in some important places:

- `verify_peer_hello` requires the peer to advertise the current protocol;
- join/invite shape validation checks protocol version;
- discovery validation checks protocol version;
- snapshot shape validation rejects unsupported protocol versions;
- recovery ballot well-formedness includes a protocol-version check.

The reviewed generic/specialized signature verifiers do not perform semantic version checks, and the membership/world-config/normal epoch/transfer/lease/sleep handler paths do not consistently add one before persistence.

The persistence loaders generally validate world identity but not record protocol version.

#### Existing test coverage

Existing tests prove handshake version mismatch and snapshot/invite/discovery version handling in their respective paths. No reviewed negative test sends an otherwise validly signed unsupported-version membership/config/epoch/control record through the live daemon acceptance path.

#### Missing test

For every signed record family, mutate only `protocol_version`, re-sign with the otherwise authorized signer, send through the real acceptance function, and assert a protocol-mismatch error with no state mutation.

#### Recommended remediation

Add a canonical semantic validator for each versioned record and call it before authorization/persistence. Prefer one explicit V1 validation entry point per record family rather than relying on scattered callers. Storage load paths for canonical control state should also reject unsupported versions before returning records to higher layers.

- **Confidence:** HIGH

---

### APC-005 — Compatibility fingerprint is not canonical with respect to `provider_hint`

- **Severity:** MEDIUM
- **Exact files:**
  - `crates/swarm-protocol/src/v2.rs`
  - `crates/swarm-protocol/src/canonical_modpack.rs`
- **Function/type:** `ArtifactRequirementV1`, `RuntimeCompatibilityManifestV1::normalize`, `RuntimeCompatibilityManifestV1::fingerprint`, `normalize_artifacts`, canonical-modpack runtime conversion
- **Invariant:** A canonical compatibility fingerprint must be determined by clearly defined canonical compatibility semantics and must be independent of non-canonical hints and input ordering.

#### Attack / failure scenario

`ArtifactRequirementV1::provider_hint` is documented as an optional **non-canonical** discovery hint. However, `RuntimeCompatibilityManifestV1::fingerprint` serializes the whole normalized manifest, including the surviving `provider_hint`.

There is a second determinism problem: `normalize_artifacts` sorts and deduplicates artifacts using artifact ID, version, artifact hash, and side, but the comparison/dedup key does **not** include `provider_hint`. Therefore two otherwise duplicate requirements that differ only in provider hint compare equal for normalization, and whichever hint survives can depend on their input order. The surviving hint is then serialized into the fingerprint.

The canonical-modpack layer also deliberately encodes exact provider provenance into `provider_hint`, so the current codebase has two conflicting notions of whether the field is canonical.

Consequences include different compatibility fingerprints, and therefore potentially different genesis/world identity, for effectively identical artifact bytes depending on acquisition metadata or duplicate input ordering.

#### Evidence

- The field comment calls `provider_hint` non-canonical.
- The fingerprint serializes the normalized full struct.
- The artifact normalization comparator/dedup predicate omits the hint.
- Canonical modpack conversion populates the hint with encoded provider provenance and treats the resulting runtime fingerprint as authoritative.

#### Existing test coverage

Existing protocol tests cover artifact-order independence for ordinary distinct artifacts and verify that changing artifact hashes changes the fingerprint. Canonical-modpack tests cover provider provenance round-trip and deterministic ordering.

No reviewed test reverses duplicate artifact requirements that are identical except for differing provider hints, and no test establishes whether changing only a non-canonical hint must preserve the compatibility fingerprint.

#### Missing test

Add both:

1. identical artifact with `provider_hint=A` versus `provider_hint=B` and assert the intended fingerprint relation;
2. duplicate artifacts differing only by hint in reversed input order and assert identical normalization/fingerprint.

The desired assertion depends on the product decision below.

#### Recommended remediation

Choose one contract and encode it consistently:

- If provider information is truly non-canonical, exclude it from fingerprint bytes entirely.
- If exact provider provenance is intentionally canonical, rename/document it accordingly and include it in canonical sort/dedup keys so normalization is order-independent and does not silently discard conflicting provenance.

Do not overload one field with both canonical provenance and non-canonical discovery semantics.

- **Confidence:** HIGH for the observed behavior; MEDIUM-HIGH for the intended semantic impact because the comments and canonical-modpack usage currently contradict each other.

---

### APC-006 — Canonical collection ordering is enforced by some writers but not by signed-record validators

- **Severity:** MEDIUM
- **Exact files:**
  - `crates/swarm-protocol/src/lib.rs`
  - `crates/swarm-storage/src/streaming.rs`
  - `crates/swarm-storage/src/world.rs`
  - `crates/swarm-cli/src/daemon.rs`
- **Function/type:** `MembershipRecordV1::signing_bytes`, `MembershipRecordV1::record_hash`, `WorldDescriptorV1::normalize`, `snapshot_state_root`, `WorldGenesisV1::world_id`, `snapshot_directory_streaming`, `validate_manifest_shape`
- **Invariant:** Semantically set-like/map-like data must have one canonical signed/hashed representation, or non-canonical representations must be rejected before acceptance.

#### Attack / failure scenario

**Membership:**

- `MembershipRecordV1` signs/hashes the member vector in the supplied order.
- The effective `WorldDescriptorV1` is normalized by sorting/deduplicating peer IDs.
- An incoming membership record is saved raw, then its member vector is copied into the descriptor and normalized.

Thus two differently ordered signed membership records can yield the same effective descriptor membership while producing different membership record hashes. Since later recovery and membership links use the record hash, equivalent effective authorization state can acquire different history identities.

Malformed duplicate member entries are also normalized rather than being rejected, so the signed record and effective descriptor need not have a one-to-one canonical representation.

**Snapshots:**

- local snapshot creation sorts paths before computing `state_root`;
- `snapshot_state_root` itself hashes the supplied entry order;
- `validate_manifest_shape` requires unique/safe paths and matching state root but does not require entries to be in canonical sorted order.

A valid authority can therefore sign an unsorted manifest for the same path→blob mapping and produce a different state root/manifest identity than the normal local writer would produce.

**World genesis:**

`WorldGenesisV1::world_id` hashes `initial_membership` in supplied vector order with no normalization at the type boundary, so callers can derive different world IDs from differently ordered representations of an otherwise set-like initial membership.

#### Evidence

The codebase already treats ordering as non-semantic in neighboring paths:

- `WorldDescriptorV1::normalize` sorts/deduplicates members;
- local snapshot creation sorts files/entries;
- world-config presentation tags and runtime artifact lists have explicit normalization.

The signed/hash functions above do not consistently normalize or reject non-canonical ordering.

#### Existing test coverage

Existing tests prove descriptor normalization and local snapshot deterministic ordering. No reviewed test proves that a receiver rejects unsorted snapshot entries, duplicate/conflicting membership entries, or differently ordered membership records that normalize to the same descriptor.

#### Missing test

Add canonical-form tests that construct semantically equivalent permutations/duplicates and assert either:

- identical canonical bytes/hash after normalization, or
- explicit rejection as non-canonical.

The same contract should be exercised at the network acceptance boundary, not only in local constructors.

#### Recommended remediation

Define canonical order and uniqueness for every set-like collection in signed/hashed records. Then either:

- canonicalize a clone inside the signing/hash function, **and** validate that accepted records match the canonical form; or
- reject non-canonical order/duplicates before signature acceptance/persistence.

Avoid silently normalizing a persisted signed record into a different effective state representation.

- **Confidence:** MEDIUM-HIGH

---

### APC-007 — Saturating arithmetic can treat `u64::MAX` as the next authority generation

- **Severity:** LOW
- **Exact files:**
  - `crates/swarm-protocol/src/v2.rs`
  - `crates/swarm-cli/src/daemon.rs`
  - `crates/swarm-storage/src/lib.rs`
- **Function/type:** `RecoveryBallotV1::generation_is_well_formed`, recovery/epoch transition comparisons, sequence/snapshot increment sites
- **Invariant:** Monotonic generation identifiers must either advance exactly once or fail closed on numeric exhaustion.

#### Attack / failure scenario

At `u64::MAX`, `base.saturating_add(1)` equals `base`. Therefore a recovery ballot with both base and target generation equal to `u64::MAX` satisfies the current `generation_is_well_formed` next-generation comparison. Similar exact-next comparisons in daemon authority transitions use `saturating_add(1)`.

Other counters are also advanced with saturating arithmetic, while `Storage::next_snapshot_number` uses ordinary `+ 1`, which would panic in overflow-checked builds or wrap in an unchecked release context if the maximum were ever reached.

This boundary is operationally remote, but it violates the security meaning of monotonic fencing/generation numbers and fails open rather than reporting exhaustion.

#### Evidence

`RecoveryBallotV1::generation_is_well_formed` directly compares targets against `base_epoch.saturating_add(1)` and `base_fencing_token.saturating_add(1)`. Storage recovery promises trust that predicate. Daemon transition checks repeat the saturating-next pattern.

#### Existing test coverage

Normal increment behavior is tested. No reviewed test covers `u64::MAX`, `u64::MAX - 1`, or explicit counter exhaustion.

#### Missing test

Boundary-test every monotonic protocol counter at `MAX-1` and `MAX`; `MAX-1 -> MAX` should succeed and any attempted successor to `MAX` should fail with an explicit exhaustion error.

#### Recommended remediation

Use `checked_add(1)` for protocol monotonic counters and return a dedicated fail-closed exhaustion error. Never use saturation for a value whose security meaning is “strictly newer generation.”

- **Confidence:** HIGH

---

## Invariant coverage matrix

| Invariant | Status | Notes |
| --- | --- | --- |
| Canonical serialization | FAIL | Postcard + domains are sound primitives, but collection/provider normalization is inconsistent. |
| World identity derivation | PARTIAL | Domain-separated deterministic hash, but initial-membership ordering is caller-sensitive. |
| Snapshot identity | FAIL | Signature/state-root integrity exists; direct-parent/strict sequence acceptance does not. |
| Operation identity | UNPROVEN / NOT IMPLEMENTED AS A GENERAL RECORD | No production `OperationV1` implementation was found. `PROTOCOL.md` describes it conceptually and `ARCHITECTURE.md` stages operation journaling after the snapshot-first MVP. |
| Hash-link integrity | FAIL | Epoch links are checked; snapshot and membership links are not consistently checked on acceptance. |
| Signature verification | PASS with caller caveat | Ed25519 verification and peer/key binding are strong; semantic authorization/version checks are not part of generic signature verification. |
| Signer/author binding | FAIL | World config signer is not bound to the accepted authority; stale membership authority can write during an epoch/membership gap. |
| Membership validation | FAIL | Stale-epoch authority and parent-link gaps; duplicate/noncanonical member representation not rejected. |
| Protocol version compatibility | FAIL | Several state-bearing record handlers accept unsupported version fields. |
| Sequence monotonicity | FAIL | Membership permits jumps; snapshots permit jumps and same-sequence conflicts. |
| Parent/previous-hash validation | FAIL | Strong for epochs/config storage; missing for live snapshot and membership acceptance. |
| Replay resistance | FAIL | Old-epoch membership replay is conditionally accepted; exact duplicates are generally idempotent. |
| Duplicate handling | PARTIAL | Exact membership/config duplicates are handled, but conflicting snapshot equality is not rejected as a conflict. |
| Cross-world substitution | PASS for reviewed signed records | World IDs are signed and domains are record-specific. |
| Cross-epoch substitution | FAIL | APC-001. |
| Malformed record rejection | PARTIAL | Snapshot path/root checks and transport limits are good; noncanonical/unknown-version canonical records remain accepted. |
| Unknown enum/version behavior | PARTIAL/FAIL | Unknown enum discriminants should fail serde decoding; explicit unknown version values are not consistently rejected. |
| Cross-platform canonical determinism | PARTIAL | Primitive/postcard encoding is deterministic, but semantic collection normalization is inconsistent. |
| Timestamp misuse | PASS in reviewed canonical ordering | No wall-clock timestamp was found defining canonical world history order. |
| Untrusted metadata handling | PARTIAL | Message transport has a raw size cap; canonical metadata validation is uneven. |
| Integer overflow/underflow | FAIL at boundary | Saturating generation arithmetic violates strict monotonic semantics at `u64::MAX`. |
| Unbounded allocations from untrusted network input | No unbounded raw request proven | Pinned CBOR codec bounds inbound request bytes to 1 MiB; application cardinality checks should still be expanded for defense in depth. |

## Test gaps that should be added before closing this audit

1. Old-authority membership replay after the next epoch is accepted.
2. Membership wrong-parent, skipped-sequence, duplicate-peer, and reordered-member tests.
3. Non-authority member signs the exact next valid `WorldConfigV1`.
4. Unsupported protocol version for every signed canonical/control record family.
5. Snapshot wrong-parent, sequence jump, same-sequence conflict, and snapshot-number overwrite/jump.
6. Canonical artifact fingerprint with conflicting/reordered `provider_hint` duplicates.
7. Noncanonical snapshot-entry ordering and membership ordering at the **acceptance boundary**.
8. `u64::MAX` generation/sequence exhaustion tests.

## Final assessment

The cryptographic building blocks are generally solid: record domains are separated, public keys are bound to peer IDs, signatures cover world context, and epoch transition logic is notably stricter than several neighboring state paths. The failure is in semantic acceptance around those primitives. A valid signature is being treated as sufficient in places where the system also needs current-authority, current-version, strict-parent, and canonical-form proofs.

Because the audited baseline permits stale-authority membership writes, non-authority configuration writes, and non-direct snapshot history acceptance, the protocol/core invariants do not meet the required adversarial standard.

VERDICT: FAIL