# Contributing to SwarmCraft

Thanks for helping explore decentralized Minecraft infrastructure.

SwarmCraft is an experimental distributed-systems project. Correctness and recoverability matter more than cleverness.

---

## Before contributing

Please read:

1. `README.md`
2. `ARCHITECTURE.md`
3. `PROTOCOL.md`
4. `SECURITY.md`
5. `ROADMAP.md`

---

## Early contribution areas

Useful work includes:

- protocol review;
- Rust networking;
- snapshot storage;
- content-addressed blobs;
- Fabric integration;
- Minecraft save consistency research;
- deterministic simulation tests;
- NAT traversal;
- crash recovery;
- fuzzing;
- security review;
- documentation.

---

## Development philosophy

### Prefer boring correctness

If one implementation is elegant but difficult to recover after a crash, and another is boring but obviously recoverable, choose the boring one.

### Never hide a fork

If two histories conflict, detect it.

Do not silently pick whichever file is newer.

### Never use wall-clock time as canonical ordering

Clocks drift.

### Assume messages can be

- delayed;
- duplicated;
- reordered;
- dropped.

### Assume processes can die between any two instructions

Especially during:

- snapshot publication;
- log append;
- authority transition;
- blob download;
- database commit.

### Assume disks can return corrupt data

Verify content hashes.

---

## Suggested coding stack

Core:

```text
Rust
Tokio
QUIC / libp2p
BLAKE3
Ed25519
Zstd
```

Minecraft:

```text
Java
Fabric
```

Do not add major protocol dependencies without explaining why they are necessary.

---

## Repository layout

Proposed:

```text
crates/
  swarm-core/
  swarm-network/
  swarm-storage/
  swarm-protocol/
  swarm-cli/

minecraft/
  fabric/

tests/
  integration/
  recovery/
  partition/
```

---

## Pull requests

A good PR should explain:

- problem;
- proposed behavior;
- protocol impact;
- failure behavior;
- compatibility impact;
- tests.

For distributed-state changes, include at least one failure-case test.

---

## Protocol changes

Protocol changes require extra care.

If a field is signed or hashed, changing its semantics can invalidate compatibility.

Rules:

- version wire records explicitly;
- do not silently repurpose existing fields;
- document migration behavior;
- add test vectors where practical.

---

## Testing expectations

Tests should cover happy path and failure path.

Examples:

```text
authority crashes after local write
authority crashes before replication
peer reconnects with stale state
blob hash mismatch
partition creates competing candidates
snapshot interrupted halfway
disk fills during snapshot
duplicate operation arrives
old authority returns
```

Property-based testing is strongly encouraged for state-machine logic.

---

## Minecraft integration

Minecraft code should be treated as an adapter.

Avoid putting distributed protocol logic directly into Fabric event handlers.

Prefer:

```text
Minecraft event
  -> normalized local message
  -> SwarmCraft core
```

This keeps the core independently testable.

---

## Security

Never introduce:

- unauthenticated remote admin endpoints;
- custom cryptographic algorithms;
- unchecked archive extraction;
- unbounded network allocation;
- secret keys in logs;
- trust based on IP address alone.

Report serious vulnerabilities privately once a security reporting channel exists.

---

## Commit style

No strict format is required initially.

Prefer descriptive commits such as:

```text
protocol: add signed epoch record
storage: verify blob hash on read
network: resume interrupted blob transfer
fabric: expose save-complete event
```

---

## Design discussions

For large architectural changes, open an issue or design document before implementing.

Useful questions:

- What happens if the authority dies here?
- What happens if this message arrives twice?
- What happens if two peers disagree?
- Can this state be recovered after power loss?
- Is this record authenticated?
- Is ordering deterministic?
- Does this create a new central dependency?

---

## Project rule

Do not call something decentralized merely because it uses peer-to-peer networking.

The world itself must not depend on one mandatory machine or service for authority or persistence.
