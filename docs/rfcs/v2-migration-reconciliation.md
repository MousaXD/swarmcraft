# V2 hosting authority reconciliation and durable cutover plan

Status: protocol, cryptographic review and replay primitives implemented;
durable cutover and live activation NOT implemented or approved.

## Exact source mapping (Sabathon #23)

- [PR #122](https://github.com/MousaXD/swarmcraft/pull/122) at
  `56b50274e7e173cd6573e678df6cd0a41963b2c0`: signed capability transcript,
  V2 challenge/action nonce books and independent source pins.
- [PR #125](https://github.com/MousaXD/swarmcraft/pull/125) at
  `c722ece5b86a41433644013bf1dabccd23142a13`: bounded signed migration
  envelope, individual voter signatures, stable and joint majorities.
- Selected source files were reconciled rather than merging either entire
  branch into the Sabathon integration tree. There is one protocol majority
  verifier, wrapped by a real Ed25519 consensus adapter. No change to V1
  wire discriminants, identity derivation, or signed historical tuples.

The current V1 membership genesis sequence is 0, with no predecessor. A
later sequence requires the predecessor hash. A historical, correctly
signed source config can have a different signer from the current
membership and epoch authority; independent replay must verify that config's
original signer. The migration proposal and initial V2 policy are signed by
the current accepted source authority. The consensus adapter pins both
identity and key to the independently selected accepted epoch.

The voter universe is independently selected, never nominated by the
proposal. Stable accepted membership needs a majority of its distinct
active voters, including eligible non-hosting voters; an independently
certified joint transition requires both old and new majorities, exact
predecessor and transition hashes, and real V1 certificate signatures.
A completed historical membership change does not permanently require
old voters. The read-only local review currently requires a stable source
and rejects any unresolved membership promise, recovery, pending join or
leave, wake, stop, transfer, unconfirmed runtime, or incomplete historical
publication. Finish the V1 transition before reviewing that source.

`swarm-cli::v2_migration_review::review_stable_v2_migration` selects the
world and checkpoint independently, walks the existing verified V1 history
and signed transition evidence, verifies all source snapshot data and an
exact current signed sleep boundary, then rechecks the source after
signature validation. It does not write a cutover, acquire a lease, or
guarantee future quiescence. The signed migration vote set is an inert
verification result, not an activation token.

The V2 transcript decoders bound transport identities and exact signature
lengths. A verifier-issued challenge binds the authenticated transport
generation, realm, migration/policy/membership/snapshot heads, fence,
candidate and signed consent revision/heap. Authentication consumes it
once, registers a monotonic deadline bounded by the original challenge and
signed expiry, and retains the registered session in a bounded book.
Disconnect, realm invalidation and challenge replacement revoke it;
action nonces additionally bind the exact registered session. A retained
session value cannot authorize new actions after registry invalidation or
restart. These APIs require independently negotiated transport and verified
canonical expectations. No live V2 stream is registered yet. Signed
candidate assertions are not evidence of actual local memory or opt-in.

## Separate durable implementation plan

The distinct-realm architecture in [hosting-policy-migration.md](hosting-policy-migration.md)
remains the boundary. Do not change a V1 world ID in place. A legacy V1
binary can keep an independent old V1 copy alive; it cannot be globally
disabled by a new sidecar. Never advertise that legacy copies have stopped.
The V2 identity is cryptographically distinct, and its storage, protocol,
authority and runtime control must be disjoint.

1. **Prepare (invisible):** Verify full local ancestry and exact final
   checkpoint/sleep boundary; pin accepted config, membership, epoch,
   fence, snapshot and proposed target. Validate both signed proposal and
   initial policy. Stage immutable, verified target bytes in a private
   same-filesystem location. No canonical target head or active listing.
   The preparation can be abandoned only before any irreversible promise.
2. **Freeze voting (irrevocable):** Implement a versioned durable per-voter
   source-world cutover promise under the same world transaction used for
   lease, recovery, membership, transfer and snapshot writes. Recheck the
   exact source head, verified runtime exclusion and complete checkpoint
   before promising one proposal. Persist the promise and source write
   fence before releasing a signed migration vote. A reboot cannot change
   the proposal, remove the fence or permit a V1 write/lease vote from
   that voter. Reject all conflicting migration targets. Require verified
   upgraded voting participants and membership-transition serialization;
   never count capability strings as proof. An honest upgraded majority
   intersection is required; independent legacy partitions are not fenced
   by this software. A stable or certified joint signed quorum is necessary,
   but an old transcript alone is not sufficient: prove the new durable
   freeze promise semantics in the migration certificate.
3. **Accept (single commit point):** Independently check the signed frozen
   source certificate and complete staged target. Under a durable
   compare-and-set of the exact source barrier, atomically publish one
   accepted cutover root (source barrier, target ID, proposal hash,
   certificate hash, initial policy and target membership, fresh epoch and
   fence). One source barrier has exactly one accepted target. All readers
   consult the same root; staging paths, votes and receipts are not
   canonical visibility. Fsync data and metadata before acknowledgement;
   an exact committed replay is idempotent, not a new cutover. No accepted
   root may be removed or downgraded by rollback or an older binary.
4. **Restart/recovery:** A crash before a promise leaves only invisible
   staging, which can be explicitly discarded. A crash after a promise
   resumes that same frozen proposal; incomplete, corrupt or uncertain
   journal state blocks source writes and target activation. No timeout
   unlock. A crash after acceptance reloads the accepted root and verifies
   referenced staged bytes before recovery. Only a separately designed
   signed cancellation protocol could safely release a freeze; it is
   intentionally not provided here. Divergent promises can strand the
   migration rather than risk double authority. Preserve recovery copies.
5. **Activation (separate gate):** Register actual mandatory V2 negotiation,
   connect session invalidation to canonical mutations/disconnects,
   implement domain-separated V2 lease/recovery/transfer/wake and
   canonical policy updates, and enforce target membership, local resource
   admission and explicit fresh consent at every producer and consumer.
   Old V1 packets, snapshots and control writes cannot act on the V2 root.
   Launch only after fresh majority-backed V2 authority and runtime proof.
   No automatic rollback to the V1 identity; an explicit recovered data
   copy is a different independent world.

## Evidence and remaining gates

- Protocol contract tests and real Ed25519 stable/joint certificate tests
  exercise source pins, historical config and membership semantics,
  signer forgery, duplicates, wrong majority and record bounds.
- Network tests cover wrong protocol, wrong head, replay, connection and
  realm invalidation, stale/replaced sessions, action nonce binding and
  monotonic expiry independent of wall clock.
- A real local-storage test reviews a signed sleeping V1 checkpoint,
  rejects a wrong source and pending membership promise, confirms signed
  history after reopen and rejects publishing V1 metadata at the distinct
  V2 realm ID.
- `swarm-consensus/tests/v2_cutover_plan_model.rs` is an executable
  **model of the planned transaction only**: it explores interruption,
  exact retry, competing joint votes, freeze and no rollback. It does NOT
  establish filesystem durability or production cutover support.

Required before activation: implemented atomic accepted-root and
freeze-vote persistence on every V1/V2 mutation path; real interruption and
corruption injection at fsync/rename/commit points; multi-process competing
proposals; independently authenticated mixed-version network rejection;
real JVM/consent/fence and installed cross-platform acceptance. All of
these are still outstanding. Keep live V2 inactive until those gates and
independent security review are complete.
