# #64: Ownerless / no-OP world creation contract

Status: offline research implementation; no live activation.
Date: 25 November 2026 UTC / 26 November UAE time.

## Why a separate identity

The supported V1 genesis has a creator key and the ordinary creation
path constructs a single-member descriptor, initial snapshot, epoch,
membership and signed configuration under that creator. It is not
sufficient to hide the founder's name or add a no-OP UI toggle. Doing
so would leave V1 authority, configuration edits, and solo recovery
available to the founder. Existing worlds and V1 wire/storage formats
are intentionally unchanged.

The isolated OwnerlessGenesisV1 format instead contains:

- format version, display name, random creation nonce;
- exact runtime-compatibility fingerprint and initial-state commitment;
- a fixed, public ownerless policy;
- a canonical sorted roster of 3 to 15 explicit validator public keys.

There is no creator, special owner key, initial authority, host
candidate, resource budget, or automatic recovery entitlement.
A founding player can be an ordinary player without a validator key.
If independently enrolled as a validator, the founder has exactly
the same one-key role as every other validator. No resource or hosting
amount appears in the signed roster or policy. Distinct keys do not
prove distinct people, machines, or failure domains.

The world ID is a domain-separated BLAKE3 hash of the postcard genesis
encoding, in a separate Rust type and textual scownerless namespace.
The policy hash has its own domain. Each creation approval is a
domain-separated Ed25519 signature over format, world ID, policy hash
and validator peer ID. The world ID binds the full genesis, including
the roster, nonce, compatibility and initial-state pin. The separate
policy hash lets the future runtime check the exact invariant set.
Compatibility and initial-state commitments are opaque caller pins at
this stage, not evidence of installed artifacts or verified save data.

## Immutable policy profile

The exact implemented profile denies permanent founder privileges,
player OP, RCON, privileged console, command blocks, solo advancement,
and resource-contribution-derived votes. It specifies authenticated
invite-only play. All rule invariants are fixed for this format. A
different rule profile needs a new format/identity and fresh reviewed
approvals; there is no in-place policy mutation. Public/anonymous
admission is deliberately not offered here; separate participation
policy work belongs to #67.

A creation candidate requires **all** named validators' explicit
signatures, not a simple majority, to avoid enrolling a nonconsenting
validator at genesis. At least three are required; duplicate keys,
out-of-order or mismatched identities, weak keys, mixed-torsion and
noncanonical key encodings are rejected. Each approval must match
the sorted founding roster exactly, including its identity and
signature. Verification uses real strict Ed25519.

The proposed stable-roster maintenance threshold is floor(n/2)+1,
independent of player/host/compute roles. This is a reported policy
value only, not a maintenance certificate or an authorization to
recover, mutate state, launch a runtime or grant privileged actions.
#66 must implement independent accepted voter history, quorum-backed
changes, durable voting, generation fencing, and loss recovery.
No maintenance vote is accepted by this implementation.

## Offline CLI creation workflow

Use a disposable test profile if testing. Obtain each prospective
validator's existing public key independently and check that the
participants consent. The organizer need not be one of those keys.
All outputs are JSON on stdout; the caller chooses where to save them.
Replace the angle-bracket placeholders with the actual reviewed values.

```sh
swarmcraft ownerless draft --name "Example" \
  --compatibility-fingerprint <64-hex-fingerprint> \
  --initial-state-commitment <64-hex-state-commitment> \
  --validator-key <public-key-a> \
  --validator-key <public-key-b> \
  --validator-key <public-key-c> > draft.json

swarmcraft ownerless inspect --draft draft.json

# Each listed validator on their own already initialized profile:
swarmcraft --data-dir <existing-validator-profile> ownerless approve \
  --draft draft.json --expected-world <scownerless:reviewed-id> \
  --confirm > approval-a.json

# Organizer after gathering ALL approvals (file order is irrelevant):
swarmcraft ownerless assemble --draft draft.json \
  --expected-world <scownerless:reviewed-id> \
  --approval approval-a.json --approval approval-b.json \
  --approval approval-c.json > certificate.json

swarmcraft ownerless verify --certificate certificate.json \
  --expected-world <scownerless:reviewed-id>
```

The expected world ID is selected independently by each reviewer,
not silently copied from a received certificate at verification time.
Review the actual policy, complete validator key roster, compatibility
and state commitments before confirming. Draft and inspect require no
identity. Approval loads an existing local identity, never creates or
repairs it. It refuses an unlisted local key or mismatched draft pin.
Assembly verifies every signature; verification returns only an
offline candidate, with canonical_world_created, runtime_activation
and recovery_authorized all false. JSON file input is bounded to 64
KiB and unknown fields are rejected. No network service or wire decoder
for this format is introduced.

All these commands return before opening the canonical world store;
no membership, snapshot, authority permit, runtime, consent, or
playable world is created. There is no certificate import or conversion
to V1 metadata, WorldId, signed configuration, or V1 authority. The
ordinary world create --ownerless flag explicitly refuses before
opening storage instead of creating an ordinary creator-owned world
under the requested name. Ordinary V1 creation without the flag remains
unchanged. Desktop has no ownerless-play toggle until the runtime and
authority enforcement gates are satisfied.

The certificate proves agreement on the exact draft, not the existence
of a trusted distributed canonical head. Separate drafts are separate
identities; the organizer cannot rewrite a signed draft, transplant
approvals from another draft, or turn compute into votes.

## Threat model and remaining implementation

The proposed governance model is crash-fault tolerant (CFT), assuming
independent honest validators and unmodified runtimes. Key count alone
does not establish that assumption. Strict signatures only verify
who signed which message. The current certificate verifies genesis
agreement but does not implement canonical voting or prevent a signer
from signing other independent genesis drafts. Majority intersection
alone does not establish Byzantine safety. A malicious OS owner can
modify JVM memory, files, binaries, or code; these signatures do not
prove compliant game ticks.

The [#65 permission lab](ownerless-permissions.md) verifies isolated real
Minecraft permissions and policy-bound reference workers; canonical runtime
admission and actual successor-host gameplay remain gated. #66 must establish
durable shared authority and founder-independent recovery, including
the inability of an isolated key to publish or change membership.
#67 must separate admission, gameplay roles, validator enrollment and
worker consent, with public access treated as a distinct policy.
#68 must verify all of these together in actual independent-host
gameplay. The #63 scale qualification failure remains open.
No live ownerless or horizontally distributed gameplay is authorized.

## Local verification

The complete protocol and consensus test suites passed (122 tests,
including seven new ownerless creation regressions). The real CLI
integration suite passed two tests with three separate temporary
validator profiles, an outsider profile and an organizer without
an identity. It exercised draft, inspect, explicit approval,
assembly and verification; refused missing/duplicate approvals,
wrong identity pin, unknown signer, missing confirmation, forged
or altered signed content and oversized input. The organizer's
profile was never created, existing validator identity keys were
unchanged, and their world directories remained empty. The ordinary
V1 creation path still registered a normal world. The tests use
real Ed25519 signatures but one physical workstation; strict Clippy
passed for protocol/consensus all targets and the CLI binary and
new integration test. No actual
player or privileged command was executed in Minecraft.

Reproduction (local/offline, no hosted CI):

```sh
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-protocol -p swarm-consensus'
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-cli --test ownerless_creation_cli'
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo clippy --locked --offline -p swarm-protocol -p swarm-consensus --all-targets -- -D warnings'
```
