# Device loss, replacement, and identity recovery

Current V1 operations are deliberately limited. A device is a cryptographic
peer, not a contact label or an installation name.

## If the old device is still available

1. Ensure the surviving devices have a verified checkpoint. If the old
   device is the authority, complete a verified authority transfer first.
2. From the old device, use the signed `world leave <world-id>` operation.
   Wait for the joint old/new membership certificate to commit and for the
   surviving devices to converge. A staged leave is not a removal.
3. From the current authority, revoke the previously shared invitations
   using their exact signed tokens (see [invite lifecycle](INVITE_LIFECYCLE.md)).
   Revocation does not remove members, and an unknown token cannot be
   revoked merely by naming its recipient.
4. Initialize the replacement with a **new** device identity. Issue a fresh
   invite from the current authority and complete normal signed admission.
   Hosting preferences and EULA consent are not inherited.

A voluntary leave alone does not permanently ban a peer. A still-valid,
unrevoked reusable V1 bearer invite may request admission again. Do not
describe such grants as revoked or single-use.

## If a device is lost or compromised

Keep the remaining verified replicas and their durable control state.
Do not copy an old private key into a fresh store, clear promises, or
manually edit membership. A private key without its latest durable vote,
lease and recovery state does not constitute safe identity restoration.
A lost authority can be replaced only through the existing supported
quorum recovery flow. See [authority recovery](AUTHORITY_RECOVERY.md).

### Retire a lost non-authority device

V1 supports a deliberate authority-initiated retirement through the existing
signed joint-membership protocol. It is not an automatic ban, contact delete
or a quorum-shrinking shortcut.

1. Retain the surviving replicas and their control history. If the lost
   device was the current authority, first establish a certified successor
   via ordinary recovery. The current authority cannot retire itself.
2. Safely stop the world at a verified signed sleep checkpoint. Finish any
   existing membership, transfer or recovery transition; do not delete
   pending votes or unconfirmed-runtime markers.
3. Read `world status <world-id>` for the signed membership hash, signed
   authority epoch/fence and checkpoint manifest. Use `peers <world-id>`
   to identify the exact old cryptographic peer ID, not the display name.
4. From the current authority, explicitly run:

```bash
swarmcraft world retire-device <world-id> <lost-peer-id> \
  --expected-membership <signed-membership-hash> \
  --expected-epoch <epoch> --expected-fence <fence> \
  --expected-checkpoint <manifest-hash> --confirm
```

The command verifies complete local signed history and the selected
checkpoint, rejects solo or conflicting authority state and unaccounted
local JVMs, and records a local vote under the world transaction lock.
The signed proposed record retains the old identity with `banned=true`
and `authority_eligible=false`. It does not remove data or change the
accepted epoch, fence or snapshot. "Retirement staged; NOT committed"
means the daemon still needs a majority of distinct active voters in
**both** the old and new configurations. The authority cannot count the
absent device as a vote, nor stage a retirement when the surviving roster
could not possibly satisfy the old quorum. In a two-member world, a single
surviving member cannot retire the other.

Keep the replication daemons of the surviving voters running even while
Minecraft remains asleep. Reopening the daemon retains and resumes the
same durable promise; competing generation, transfer or recovery writes
are blocked. Check the committed signed roster after convergence. Do not
assume a staged retirement has revoked access. There is no timeout cancel
or V1 unban command. A conflicting durable promise can require explicit
review rather than unsafe automatic recovery.

After commitment, the banned identity cannot use its old reusable invite
to rejoin, even if the token remains valid for other identities. Subsequent
V1 membership changes must preserve banned identities. This applies to
upgraded peers with the signed current roster; a disconnected device can
still possess previously replicated bytes and stale local metadata.
Retirement cannot remotely erase those copies or globally stop an
independent legacy partition. Revoke known shared tokens separately,
especially if a lost device held bearer tokens usable by other identities.

Initialize a replacement with a new identity and issue a fresh invitation.
Do not inherit hosting preference, EULA consent or previous private keys.
No Desktop retirement button is exposed yet; the CLI requires explicit
operator confirmation.

If quorum cannot be recovered, an authorized surviving replica can create
a separate new private world from an independently verified checkpoint via
[recovered copy](RECOVERED_COPY.md). This preserves retained bytes, not the
original world identity or canonical authority. If no surviving verified
copy exists, there is no guaranteed data recovery.

## Local contact removal

Use the exact peer ID and public key supplied by a contact's `identity`
command. Labels are only local descriptions, not cryptographic identities.

```bash
swarmcraft contact add <peer-id> --public-key <hex-public-key> --label "Old PC"
swarmcraft contact list
swarmcraft contact remove <peer-id>
```

Removing a contact is idempotent and never changes signed world membership,
authority, invitation validity, or replicated bytes. Updated discovery
providers refuse fresh presence signatures for the forgotten identity even
if stale DHT provider hints persist, and local presence queries discard
observations if the contact changes before the query finishes. Already
issued short-lived signed observations cannot be remotely erased; they
expire normally. Use committed device retirement for world-access removal.

## Remaining implementation

- Add a reviewed Desktop operator flow for exact-hash confirmation and
  pending/committed retirement status, without treating a local UI action
  as a grant. Installed cross-platform acceptance and mixed-version
  security review remain separate.
- Expose contact management and presence refresh through a reviewed Desktop
  flow. The CLI uses the existing local contact store; no contact action
  grants world membership or authority.
- Do not export private recovery material automatically. Same-identity
  restoration requires a reviewed complete durable-state backup/restore
  protocol, exclusive identity possession, anti-rollback checks, and
  explicit export/import confirmation. Until that exists, new-identity
  replacement is the supported approach; key-only export/import is not.
