# Local capacity lab (#35)

This is a manual research harness, not a new production server setting, a
platform certification, or a capacity guarantee. Minecraft simulation, voter
recovery, verified replica storage, and relay forwarding are separate tests.

## Real game-server baseline

On Linux with Python 3.11+, Node 22+, Java 25 and an existing authorized
Minecraft 26.1.2 server JAR:

```sh
cd research/game-capacity
npm ci --ignore-scripts --no-audit --no-fund
python3 measure.py \
  --java /absolute/path/to/java25/bin/java \
  --jar /absolute/path/to/minecraft-26.1.2/server.jar \
  --accepted-eula /absolute/path/to/existing/accepted/eula.txt \
  --output runs/local-01 --heap 1536 --seconds 16 --players 0 1 2 4 8
```

For a separate, fresh-world larger-client trial, use
`--output runs/local-02 --seconds 30 --players 12 16`. Only run one lab
at a time and retain enough free host memory.

The EULA input must already contain an explicit acceptance. Never point
this script at an existing world directory: it creates and deletes its own
temporary world. It binds exclusively to IPv4 loopback, disables public
query/RCON, and uses offline synthetic player identities **only for this
isolated baseline**; it does not modify SwarmCraft login, consent, authority,
or networking rules. Do not deploy this test configuration.

The server uses a fixed terrain seed, peaceful survival, view distance 6,
simulation distance 4, maximum 16 slots, and a bounded heap. JVM
`ActiveProcessorCount=4` controls ergonomics, not an OS CPU quota. The bots
are actual protocol clients, not fake server-side player entities. They
walk and jump in different directions; the server must report exactly the
requested connected count at both measurement boundaries.

The observed TPS is the actual game-time tick delta divided by monotonic
wall time, not the configured target TPS. Short intervals can slightly
exceed 20 due to clock sampling and catch-up. CPU percent uses 100% per
logical CPU; RSS is sampled process residency (not heap), and the world
size is logical file bytes after an explicit save. Server-reported average
milliseconds per tick is sampled separately from wall-clock TPS. Per-client
socket byte totals include connection/setup traffic; the aggregate traffic
measurement starts after all clients spawn and its peak is a one-second
sample, not instantaneous link saturation. Stages share a growing world and
are not independent cold-cache trials. There is a memory-headroom guard;
unavailable stages are not passed or extrapolated.

Output is written to a fresh directory: structured measurements, server
log, and client logs. The benchmark requires no GitHub Actions or third-party
hosted test service.

## Independent SwarmCraft measurements

Run the following **after** the game-server process exits, using the
repository's local Cargo lock when sharing the workstation:

```sh
flock /tmp/swarmcraft-sabathon-cargo.lock cargo test --locked -p swarm-network --test game_stream_transport --test game_stream_relay -- --nocapture --test-threads=1
flock /tmp/swarmcraft-sabathon-cargo.lock cargo test --locked -p swarm-storage --test replica_capacity_lab -- --ignored --nocapture
flock /tmp/swarmcraft-sabathon-cargo.lock cargo test --locked -p swarm-cli --test consensus_partition_safety unclean_quorum_loss -- --nocapture --test-threads=1
flock /tmp/swarmcraft-sabathon-cargo.lock cargo test --locked -p swarm-cli --test discovery_freshness capacity_signed_freshness_proofs_by_voter_count -- --ignored --nocapture
```

* Transport: one MiB of sequential 4 KiB request/echo payloads on real
  authenticated libp2p direct and relayed substreams; timings include
  stream negotiation. The two-circuit test verifies that exhausting a
  bounded bulk circuit does not disconnect the independent circuit.
  This is not a Minecraft login, WAN, NAT, or public relay benchmark.
* Replication: actual V1 encoded capture, chunk ingest, publication,
  reopen, verify and byte-for-byte restore for 1/8/32 MiB synthetic
  worlds, with 1/2/3 verified destination stores. Report cumulative
  encoded payload/manifest and logical disk bytes, capture and verified
  copy time (excluding previous restore time), and restore time. All
  stores share one disk; this is a storage-only test with fixture
  signatures, not an authenticated network admission or independent
  machine-failure experiment.
* Voters: the existing three- and five-daemon unclean-quorum-loss tests
  report initial permit, old-permit removal and surviving-majority
  recovery-permit time. The measurement includes fixture startup/polling;
  a recovery permit is not first playable Minecraft action. A host-memory
  admission skip provides no positive timing evidence.
  The separate signed-discovery proof benchmark measures actual majority
  Ed25519 signing and full freshness verification at 3/5/9/17 members,
  checks insufficient and replayed proofs fail, and records serialized
  vote bytes. It does not bypass hosting admission or replace recovery
  measurements.

No replica or voter count is added to the simulation player count. The
real game-server baseline does not certify modded gameplay, the full
authenticated Fabric/gateway path, multi-machine recovery, or WAN relay
capacity. Publish these limitations alongside the numbers.
