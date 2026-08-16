# Network connectivity validation

SwarmCraft includes libp2p support for direct connections, mDNS, Kademlia, AutoNAT, DCUtR hole punching and relays. Presence of those code paths is not the same as proving every consumer NAT/carrier environment works.

## Intended fallback

```text
LAN / direct address
        ↓
known or observed public address
        ↓
DCUtR hole punch
        ↓
relay transport
```

A relay can carry encrypted peer traffic but cannot define world identity, membership, recovery votes, snapshots or canonical history. No single relay is required for a world's existence.

## Exposed diagnostic state

The runtime diagnostic model records, where libp2p can determine it:

- local listen addresses;
- observed public address;
- NAT status: unknown/public/private;
- whether a direct connection has succeeded;
- relay connectivity;
- hole-punch state: idle/attempting/succeeded/failed;
- selected relay address;
- last connection failure reason.

Wire requests also enforce bounded sizes for blob chunks, missing-blob lists, membership sets, recovery certificates and world compatibility metadata.

## Automated coverage

Automated tests deliberately cover more than a clean loopback connection:

- signed application authentication over QUIC/libp2p;
- persisted transport identity across a hard peer restart;
- replacement connections racing stale/dead connections;
- re-authentication and authenticated requests after reconnect;
- chunked transfer resume negotiation after reconnect;
- a receiver committing a chunk while its acknowledgement is deliberately lost;
- resume from the receiver's committed offset rather than retransmitting already accepted data;
- deterministic byte-for-byte payload verification for every received chunk;
- Linux `tc netem` impairment with latency variation, packet loss and bandwidth limiting.

The normal CI matrix contains a dedicated **Network impairment (QUIC resume)** job. It transfers 64 MiB while repeatedly hard-restarting the sender every 16 MiB under WAN-like loopback impairment.

The separate **Network Soak** workflow runs the same protocol path at multi-gigabyte scale. Its default profile transfers **2 GiB**, forces a hard sender restart every **256 MiB**, deliberately loses the final acknowledgement before each restart, renegotiates the committed resume offset, and continues until the full transfer completes. The workflow runs:

- on pull requests that change `swarm-network`, `swarm-storage`, or the soak workflow itself;
- on matching pushes to `main`;
- every Sunday;
- manually with selectable 1 GiB, 2 GiB, or 5 GiB profiles.

Each soak run uploads the test log, tested commit/profile metadata and the exact `tc netem` queue configuration as workflow artifacts.

This is strong transport/resume evidence, but it is still **not** equivalent to a real residential router, CGNAT carrier, mobile network or ISP IPv6 path.

## Manual real-world validation matrix

Do not change a row to **Verified** unless it has been exercised between real machines/networks and the evidence is recorded with the tested SwarmCraft commit/version.

| Scenario | Expected path | 0.2.x repository status |
| --- | --- | --- |
| Same LAN | mDNS + direct QUIC | Automated local/impaired coverage; real-router record still recommended |
| Normal home NAT to reachable public peer | direct/observed address, possibly hole punch | Not certified across representative routers |
| Two independently NATed home peers | DCUtR hole punch, then relay fallback | Not certified across representative routers |
| CGNAT | relay likely; hole punch where carrier permits | Not certified |
| Mobile hotspot | direct/hole punch/relay depends on carrier | Not certified |
| IPv6 peers | direct IPv6 where routable | Not certified across representative ISPs |
| Direct path fails but relay is reachable | relay fallback | Protocol support present; deployment-specific relay test required |
| Relay disappears after world was replicated | reconnect through another path/relay; canonical world remains valid | Canonical design does not depend on relay; multi-relay field validation pending |

## Field-validation record

Use [`network-validation/FIELD_RECORD_TEMPLATE.md`](network-validation/FIELD_RECORD_TEMPLATE.md) for each real-network run. A useful record includes:

- exact commit/application/wire version;
- endpoint OS/architecture and network class;
- NAT classification and IPv4/IPv6 availability;
- expected and actual direct/hole-punch/relay path;
- world ID, snapshot number, manifest hash and state root;
- bytes transferred and final verification result;
- the disruption used to force reconnection;
- committed offset before disruption and negotiated resume offset afterward;
- confirmation that application identity and canonical world state did not change because the transport path changed;
- sanitized logs and diagnostics from both endpoints.

Do not store private keys, IPC tokens, invitation secrets, credentials, or unredacted sensitive addresses in a public validation record.

## Suggested manual procedure

1. Initialize two or more independent SwarmCraft peers and record their application peer IDs.
2. Create or join the same private test world through canonical membership.
3. Run the daemon with diagnostic logging enabled.
4. Record local and observed addresses, NAT classification, attempted direct/hole-punch/relay path and failure reason.
5. Replicate a signed snapshot and verify its hash on the remote peer.
6. Interrupt the preferred path by changing something real, such as Wi-Fi, router connectivity, address assignment, relay availability or the peer process.
7. Confirm either the documented fallback or a clear failure state, then record the resume offset and final verification result.
8. Ensure transport changes do not alter world ID, membership, epoch, fencing token, snapshot hash or recovery authority except where the authority protocol itself legitimately requires a transition.
9. Attach sanitized logs/evidence to a completed field-validation record.

## Relay deployment note

Community relays and bootstrap peers are infrastructure conveniences, not trusted state authorities. Private-world data still requires normal authenticated membership authorization.

## Claim discipline

The automated impairment and multi-gigabyte soak gates justify saying that SwarmCraft's QUIC transport and resume path are tested under sustained transfer, repeated disconnects and synthetic packet impairment.

Until representative hardware/carrier records exist, documentation must still say that AutoNAT/DCUtR/relay functionality and diagnostics are implemented, not that SwarmCraft is universally verified across home NAT, CGNAT, mobile hotspot and IPv6 environments.
