# Ownerless participation (#67) — separate reference roles

**Scope:** offline/local reference qualification. Not a public game
listener, authenticated Minecraft account, live quorum, worker
reservation, validator enrollment or execution permit.

## Creation policy and backward compatibility

The existing format 1 invite-only policy, world IDs, signing domains and
private policy hash are unchanged. The existing #65 permission pin is
unchanged. Public authenticated participation is selected explicitly
with `ownerless draft --public-play`; this creates format 2 with new
genesis, policy and approval signing domains. The complete founding
roster still must explicitly sign the exact new identity. Public and
private certificates cannot be swapped or converted. In both modes,
founder privileges, player OP, RCON, privileged console, command blocks,
solo advancement and compute-derived voting remain forbidden.

Only format 1 with its exact immutable private policy and format 2 with
its exact immutable public policy are accepted. No governance action
can change visibility or the protected policy. An older reader must
refuse the unsupported public format, not silently fall back.
The existing private Fabric permission lab explicitly refuses the
public format; this change does not claim a public Fabric login path.

Public authenticated reference play means that a subject can be
previously unknown to the roster, but must prove ownership of its
canonical application identity key. It does **not** mean offline-mode
Minecraft, anonymous unauthenticated account access, a verified Mojang
account, a public IP listener, or permission to download private saves.
An unauthenticated observer can have no execution or voting rights.
No public discovery, gameplay moderation or account/Sybil protection is
introduced by this reference contract.

## Independent permissions

| Role | Source of eligibility | What it does not confer |
| --- | --- | --- |
| Reference player, private | Current quorum-certified explicit invitation and proof of exact invited key | Worker eligibility, vote, OP, write authority |
| Reference player, public | Proof of subject key without a private invitation | Worker eligibility, vote, OP, write authority |
| Reference volunteer | Separate quorum-certified worker cap and explicitly signed positive offer within the cap | Player invitation, vote, OP, execution reservation |
| Validator | Existing accepted roster or the #66 joint old/new majority plus entrant approval | Automatic player invitation, compute consent, OP, unilateral authority |

Newly appended `SetPlayer` and `SetWorker` authority action variants
preserve the existing proposal tags and hashes. They extend the same
verified ownerless history using the existing one-vote-per-parent
durable journals; no sidecar may change accepted roles. Each action
requires the current majority, retains the exact accepted checkpoint,
advances the reference fence and leaves the validator roster unchanged.
Private invite addition/removal and worker cap creation/change/removal
are separately validated. Redundant or inconsistent changes fail.
Public play does not use a private invitation roster; worker caps remain
necessary in both modes. A worker can volunteer without being invited
to play, and a validator can vote without volunteering or receiving a
private invitation. Enrollment of a new validator continues to require
the #66 joint transition and entrant consent.

Limits: at most 128 explicit private player keys and 32 eligible worker
keys within the bounded reference history. Worker budget is positive
whole reference-process MiB (512–65536) and CPU millicores (250–64000);
both must be present, bounded and at most the accepted cap. These are
declared limits, not Java heap, measured physical capacity, actual
kernel enforcement, or an active workload assignment. Zero means no
positive compute offer; never an uncapped worker.

## Proof of identity and explicit volunteer offer

A verifier with an existing private local journal issues a fresh random
32-byte challenge pinned to the exact world, locally accepted head,
reference fence, policy, compatibility, verifier, subject and intent.
Both verifier and subject are canonical prime-order Ed25519 keys.
Request and response signatures use separate domain tags. The subject
reviews the challenge hash and pinned history; responding requires
`--confirm` and, only for a volunteer offer, the separate explicit
`--confirm-volunteer`. The subject's existing identity is required;
no identity, validator roster, private invitation or worker grant is
implicitly created.

The verifier's outstanding challenge is a separately signed local
record, protected by the same stable per-world kernel lock as the
authority journal. Issuance atomically syncs before output. The verifier
checks its own exact stored request, accepted current head and subject
signature, then applies role eligibility. Successful proof consumption
is atomically durable **before** successful output. Concurrent processes
cannot consume one proof twice; lost output does not reset the proof.
Issuing a new challenge replaces the old one; explicit cancellation
also consumes it. Invalid inputs do not authorize any role. A changed
accepted head invalidates the old challenge, including a revoked
invitation or worker cap. Role proof consumption does not modify the
authority journal.

The report explicitly sets validator-granted, write-authority,
live-quorum-verified, resource-enforcement-verified, runtime-activation,
worker-admission and player-admission false. A separate observation
can say that the subject was **already** enrolled as a validator.
This is only an offline/local key-possession and reference-eligibility
proof against a pinned accepted history. It is not network session
authentication, a freshness lease, or an ongoing consent token. No
duration or live capacity is inferred from a signed offer. A subject
can refuse to sign; after signature delivery a verifier can cancel the
outstanding challenge. There is no active worker to withdraw in this
workflow. Production withdrawal will require an observed current
consent revision, expiry, session binding, fencing, resource accounting
and actual stop/drain verification (see #57–62).

## Local CLI workflow

Use an existing #66 isolated journal and independently reviewed world
and head. The organizer can draft a private invitation or a separate
worker eligibility proposal without obtaining a key or opening storage:

```sh
swarmcraft ownerless authority participation propose-player \
  --history history.json --expected-world "$WORLD" --expected-head "$HEAD" \
  --player-key "$PLAYER_KEY" --allow > proposal.json
swarmcraft ownerless authority participation propose-worker \
  --history history.json --expected-world "$WORLD" --expected-head "$HEAD" \
  --worker-key "$WORKER_KEY" --memory-mib 4096 --cpu-millicores 2000 \
  > worker-proposal.json
```

Use the existing #66 `vote`, `assemble` and `accept` workflow for
each separately reviewed proposal, with the unchanged accepted
checkpoint bytes. Revocation uses `--revoke` instead of allow or
positive worker budgets. No player or worker can vote on this proposal
merely because it is the target.

For a disposable verifier profile with a locally accepted head:

```sh
swarmcraft --data-dir /private/verifier ownerless authority participation challenge \
  --expected-world "$WORLD" --expected-head "$HEAD" --subject-key "$PLAYER_KEY" \
  --play > request.json
swarmcraft --data-dir /private/player ownerless authority participation respond \
  --history history.json --expected-world "$WORLD" --expected-head "$HEAD" \
  --request request.json --expected-challenge "$CHALLENGE" --confirm > response.json
swarmcraft --data-dir /private/verifier ownerless authority participation verify \
  --expected-world "$WORLD" --expected-head "$HEAD" \
  --expected-challenge "$CHALLENGE" --response response.json
```

For volunteer proof, issue a challenge with both memory and CPU instead
of `--play`, and require `--confirm-volunteer` on the subject side.
`cancel` takes the expected world, current head and exact outstanding
challenge hash. These commands never enable normal Minecraft routing,
canonical world storage, authority permits or a public network listener.

## Trust boundary and remaining gates

The CFT trust assumptions from #66 still apply. A malicious local OS
owner can clone an identity, restore a complete older signed local
challenge/journal, or modify software. The journal is not an
anti-rollback hardware counter. Identity proofs alone do not establish
independent failure domains, honest tick execution or resource
enforcement. Private proof files remain local; no claim about the
latest global head is made without a fresh live quorum.

The #65 private Fabric lab deliberately still denies real player
admission. Actual public/private Minecraft authentication and admission,
live governance transport, runtime permit integration, independent
worker enrollment, current consent and production enforcement remain
gated. #68 requires real combined gameplay on independent machines
and the #63 sustained capacity/service target remains unresolved.

## Local verification — 26 November 2026

The protocol and consensus suites passed 131 tests, including four
new role/creation/signature regressions. Actual CLI integration tests
passed seven authority scenarios (three new participation scenarios)
and two prior creation/permission scenarios. The public scenario used
the actual public draft, all three explicit CLI approvals and certificate
assembly, then confirmed that the old private #65 Fabric lab refused it.
The private scenario showed that even the founding validator is not
implicitly invited. A separate worker could volunteer without player
eligibility or validator enrollment. Revocation invalidated old proofs
and left unrelated roles unchanged. A single key could not authorize
worker enrollment or private invitations.

Concurrent verification processes consumed a challenge exactly once.
A real stdout ENOSPC failure did not reset consumption; replaced and
canceled challenges failed on replay. A tampered locally signed
challenge record was refused. Role proof consumption left the authority
journal byte-for-byte unchanged. All profiles retained empty V1 world,
control and runtime stores. The retained #65 private signed evidence
was independently reverified with the updated CLI (57 Fabric checks,
five unsafe startups, 34 worker refusals and two reference ticks).
No new actual Minecraft login or public listener was attempted.

## Local reproduction

```sh
umask 077
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-protocol -p swarm-consensus'
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-cli --test ownerless_authority_cli --test ownerless_creation_cli'
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo clippy --locked --offline -p swarm-cli --bin swarmcraft --test ownerless_authority_cli --test ownerless_creation_cli -- -D warnings'
```
