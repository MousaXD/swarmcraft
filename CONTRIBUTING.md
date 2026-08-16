# Contributing to SwarmCraft

Thanks for helping build decentralized Minecraft infrastructure.

SwarmCraft is an experimental distributed-systems project in active technical preview. Correctness, recoverability and clear failure behavior matter more than cleverness.

---

## Before contributing

Please read, in this order:

1. `README.md`
2. `docs/IMPLEMENTATION_STATUS.md`
3. `ARCHITECTURE.md`
4. `PROTOCOL.md`
5. `SECURITY.md`
6. `ROADMAP.md`
7. `AGENTS.md` when agent/frontend guidance is relevant

The implementation-status document is important because some roadmap/product-vision documents are intentionally aspirational.

---

## Useful contribution areas now

High-value work includes:

- automatic Minecraft runtime launch after authority recovery;
- player reconnection/host-migration UX;
- complete manual authority-transfer workflow;
- crash/recovery soak testing;
- disk-full and corruption tests;
- malicious-peer/fuzz/property testing;
- snapshot replication efficiency and retention;
- NAT traversal field validation;
- relay/bootstrap deployment tooling;
- Minecraft/Fabric/modpack preparation;
- desktop UX and accessibility;
- protocol/security review;
- documentation and release discipline.

Distributed region/tick simulation is intentionally not an MVP priority.

---

## Development philosophy

### Prefer boring correctness

If one implementation is elegant but difficult to recover after a crash, and another is boring but obviously recoverable, choose the boring one.

### Never hide a fork

If two histories conflict, detect and preserve the conflict.

Do not silently pick whichever file is newer.

### Never use wall-clock time as canonical ordering

Clocks drift. Canonical ordering must come from protocol state, not local clock confidence.

### Assume messages can be

- delayed;
- duplicated;
- reordered;
- dropped.

### Assume processes can die between any two instructions

Especially during:

- snapshot publication;
- membership changes;
- authority transition;
- recovery voting;
- blob download;
- persistence/rename/fsync boundaries;
- Minecraft save/shutdown barriers.

### Assume disks can return corrupt data

Verify content hashes and signed state before trusting persisted material.

### Distinguish control-plane correctness from product completeness

A protocol type or state-machine transition is not the same as a complete player-facing feature.

For example, authority recovery logic may be correct while automatic Minecraft launch/reconnection is still incomplete. Documentation and PR descriptions should preserve that distinction.

---

## Current implementation stack

Core/runtime:

```text
Rust
Tokio
libp2p / QUIC
BLAKE3
Ed25519
postcard / serde
Zstd
```

Minecraft integration:

```text
Java
Fabric
loopback IPC
```

Desktop:

```text
Tauri 2
HTML
CSS
plain JavaScript
Rust sidecar/runtime process management
```

Do not add major protocol/runtime dependencies without explaining why they are necessary.

For desktop work, follow the constraints in `AGENTS.md`; do not introduce a frontend framework or build stack merely to restyle the application.

---

## Repository layout

Current high-level layout:

```text
apps/
  desktop/

crates/
  swarm-cli/
  swarm-consensus/
  swarm-core/
  swarm-ipc/
  swarm-network/
  swarm-protocol/
  swarm-storage/

minecraft/
  fabric/

docs/
  implementation/recovery/release/network/product documents

.github/workflows/
  CI, installers, release and version guards
```

Keep distributed protocol logic in the Rust core/runtime layers rather than embedding it into desktop DOM code or Fabric lifecycle callbacks.

---

## Pull requests

A good PR should explain:

- the problem;
- proposed behavior;
- protocol impact;
- persistence impact;
- failure behavior;
- compatibility impact;
- user-facing impact;
- tests/evidence.

For distributed-state changes, include at least one failure-case test.

For product/UI work, state whether the change alters only presentation or changes actual protocol/runtime behavior.

---

## Protocol changes

Protocol changes require extra care.

If a field is signed, hashed, canonically encoded or stored durably, changing its semantics can invalidate compatibility.

Rules:

- version wire/state records explicitly where required;
- do not silently repurpose existing fields;
- preserve enum discriminant compatibility where the current wire format depends on ordering;
- document migration behavior;
- add test vectors/round-trip tests where practical;
- explain replay, stale-state and downgrade behavior;
- update `PROTOCOL.md`, `SECURITY.md` and implementation-status docs when claims change.

Application version and wire protocol version are separate. Do not bump the wire protocol only to match an application release number.

---

## Testing expectations

Tests should cover happy path and failure path.

Examples:

```text
authority crashes after local write
authority crashes before replication
recovery successor dies mid-transition
peer reconnects with stale authority
blob hash mismatch
partition creates competing candidates
snapshot interrupted halfway
disk fills during persistence
duplicate request arrives
old authority returns
solo branches diverge
Fabric save barrier fails
runtime permit expires
```

Property-based testing is strongly encouraged for state-machine logic.

Process-level tests are strongly preferred for claims involving daemon coordination, IPC, authority recovery or runtime lifecycle.

See `docs/RELEASE_GATES.md` for permanent acceptance expectations.

---

## Minecraft integration

Minecraft code should remain an adapter around the distributed core.

Prefer:

```text
Minecraft lifecycle/event
  -> authenticated local IPC
  -> SwarmCraft runtime/core
  -> durable/replicated protocol state
```

Do not put membership, recovery election or canonical-history policy directly into Fabric event handlers.

The Fabric bridge should focus on:

- lifecycle observation;
- save barriers;
- compatibility/runtime reporting;
- authority-permit enforcement;
- controlled shutdown.

---

## Desktop integration

The desktop UI is a player-facing shell, not the source of truth.

It may invoke runtime operations and explain state, but it must not reinterpret safety semantics for convenience.

Keep distinctions explicit:

- canonical vs. solo/degraded vs. conflicted;
- authority vs. storage replica;
- membership vs. discovery;
- compatible authority candidate vs. storage-only peer;
- local process state vs. canonical world state.

---

## Networking validation

Having AutoNAT, DCUtR or relay code is not proof that all consumer networks work.

When contributing network/NAT changes:

- add deterministic/local coverage where possible;
- record real-network validation separately;
- sanitize logs before attaching evidence;
- never expose invite secrets or private keys;
- update `docs/NETWORK_VALIDATION.md` only when the tested environment and commit/version are known.

---

## Security

Never introduce:

- unauthenticated remote admin endpoints;
- custom cryptographic algorithms without strong justification/review;
- unchecked archive extraction;
- unbounded network allocation;
- secret keys/invite secrets in logs;
- trust based on IP address alone;
- stale authority writes accepted for liveness convenience.

Read `SECURITY.md` before changing authentication, membership, recovery, storage acceptance or authority behavior.

---

## Commit style

No single commit-prefix scheme is mandatory, but use descriptive commits.

Examples:

```text
protocol: add bounded recovery certificate validation
storage: verify blob hash on resumed transfer
network: record relay fallback diagnostics
fabric: enforce authority permit heartbeat
ui: clarify solo and conflict safety states
docs: refresh implementation status
```

---

## Design discussions

For large architectural changes, open an issue/design document before implementation when practical.

Useful questions:

- What happens if the authority dies here?
- What happens if this message arrives twice?
- What happens if two peers disagree?
- Can this state be recovered after power loss?
- Is this record authenticated?
- Is ordering deterministic?
- Is state persisted before it becomes externally authoritative?
- Does this create a new central dependency?
- Does the UI claim more safety than the protocol guarantees?

---

## Project rule

Do not call something decentralized merely because it uses peer-to-peer networking.

The world itself must not depend on one mandatory machine or service for authority or persistence.
