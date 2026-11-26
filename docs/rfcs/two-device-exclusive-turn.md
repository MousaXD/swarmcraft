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

### Real reference process and route fencing

The bounded `exclusive-turn writer` service is a separate **opaque-state
reference process**, not a Minecraft launcher. It holds the existing
stable journal lock for its whole lifetime. The listener and every
accepted TCP stream belong to that process; there is no detached worker
or child process. No ordinary world, runtime directory, control record,
Fabric permit or game gateway is created. A live writer therefore blocks
all other supported journal operations, including checkpoint replacement,
preparation, commitment, network handoff and another writer launch.
Before preparing a handoff the operator must stop the writer and acquire
the same lock. In-process execution can make no further state mutations
after the writer exits, including on SIGKILL.

The holder explicitly authorizes a bounded session service for the other
fixed device only. Startup independently checks exact world, grant and
checkpoint pins, verified local Held phase, fixed roster and private
listener. It accepts 1–32 sessions and a 1–120-second service window,
with an additional 10-second per-session monotonic I/O bound. Local
durable I/O is not preempted. Every accepted session rereads the signed
journal and checks the accepted head, holder, attempt and exact retained
bytes under the same stable lock. The authenticated Noise prologue binds
world, grant, epoch, fence, compatibility, holder and client; its
domains are separate from handoff and ordinary ownerless decisions.

```sh
swarmcraft --data-dir "$HOLDER_PROFILE" exclusive-turn writer serve \
  --expected-world "$WORLD" --expected-grant "$GRANT" \
  --expected-checkpoint "$CURRENT_COMMITMENT" \
  --from "$OTHER_PEER" --bind 127.0.0.1:0 --max-sessions 2 --confirm

swarmcraft --data-dir "$OTHER_PROFILE" exclusive-turn writer observe \
  --expected-world "$WORLD" --expected-grant "$GRANT" \
  --expected-checkpoint "$CURRENT_COMMITMENT" \
  --peer "$HOLDER_PEER@$LISTENER_ADDRESS" --confirm

swarmcraft --data-dir "$OTHER_PROFILE" exclusive-turn writer replace \
  --expected-world "$WORLD" --expected-grant "$GRANT" \
  --expected-checkpoint "$CURRENT_COMMITMENT" \
  --checkpoint "$NEW_OPAQUE_BYTES" \
  --peer "$HOLDER_PEER@$LISTENER_ADDRESS" --confirm
```

Each client generates a fresh request nonce; the holder verifies the
scope and exact content-based compare-and-swap pin. A replacement is
bounded to 1 MiB, and the holder independently derives both manifest
and state-root digests using separate **reference-only** domains, as
well as verifying the opaque content commitment. It uses the same
durable checkpoint transition as the local file command. No reply
precedes the verified durable publication. The signed response binds
the complete request including nonce, scope and previous commitment
to the resulting checkpoint. The client verifies all fields and the
fixed holder's signature. An observation or receipt is not a live
lease, game authorization, Minecraft snapshot or promise of continued
availability. Failure terminates the bounded writer service rather
than continuing on uncertain in-memory state. A lost write reply
requires fresh observation; stale content CAS refuses. This is not
exactly-once operation logging and content CAS alone does not detect an
intentional return to identical earlier bytes within the same grant.

Two new real subprocess regressions exercise authenticated observations
and mutation, two independent signed nonce receipts, competing writer
launch and preparation/transfer denial, hard-killing the writer while
its TCP handshake is open, closure of the old connection and listener,
cold reload of the exact saved bytes, a real encrypted handoff whose
committed release is lost, denial of both devices while the grant is
in flight, exact outbox retry, and a fresh recipient writer at epoch
and fence 2. Another fault proxy drops an already durable write
response, then verifies byte-identical journal retention, stale CAS
denial and new signed observations. All six exclusive-turn process
tests passed in the first run (34.82 seconds). The final lint-clean
run passed all six again (31.71 seconds), plus 210 enabled CLI library
tests (three delegated cgroup tests ignored), strict Clippy, formatting
and workflow/release checks. [Retained test output and source/artifact pins](../../research/exclusive-turn/evidence/2026-11-26-opaque-writer-fencing.txt).

This qualifies only a single in-process reference writer on disposable
Linux profiles. It does not prove JVM/descendant quiescence, real
Minecraft save integrity, production hosting consent, game-stream
fencing, or independent physical host behavior. The actual runtime
adapter must retain the machine-wide hosting reservation, verified
supervised process ownership, real checkpoint barrier and Fabric
attestation; opaque digests or a reference writer lock cannot substitute
for those requirements. No normal hosting admission was relaxed.

### Offline save-data manifest and authenticated blob transfer

The separate Linux `exclusive-turn save` adapter provides bounded
file-level integrity, capture and restore for an explicitly chosen
stopped Minecraft-shaped save directory. It is intentionally not a
Minecraft semantic/NBT parser, supervised runtime checkpoint barrier,
canonical snapshot importer or permission to start a JVM. Neither the
ordinary reference journal nor its handoff receipt implies the presence
of these blobs. A future live adapter must require verified complete
save data and quiescence before issuing its own release/activation.

Capture requires a nonempty regular `level.dat` and an existing safe
root `session.lock`. It acquires an exclusive nonblocking POSIX record
lock (compatible with Java FileChannel locks) for the entire capture and
publication, verifies the lock inode, refuses links and unsafe paths,
and compares the source file and directory identities, size, modification
and change times before/after reading and immediately before publication.
It assumes a cooperative local account and filesystem; the session
lock cannot rule out unrelated applications ignoring it, malicious
same-account mutation or complete journal rollback. It never writes
to the source. Root `session.lock` is transient and excluded from the
canonical state; a new empty lock file is created on offline restore.

The canonical bounded postcard manifest records a compatibility digest
and sorted ASCII portable relative paths, directories (including empty
directories), raw file lengths and domain-separated BLAKE3 content
digests. It rejects path aliases (including directory case collisions),
traversal, reserved names, links, special files, cross-device entries,
missing parents, duplicates, inconsistent blob lengths, unbounded
lengths and noncanonical serialization. The manifest hash commits to
all encoded fields; the independently derived state root commits to the
directory/file list. The existing reference commitment covers exactly
the manifest bytes, not the full save. Raw content-addressed blobs are
deduplicated and verified against their manifest descriptors. The limits
are 4,096 entries, 512 MiB per file, 2 GiB total logical bytes and a
1 MiB manifest. Capture/receive/restore use bounded 32 KiB file buffers,
a conservative 1 GiB free-space margin (a check, not an OS reservation),
owner-only temporary sibling directories, file/directory synchronization
and Linux atomic no-replace directory rename. An existing target is
never overwritten. A normal error removes private temporary state; a
hard-killed process may leave an unpublished scratch directory. After
a successful rename but lost output or acknowledgment, reverify the
existing complete destination rather than retrying over it.

```sh
swarmcraft exclusive-turn save capture \
  --source "$STOPPED_SAVE" --destination "$BUNDLE" \
  --compatibility "$COMPATIBILITY" --confirm

swarmcraft exclusive-turn save verify --bundle "$BUNDLE" \
  --expected-manifest "$MANIFEST" --expected-state-root "$ROOT" \
  --expected-compatibility "$COMPATIBILITY"

swarmcraft exclusive-turn save restore --bundle "$BUNDLE" \
  --expected-manifest "$MANIFEST" --expected-state-root "$ROOT" \
  --expected-compatibility "$COMPATIBILITY" \
  --destination "$NEW_OFFLINE_SAVE" --confirm
```

Use a new destination inside an existing private parent. To test the
separate network data workflow, update the holder's **reference**
checkpoint to `$BUNDLE/manifest.postcard` with the captured manifest
and state-root digests, then prepare an exact reference offer. Both
devices independently review the world, prior grant, offer and
compatibility. The source must hold the prepared journal lock for the
entire one-shot data transfer. The intended replica or released
recipient verifies the signed offer against its exact current base
and holds its own journal lock throughout. The Noise channel has
separate data-transfer and authentication domains, binds both fixed
identities, world/base/offer and uses a 15-second accept window and
120-second monotonic session I/O deadline. The manifest is received
and independently matched against all three signed checkpoint digests
and compatibility before blob reception. Blobs are streamed in
deterministic hash order, verified on receipt, fully reverified from
disk and published before an encrypted manifest-hash completion
marker is returned. No signed handoff receipt or grant is emitted;
neither local journal changes.

```sh
swarmcraft --data-dir "$SOURCE_PROFILE" exclusive-turn save serve \
  --expected-world "$WORLD" --expected-grant "$BASE" \
  --expected-offer "$OFFER_HASH" --bundle "$BUNDLE" \
  --bind 127.0.0.1:0 --confirm

swarmcraft --data-dir "$RECIPIENT_PROFILE" exclusive-turn save fetch \
  --expected-world "$WORLD" --expected-grant "$BASE" \
  --expected-offer "$OFFER_HASH" --offer "$REVIEWED_OFFER" \
  --destination "$RECIPIENT_BUNDLE" \
  --peer "$SOURCE_PEER@$LISTENER_ADDRESS" --confirm
```

This is a full transfer, not resumable blob synchronization; a failed
unpublished transfer can be retried with the exact original pins.
Both capture and restore output explicitly false canonical authority,
write authority, runtime activation, player admission, handoff receipt
and Minecraft gameplay flags. The 1 MiB opaque reference limit is
unchanged. Actual runtime compatibility, Minecraft save semantics,
Fabric checkpoint/stream fencing and production resource consent
still require their respective independent gates.

The two new real CLI subprocess regressions passed along with all six
existing exclusive-turn tests (8/8 in 32.22 seconds), strict all-target/
all-feature CLI Clippy, formatting, workflow and release checks. The
synthetic fixture has 2,621,469 logical bytes, including duplicate
1,310,721-byte region-shaped files, one empty file and one empty
directory. The tests exercise an actual competing POSIX session lock,
portable aliases, symlinks/hardlinks, exact restoration, corrupt and
extra blobs, noncanonical manifests, destination no-clobber, encrypted
blob corruption and retry, and byte-identical source and recipient
journals before/after transfer. [Retained output and source/artifact pins](../../research/exclusive-turn/evidence/2026-11-26-offline-save-transfer.txt).

### Genesis-committed save-backed reference policy (format 2)

The existing format-1 opaque reference policy is unchanged. A new
format-2 genesis commits to the save-backed policy using the same
canonical record envelope and separate world identity. Both devices
must independently approve the exact new identity; changing the format
changes the identity, initial grant and approval signing inputs.
No in-place upgrade, downgrade or migration from an existing opaque
reference world is permitted. The verified history retains this policy
through every release. Older binaries reject the new format. The
cryptographic verifier alone still proves signed history, not possession
of any file or runtime authority.

Creation now has explicit save-backed commands:

```sh
swarmcraft exclusive-turn save draft \
  --device-key "$A_KEY" --device-key "$B_KEY" \
  --initial-holder "$A_PEER" --bundle "$VERIFIED_INITIAL_BUNDLE"

# Run independently on each fixed device, with the reviewed genesis pin.
swarmcraft --data-dir "$PROFILE" exclusive-turn save approve \
  --draft "$DRAFT" --expected-world "$WORLD" \
  --bundle "$VERIFIED_INITIAL_BUNDLE" --confirm

# The ordinary assemble command still verifies both exact approvals.
swarmcraft exclusive-turn assemble --draft "$DRAFT" \
  --expected-world "$WORLD" --approval "$A_APPROVAL" --approval "$B_APPROVAL"

swarmcraft --data-dir "$PROFILE" exclusive-turn save init \
  --expected-world "$WORLD" --expected-grant "$INITIAL_GRANT" \
  --history "$HISTORY" --bundle "$VERIFIED_INITIAL_BUNDLE" --confirm
```

The draft, each approval and each initialization independently rederive
all three checkpoint digests and verify every blob. Initialization copies
the complete bundle into the local private journal directory under
`save-bundles/<reference commitment>/`, with atomic no-replace publication
and parent synchronization. Only then is the initial signed journal
published. The format-1 approve/init commands refuse format 2; signature
inspection explicitly distinguishes the signed policy from local data
verification. Current and staged bundles are fully verified from disk
on every cold journal load and before durable journal publication.
The managed store is bounded to 16 retained bundle entries per world
(including unpublished crash scratch), each subject to the preceding
size/disk limits. It has no automatic deletion or corruption repair.

The held device explicitly imports the next independently verified
offline bundle:

```sh
swarmcraft --data-dir "$HOLDER_PROFILE" exclusive-turn save checkpoint \
  --expected-world "$WORLD" --expected-grant "$GRANT" \
  --expected-checkpoint "$PREVIOUS_COMMITMENT" \
  --bundle "$NEW_BUNDLE" --confirm
```

The exact current checkpoint pin and signed compatibility must match.
The opaque checkpoint and writer operations are not available to a
format-2 world. Preparation remains a durable fence; the source cannot
mutate the checkpoint or restore a held save while prepared. The
recipient must fetch the prepared offer's complete data into its own
managed store before the existing authenticated network handoff may
begin. In format 2 omit the source's `save serve --bundle` and the
recipient's `save fetch --destination`: the paths are derived from the
exact reviewed journal/offer and cannot be overridden. The same offer
can retransmit already retained verified bytes (including after a lost
completion marker) without replacing any bundle or changing the
journal. Format 1 still requires the explicit external bundle/path.

The ordinary network handoff verifies the complete matching recipient
bundle before advertising its listener, after reacquiring the lock and
before signing the stage receipt. The source rechecks its own exact
bundle before committing the irreversible release. The recipient
rechecks retained bytes and exact staged receipt before accepting and
before acknowledging an identical already-accepted release. A corrupted
or missing current or staged blob fences further supported journal
operations, including retry. File-only stage, commit, release export
and accept commands refuse format 2; exact outbox recovery is through
the authenticated network exchange with the original pins. The
two-device CFT assumption and all original counter/identity checks
remain mandatory. A failed exchange can leave neither device held;
there is no automatic takeover, data repair or reset of a release.

The held reference device can restore the exact managed checkpoint
to a new offline directory outside the authority profile:

```sh
swarmcraft --data-dir "$HOLDER_PROFILE" exclusive-turn save restore-held \
  --expected-world "$WORLD" --expected-grant "$GRANT" \
  --expected-checkpoint "$COMMITMENT" \
  --destination "$NEW_OFFLINE_SAVE" --confirm
```

This is a data materialization operation under the journal lock, NOT
permission to start a JVM. An exported offline save is not a live
exclusive writer, and its later use outside this API is not fenced.
The generic offline data-only restore remains available independently
of authority state. Neither format creates canonical authority,
Minecraft runtime activation, player admission or a freshness lease.
Full rehash/fsync is intentionally conservative and may cause refusal
on slow disks or large saves; the original handoff I/O deadlines are
unchanged, and local filesystem I/O is not preemptible. The future
production adapter must independently validate runtime compatibility,
actual Minecraft semantics and save barrier, supervise/retire the JVM
and all descendants, enforce Fabric epoch/route fencing, and apply
explicit resource consent. Do not use these reference records as
production runtime grants.

The new protocol regression verifies the format-committed identity,
invalid old approval reuse, no cross-policy release, downgrade refusal
and preservation through verified release history. All 140 protocol
and consensus tests passed. The new real two-profile subprocess gate
requires full initial approvals, refuses manifest-only init, opaque
writer and file-only handoff; captures changed save bytes, refuses
missing/corrupt bundle data before stage and after a committed release
is lost, then accepts only the exact retry. It independently verifies
the signed recipient acknowledgment, restores exact data under the held
grant, and returns the turn at epoch and fence 3 using an existing
verified bundle. All nine exclusive-turn CLI tests passed in 52.61s.
These are synthetic save-tree and TCP process tests, not actual
Minecraft runtime or independent-device acceptance.

The final CLI library regression passed 210 enabled tests in 28.09s
(three delegated cgroup tests ignored). Strict all-target/all-feature
protocol, consensus and CLI Clippy, formatting, workflow and release
checks passed. [Retained output and source/artifact pins](../../research/exclusive-turn/evidence/2026-11-26-save-backed-policy.txt).
