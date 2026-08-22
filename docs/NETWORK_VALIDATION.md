# Network connectivity validation

SwarmCraft includes libp2p support for direct connections, mDNS, Kademlia, AutoNAT, DCUtR hole punching and circuit relays. Presence of those code paths is not the same as proving every consumer NAT, carrier or firewall environment works.

This document separates three things that are easy to blur together:

1. behavior implemented in the networking backend;
2. behavior exercised by automated CI or synthetic impairment;
3. behavior that still requires real-machine field certification.

## Intended path order

For a peer with known direct addresses and an available relay, the preferred order is:

```text
LAN / direct address
        ↓
known or observed public address
        ↓
DCUtR direct upgrade where a relayed connection exists
        ↓
relay transport
```

`SwarmNode::dial_with_relay_fallback` makes the direct-first rule explicit for a dial plan. It filters circuit addresses out of the direct candidate set, attempts the direct candidates first, and makes at most one relay fallback attempt if that direct dial fails. Calling the method again replaces the previous plan. There is no application-level infinite retry loop.

DCUtR continues to use libp2p's own bounded hole-punch upgrade attempts. AutoNAT uses fixed probe intervals rather than a tight retry loop.

A relay can carry encrypted peer traffic but cannot define world identity, membership, recovery votes, snapshots, fencing tokens or canonical history. Relay loss is a transport-path failure, not an authority transition. No single relay is required for a world's existence.

## Structured connectivity diagnostics

`ConnectivityDiagnosticsV1` remains serializable backend state rather than frontend presentation text. The diagnostic snapshot includes the existing low-level fields and an explicit `ConnectivityStateV1`:

- `NatStatusUnknown`: AutoNAT has not established public/private reachability and no stronger path state is known;
- `DirectReachable`: a non-relayed peer path succeeded, or AutoNAT classified the node as public;
- `HolePunched`: DCUtR reported a successful direct upgrade;
- `RelayConnected`: a circuit-relay reservation or circuit is available;
- `RelayRequired`: a relay is configured and private reachability or a failed direct/hole-punch path makes fallback necessary;
- `PrivateUnreachable`: AutoNAT reports private and no usable direct or relay path is known;
- `BootstrapUnavailable`: the current actionable failure is bootstrap discovery/connectivity;
- `NoViablePath`: a bounded direct-plus-relay dial plan exhausted both paths.

The same snapshot records:

- local listen addresses;
- observed public address;
- NAT status: unknown/public/private;
- direct and relay connectivity flags;
- hole-punch state: idle/attempting/succeeded/failed;
- selected relay and number of configured relay peers;
- number of configured bootstrap peers and whether bootstrap connectivity has succeeded;
- whether an IPv4 or IPv6 multiaddress has actually been observed by the node;
- the most recent failure;
- a bounded history of the eight most recent structured connectivity issues, including peer/address context where available.

Failure details are length-bounded before being retained. This keeps diagnostics useful without allowing an unbounded error history to grow during a bad network period.

### NAT and carrier interpretation

AutoNAT `Private` means the node was not proven publicly dialable by the AutoNAT probes. It does **not** reliably distinguish ordinary home NAT, symmetric NAT, CGNAT, a mobile carrier policy or a local firewall. Those environment classes must be recorded by the field tester or known network operator.

Likewise, `ipv6_available` means an IPv6 multiaddress was observed by SwarmCraft. It is useful evidence, but it is not by itself proof that an end-to-end IPv6 path to the tested peer works.

## Typed libp2p event handling

NAT, relay and DCUtR state are derived from the typed events in the pinned rust-libp2p revision:

- AutoNAT `StatusChanged` drives public/private/unknown state;
- DCUtR's result drives hole-punch success/failure;
- relay reservation/circuit events drive relay connectivity;
- `ConnectedPoint::is_relayed()` distinguishes relayed from non-relayed established connections;
- `OutgoingConnectionError` is classified as a direct, relay or bootstrap connectivity issue where the peer role is known.

This avoids relying on the formatting of libp2p `Debug` strings for production diagnostics.

## Bootstrap and relay configuration

Comma-separated bootstrap and relay multiaddresses are still read from:

- `SWARMCRAFT_BOOTSTRAP`;
- `SWARMCRAFT_RELAY`.

Bootstrap and relay addresses that are used as configured infrastructure must contain `/p2p/<peer-id>`. Missing peer IDs are rejected and reported as structured `InvalidAddress` diagnostics when added programmatically. Syntactically invalid environment multiaddresses remain startup configuration errors rather than being silently ignored.

Kademlia bootstrap peers and relay peers are tracked separately so an outbound infrastructure failure can be reported as `BootstrapUnavailable` or `RelayUnavailable` instead of a generic connection string.

## Field connectivity probe

The repository includes a small networking-only probe that runs the real `SwarmNode`, opens IPv4 QUIC, attempts IPv6 QUIC when the host supports it, consumes the normal bootstrap/relay environment configuration, and prints diagnostic snapshots as network events arrive.

Basic probe:

```bash
SWARMCRAFT_PROBE_SECONDS=60 \
  cargo run -p swarm-network --example connectivity_probe --locked
```

Probe a known peer directly:

```bash
SWARMCRAFT_PROBE_SECONDS=60 \
SWARMCRAFT_PROBE_TARGET='/dns4/peer.example/udp/4001/quic-v1/p2p/<peer-id>' \
  cargo run -p swarm-network --example connectivity_probe --locked
```

Probe with configured discovery/relay infrastructure:

```bash
SWARMCRAFT_BOOTSTRAP='/dns4/bootstrap.example/udp/4001/quic-v1/p2p/<bootstrap-peer-id>' \
SWARMCRAFT_RELAY='/dns4/relay.example/tcp/4001/p2p/<relay-peer-id>' \
SWARMCRAFT_PROBE_SECONDS=90 \
  cargo run -p swarm-network --example connectivity_probe --locked
```

The probe duration is bounded to 5 through 300 seconds. It is intentionally a connectivity probe, not proof of world replication, authority correctness or universal NAT traversal. Use the full daemon/world exercise below for certification records.

## Automated coverage

The repository's automated networking evidence includes:

- signed application authentication over QUIC/libp2p;
- direct-path diagnostics on a real loopback QUIC connection;
- persisted transport identity across a hard peer restart;
- replacement connections racing stale/dead connections;
- re-authentication and authenticated requests after reconnect;
- invalid bootstrap-address rejection with structured diagnostics;
- connectivity-state transition tests for public/private NAT, direct, hole-punched, relay-required, relay-connected, bootstrap-unavailable and no-viable-path states;
- bounded connectivity failure history;
- chunked transfer resume negotiation after reconnect;
- a receiver committing a chunk while its acknowledgement is deliberately lost;
- resume from the receiver's committed offset rather than retransmitting already accepted data;
- deterministic byte-for-byte payload verification for every received chunk;
- Linux `tc netem` impairment with latency variation, packet loss and bandwidth limiting.

The normal CI matrix contains a dedicated **Network impairment (QUIC resume)** job. It transfers 64 MiB while repeatedly hard-restarting the sender every 16 MiB under WAN-like loopback impairment.

The separate **Network Soak** workflow runs the same protocol path at multi-gigabyte scale. Its default profile transfers **2 GiB**, forces a hard sender restart every **256 MiB**, deliberately loses the final acknowledgement before each restart, renegotiates the committed resume offset, and continues until the full transfer completes. The workflow can also run 1 GiB and 5 GiB profiles.

These tests are strong evidence for transport authentication, replacement connections, bounded diagnostics and resume behavior. Synthetic impairment is still not equivalent to a residential router, CGNAT carrier, mobile network, ISP IPv6 path or UDP-blocking enterprise network.

## CI-proven behavior versus field certification

The following distinction is intentional.

### Suitable for automated CI evidence

- direct loopback QUIC connectivity and signed authentication;
- stale connection replacement after process restart;
- bounded failure/state-machine behavior;
- invalid-address handling;
- transfer resume after disconnect/lost acknowledgement;
- latency/loss/bandwidth impairment with `tc netem`;
- compile-time use of the pinned typed AutoNAT/DCUtR/relay event APIs.

### Still requires representative real networks

- two independently NATed residential networks;
- ordinary cone/restricted NAT diversity across consumer routers;
- symmetric/strict NAT behavior;
- CGNAT carriers;
- mobile hotspots and mobile carrier filtering;
- routable IPv6 between independent ISPs;
- networks where UDP/QUIC is blocked or heavily shaped;
- relay fallback through the intended production/community relay deployment;
- replacing a failed relay with another viable relay/path in the field.

## Manual real-world validation matrix

Do not change a row to **Verified** unless it has been exercised between real machines/networks and the evidence is recorded with the tested SwarmCraft commit/version.

| Scenario | Expected path | Certification status |
| --- | --- | --- |
| Same LAN | mDNS + direct QUIC | Automated local coverage; real-router record still recommended |
| Normal home NAT to reachable public peer | direct/observed address, possibly hole punch | Field certification required across representative routers |
| Two independently NATed home peers | DCUtR hole punch, then relay fallback | Field certification required |
| Symmetric/strict NAT | relay likely; hole punch only if the NATs permit it | Field certification required |
| CGNAT | relay likely; hole punch where carrier permits | Field certification required |
| Mobile hotspot | direct/hole punch/relay depends on carrier | Field certification required |
| IPv6 peers | direct IPv6 where routable | Field certification required across representative ISPs |
| UDP blocked | TCP/relay path if configured and reachable | Field certification required |
| Direct path fails but relay is reachable | one bounded relay fallback for an explicit dial plan | Real relay deployment test required |
| Relay disappears after world was replicated | another direct/relay path may reconnect; canonical world history remains independent of relay | Multi-path/multi-relay field validation required |

## Manual authority transfer under adverse networks

The Desktop transfer wizard drives six signed stages (`prepare`, `export`, `accept`, `commit`, `activate`, `observe`) between two devices. Each stage is validated by the Rust backend against canonical membership, the exact checkpoint, signatures and generation fencing, so a transport disruption fails safely instead of producing a divergent world. The following checklist records what must be exercised before transfer acceptance can be claimed across real networks. None of these rows are certified yet.

For each scenario, run one complete host transfer from the Desktop wizard on both devices and record:

1. which stage failed or stalled, if any, and the exact wizard error text;
2. that no prepared/accepted/committed token was accepted after the source stopped being authority for that generation;
3. that both wizards showed the same successor peer ID and epoch after `activate`/`observe`;
4. whether reconnection of the two peers after the network change restored wizard readiness without restarting either app;
5. sanitized diagnostics from both endpoints attached to the field-validation record.

Scenarios to certify:

- [ ] Stable same-LAN baseline: complete happy-path transfer with direct QUIC.
- [ ] Normal home NAT pair: complete transfer over a hole-punched path.
- [ ] Two independently NATed home networks: transfer across DCUtR hole punch, then repeat across relay fallback.
- [ ] Relay-only connectivity (symmetric NAT/CGNAT): full transfer while both peers are relayed.
- [ ] Network change mid-flow: Wi-Fi/carrier switch between `prepare` and `accept`, then again between `commit` and `activate`; confirm bounded failure or safe resume, never partial adoption.
- [ ] Relay drop mid-flow: relay disappears after `export`; confirm the accept device either reconnects through another path or fails closed.
- [ ] Mobile hotspot on either side: complete transfer with the hotspot peer as target, then as source.
- [ ] IPv6-only path where routable: complete transfer without IPv4.
- [ ] Long checkpoint wait: source Minecraft runtime takes minutes to reach the transfer save barrier over an impaired link; confirm the wizard stays bounded and never force-kills the game.

Transfer completion is measured in signed generations, not bytes: unlike snapshot replication there is no resume offset. An interrupted flow must end in a clear error plus a retryable state on both sides, and any record claiming certification must include both endpoints' migration status output showing the same authority and epoch.

## Field-validation record

Use [`network-validation/FIELD_RECORD_TEMPLATE.md`](network-validation/FIELD_RECORD_TEMPLATE.md) for each real-network run. A useful record includes:

- exact commit/application/wire version;
- endpoint OS/architecture and network class;
- NAT classification reported by SwarmCraft plus the tester-known environment class;
- IPv4/IPv6 availability and whether an end-to-end path actually succeeded;
- expected and actual direct/hole-punch/relay path;
- structured connectivity state and recent issue kinds before/after disruption;
- world ID, snapshot number, manifest hash and state root for a full replication exercise;
- bytes transferred and final verification result;
- the disruption used to force reconnection;
- committed offset before disruption and negotiated resume offset afterward;
- confirmation that application identity and canonical world state did not change because the transport path changed;
- sanitized logs and diagnostics from both endpoints.

Do not store private keys, IPC tokens, invitation secrets, credentials, or unredacted sensitive addresses in a public validation record.

## Suggested manual procedure

1. Record the exact SwarmCraft commit and run the connectivity probe on each endpoint before the world test.
2. Initialize two or more independent SwarmCraft peers and record their application peer IDs.
3. Create or join the same private test world through canonical membership.
4. Record local/observed addresses, `ConnectivityStateV1`, NAT status, IPv4/IPv6 flags, bootstrap state, selected relay, hole-punch state and recent issue kinds.
5. Replicate a signed snapshot and verify its hash on the remote peer.
6. Interrupt the preferred path by changing something real, such as Wi-Fi, router connectivity, address assignment, UDP availability, relay availability or the peer process.
7. Confirm either the documented fallback or a clear bounded failure state. Record the path and diagnostics before and after the disruption.
8. If transfer was interrupted, record the committed offset and negotiated resume offset and allow the transfer to complete.
9. Ensure transport changes do not alter world ID, membership, epoch, fencing token, snapshot hash or recovery authority except where the authority protocol itself legitimately requires a transition.
10. Attach sanitized logs/evidence to a completed field-validation record.

## Relay deployment note

Community relays and bootstrap peers are infrastructure conveniences, not trusted state authorities. Private-world data still requires normal authenticated membership authorization. A relay outage must never create, choose or rewrite canonical world history.

## Claim discipline

Automated impairment and multi-gigabyte soak gates justify saying that SwarmCraft's QUIC transport and resume path are tested under sustained transfer, repeated disconnects and synthetic packet impairment.

The structured diagnostics and direct-first bounded fallback code justify saying those mechanisms are implemented and testable. They do **not** justify saying SwarmCraft works behind every NAT.

Until representative hardware/carrier records exist, documentation must continue to describe home NAT, symmetric NAT, CGNAT, mobile hotspot, blocked-UDP and independent-ISP IPv6 behavior as requiring field certification.
