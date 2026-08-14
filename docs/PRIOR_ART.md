# SwarmCraft Prior Art

SwarmCraft builds on ideas that other Minecraft projects have already explored. This document exists so contributors can learn from them and so the project does not make exaggerated novelty claims.

SwarmCraft should **not** claim that it invented P2P Minecraft, host handoff, or distributed Minecraft simulation.

Its target distinction is the combination of a peer-replicated canonical world, temporary runtime authority, no mandatory canonical cloud/master world store, and a lobby of public/private worlds with their own Minecraft versions and Fabric environments.

## Comparison

| Project | Main idea | Central dependency relevant to world state | What SwarmCraft learns |
|---|---|---|---|
| SharedWorld | Player-hosted world with automatic host replacement | Google Drive synchronization and backend services | Host handoff and reconnect UX |
| World Host | Easy access to a player's singleplayer world | Discovery/proxy infrastructure | Connectivity and onboarding |
| e4mc | Makes a local Minecraft world reachable | Tunnel/relay infrastructure | Separate connectivity from authority |
| Seamless Serverless Multiplayer | P2P-style hosting, NAT traversal and relay fallback | Session/connectivity infrastructure | NAT traversal, invites and compatibility UX |
| MultiPaper | One world across multiple Minecraft server processes | MultiPaper-Master stores/co-ordinates world data | Chunk ownership and distributed ticking |
| Mammoth / WorldQL | Multiple Minecraft processes sharing synchronized state | WorldQL stores permanent world changes | Region/server distribution lessons |

---

## SharedWorld

Project: https://modrinth.com/mod/sharedworld

SharedWorld is the closest prior project to SwarmCraft's early host-migration experience. Its published description says the world is synchronized through Google Drive, the first player becomes host, and another online player is selected when the host leaves.

That means this experience already has real prior art:

```text
Alice hosts
Alice leaves
Bob becomes host
players reconnect
```

SwarmCraft should study its lifecycle, handoff and reconnect behavior.

The architectural difference is persistence. SwarmCraft wants the canonical state replicated directly among peers instead of requiring a canonical Google Drive copy.

```text
SwarmCraft target

      World ID
         |
 canonical history
         |
 +-------+-------+
 |       |       |
Alice   Bob   Charlie
copy    copy     copy
```

---

## MultiPaper

Repository: https://github.com/MultiPaper/MultiPaper

MultiPaper distributes one Minecraft world across multiple server processes. Its documentation describes chunk caching, synchronization and ownership, while MultiPaper-Master stores world files/data and coordinates which server ticks chunks.

This is highly relevant to SwarmCraft's **future** region-authority research.

Study:

- chunk ownership;
- ownership transfer;
- synchronized block changes;
- subscribed chunk updates;
- distributed ticking;
- behavior when simulation nodes change.

SwarmCraft differs because it does not want a permanently required master to remain the canonical world authority/storage node.

---

## Mammoth / WorldQL

Architecture article: https://worldql.com/posts/2021-08-worldql-scalable-minecraft/

Mammoth explored distributing Minecraft across multiple server processes. Early versions geographically divided the map; later versions used WorldQL to store permanent world changes and synchronize server processes.

This is important because it documents both useful ideas and painful failure modes of geographical partitioning.

Study:

- region partitioning;
- entity movement between servers;
- cross-server player visibility;
- synchronization of permanent world changes;
- why simple geographical slicing was not sufficient.

SwarmCraft's durable state, however, is intended to be peer-replicated instead of residing in a mandatory WorldQL service.

---

## World Host

Project: https://modrinth.com/mod/world-host

World Host makes singleplayer worlds accessible to friends without normal dedicated-server setup. Its documented design focuses on finding a route to the current host, including direct and proxy-style connectivity.

SwarmCraft should learn from its onboarding and networking UX.

The difference is that reachability does not answer:

> Who holds the latest valid world after the current host disappears?

SwarmCraft adds persistent world identity, replication, host migration and sleep/wake recovery.

---

## e4mc

SharedWorld explicitly uses e4mc for connectivity. The architectural lesson is important even if SwarmCraft uses a different networking implementation:

```text
connectivity != world authority
```

A relay, tunnel, bootstrap node or NAT traversal service may help peers find and reach one another without becoming trusted as the source of truth for the world.

SwarmCraft should preserve this boundary throughout the protocol.

---

## Seamless - Serverless Multiplayer

Project: https://modrinth.com/mod/seamless-serverless-multiplayer

Its published description includes direct connections, NAT traversal/hole punching, relay fallback, invite codes and mod compatibility checks.

These are relevant references for SwarmCraft's player-facing connectivity layer.

SwarmCraft's additional problem is durable replicated state and automatic world authority migration.

---

## What SwarmCraft should actually claim

Avoid broad claims such as:

- "first P2P Minecraft";
- "first Minecraft host migration";
- "first distributed Minecraft world".

A more precise target is:

1. persistent cryptographic World IDs;
2. peer-replicated snapshots/content-addressed world data;
3. no mandatory canonical cloud save or master world store;
4. temporary/elected runtime authority;
5. automatic host migration;
6. world sleep when nobody is online;
7. recovery when a valid replica returns;
8. public, unlisted and private worlds;
9. per-world Minecraft versions;
10. per-world Fabric mod environments;
11. optional background seeding without Minecraft running;
12. future region-based distributed simulation.

The project should prove this combination through working software rather than relying on novelty language.

---

## Engineering lessons

### Networking and state are separate layers

Peer discovery, relays, hole punching and NAT traversal must not define canonical world state.

### Host migration alone is not the product

SharedWorld demonstrates that host replacement can already be done. SwarmCraft's harder differentiator is peer-native world persistence and the world/lobby model.

### Distributed Minecraft simulation is a later milestone

MultiPaper and Mammoth show how many Minecraft-specific problems appear once simulation itself is distributed. SwarmCraft should first make replicated persistence, recovery and authority migration reliable.

### Central coordination is tempting for a reason

Existing distributed-Minecraft projects often use central coordinators because ordering and consistency become much easier. SwarmCraft deliberately chooses a harder constraint, so partition handling must be conservative and explicit.

---

## Licensing

Related projects use different licenses. Public source does not automatically mean code can be copied into an Apache-2.0 project.

Before reusing code, contributors must inspect the relevant license and satisfy its requirements. When compatibility is unclear, learn from the architecture and implement the SwarmCraft design independently.

---

## Keep this document alive

Add new projects when they meaningfully overlap with:

- Minecraft host handoff;
- peer-hosted multiplayer;
- replicated world storage;
- distributed save synchronization;
- leader/authority migration;
- distributed Minecraft ticking;
- modpack synchronization;
- decentralized persistent game worlds.
