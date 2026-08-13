# SwarmCraft Product Vision

## SwarmCraft is not just server hosting

SwarmCraft is intended to become a **decentralized Minecraft world platform and lobby**.

The core object in SwarmCraft is not a rented server machine. It is a persistent **world swarm** with its own identity, rules, version, mods, privacy policy, membership and replicated state.

Players open SwarmCraft and discover or return to worlds. A world may be public or private. A temporary peer may currently execute the Minecraft simulation, but no permanent machine is the owner of the world's canonical existence.

```text
                         SwarmCraft

                    decentralized lobby
                           |
           +---------------+---------------+
           |               |               |
           v               v               v
      Public World    Private World    Friends World
           |               |               |
      Fabric mods      Fabric mods      Fabric mods
      MC version       MC version       MC version
      own rules        invite only      own rules
           |               |               |
           +------- peer-hosted worlds ----+
                           |
                 replicated world state
```

## Product principle

> Players should choose a world because they like the world, its community, rules, Minecraft version and mods, not because they know which computer currently hosts it.

The current authority is infrastructure. It should normally be invisible to the player.

---

## The lobby

SwarmCraft should provide a native lobby/world browser rather than behaving like a simple `Open to LAN` replacement.

A user's home screen may eventually contain:

```text
SwarmCraft

Continue
  Hardcore Friends SMP       3 online
  Create Everything          sleeping

Public Worlds
  Vanilla+ Community         18 online
  Create Tech                7 online
  Old School 1.20.1          5 online

Friends
  Mousa       playing Create Tech
  Alice       offline
  Bob         playing Vanilla+
```

The lobby is a discovery and presentation layer. It must not become the canonical database for world state.

A centralized service may optionally help with search, indexing, moderation or bootstrap discovery, but the world should remain recoverable without that service when peers can locate one another through another supported discovery mechanism.

---

## World types

### Private world

A creator may create an invite-only swarm.

Possible controls:

- invite keys or invite links;
- explicit membership;
- friends-only membership;
- authority eligibility restrictions;
- optional password/passphrase layer;
- owner/admin moderation permissions.

Private does not mean centralized. Authorized peers can still replicate the world and take over authority according to the world's policy.

### Public world

A creator may publish a world to the SwarmCraft discovery layer.

A public world can advertise metadata such as:

- name;
- description;
- icon/banner;
- Minecraft version;
- Fabric modpack;
- gameplay tags;
- language;
- current online players;
- world rules;
- join policy;
- approximate region/latency information;
- required storage size;
- protocol version.

The listing is not the source of truth for the world. It is only a way to find the swarm.

### Unlisted world

A useful middle option:

- not shown in public search;
- joinable through a World ID or invite;
- still decentralized.

---

## Every world has its own identity

Each world receives a stable cryptographic World ID.

Example:

```text
scworld:7f22c9...
```

The World ID should bind to a genesis record containing the initial protocol/world configuration.

A human-friendly name may change without changing the underlying World ID.

This distinction matters because two worlds may both be called `Survival SMP`, while their histories remain cryptographically distinct.

---

## Per-world Minecraft version

Worlds choose their Minecraft version.

Example lobby entries:

```text
Create Everything
Minecraft 1.21.1

Old School Tech
Minecraft 1.20.1

Experimental Survival
Minecraft 1.21.11
```

The SwarmCraft launcher should eventually install or select the correct compatible Minecraft instance automatically.

A peer that does not have a compatible runtime may still be able to act as a storage replica, but it must not become simulation authority unless it can run the exact required environment.

---

## Fabric first

For the initial SwarmCraft protocol and preview releases, **Fabric is the supported Minecraft mod loader**.

This is a deliberate scope decision.

Supporting multiple loaders before the world replication, migration and recovery model is reliable would multiply compatibility work at the worst possible time.

Initial support:

```text
Minecraft Java Edition
        +
      Fabric
```

Potential future adapters may include other loaders, but they are not part of the initial compatibility promise.

The distributed SwarmCraft core should remain loader-independent so additional adapters can be introduced later without redesigning peer identity, snapshot storage, world discovery or authority election.

---

## Per-world mods

A world may define its own Fabric mod set.

Example:

```text
World: Create Everything
Minecraft: 1.21.1
Loader: Fabric

Required mods:
  Fabric API
  Create
  Lithium
  FerriteCore
  Simple Voice Chat
```

The world configuration should include a deterministic compatibility manifest/fingerprint.

The launcher can compare the local instance against that manifest before joining.

Long term, the player experience should approach:

```text
Click Join
   |
SwarmCraft checks version
   |
checks Fabric
   |
checks required mods
   |
prepares isolated instance
   |
connects to swarm
   |
Play
```

This should be implemented carefully around mod redistribution licenses. SwarmCraft must not blindly redistribute third-party mod JARs when their licenses do not allow it. A manifest may instead reference approved distribution sources and hashes.

---

## World manifest

A world should have a signed configuration describing its execution environment.

Conceptually:

```text
WorldManifest {
    world_id
    minecraft_version
    loader = FABRIC
    loader_version
    protocol_version
    mods[]
    datapacks[]
    world_rules
    visibility
    membership_policy
    authority_policy
}
```

Changes to compatibility-critical configuration should be represented as canonical world configuration changes rather than mutable metadata on a central website.

---

## Modpack identity

Do not identify a modpack only by its display name.

Use content/version hashes.

Conceptually:

```text
ModRequirement {
    project_id
    version
    artifact_hash
    required_side
}
```

The simulation compatibility fingerprint can be derived from all server-relevant components.

```text
compatibility_fingerprint = HASH(
    minecraft_version,
    loader_version,
    server_relevant_mods,
    datapacks,
    SwarmCraft_adapter_version
)
```

Peers with a different fingerprint must not automatically become authority.

---

## Players can create their own worlds

Creating a world should eventually feel closer to creating a Discord server than renting a VPS.

Possible flow:

```text
Create World
   |
Name
   |
Public / Unlisted / Private
   |
Minecraft version
   |
Fabric mods / modpack
   |
World settings
   |
Create
```

SwarmCraft then creates:

- World ID;
- genesis record;
- authority policy;
- local Minecraft instance;
- initial snapshot;
- invite/discovery metadata.

The user's machine starts as the first authority because somebody has to execute the initial world, not because it permanently owns the world.

---

## Anyone can create a different kind of community

SwarmCraft should not prescribe one Minecraft experience.

Examples:

```text
Vanilla survival
Create engineering
Cobblemon
Hardcore
Anarchy
Creative building
Adventure maps
Technical Minecraft
Private friend SMP
Public community SMP
```

Each world controls its own rules and compatibility environment.

The protocol provides persistence, discovery, replication and authority migration. It should not become a gameplay-policy dictator.

---

## Decentralized persistence

A world's canonical state is replicated among peers.

```text
World X
  |
  +-- Alice: replica
  +-- Bob: replica + current authority
  +-- Charlie: replica
  +-- Dave: background seed
```

If Bob disappears, another eligible synchronized peer can take authority.

If everyone disappears, the world sleeps.

When a peer returns, the world wakes from the latest valid canonical state available to the swarm.

---

## Authority eligibility

Not every peer has to be capable of hosting every world.

A device may participate as:

```text
PLAYER
REPLICA
AUTHORITY_ELIGIBLE
BACKGROUND_SEED
SPECTATOR
```

Authority eligibility may depend on:

- compatible Minecraft version;
- compatible Fabric environment;
- correct required mods;
- complete enough world state;
- world membership permissions;
- sufficient resources;
- protocol compatibility.

A low-powered machine may still strengthen the swarm by storing snapshots without ever executing Minecraft.

---

## Background seeding

Eventually SwarmCraft should allow:

```text
Minecraft: OFF
SwarmCraft: ON
```

A machine could still:

- store encrypted/private world replicas where permitted;
- seed snapshot blobs;
- participate in discovery;
- assist peer connectivity;
- preserve sleeping worlds.

This makes the system more torrent-like without requiring the user to run Minecraft continuously.

---

## Discovery is allowed to be convenient

Decentralization does not require making the UX miserable.

SwarmCraft can operate optional public services for:

- lobby search;
- featured worlds;
- abuse reports;
- discovery bootstrap;
- relay bootstrap;
- update metadata.

But these services must be **non-authoritative** regarding canonical world state.

The important boundary is:

```text
Central service may say:
"Here are peers for World X."

Central service must not be required to say:
"This is the true state of World X."
```

---

## Public-world moderation

Public discovery introduces problems that private friend worlds do not have.

The platform will eventually need policies for:

- spam worlds;
- malicious modpacks;
- impersonation;
- illegal/abusive listings;
- malware links;
- misleading metadata;
- ban evasion.

Platform moderation and world governance are separate concepts.

A world can govern its players independently, while SwarmCraft's public directory can govern what it is willing to index or promote.

Removing a listing from the public lobby should not magically erase a decentralized world from machines that already possess it.

---

## World ownership vs governance

SwarmCraft should avoid confusing these concepts:

### Infrastructure ownership

No permanent peer owns the canonical machine.

### World governance

A world can still have admins, operators, moderators and membership rules.

Decentralization does not mean every random peer receives operator permissions.

A private world creator may choose a governance policy where they retain admin rights while runtime authority migrates between approved peers.

---

## v0.1.0-preview product scope

The preview should prove the infrastructure before attempting the entire social platform.

Required foundation:

- Fabric-only Minecraft integration;
- per-world Minecraft version metadata;
- per-world mod compatibility manifest/fingerprint;
- private/invite world creation;
- basic public/unlisted visibility metadata;
- replicated snapshots;
- automatic host migration;
- world sleep/wake;
- Windows and Linux;
- CLI/debugging tools;
- minimal world list UI.

The preview does not need a massive public discovery network on day one.

A simple bootstrap/index service can be introduced while the canonical state remains peer-controlled.

---

## Future lobby experience

The north-star UX is:

```text
                 SWARMCRAFT

Discover     Friends     My Worlds     Create

------------------------------------------------
Create: Astral Engineering
1.21.1 | Fabric | 24 mods
12 online | Public | 5 healthy replicas
[ Join ]
------------------------------------------------
Vanilla Forever
1.21.11 | Fabric | Vanilla+
39 online | Public | 11 healthy replicas
[ Join ]
------------------------------------------------
Friends SMP
1.20.1 | Fabric | Private
2 online | 4 replicas
[ Join ]
------------------------------------------------
```

The player should not need to know:

- current host IP;
- router configuration;
- which friend owns the save folder;
- which machine was the original server;
- where the canonical cloud backup lives.

Ideally, there is no canonical cloud backup.

---

## Long-term north star

SwarmCraft begins by decentralizing **world existence and host authority**.

Later research may decentralize **simulation itself** through region or chunk authorities.

Those are different milestones.

```text
Stage 1
Decentralized persistence
        +
automatic host migration

Stage 2
Rich decentralized lobby/platform
        +
world-specific versions/modpacks

Stage 3
Distributed region simulation
```

The project should not delay a useful Stage 1 and Stage 2 product while chasing the much harder Stage 3 problem.

---

## Product north star

> SwarmCraft should feel like browsing communities, not browsing servers.

And beneath that friendly lobby, the world should remain able to survive the disappearance of the machine, person or service that originally created it.
