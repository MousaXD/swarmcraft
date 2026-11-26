# RFC: two-device exclusive-turn hosting (experimental, not active)

Roadmap #8 follows the zero-Java third-replica profile (#9). Keep ordinary
quorum worlds unchanged. This is a separate possible creation-time policy,
not permission to lower the majority for existing worlds. The existing
daemon, Fabric permit, signed route verifier and storage paths do not
recognize an exclusive-turn grant.

## Decision and availability

Start with exactly two fixed, mutually authorized device identities and
**online overlap at handoff**. The active device may later go offline
after an explicit completed handoff. There is no automatic crash election,
no lease expiry that creates a new writer, and no first-click-wins wake.
If either device is offline before synchronization and commitment, the
handoff is unavailable. An encrypted async dropbox (#31) may eventually
carry the already committed bytes and exact grant; it cannot grant a turn
itself or infer relinquishment. This RFC does not implement that adapter.

Loss of an uncommitted holder, committed grant, or recipient device can
make the original world unavailable indefinitely. A verified local copy
may be exported/imported as a clearly labeled **new world identity** (#11),
not an offline reset of the old turn. If the product needs automatic crash
failover, use #9 or a separately reviewed independent coordinator.

## Signed records and trust assumptions

Use a new versioned, domain-separated policy identifier signed at creation
by both fixed identities. Bind the world ID, ordered distinct application
public keys, initial holder, accepted checkpoint, epoch/fencing token,
compatibility policy and predecessor grant hash. Do not allow ordinary
membership changes, solo mode, legacy wake, V1 recovery or mixed policy
interpretation. Conversion of existing worlds is a separate migration RFC.

Each handoff identifies the exact predecessor grant and next checked
epoch/fencing token, source, recipient, complete manifest hash, canonical
state root, compatibility binding, and a monotonically increasing source
attempt identifier. The target's signed receipt acknowledges verified
bytes for that exact proposal only. It is not hosting authority or
permission to reset a runtime. The source's signed committed release
includes that receipt, commits to a single next holder, and is
cryptographically immutable. All record validation and durable writes are
required independently at both devices.

The first policy is crash-fault-tolerant (CFT) for cooperative devices,
with durable control journals and ordinary process failures. A modified
binary, a copied private identity, or complete rollback of a device's
control journal to a former held grant can break an offline protocol.
Without an independent monotonic anchor a device cannot detect every
complete disk rollback. Do not advertise Byzantine resistance or safe
offline restoration of a stale authority journal. Detectable missing,
corrupt, conflicting or regressed journal state is a denial. A restored
world directory alone does not reset its control journal.

## Durable state machine

Every participating process must hold the same local runtime admission
and generation-ownership locks, use the same accepted grant/checkpoint,
and obey a durable atomic journal. A live route still needs a fresh,
nonce-bound authenticated proof from the sole held grant and the exact
supervised Minecraft/Fabric runtime; merely possessing a release or
showing a cached Ready state cannot establish it.

| Phase | Source | Target | Possible writers |
|---|---|---|---|
| Held | Current durable holder, exact checkpoint | Non-holder | Source only |
| Prepared | Stop accepting game traffic; quiesce/checkpoint/exit; persist proposal | Verify exact checkpoint, persist receipt | None until cancel or commit |
| Cancel before release | Persist cancellation and a new attempt floor; revalidate own checkpoint, consent and runtime | Old receipt is inert | Source only |
| Committed release | Atomically persist relinquishment and next grant, invalidate all local old permits, keep exact outbox for retry | Receipt remains inert without commit | None until target activation |
| Activated | Permanently denied for old grant | Verify exact commit/receipt and retained bytes, durably install new grant and enforce consent/runtime | Target only |

Prepare requires completed checkpoint and proof of runtime quiescence
under the local exclusive lock. A timeout during save/exit is a denial,
not proof that the old runtime stopped. The target must retain the exact
checkpoint before receipt and reverify it at activation. If consent
changes, target activation/hosting is refused without restoring the
source's relinquished turn.

The source must not emit the committed release before the durable
relinquishment is installed and the old game stream/permit is fenced.
A crash after this write but before send/ack retries the exact outbox;
the source cannot cancel or reclaim it, even when disconnected. The
target cannot activate from Prepared or Accepted alone, from a partial
download, from an unrelated receipt, or from an unverified/future grant.
Duplicate delivery is idempotent for the same still-current accepted
grant; a release from an older generation cannot restore hosting.

The next handoff repeats the same state machine in the opposite direction.
Retain a monotonic durable accepted generation and predecessor history.
Counter exhaustion and divergent or non-prefix history remain blocked.

## Failure matrix

- Disconnect before receipt: source remains prepared or may explicitly
  cancel before commitment; target cannot write.
- Crash/restart before commitment: resume exact prepared state or cancel
  after checking durable journal; never infer commitment.
- Crash/restart after release: source remains retired and can resend only
  the exact committed release. Target can finish if its bytes verify.
- Target has a receipt but lost/corrupted checkpoint bytes: no target
  activation until the exact bytes are restored.
- Delayed, duplicate or conflicting message: exact current plan/grant,
  receipt and attempt match required. Never overwrite accepted history.
- Lost recipient after release: neither side may elect or mint another
  old-world writer. Use explicitly new-world recovery or a later reviewed
  coordinator.
- Restored old world save: deny writes against the durable accepted head;
  do not reset generation. Complete malicious control-journal rollback
  remains outside the CFT guarantee and blocks product claims.

## Executable design evidence and gates

`swarm-consensus/src/exclusive_turn_model.rs` is a **test-only** two-local-
journal model. It explores delayed/repeated messages, disconnects, canceled
attempts, irreversible release, restart, checkpoint loss and transfers
in both directions. Model messages stand for records produced by a
correct signer and verified against the exact proposal; the model does
not supply real cryptography, disk atomicity, process exit proof or
Minecraft routing. No production code calls it.

Before implementation/activation, review this threat model and design
versioned signed records, immutable atomic storage, real process and
route fencing, interrupted journal/outbox replay, explicit creation-time
opt-in, deny-only compatibility with older peers, and migration/rollback
policy. Then prove on two independent devices that the default quorum
rules are unchanged and the exclusive-turn world has at most one writer
under actual partition/crash/restart. Until those gates pass, recommend
the existing headless third replica rather than shipping an override.

### Signed reference contract (26 November 2026)

The new `swarm-protocol/src/exclusive_turn.rs` and
`swarm-consensus/src/exclusive_turn.rs` implement the separate version-1
cryptographic record contract. The explicit `scpass:` identity commits to
two canonically ordered distinct strong Ed25519 device keys, the initial
holder, creation nonce, compatibility and initial checkpoint. Format 1
means the fixed online-overlap policy; initial epoch/fence are both 1.
Both devices approve the exact pinned genesis. No conversion from V1 or
ownerless identities exists.

Each signed offer binds the exact prior grant, both generations, source,
recipient, monotonically increasing per-source attempt, compatibility and
checkpoint. A separately domain-signed recipient receipt names the exact
signed offer; the source signs the entire offer and receipt to commit a
release. The next grant commits to the complete signed release. Verification
requires both actual device signatures, exact identity and predecessor,
strict key validation, and checked counter advancement. Both source attempt
floors survive reverse transfers. Histories are bounded to 128 releases and
256 KiB canonical postcard bytes; decoding rejects trailing or noncanonical
encoding and incorrect signature lengths. Reconciliation verifies both
histories and requires exact prefix ancestry, refusing valid conflicting
CFT-violating chains rather than electing a winner.

The reference checkpoint binds nonzero manifest and state-root claims plus
an exact content commitment for 1 MiB or smaller opaque bytes. It is NOT a
Minecraft snapshot verifier; these fields and the reference payload must
never become an ordinary canonical grant or a live launch permit. The
record verifier proves only signed history. In particular, a malicious or
rolled-back source could sign conflicting releases; durable non-equivocation
and actual process quiescence are separate mandatory layers.

Five real-signature integration tests plus canonical history-codec coverage
passed; the full protocol/consensus run passed 139 tests, strict all-target
all-feature Clippy, formatting and workflow/release policy checks. No CI
was used. The next step is the isolated owner-only atomic reference journal
with cancel, exact receipt, irreversible release/outbox and restart-safe
acceptance; live process/route fencing and installed clients remain disabled.

### Local durable reference journal

The explicit `swarmcraft exclusive-turn` commands operate only on
`exclusive-turn-reference/<scpass hash>/` in an existing private identity
profile. They never open ordinary world storage or the live daemon.
`draft`, `approve`, `assemble` and `inspect` provide the separate signed
creation workflow; initialization requires both exact approvals, an
independently pinned world/grant, the original verified opaque bytes and
explicit confirmation. An existing directory without a journal is an
error, not a request to reset a lost grant. The initial holder and the
replica maintain independent journals; no shared authority file exists.

The owner-only Linux journal is signed by the existing local identity,
canonically encoded, bounded to 512 KiB and protected by one stable
exclusive lock per reference world. Every operation reloads and verifies
the full signed grant chain, local phase/role/attempt invariants and the
current and staged checkpoint bytes. Immutable opaque checkpoint files
are limited to 1 MiB each and 512 retained entries; creation and atomic
replacement synchronize the file and directory. Missing/corrupt bytes,
unsafe paths, symlinks, hardlinks, busy locks and malformed journals fail
closed. Complete disk rollback by the device owner remains outside the
CFT model; this is not an anti-rollback hardware trust anchor.

The holder may update only its opaque reference checkpoint. Preparing
persists the exact signed offer and advanced attempt before returning it;
while prepared, new reference mutation is refused. Cancel is permitted
only before release and cannot reuse the attempt. The recipient must
retain the exact opaque bytes and signed receipt before sending it;
newer attempts replace old staged receipts, while old/conflicting ones
are denied. A source commit checks the exact prepared offer and receipt,
then atomically advances its history and sets Released before exposing
any committed release. The exact still-current release remains in the
outbox for retry if output is lost. Only the intended recipient with its
matching durable receipt and verified bytes can accept that release.
Acceptance cannot restore a changed, prepared or retired turn. Reverse
transfer preserves the local canceled-attempt floor. No election or
reset command exists. Status exposes explicit false canonical authority,
write authority, runtime activation and player admission flags.

The three real subprocess regressions passed in 10.34 seconds and again
in 15.97 seconds after the strict-Clippy representation fix. The final
full CLI library regression passed 210 enabled tests (three separately
scoped cgroup tests ignored), strict all-target/all-feature CLI Clippy,
formatting and workflow/release checks. They drive
separate profiles through ordinary draft/approve/assemble/init, checkpoint,
prepare/cancel/reprepare, staging, irreversible commit, loss of the actual
commit stdout pipe, exact outbox export, acceptance and reverse transfer.
They also race cancel against commit, reject stale and malformed records,
check lock exclusion and journal/byte corruption, refuse symlink/hardlink
and nonprivate files, and deny reinitialization of missing state. No
canonical world or runtime is created. This is file-based reference
transport, NOT proof of online overlap, actual process quiescence, a
Minecraft save, a licensed player session or actual host partition safety.
A human-readable draft/record pin inspector, real runtime/route fencing
and production creation policy remain to be implemented and independently
reviewed before activation.

### Authenticated reference handoff

The two new `exclusive-turn network` commands use the existing bounded
Noise NN / ChaChaPoly transport primitives with separate exclusive-turn
prologue and transcript-signature domains. The prologue commits to the
exact world, prior grant, reviewed signed offer hash, source and recipient.
Both Ed25519 device identities are pinned to the independently verified
fixed roster. The responder approves a local copy of the signed offer,
not a proposal selected by a remote peer. No application data is
exchanged until the channel has authenticated both identities.

After the ordinary reference journal initialization and source
`prepare`, review the offer and its expected hash on both devices.
The source's status reports its prepared offer hash; the recipient must
obtain that pin and review the signed offer independently. Run:

```sh
# Recipient (existing profile, locally reviewed offer document)
swarmcraft --data-dir "$RECIPIENT_PROFILE" exclusive-turn network serve-handoff \
  --expected-world "$WORLD" --expected-grant "$BASE" \
  --expected-offer "$OFFER_HASH" --offer "$REVIEWED_OFFER" \
  --from "$SOURCE_PEER" --bind 127.0.0.1:0 --confirm

# Source (same independently reviewed world/base/offer, explicit endpoint)
swarmcraft --data-dir "$SOURCE_PROFILE" exclusive-turn network handoff \
  --expected-world "$WORLD" --expected-grant "$BASE" \
  --expected-offer "$OFFER_HASH" \
  --peer "$RECIPIENT_PEER@$LISTENER_ADDRESS" --confirm
```

The loopback example is for separate local test profiles; on two hosts
use an explicit private LAN listener and its actual private address.
Wildcard/public sockets and non-roster peers are refused. The one-shot
listener waits at most 15 seconds for one connection; a session has one
15-second monotonic I/O deadline (local durable I/O is not preempted).
Messages are bounded to 16 KiB canonical postcard records and at most
1 MiB of opaque checkpoint bytes in 32 KiB encrypted chunks.
The listener releases its local lock while waiting for the connection,
then reopens and rechecks the pinned state and holds the stable lock
through the session. The source holds its lock throughout the exchange.

1. The source sends its exact already-prepared signed offer and retained
   opaque checkpoint bytes. The target checks its locally reviewed
   offer and bytes, then durably stages the same receipt using the
   file-based workflow's shared transition.
2. The source verifies the receipt and atomically commits the exact
   release to its durable journal before sending it. There is no
   cancellation or reacquisition after that commit, even if the
   connection fails and the recipient has not accepted.
3. The target verifies the release against its exact staged receipt,
   durably accepts it, and only then signs a separate acknowledgment
   binding world, base, offer, grant, recipient, epoch, fence and
   checkpoint. The source checks every field and the recipient's
   signature. That acknowledgment is historical reference evidence,
   not a lease or proof of Minecraft runtime authority.
4. If receipt delivery fails before source commitment, the source is
   still prepared; it may explicitly cancel that exact attempt or retry.
   If the release or acknowledgment is lost after commitment, rerun both
   commands with the **same original base and offer pins**. The source
   reads only its exact still-current released outbox. A recipient that
   already accepted may acknowledge the identical release only while
   the accepted turn and checkpoint remain unchanged and held. A
   changed, prepared or later retired turn refuses old retries. No
   timeout resets any durable decision. Loss of the target can leave
   the original reference world unavailable.

The real local process regression uses a TCP frame proxy with no private
keys or plaintext access. It corrupts an encrypted checkpoint, drops a
durably staged receipt, drops the committed release after source
relinquishment, and drops the target's acceptance acknowledgment.
It verifies byte-identical journals on rejected operations and retries,
explicit cancellation before release only, different Noise prologues
after cancel/reprepare, exact signed acknowledgment, and reverse
transfer of 70,123 bytes across multiple encrypted frames. It rejects
public/wildcard listeners, an outsider, an unavailable endpoint and an
old retry after target checkpoint change. All four exclusive-turn CLI
tests passed (20.69 seconds). Strict all-target/all-feature CLI Clippy
passed. The shared transport regression initially passed 28 of 29
ownerless authority tests; the stalled-peer observation safely expired
its original deadline. That exact case passed on a separate rerun in
5.11 seconds, and both ownerless creation tests passed. No timeout or
quorum condition changed. [Retained output and source/artifact pins](../../research/exclusive-turn/evidence/2026-11-26-authenticated-handoff.txt).
The reference processes created no ordinary
world, runtime, control or permit state.

This is real authenticated TCP between disposable profiles on one Linux
machine, not physical network-partition qualification. The separate
file-based reference commands remain available for inspecting journal
behavior but cannot prove online overlap. Neither transport establishes
actual Minecraft checkpoint verification, quiescence, generation fencing,
game-stream closure, resource consent, installed clients or production
activation. Ordinary quorum worlds are unchanged.
