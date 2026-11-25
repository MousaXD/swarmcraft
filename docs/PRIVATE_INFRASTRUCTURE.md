# Group-operated discovery and relay infrastructure

The optional `swarmcraft-infra` executable is independent of the Desktop app
and world daemons. A locator stores bounded, expiring DHT provider hints; a
private relay forwards encrypted libp2p connections. Neither opens player
storage, reads an invitation, selects a host, votes, holds a lease, approves a
member, or authorizes a Minecraft route. Players still verify signed
announcements, the current membership, the freshness quorum and the live host
route. A private world is not advertised by the discovery publisher.

## Build and start

Build all three operator tools from the same compatible source checkout:

```bash
cargo build --release -p swarm-cli --bin swarmcraft-infra \
  --bin swarmcraft --bin swarmcraft-runtime --locked
```

Local evaluation (separate private service directories, no player data). Use a
private directory under your home whose ancestors are not group/world-writable;
do not put these directories inside a group-writable source checkout.

```bash
umask 077
target/release/swarmcraft-infra --state-dir "$HOME/.local/state/swarmcraft-locator" locator \
  --listen /ip4/127.0.0.1/tcp/41001
```

Record the printed `Locator address:` including its transport peer ID. On
each participating device, initialize its normal daemon once, then obtain its
public **transport** identity (not its application peer ID or private key):

```bash
target/release/swarmcraft --data-dir ./player-data transport-id
```

Copy [relay.example.json](../deploy/infrastructure/relay.example.json) to a
private operator-owned config file. Populate `allowed_transport_peers` with
the exact transport identities of the devices that should reserve and dial
circuits; an empty allowlist refuses startup. The example is loopback-only.
For actual group deployment, choose a reachable private interface, or set a
concrete external TCP address in `advertise`. Do not advertise a wildcard.

```bash
target/release/swarmcraft-infra --state-dir "$HOME/.local/state/swarmcraft-relay" relay \
  --config ./relay.json
```

Record the complete printed `Relay address:`. Both state directories retain
their identities across restart. Never delete state to fix a configuration
error. Keep the state on private local storage and run at most one process for
each directory. Do not put keys in logs, shared configuration, or backups
without access controls.

## Discovery and private rendezvous

Run a discovery publisher explicitly on the device already holding canonical
world membership, using its **existing** player data directory. Bind to a
reachable concrete IP interface, preferably the group's private VPN address.
The following loopback example is for same-machine testing:

```bash
export SWARMCRAFT_BOOTSTRAP='<complete locator multiaddress>'
export SWARMCRAFT_DISCOVERY_MDNS=0
target/release/swarmcraft-runtime --data-dir ./player-data discovery-serve \
  --listen /ip4/127.0.0.1/tcp/0
```

Other devices on the same private network can browse and resolve:

```bash
target/release/swarmcraft-runtime --data-dir ./other-player discovery-search \
  --bootstrap '<complete locator multiaddress>'
target/release/swarmcraft-runtime --data-dir ./other-player discovery-resolve \
  '<world-id>' --bootstrap '<complete locator multiaddress>'
```

The locator is not a proxy for an unreachable discovery publisher: a client
must still reach the publisher and enough current world members for freshness.
The standalone discovery transport does not implement relay-client routing.
Use a directly reachable LAN/VPN/TCP or QUIC publisher address. Disabling
discovery mDNS prevents accidental LAN discovery but is not an access-control
mechanism. The private rendezvous is an operator-managed deployment behind a
private VPN/firewall, **not** an isolated cryptographic DHT namespace. Limit
ingress and egress to the intended group; never connect the private locator
and its publishers to public DHT peers. Provider hints are visible to the
operator and other reachable DHT nodes, and unlisted world IDs must not be
treated as secrets or authorization. For a truly private world, use signed
invitations, not publication.

The publisher does not replace the normal authenticated replication daemon and
is not automatically launched by the Desktop app. Configure the player
daemon's normal relay environment before starting it:

```bash
SWARMCRAFT_RELAY='<complete relay multiaddress>' \
  target/release/swarmcraft --data-dir ./player-data daemon
```

Use reachable signed invitation addresses or explicit current peer hints.
A relayed peer address includes the relay identity, `/p2p-circuit`, and the
destination's transport identity. Do not confuse the relay address with an
application peer address. Core TCP/Noise direct substreams are supported,
but normal explicit daemon peer hints use QUIC direct or relay circuits,
not arbitrary direct TCP hints. UDP-blocked operation requires a viable TCP
relay path; no universal firewall bypass is promised.

## Linux service configuration

On a dedicated operator Linux machine, create a static unprivileged service
account and install the binary, example config and service units. Review
addresses, firewall rules and resource budgets before enabling them. The
example configs start on loopback only; the relay cannot start with its empty
allowlist.

```bash
sudo useradd --system --user-group --shell /usr/sbin/nologin swarmcraft-infra
sudo install -m 0755 target/release/swarmcraft-infra /usr/local/bin/swarmcraft-infra
sudo install -d -m 0750 -o root -g swarmcraft-infra /etc/swarmcraft-infra
sudo install -m 0640 -o root -g swarmcraft-infra \
  deploy/infrastructure/locator.env.example /etc/swarmcraft-infra/locator.env
sudo install -m 0640 -o root -g swarmcraft-infra \
  deploy/infrastructure/relay.example.json /etc/swarmcraft-infra/relay.json
sudo install -m 0644 deploy/systemd/swarmcraft-{locator,relay}.service /etc/systemd/system/
# Edit both configurations for the actual private group first.
sudo systemctl daemon-reload
sudo systemctl enable --now swarmcraft-locator.service
sudo systemctl enable --now swarmcraft-relay.service
sudo journalctl -u swarmcraft-locator -u swarmcraft-relay -n 30
```

Skip account creation if that dedicated account already exists. The service
units create private persistent directories, drop privileges and cap file
descriptors, tasks and memory. The examples are not public multi-tenant
infrastructure or an installer. No services are installed by building or
testing the repository.

## Swap or recover a relay

1. Keep canonical world/peer identities and world data unchanged. Deploy a
   replacement relay with its own state and an allowlist for the same intended
   devices. Confirm its listening address and transport ID.
2. Configure the replacement complete relay address on each device. Restart
   the normal daemon after changing its environment; do not rely on the old
   relay's circuit addresses. Supply a current destination peer hint using
   the replacement relay's circuit. Renew signed invitation addresses if
   necessary.
3. Confirm a new reservation and live application authentication. Only then
   retry game-route verification and gameplay. On relay loss, the old
   connection must close; the old route must not authorize the replacement.
   If no route remains reachable, preserve saves and fail closed.
4. Retire the old relay when replacement is verified. Removing an allowed
   transport identity requires a config change and relay restart, which
   disconnects circuits. The relay allowlist is not a world membership list.

Automatic selection of a new relay in a running Desktop session is not
certified by this procedure. The local integration test exercises explicit
replacement using the same live peer identities.

## Resource and privacy boundaries

| Budget | Policy |
| --- | --- |
| Allowed transports | 1-32 explicitly configured public transport peer IDs |
| Pending connections | 32 inbound, 8 outbound |
| Established connections | 64 total, 2 per transport |
| Reservations | 32 total, 1 per transport, 30-minute lifetime |
| Circuits | Configurable 1-32 total, 2 per transport |
| Circuit byte budget | 4 KiB-1 GiB configurable, counted by circuit protocol |
| Circuit duration | 1-3600 seconds configurable |
| Rate limits | Pinned libp2p relay defaults for per-IP/per-peer creation |
| Locator | 1024 DHT keys, up to 20 providers per key, 10-minute expiry; no arbitrary value records |
| Logs | Operator service address and startup failures; no peer event or packet logging |

These are connection and resource bounds, not guaranteed bandwidth fairness
between worlds. The relay does not know which world encrypted traffic belongs
to. A local four-peer regression sets a 256 KiB encrypted circuit budget,
acknowledges 124 KiB of bulk payload (with matching echo), then verifies the
bulk connection loses authentication while an independent small circuit
continues to exchange bytes. Payload totals can vary with framing overhead.
Production multi-world fairness and measured bandwidth targets require
separate evidence; do not promise a public unlimited community relay.
A network administrator can observe traffic timing and transport metadata.
Keep service addresses and diagnostics private where appropriate.

## Verification scope

From a debug build of the three binaries:

```bash
python3 scripts/e2e/private_infrastructure_smoke.py
cargo test -p swarm-network --test game_stream_relay --locked
cargo test -p swarm-network --test game_stream_transport --locked
```

The smoke starts independent local processes with mDNS disabled and verifies
signed discovery, private-world exclusion, process locks and stable relay
identity. The production relay integration tests authenticated control and
game-stream bytes, denied unlisted clients, relay outage and explicit
replacement. These are local transport tests, **not** actual Minecraft login,
public Internet reachability or physical NAT certification. See
[network validation](NETWORK_VALIDATION.md) for field evidence requirements.
