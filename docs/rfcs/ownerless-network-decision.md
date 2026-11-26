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
The transport itself neither assembles nor publishes a certificate.

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

The CFT threat model, bounded history and trusted profile/disk
assumptions remain unchanged. An OS-level key/profile clone or complete
disk rollback is outside those assumptions. This application binding
of Noise requires independent protocol review before production use.
There is no automatic proposer selection, certificate dissemination,
restart-safe distributed certificate reconciliation, canonical Minecraft
publication, or game authorization. An unavailable majority or split
pending votes can halt decisions. A signed vote is not a lease, an
attestation of rules-compliant execution, or a write permit. Production
ownerless and distributed gameplay remain disabled.
