# Encrypted ownerless reference decision exchange (#66–68)

Scope: isolated, explicitly reviewed reference decisions on Linux. This is
not live Minecraft authority, a freshness lease, a canonical game writer,
automatic consensus, or a worker permit.

## Vote transport

Each side starts from its own existing signed journal, exact world and
accepted head, and independently reviewed proposal document/hash. A request
initiator must be a current validator; the target must be an eligible
validator for that exact proposal. The sole exception to current roster
membership on the responder side is an explicitly proposed entrant in a
joint transition, using the exact proposed key and explicit confirmation.
Unrelated nonmembers cannot initiate or respond. No enrollment or admission
is performed just by authenticating a connection.

The channel uses the existing locked Snow 0.10 implementation of
Noise_NN_25519_ChaChaPoly_BLAKE2s with fresh ephemeral keys. The Noise
prologue binds a dedicated versioned domain, world, accepted head,
proposal hash, initiator and responder IDs. Both sides authenticate
the handshake hash with their Ed25519 application identities, using a
separate signing domain and distinct initiator/responder roles. Keys are
selected from the independently verified roster or exact reviewed entrant
list, never provided by the remote endpoint. The initiator verifies
the responder before sending its authentication signature; the responder
verifies the initiator before accepting decision data. A session cannot
reuse an authentication signature from another Noise handshake or role.

After authentication, the initiator sends the exact proposal and opaque
candidate checkpoint. The responder verifies the proposal equals its
local review, reopens the same stable per-world lock nonblockingly,
verifies its signed history and accepted checkpoint again, and checks
that the pinned head, identity and proposal have not changed. It calls
the existing durable vote operation: exact checkpoint commitment, key
eligibility, one vote per parent, retained checkpoint fsync, signed journal
replacement and directory fsync, then returns the vote. The initiator
revalidates its local head and review and verifies the signature of the
exact targeted validator before emitting the vote. No pending vote is
cleared on disconnect, deadline, output failure, or same-head witness
change. Restart with the same reviewed proposal returns the exact
durable vote; a different proposal at the same parent remains refused.

The transport is one-shot and accepts only explicit loopback, private
IPv4 or IPv6 ULA addresses. No public listener, discovery, implicit
port forwarding or daemon integration is provided. It uses length-
prefixed Noise frames, at most 32 KiB of plaintext plus the authentication
tag, with encrypted logical message lengths and fixed chunk boundaries.
Proposal/vote documents are capped at 64 KiB and checkpoint data at
1 MiB. The listener has a 15-second accept window and each connection
has a single 10-second monotonic deadline, not a renewed per-byte idle
timeout. Private storage stalls may delay process exit; expired sessions
do not return success. An unauthenticated peer can still consume a
one-shot listener or infer timing, frame sizes and addresses. This is
not anonymity, traffic-flow confidentiality, or denial-of-service
resistance.

## Example

With the same independently reviewed proposal available on both
validator profiles, run on the target:

```sh
swarmcraft --data-dir /private/validator-b ownerless authority decision serve-vote \
  --expected-world "$WORLD" --expected-head "$HEAD" \
  --proposal reviewed.json --expected-proposal "$PROPOSAL" \
  --from "$VALIDATOR_A" --bind 127.0.0.1:0 --confirm
```

Use the listener's printed address on the initiating profile:

```sh
swarmcraft --data-dir /private/validator-a ownerless authority decision request-vote \
  --expected-world "$WORLD" --expected-head "$HEAD" \
  --proposal reviewed.json --expected-proposal "$PROPOSAL" \
  --checkpoint candidate.bin --peer "$VALIDATOR_B@$ADDRESS" --confirm > vote-b.json
```

The emitted vote is the existing signed reference-vote format. Existing
offline certificate assembly still verifies distinct signers, current
majority, joint proposed majority and every entrant's approval. Existing
history acceptance validates the exact checkpoint and accepted ancestry.
Individual vote requests neither assemble nor publish a certificate.

## Certified collection and checkpoint reconciliation

`decision collect` uses the same reviewed proposal, current accepted
parent, and candidate checkpoint as `request-vote`. It checks the explicit
distinct validator/endpoint list before persisting the collector's own
vote. The configured identities must be sufficient for the old majority,
and, for a joint transition, the new majority and every entrant.
The collector must remain in the proposed roster. This deliberately
excludes a departing collector; the existing individual vote/assembly
workflow remains available.

After the collector durably votes, it requests the remote votes in
parallel. All requests share a seven-second network deadline within a
ten-second collection window, reserving time for local verification and
durable publication. Only the exact pinned validator's verified vote is
counted. The existing certificate verifier checks the actual signer set,
including joint approval. The collector reopens its journal, rechecks
the reviewed proposal and exact parent, and atomically accepts the
certified history and exact checkpoint locally. A missing quorum leaves
the local and remote vote locks in place; it never resets a vote. A
retry of the same proposal can recover the same durable votes. If the
collector already accepted that exact decision, it exports the retained
history after validating the parent, reviewed proposal and exact
checkpoint, without contacting peers or mutating the journal. A
different or newer local head is not retargeted automatically.

With the target validators listening using the preceding `serve-vote`
command (each independently reviewing the proposal):

```sh
swarmcraft --data-dir /private/validator-a ownerless authority decision collect \
  --expected-world "$WORLD" --expected-head "$BASE" \
  --proposal reviewed.json --expected-proposal "$TARGET" \
  --checkpoint candidate.bin --peer "$VALIDATOR_B@$ADDRESS" \
  --confirm > certified-history.json
```

This output is the ordinary signed history format. Local acceptance is
not a distributed commit notification. For dissemination, both parties
explicitly pin the same world, accepted base, target, and peer. The
sender must be a validator in the base and target rosters; the recipient
must be in the target roster (including a certified entrant). A sender
that was not a member at the recipient's base is intentionally
unsupported by this bounded reference protocol. The sender must have
already accepted the exact target locally. The recipient must have an
existing verified journal at the exact base or exact target. No network
initialization, unapproved enrollment, or automatic head discovery.

```sh
swarmcraft --data-dir /private/validator-b ownerless authority decision serve-sync \
  --expected-world "$WORLD" --expected-base "$BASE" --expected-head "$TARGET" \
  --from "$VALIDATOR_A" --bind 127.0.0.1:0 --confirm

swarmcraft --data-dir /private/validator-a ownerless authority decision push-sync \
  --expected-world "$WORLD" --expected-base "$BASE" --expected-head "$TARGET" \
  --peer "$VALIDATOR_B@$ADDRESS" --confirm
```

Sync uses a separate Noise prologue domain from voting, and binds the
world, exact base, target, sender and recipient. After authentication,
the sender rechecks its local accepted target and eligibility under
the stable journal lock and sends only the bounded signed history
(4 MiB maximum) and exact checkpoint (1 MiB maximum). The receiver
verifies the entire history, pinned ancestry, sequence, fences, policy,
compatibility, and both peers' roster eligibility. It reopens its own
stable lock, refuses rollback or a conflicting accepted chain, verifies
the checkpoint commitment, and uses the same durable acceptance
operation as the offline CLI. It signs the receipt only after
publication, binding world, base, target, sequence, fence, checkpoint
and recipient. The sender checks the exact receipt and signature.
The signed body uses the protocol's binary hash/ID JSON serialization.

The recipient can restart and accept a certified multi-decision suffix
from its exact base. Loss of the receipt after publication does not roll
back the receiver. A retry at the exact accepted target verifies the
checkpoint and signs the same receipt without replacing the journal
or clearing a vote for a later proposal. A sender or recipient that
has advanced past the target refuses the stale operation. The receipt
is a historical reference publication acknowledgment, not a freshness
lease, proof of future availability, or a canonical game permit.

## Recovery after permanent collector loss

The original collector is not a recovery authority. A surviving validator
can reconstruct the same decision from existing durable signed votes,
or retrieve a complete certificate from another validator that has
already accepted it. No new vote is cast during this operation.

Both operators explicitly review the same world, base head, exact
proposal document and target hash. A requester must be a validator in
the base and, for a roster transition, must remain in the target
roster. A responder must be eligible to vote on that reviewed proposal;
an accepted-target responder must still belong to its target roster.
A pending old-roster validator may supply its existing vote for a
joint transition even when that decision removes it. An entrant may
serve its previously persisted enrollment vote, but cannot initiate
recovery from a roster to which it does not yet belong.

```sh
swarmcraft --data-dir /private/validator-b ownerless authority decision serve-recovery \
  --expected-world "$WORLD" --expected-head "$BASE" \
  --proposal reviewed.json --expected-proposal "$TARGET" \
  --from "$VALIDATOR_A" --bind 127.0.0.1:0 --confirm

swarmcraft --data-dir /private/validator-a ownerless authority decision recover \
  --expected-world "$WORLD" --expected-head "$BASE" \
  --proposal reviewed.json --expected-proposal "$TARGET" \
  --peer "$VALIDATOR_B@$ADDRESS" --confirm > recovered-history.json
```

The recovery Noise domain is distinct from voting and synchronization.
The responder revalidates its journal and membership under the stable
lock after authenticating the requester, then sends either its exact
pending signed vote or the accepted certificate for this exact next
decision, plus the retained checkpoint bytes. It never signs a vote,
changes its journal, or discloses evidence from an unrelated proposal.
An advanced or conflicting accepted head is refused.

Recovery uses the same bounded parallel collection window and distinct
explicit peer identities/endpoints as collection. It verifies each
checkpoint commitment and exact byte equality, verifies each
pending vote against its authenticated voter identity, and verifies
any returned complete certificate against its independently accepted
base and exact proposal. It can reconstruct a certificate only from
already-existing votes meeting the normal current and joint majority
rules and every entrant's explicit approval. A single surviving
complete certificate is sufficient for historical acceptance, not
evidence of a currently available majority. Insufficient evidence
leaves the requester journal unchanged; only a verified certificate
can resolve a pending conflict. A retry at the accepted target
exports the local history, preserving any later pending vote.

Recovery is deliberately scoped to one explicitly identified decision
from an existing local verified base. No automatic discovery of
unknown proposals, background recovery service, or reset of a split
vote is provided. If too few votes or no valid certificate survive,
the operation fails closed. Subsequent dissemination uses the existing
certified sync command.

## Local verification (26 November 2026)

Fourteen authority CLI and two creation CLI scenarios passed on Linux,
including three new real TCP subprocess regressions:

- A full 1 MiB candidate crossed the actual encrypted channel in 32
  authenticated chunks. A TCP proxy captured the framed exchange and
  checked that a known checkpoint marker did not appear in plaintext.
  Altering the final ciphertext chunk refused the request without changing
  the responder journal. Dropping the actual encrypted vote response after
  the responder wrote it left the signed pending vote durable. Restart
  refused a conflicting proposal and returned the same vote byte for
  byte. The two remaining validators then assembled and accepted the
  checkpoint with the founder journal unchanged.
- Wrong expected responder and wrong initiating validator were refused.
  A nonmember could not initiate, and a responder whose accepted head
  advanced after listening refused the old decision without modifying
  the advanced journal.
- A joint roster change received remote votes from an existing validator
  and an explicitly reviewed entrant. An unrelated nonmember could not
  serve a vote; assembly without the entrant failed; the full signed
  certificate advanced the reference fence with the expected roster.

Strict targeted Clippy, formatting, workflow-policy and release-version checks
also passed. All three regressions verified that no live world or runtime/control
records were created. This is one-machine process-level evidence, not an
independent cryptographic audit or physical-network qualification.

```sh
umask 077
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'CARGO_BUILD_JOBS=2 cargo test --locked --offline --jobs 2 -p swarm-cli --test ownerless_authority_cli --test ownerless_creation_cli'
```

## Acceptance boundary

### Certified collection/sync verification (26 November 2026)

The expanded local run passed all 16 authority CLI and two creation CLI
integration tests, including two new real TCP/process scenarios. The
first test run reached signed receipt checks but failed two assertions
that compared protocol-serialized hashes with their display strings;
the corrected typed assertions passed without changing the wire format
or signature checks.

- Duplicate voters/endpoints fail before a vote is locked. A missing
  majority leaves the exact durable pending proposal; a conflicting
  retry is refused. The next invocation obtains a valid majority even
  with a silent optional endpoint. The founder's journal remains
  unchanged. The accepted certificate can be exported again without
  any remote connection or local journal mutation.
- Altered encrypted checkpoint data leaves the receiver's pending
  journal byte-identical. A lost real signed sync receipt leaves the
  certified checkpoint accepted. After restart, an exact same-target
  retry returns a verified receipt and preserves a later pending vote.
- Joint collection refuses a missing entrant before locking the local
  vote, requires the actual joint certificate, and publishes the
  new fence. The sender refuses removed and unrelated recipients.
  A newly admitted validator receives the certified target and votes
  in the new roster. A surviving validator that missed the first
  transition accepts the full two-decision suffix from its pinned
  genesis; stale targets are then refused.

Both new scenarios assert no ordinary live world/control/runtime
records are created. The results qualify the explicit isolated
reference workflow only, not physical independent hosts.

### Collector-loss recovery verification (26 November 2026)

All 19 authority CLI and two creation CLI scenarios passed, including
three new real TCP/process scenarios:

- The founder collected a decision from two other validators and
  then was never contacted again. The remaining validators rebuilt
  the certificate solely from their two retained votes and exact
  checkpoint. Unreachable and wrong-identity endpoints left both
  pending journals unchanged. The exact recovered history was
  exported again without a remote connection or journal replacement,
  then synchronized to the other surviving validator.
- After one surviving validator received the original certificate,
  another validator with a conflicting pending vote recovered that
  complete certificate without fresh votes or any contact with the
  original collector. Only the certified extension resolved the
  conflicting lock. The responder and founder journals remained
  byte-identical.
- After the collector of a joint transition disappeared, recovery
  refused a requester removed by the reviewed target. Insufficient
  old or new majorities failed without changing the pending journal.
  The surviving old validator, departing validator's existing vote,
  and entrant's existing enrollment vote formed the exact joint
  certificate. The reference fence advanced, the entrant received
  the certified checkpoint, and dissemination to the removed
  validator was refused.

No ordinary world, control or runtime records were created by these
scenarios. The collector's absence is modeled by no further process
or network access to its profile; this is not independent-host failure
qualification.

The CFT threat model, bounded history and trusted profile/disk
assumptions remain unchanged. An OS-level key/profile clone or complete
disk rollback is outside those assumptions. This application binding
of Noise requires independent protocol review before production use.
There is no automatic proposer selection, persistent dissemination
or recovery service, canonical Minecraft publication, or game authorization.
The explicit collection, recovery and synchronization commands do not establish
the globally latest head. An unavailable majority or split
pending votes can halt decisions. A signed vote is not a lease, an
attestation of rules-compliant execution, or a write permit. Production
ownerless and distributed gameplay remain disabled.
