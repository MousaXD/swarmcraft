# Ownerless current-head observation transport (#66–68 follow-up)

**Scope:** isolated Linux reference authority journals and explicit one-shot
TCP observations. Not a live authority lease, consensus transport for new
decisions, route proof, Minecraft authentication or runtime permit.

## Contract

The collector independently selects the exact locally accepted world and
head, opens its existing private journal under the per-world lock, verifies
its complete signed history and committed checkpoint bytes, and requires
its own key to be a current validator without a pending governance vote.
The signed observation context binds world, head, sequence, fence,
checkpoint, policy, compatibility, verifier identity and a fresh 32-byte
nonce. It uses separate request, context and response signing domains;
an observation cannot be used as a governance vote or a role proof.
The collector contacts only explicitly named current remote validators
at independently selected private IP endpoints. The responses must be
signed by the exact targeted validator from the accepted roster, never
by a key supplied by the untrusted connection. The collector counts
distinct identities and requires its own local signature and the
existing strict majority threshold.

A responder is a one-shot process explicitly started against a pinned
local head, not a persistent public network service. Before listening,
and again after reading the signed request, it verifies its current
journal, checkpoint, validator membership and lack of pending vote.
It acquires the same stable per-world lock nonblockingly; a busy journal
fails closed. A listener started on a now-retired head cannot sign.
A pending governance vote is not cleared or treated as acceptance.
The responder verifies the requesting validator's signature and exact
context, then signs and writes the response while holding its lock.
No journal, checkpoint, membership, worker eligibility, pending vote
or V1 runtime state is changed by a successful observation.

## Transport and resource limits

- Explicit loopback, RFC1918 IPv4 or IPv6 ULA addresses only. No wildcard,
  public listener, DNS discovery, implicit port forwarding or ordinary
  SwarmCraft network protocol change. A listener accepts one connection.
- Four-byte big-endian frame length followed by strict JSON, at most
  8,192 bytes. Unknown fields, truncated and oversized frames fail.
  Read/write timeouts use the remaining monotonic deadline, not a
  renewed idle timeout after each byte.
- At most 14 distinct remote validators and endpoints, never the local
  identity. All connections are collected in parallel with one shared
  three-second network budget and a five-second collection window,
  including final local head and signature checks. Failed remote
  attempts do not count; a valid majority may still be observed.
  Listener accept window is 15 seconds; the accepted connection has a
  five-second network window. Local storage/OS stalls can delay process
  exit, but an expired collection does not return success.
- The collector reopens the exact journal before final verification and
  output. A changed local identity/head, pending vote, corrupted
  checkpoint, missing majority, expired deadline or duplicate remote
  identity cannot produce a successful report.
- Application keys authenticate signed content. This is not encrypted
  transport or proof of endpoint ownership. An adversary can relay a
  request, observe metadata, or deny service by occupying the one-shot
  connection. The protocol must not be exposed to untrusted networks.

## Local workflow

Use separate existing validator profiles and an independently reviewed
world and accepted head from the #66 workflow. On the remote validator:

```sh
swarmcraft --data-dir /private/validator-b ownerless authority observe serve \
  --expected-world "$WORLD" --expected-head "$HEAD" \
  --bind 127.0.0.1:0 --confirm
```

Read the printed JSON address and validator ID. On the collecting
validator:

```sh
swarmcraft --data-dir /private/validator-a ownerless authority observe collect \
  --expected-world "$WORLD" --expected-head "$HEAD" \
  --peer "$VALIDATOR_B@$ADDRESS" --confirm
```

For a separately configured private network use explicit private
addresses on the two hosts. The remote endpoint is a manually reviewed
operational input; do not discover or contact arbitrary peers. Use
multiple `--peer` flags as needed for the accepted roster.

The output contains the verifier-signed request, sorted responder
signatures, actual elapsed collection time and unavailable/invalid
peer IDs. A success is classified as
`observed_recent_ownerless_reference_majority`.

## Why it is not live authority

A signature is an observation of a local journal during the collection,
not a lock on future decisions. A validator can commit a new vote after
responding, and the observations are not a simultaneous distributed
snapshot. The CFT assumptions and full-journal rollback/identity-clone
limits of #66 still apply. The verifier can recheck historical
signatures but cannot re-establish the collection's original timing.
A captured request may be answered again while the same head remains
accepted; it is the collecting verifier's fresh unpredictable nonce and
monotonic deadline that prevent an old response from being counted.
There is no externally reusable freshness token.

Every result explicitly sets freshness lease, latest global head proven,
live quorum verified, canonical authority, write authority, runtime
activation, player admission and worker admission **false**. There is
no integration into the V1 daemon, Fabric, game gateway, permit issuance
or canonical publication. Those gates require a separately designed live
governance/authority protocol, mutually authenticated encrypted transport,
and production runtime/consent integration. Independent physical-host
and sustained Minecraft qualification remain open.

## Local regression commands

```sh
umask 077
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-protocol -p swarm-consensus'
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-cli --test ownerless_authority_cli --test ownerless_creation_cli'
```

## Local evidence — 26 November 2026

133 protocol/consensus tests passed, including two new observation
contract regressions. Ten authority CLI integration tests and two
creation CLI tests passed; three new CLI cases used real TCP processes
to exercise distinct nonces, replayed valid signatures, wrong targeted
validator, pending votes after listen, head retirement after listen,
duplicate endpoints, nonmembers, public/wildcard endpoints, oversized
frames, and a silent connected peer alongside a healthy majority.
A further retained-evidence signature test passed (13 total CLI
integration scenarios across the two targets). Strict Clippy passed
for both changed protocol/consensus crates and the relevant CLI targets.

A separate run on one Linux machine used two distinct mapped private
network namespaces connected by a real veth pair. The founder was
excluded before a certified recovery decision. The remaining two
validators observed the recovered head in 310 ms. Bringing the
private link down refused the observation in approximately 247 ms;
restoring the link yielded a new signed majority observation in 339 ms.
All listener processes exited; the validator and departed founder
journals were byte-for-byte unchanged. These timings are observations
of this one local run, not remote performance guarantees.

[Retained public evidence](../../research/distributed-simulation/evidence/2026-11-26-ownerless-network-observation.json)
includes the signed history, exact checkpoint and both signed observation
samples. The retained Rust integration test independently verifies the
history, checkpoint commitment and both signature thresholds, and
refuses using the first sample's signatures for the restored sample's
nonce. Historical timing and physical deployment are not independently
attested by those signatures.

Reproduce in a fresh mapped user/network namespace (Linux with iproute2):

```sh
unshare --user --map-root-user --net \
  python3 research/distributed-simulation/ownerless_observation_lab.py \
  --cli "$PWD/target/debug/swarmcraft" --output /tmp/new-ownerless-observation.json
```
