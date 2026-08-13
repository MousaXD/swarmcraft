# SwarmCraft Protocol Draft

**Status:** exploratory  
**Stability:** none  
**Compatibility guarantee:** none

This document describes a conceptual wire/state protocol for early implementation.

It is intentionally conservative and should evolve through prototypes.

---

## 1. Goals

The protocol should allow peers to:

- identify themselves;
- identify worlds;
- discover compatible peers;
- compare world histories;
- transfer snapshots;
- transfer missing blobs;
- elect/change temporary authority;
- replicate durable progress;
- detect stale histories;
- recover from crashes;
- reject invalid data.

---

## 2. Non-goals

The first protocol does not attempt:

- permissionless Byzantine consensus;
- cryptocurrency;
- blockchain mining;
- arbitrary fork merging;
- trustless public execution;
- fully distributed Minecraft simulation.

---

## 3. Canonical encoding

Anything that is hashed or signed must have exactly one byte representation.

Potential choices:

- Protobuf with strict canonicalization rules;
- CBOR canonical encoding;
- custom binary format;
- postcard with frozen schemas.

Do not sign JSON.

Do not hash platform-dependent file metadata.

---

## 4. Hashes

Recommended:

```text
BLAKE3
```

Use domain separation.

Example:

```text
BLAKE3("swarmcraft/world-genesis/v1" || canonical_bytes)
BLAKE3("swarmcraft/blob/v1" || blob_bytes)
BLAKE3("swarmcraft/op/v1" || canonical_op)
```

This reduces accidental cross-type collisions at the application layer.

---

## 5. Signatures

Recommended:

```text
Ed25519
```

Signed records should include:

- world ID;
- protocol version;
- record type;
- monotonic identifiers;
- payload hash.

Never sign ambiguous concatenated strings.

---

## 6. Peer ID

Possible definition:

```text
peer_id = BLAKE3(public_key)
```

Or use the libp2p peer identity model if the project standardizes on libp2p.

---

## 7. World genesis

Conceptual schema:

```text
WorldGenesisV1 {
    protocol_version
    minecraft_version
    loader
    compatibility_fingerprint
    seed_commitment
    creation_nonce
    initial_policy
    creator_public_key
}
```

Then:

```text
world_id = hash(genesis_record)
```

The world ID remains stable.

---

## 8. Compatibility fingerprint

A peer needs to know whether it can safely simulate a world.

Possible fingerprint inputs:

```text
Minecraft version
Fabric loader version
required mods + versions
datapacks
SwarmCraft integration version
simulation feature flags
```

Cosmetic/client-only mods should ideally be excluded.

This will require Minecraft-specific classification.

---

## 9. Epoch record

An epoch identifies one period of temporary authority.

```text
EpochRecordV1 {
    world_id
    epoch_number
    previous_epoch_hash
    base_state_hash
    authority_peer_id
    mode
    fencing_token
    reason
    endorsements[]
    signature
}
```

Modes:

```text
QUORUM
SOLO
RECOVERY
```

The exact semantics need formal definition before production use.

---

## 10. Fencing

Every authoritative write includes the current fencing token.

Example:

```text
epoch 77
fencing_token 77
```

Once epoch 78 becomes valid, peers reject writes from token 77.

This helps prevent a previous authority that reappears after a partition from continuing to mutate canonical state.

---

## 11. Operation record

Conceptual:

```text
OperationV1 {
    world_id
    epoch
    fencing_token
    sequence
    previous_operation_hash
    payload_type
    payload
    authority_peer_id
    signature
}
```

The payload may initially represent high-level save/checkpoint events rather than every Minecraft event.

---

## 12. Commit certificate

When multiple peers acknowledge a state:

```text
CommitCertificateV1 {
    world_id
    epoch
    sequence
    operation_hash
    acknowledgements[]
}
```

An acknowledgement includes:

```text
peer_id
signature
```

Whether this constitutes a true quorum depends on membership semantics.

Do not call something "quorum committed" until the membership model is defined rigorously.

---

## 13. Snapshot manifest

```text
SnapshotManifestV1 {
    world_id
    snapshot_number
    epoch
    sequence
    previous_snapshot_hash
    metadata_blob
    region_blobs[]
    player_blobs[]
    global_blobs[]
    state_root
    authority_peer_id
    signature
}
```

Every referenced blob is content-addressed.

---

## 14. Blob record

A blob is immutable.

```text
BlobDescriptor {
    hash
    uncompressed_size
    encoded_size
    encoding
}
```

Possible encodings:

```text
RAW
ZSTD
```

The hash semantics must be fixed:

Either hash raw bytes or encoded bytes.

Never mix both.

---

## 15. Blob transfer

Messages:

```text
HaveBlob
WantBlob
BlobChunk
BlobComplete
BlobReject
```

Support:

- range requests;
- resumable transfer;
- parallel peers;
- integrity verification;
- rate limits.

A peer should be able to receive different snapshot blobs from multiple peers.

---

## 16. Peer hello

Initial handshake:

```text
HelloV1 {
    peer_id
    public_key
    supported_protocols[]
    swarmcraft_version
    capabilities[]
    nonce
    signature
}
```

Capabilities might include:

```text
SNAPSHOT_SEED
AUTHORITY_ELIGIBLE
RELAY
BACKGROUND_DAEMON
REGION_SIMULATION
```

Do not trust self-declared capabilities without observation where security matters.

---

## 17. World status exchange

Peers joining the same world compare:

```text
WorldStatusV1 {
    world_id
    latest_epoch
    latest_sequence
    latest_operation_hash
    latest_snapshot
    compatibility_fingerprint
}
```

This allows fast determination of:

- who is ahead;
- who is stale;
- whether histories agree;
- what needs downloading.

---

## 18. Divergence detection

Suppose:

```text
Alice:
epoch 20
sequence 900
hash A

Bob:
epoch 20
sequence 900
hash B
```

Same epoch and sequence but different hash means divergence.

The protocol must:

1. stop automatic state acceptance;
2. find common ancestor;
3. inspect epoch authority proof;
4. determine canonical branch if possible;
5. require manual recovery if protocol evidence is insufficient.

Never use file modification time.

---

## 19. Solo epochs

If a single peer is allowed to play alone, record it.

Example:

```text
Epoch 21
mode: SOLO
authority: Alice
base: state 900
```

When Bob appears:

1. Bob verifies Alice's epoch transition;
2. Bob verifies the history;
3. Bob downloads missing state;
4. Bob stores it durably;
5. replica count increases.

The policy for accepting competing solo histories must be extremely conservative.

---

## 20. Competing solo histories

Worst case:

```text
Alice and Bob both believed they were alone
```

Both advance from state 900.

```text
       900
      /   \
   Alice   Bob
   950     930
```

There is no universally correct automatic merge.

The protocol should not pretend otherwise.

Possible policies:

- require explicit user choice;
- preserve both branches;
- designate one canonical branch;
- allow admin-defined recovery;
- optionally create a copy/fork world from the losing branch.

For an MVP, manual branch selection is safer than clever automatic merging.

---

## 21. Membership

Membership is difficult.

Possible models:

### Invite-only world

World policy lists authorized peer/user keys.

### Open world

Anyone can connect, but only authorized peers can become authority.

### Permissionless authority

Much harder and outside MVP.

Recommended first target:

**invite-only friend groups.**

This sharply reduces the Byzantine threat model.

---

## 22. Roles

Possible roles:

```text
PLAYER
REPLICA
AUTHORITY_ELIGIBLE
ADMIN
SPECTATOR
RELAY
```

One identity can have multiple roles.

Permissions should themselves be part of canonical world history.

---

## 23. Discovery records

DHT records should contain hints, not truth.

Example:

```text
WorldPeerRecord {
    world_id
    peer_id
    addresses[]
    expires_at
    signature
}
```

A malicious DHT cannot change canonical world state if peers verify everything after connection.

---

## 24. Transport security

Even signed protocol records should travel over encrypted authenticated channels.

Possible choices:

- QUIC/TLS;
- Noise;
- libp2p secure channels.

Do not invent custom cryptography.

---

## 25. Replay protection

Records should contain enough context to reject old valid messages replayed in the wrong place.

Use:

- world ID;
- epoch;
- sequence;
- fencing token;
- nonces where appropriate;
- channel/session binding where appropriate.

---

## 26. Rate limiting

Peers can be hostile or buggy.

Limit:

- handshake attempts;
- snapshot requests;
- blob requests;
- invalid signatures;
- malformed frames;
- concurrent streams;
- decompression work;
- advertised object sizes.

---

## 27. Size limits

Every network message type needs a maximum size.

Especially:

- manifests;
- peer lists;
- signatures;
- mod lists;
- blob chunks;
- metadata.

Never allocate memory based solely on an untrusted length field.

---

## 28. Compression safety

Compressed data can cause decompression bombs.

Validate:

- declared uncompressed size;
- maximum expansion ratio;
- per-blob size;
- cumulative snapshot size.

---

## 29. Protocol state machine

A joining peer might progress through:

```text
DISCONNECTED
    |
CONNECTED
    |
AUTHENTICATED
    |
WORLD_IDENTIFIED
    |
COMPATIBILITY_CHECKED
    |
HISTORY_COMPARED
    |
SYNCING
    |
READY
    |
PLAYING / REPLICA
```

Invalid transitions should be rejected.

---

## 30. Example join

Alice is already playing.

Bob starts SwarmCraft.

```text
Bob -> DHT: peers for World X?
DHT -> Bob: Alice

Bob -> Alice: Hello
Alice -> Bob: Hello

Bob -> Alice: WorldStatus?
Alice -> Bob:
  epoch 12
  sequence 18020
  snapshot 150

Bob has:
  epoch 12
  sequence 17100
  snapshot 140

Bob downloads:
  snapshot delta / new blobs
  operations 17101..18020

Bob verifies state.

Bob -> Alice:
  replica ready

Alice records Bob as current replica.
```

---

## 31. Example authority migration

Alice is authority.

Bob is synchronized.

Alice disconnects cleanly.

```text
Alice:
  flush save
  publish final checkpoint
  announce relinquish

Bob:
  validate checkpoint
  create Epoch 13
  fencing token 13
  become authority
  start/continue Minecraft server role
```

Crash failover is harder because the relinquish message does not exist.

---

## 32. Crash failover

Alice disappears unexpectedly.

Bob and Charlie:

1. detect authority lease expiry;
2. compare latest valid states;
3. elect candidate;
4. create next epoch;
5. establish new fencing token;
6. restore from best accepted state;
7. continue.

The lease model must tolerate clock skew.

Prefer monotonic local timers for expiry observation.

---

## 33. Snapshot retention

Possible policy:

```text
latest 10 snapshots
hourly snapshots for 24h
daily snapshots for 30d
weekly snapshots for 6mo
```

Do not bake this policy into protocol consensus.

Retention is largely local configuration, provided necessary canonical evidence remains.

---

## 34. Garbage collection

Content-addressed blobs can accumulate.

A blob may be deleted locally if:

- no retained snapshot references it;
- no recovery branch references it;
- local replication policy allows deletion.

Garbage collection must never delete the only known copy of required canonical data without warning.

---

## 35. Protocol evolution

Every hashed/signed record should carry an explicit version.

Example:

```text
OperationV1
SnapshotManifestV1
EpochRecordV1
```

Do not mutate the meaning of V1 fields later.

Create V2.

---

## 36. Testing rule

Protocol code should be testable without launching Minecraft.

A deterministic simulator should create fake peers and inject:

- delay;
- loss;
- reordering;
- partitions;
- crashes;
- disk corruption;
- stale replicas.

If the protocol only works in happy-path manual Minecraft tests, it is not ready.
