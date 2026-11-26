# Ownerless shared authority (#66) — bounded CFT reference contract

**Status:** isolated local reference history. This is not a live quorum,
accepted Minecraft save, worker enrollment, game route or authority permit.

## Signed decisions and immutable identity

The [creation certificate](ownerless-creation.md) remains the sole genesis
anchor. It has no creator key and fixes the no-OP policy and compatibility.
The separate ownerless history starts at sequence zero and reference fence
one. Its initial commitment is checked against bounded opaque checkpoint
bytes by the local journal. The reference head hash anchors the exact signed
genesis identity; later heads hash the proposal, not its potentially different
valid certificate witness subsets.

Every proposal binds the ownerless world, next sequence, exact parent head,
policy hash, compatibility hash, base fence and base checkpoint. There are
only three allowed actions:

| Action | Required certificate | Effect |
| --- | --- | --- |
| Checkpoint | Strict majority of current validators | New nonzero, different checkpoint commitment; same fence |
| Recover | Strict majority of current validators | Exact same checkpoint bytes; fence increments |
| Reconfigure | Strict majority of the old **and** proposed rosters, plus every newly admitted validator's explicit vote | Exact same checkpoint; new roster; fence increments |

No action changes the immutable policy or compatibility, creates a different
world, grants an executor, converts donated compute into voting rights, or
gives the founder a privileged vote. Recovery does not use a special founder
signature. A removed validator may retain and synchronize verified data as
a replica; it cannot vote at the new head unless explicitly enrolled again.

Votes are domain-separated strict Ed25519 signatures over the exact proposal
hash and validator identity. Roster keys are bounded (3–15), sorted, unique,
canonical prime-order keys; vote sets are sorted and unique, with no unknown
signer or duplicate. Each candidate certificate is verified against the
current roster derived from the entire preceding signed history. During a
joint transition a new validator may sign that exact enrollment only, not a
checkpoint or recovery under the old roster. All newly proposed keys must
explicitly consent even if overlapping members already satisfy both majorities.

## Durable voting and recovery

The Linux reference journal lives under
`<data-dir>/ownerless-authority-lab-v1/<ownerless-world-hash>/`, entirely
separate from V1 worlds, authority control, runtime and consent records.
It uses an existing owner-only identity and a stable per-world kernel lock.
Initialization is create-only; missing state in an existing directory is
not an empty journal. A locally signed journal contains the accepted
history and, at most, one exact pending vote for that head. The store
validates regular private files and directories, rejects symbolic/hard
links to journal files, and refuses invalid local signatures or checkpoint
bytes. The lock and journal are never replaced by a timeout/reset command.

Each vote explicitly pins the reviewed head and proposal. Before output,
the process verifies and durably retains exact candidate checkpoint bytes,
then atomically publishes its signed pending vote with file and directory
synchronization. An identical retry returns the same signature. A different
proposal at the same accepted parent is refused. On a previous uncertain
write, file and parent-directory sync are retried before reissuing a vote.
A crash before vote output does not unlock the parent.

Accepting a history requires the independently expected final head, verified
genesis and every certificate, exact accepted ancestry, and bytes matching
the final state commitment. A shorter or different accepted prefix is refused.
Different valid witness subsets for the same proposals do not change a head
or clear a pending vote. Only an actually certified extension can resolve
the lock, including a competing proposal certified by the other majority.
The new head and pending-vote clearance are one journal replacement; the
checkpoint bytes are durable first.

The old/current majority intersection makes conflicting certificates
impossible under the **honest durable one-vote-per-parent** CFT assumption.
Joint approval transfers that property to the next roster. A live quorum
is not inferred from a historical certificate. The governance protocol intentionally
has no leader election, vote timeout, automatic view change, lease or live
canonical authority. The separate [network observation follow-up](ownerless-network-observation.md)
provides nonce-bound private TCP head observations, not live authority.
The [encrypted reference decision exchange](ownerless-network-decision.md)
transports explicitly reviewed proposals, candidate bytes and durable votes;
it also provides explicit bounded quorum collection, local certified
publication, and encrypted checkpoint/history synchronization with
restart-safe receipts. It does not activate gameplay. Different locked minority proposals can stall permanently;
the design fails closed rather than resetting votes to manufacture progress.

A minority cannot certify another head or recovery fence. It can export
the exact locally verified accepted checkpoint bytes to a new, no-clobber
file and separately export the full signed history for independent review.
This is a recovery copy, not a permission to overwrite or resume the same
live world. The source journal and accepted head remain unchanged. A local
copy is only as current as its accepted history; no offline operation
claims the latest global head.

## Local CLI workflow

These commands operate on disposable explicitly selected profiles; they
do not initialize or open the ordinary canonical world store. The
`ownerless authority` subcommand is non-activatable. For a new lab genesis,
use `state-hash --checkpoint <file>` to obtain the initial commitment
before all founding validators review and approve the exact #64 draft.

```sh
swarmcraft ownerless authority genesis-history \
  --certificate creation.json --expected-world "$WORLD" > history.json
swarmcraft ownerless authority inspect \
  --history history.json --expected-world "$WORLD"
swarmcraft --data-dir /private/validator-a ownerless authority init \
  --history history.json --expected-world "$WORLD" --expected-head "$HEAD" \
  --checkpoint initial.bin
swarmcraft ownerless authority propose \
  --history history.json --expected-world "$WORLD" --expected-head "$HEAD" \
  --checkpoint next.bin > proposal.json
```

Each validator independently reviews the history, checkpoint and proposal
including its displayed proposal hash. Draft recovery using `--recover`
instead of `--checkpoint`; draft a joint transition with repeated
`--validator-key <hex>` for the entire proposed roster. Transfer reviewed
documents manually between profiles; no network protocol is enabled.

```sh
swarmcraft --data-dir /private/validator-a ownerless authority vote \
  --expected-world "$WORLD" --expected-head "$HEAD" \
  --proposal proposal.json --expected-proposal "$PROPOSAL" \
  --checkpoint next.bin --confirm > vote-a.json
# A separate eligible validator performs its own reviewed vote.
swarmcraft ownerless authority assemble \
  --history history.json --expected-world "$WORLD" --expected-head "$HEAD" \
  --proposal proposal.json --expected-proposal "$PROPOSAL" \
  --vote vote-a.json --vote vote-b.json > next-history.json
swarmcraft --data-dir /private/validator-a ownerless authority accept \
  --history next-history.json --expected-world "$WORLD" \
  --expected-head "$PROPOSAL" --checkpoint next.bin
```

For recovery or roster-only transitions the checkpoint file must still
match the exact previously accepted state. Use `status` with the pinned
world to inspect the accepted head and pending lock, `export-history` to
retrieve its proof, and `recovery-copy --expected-world "$WORLD"
--expected-head "$HEAD" --output recovered.bin` for a verified local copy.
Every status explicitly sets live quorum, player admission, and runtime
activation false. The command cannot be used to make a V1 world or permit.

The deliberately bounded reference format permits at most 128 decisions,
4 MiB history JSON and 1 MiB nonempty checkpoint bytes. No history compaction,
large real save transfer or production checkpoint format is implied.

The [#67 participation contract](ownerless-participation.md) appends separate
majority-certified private invitations and worker caps to this history.
Existing checkpoint/recovery/reconfiguration encodings are unchanged.

## CFT and remaining gates

This is not Byzantine consensus. Valid signatures alone cannot detect
equivocation across independently observed histories; the verifier
requires durable non-equivocating honest validators. A hostile OS owner
can clone keys, replace the entire signed journal with an older version,
delete the complete profile, change binaries, or sign externally. Complete
disk rollback, copied identity profiles and malicious execution are outside
the CFT assumption; do not recreate a lost validator journal using the
same key. Journal signatures catch corruption, not malicious local rollback.
New or replaced physical failure domains require separate operational
enrollment and review.

The reference tests exercise real process persistence on one Linux
workstation, not an independently deployed network or real Minecraft
accounts. There is no automatic host selection/transfer, canonical
Minecraft state transition, fresh route proof, or production worker
activation. #67 must separate public/private authenticated player admission,
validator enrollment and consented worker eligibility. #68 and the #63
throughput gates require actual independent-machine combined gameplay.

## Local verification — 26 November 2026

The complete protocol and consensus suites passed (127 tests including five
new ownerless-authority contract tests). Four actual CLI integration
scenarios and both prior creation/permission regressions passed. Five
separate temporary profiles in the founder-offline scenario represented
three founding validators, a proposed replacement and an outsider. Four
quorum-certified decisions advanced the reference history: a checkpoint,
recovery, joint roster replacement and another checkpoint. The original
founder journal remained byte-for-byte unchanged during the first three
decisions; after catch-up the removed key could not vote. A single surviving
validator could not certify recovery but exported exact accepted bytes
and the full signed proof.

Concurrent CLI processes attempting conflicting votes on the same journal
produced one successful vote. The losing process and later retries of the
conflict were refused. A real stdout ENOSPC failure on a separate voter did
not clear its lock. Another integration regression verifies that changing
only certificate witnesses cannot unlock a vote, but a competing majority-
certified extension can. Missing, malformed, signature-tampered, linked
journals and corrupt checkpoint bytes were refused. The organizer did not
acquire an identity; ordinary world/control/runtime stores stayed empty.

A test-only child process called the actual durable vote publication
routine and received SIGKILL while holding its lock, before emitting its
signature. A fresh open recovered the same pending vote and checkpoint,
refused a conflicting proposal, and returned the exact original signature
on retry. This is process-death testing, not simulated disk/power failure.
Strict Clippy passed for protocol/consensus all targets and the CLI binary
and two relevant integration targets. All verification was local.

## Local reproduction

```sh
umask 077
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-protocol -p swarm-consensus'
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-cli --test ownerless_authority_cli --test ownerless_creation_cli'
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-cli --bin swarmcraft ownerless_authority::tests::hard_kill_after_durable_vote'
```
