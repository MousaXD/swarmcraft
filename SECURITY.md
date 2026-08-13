# SwarmCraft Security Model

## 1. Scope

SwarmCraft moves authority and world storage onto player-controlled machines.

That means every peer should be treated as potentially:

- compromised;
- buggy;
- stale;
- dishonest;
- malicious;
- partially corrupted.

The initial project should target **small trusted or semi-trusted groups**, not hostile permissionless internet-scale consensus.

---

## 2. Security goals

SwarmCraft should aim to protect:

- world integrity;
- peer identity;
- authority transitions;
- snapshot integrity;
- history integrity;
- transport confidentiality;
- replay resistance;
- recovery from corrupt replicas.

The system should make unauthorized history rewriting detectable.

---

## 3. What cryptography can prove

Cryptography can help prove:

- which key signed a record;
- whether data changed after hashing;
- whether a snapshot matches its manifest;
- whether a history references its declared predecessor.

Cryptography cannot prove:

- that the Minecraft simulation was fair;
- that an authority did not cheat;
- that a player did not use a hacked client;
- that a signed operation reflects legitimate gameplay;
- that a majority of authorized peers is honest.

Those require separate policy or simulation verification.

---

## 4. Threats

### Malicious authority

An authority may attempt to:

- spawn items;
- alter NBT;
- rewrite player inventories;
- create invalid operations;
- censor players;
- publish conflicting histories.

Mitigation depends on how deterministic and verifiable the replicated simulation becomes.

The MVP should be honest about this limitation.

### Malicious replica

A replica may:

- serve corrupt blobs;
- lie about available state;
- withhold data;
- flood requests;
- replay old messages.

Hashes, signatures, peer scoring, and limits reduce this risk.

### Identity theft

If a peer private key is stolen, the attacker can impersonate that peer.

Support:

- key revocation;
- membership changes;
- device replacement;
- optional encrypted key export.

### Split brain

Two peers may both believe they are authority.

Use:

- authority leases;
- epochs;
- fencing tokens;
- quorum rules where available;
- conservative recovery.

### Corrupt snapshot propagation

A buggy peer may produce a bad snapshot and distribute it widely.

Use:

- content hashes;
- state validation;
- historical snapshots;
- multiple replicas;
- checkpoint verification;
- rollback tools.

---

## 5. Trust model

Recommended MVP trust model:

> Participants are invited, identities are known to the world, but devices and networks may fail arbitrarily.

This is primarily a crash-fault-tolerant model with limited malicious-peer defenses.

Do not market the MVP as Byzantine fault tolerant unless it actually is.

---

## 6. Permissions

World membership and roles should be canonical state.

Example:

```text
Alice:
  ADMIN
  PLAYER
  AUTHORITY_ELIGIBLE

Bob:
  PLAYER
  AUTHORITY_ELIGIBLE

BackupPC:
  REPLICA
```

Permission changes should be signed and recorded.

---

## 7. Bootstrap security

A bootstrap server or DHT may lie about peer addresses.

That should only affect discovery.

It must not be able to declare:

- current authority;
- canonical snapshot;
- official history;
- player permissions.

Those must be verified from the world protocol.

---

## 8. Relays

Relays should see encrypted traffic where possible.

A malicious relay should at worst be able to:

- drop packets;
- delay traffic;
- observe limited metadata.

It should not be able to forge valid world records.

---

## 9. Local daemon

The local daemon is security-sensitive.

Protect IPC between Minecraft and the daemon.

Requirements:

- bind locally;
- authenticate the local client;
- use restrictive socket/file permissions;
- do not expose an unauthenticated admin API;
- validate all Minecraft-provided lengths/payloads anyway.

---

## 10. File system safety

Snapshot extraction must prevent:

- path traversal;
- symlink attacks;
- absolute-path writes;
- overwrite outside world directory.

Never trust filenames received from peers.

Prefer content-addressed internal paths generated locally.

---

## 11. Deserialization

Network parsing must be defensive.

Rules:

- cap message size before allocation;
- reject unknown mandatory fields;
- validate enums;
- reject integer overflow;
- limit nesting;
- limit collections;
- fuzz parsers.

Rust reduces memory-safety risk but does not eliminate logic bugs.

---

## 12. Denial of service

Potential attacks:

- connection floods;
- signature verification floods;
- huge manifest claims;
- endless blob requests;
- decompression bombs;
- DHT spam;
- repeated invalid elections.

Mitigation:

- rate limits;
- quotas;
- request budgets;
- bounded concurrency;
- maximum object sizes;
- reputation/temporary bans;
- proof-of-work only if ever justified, not by default.

---

## 13. Snapshot encryption

Public worlds may not need encrypted snapshots.

Private worlds may want encryption at rest/on untrusted replica nodes.

Possible future model:

```text
world data key
  encrypted separately for authorized members
```

Key rotation becomes difficult after removing a member because that member may retain old snapshots.

Be explicit:

Cryptography can prevent access to future encrypted state after rotation, but cannot make already-downloaded plaintext disappear.

---

## 14. Privacy

Peer-to-peer networking may expose IP addresses to other peers.

Documentation should say this plainly.

Possible privacy options:

- relay-only mode;
- VPN overlay;
- Tor-like routing is possible in theory but unsuitable for low-latency gameplay without serious tradeoffs.

Do not claim P2P is anonymous.

---

## 15. World deletion

In a decentralized system, "delete world everywhere" cannot be guaranteed.

Once another peer holds a copy, they may retain it.

Deletion can mean:

- delete local copy;
- revoke future authority;
- rotate keys;
- stop official replication.

This is a fundamental consequence of decentralization.

---

## 16. Rollback attacks

A stale peer may present an old but correctly signed snapshot.

Peers should remember the highest accepted epoch/checkpoint and reject unexplained rollback.

Recovery to an older snapshot should be an explicit signed/manual action.

---

## 17. Equivocation

An authority could sign two different operations for the same:

```text
world
epoch
sequence
```

That is cryptographic evidence of equivocation.

Peers should retain this evidence.

Possible consequence:

- automatic removal from authority eligibility;
- alert;
- manual review.

---

## 18. Audit log

Security-relevant events should be auditable:

- epoch changes;
- permission changes;
- key changes;
- protocol upgrades;
- branch selection;
- recovery rollback;
- peer bans.

Avoid logging secrets.

---

## 19. Secret handling

Never commit:

- private peer keys;
- world encryption keys;
- auth tokens;
- test credentials tied to real services.

Provide development-only generated fixtures.

---

## 20. Supply chain

Because SwarmCraft could run on many player machines, releases should eventually use:

- signed release artifacts;
- reproducible builds where practical;
- dependency scanning;
- lockfiles;
- SBOM generation;
- GitHub branch protection;
- CI provenance.

---

## 21. Vulnerability reporting

Before public adoption, create:

```text
SECURITY.md
```

with a private reporting route.

Do not ask researchers to disclose exploitable world-corruption vulnerabilities publicly before a fix exists.

---

## 22. Security north star

SwarmCraft should make this distinction visible:

> Decentralized storage removes a single point of failure. It does not remove the need for a clear trust model.
