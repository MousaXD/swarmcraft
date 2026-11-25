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
