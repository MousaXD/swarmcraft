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

Repository tests can validate deterministic state transitions and local authenticated networking. They cannot faithfully reproduce every residential router, carrier-grade NAT, mobile carrier, ISP IPv6 policy or firewall.

Automated CI covers local authenticated transport and bounded diagnostic/state-machine behavior. Public-network field validation remains manual.

## Manual real-world validation matrix

Do not change a row to **Verified** unless it has been exercised between real machines/networks and the evidence is recorded with the tested SwarmCraft commit/version.

| Scenario | Expected path | 0.2.0 repository status |
| --- | --- | --- |
| Same LAN | mDNS + direct QUIC | Automated/local coverage; real-router spot check still recommended |
| Normal home NAT to reachable public peer | direct/observed address, possibly hole punch | Not certified across representative routers |
| Two independently NATed home peers | DCUtR hole punch, then relay fallback | Not certified across representative routers |
| CGNAT | relay likely; hole punch where carrier permits | Not certified |
| Mobile hotspot | direct/hole punch/relay depends on carrier | Not certified |
| IPv6 peers | direct IPv6 where routable | Not certified across representative ISPs |
| Direct path fails but relay is reachable | relay fallback | Protocol support present; deployment-specific relay test required |
| Relay disappears after world was replicated | reconnect through another path/relay; canonical world remains valid | Canonical design does not depend on relay; multi-relay field validation pending |

## Suggested manual procedure

1. Initialize two or more independent SwarmCraft peers and record their application peer IDs.
2. Create or join the same private test world through canonical membership.
3. Run the daemon with diagnostic logging enabled.
4. Record local and observed addresses, NAT classification, attempted direct/hole-punch/relay path and failure reason.
5. Replicate a signed snapshot and verify its hash on the remote peer.
6. Interrupt the preferred path and confirm either the documented fallback or a clear failure state.
7. Ensure transport changes do not alter world ID, membership, epoch, fencing token, snapshot hash or recovery authority.
8. Attach sanitized logs to the validation record, excluding private keys and invite secrets.

## Relay deployment note

Community relays and bootstrap peers are infrastructure conveniences, not trusted state authorities. Private-world data still requires normal authenticated membership authorization.

## Claim discipline

Until representative hardware/carrier testing is completed, documentation should say that AutoNAT/DCUtR/relay functionality and diagnostics are implemented, not that SwarmCraft is universally verified across home NAT, CGNAT, mobile hotspot and IPv6 environments.
