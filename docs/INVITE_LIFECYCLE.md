# Private world invitations and membership

An invitation is a signed reusable bearer grant, not a membership
certificate. Anyone who receives the complete token can request admission
while it is valid. Admission still requires the current issuer and the
existing joint membership approval rules.

## Inspect and revoke

```sh
swarmcraft invite create "$WORLD" --expires-minutes 60
swarmcraft invite inspect "$INVITE"
swarmcraft invite revoke "$INVITE"
# Machine-readable, exact-grant confirmation (also used by Desktop):
swarmcraft invite inspect "$INVITE" --json
swarmcraft invite revoke "$INVITE" --expected-world "$WORLD" --expected-invite-id "$INVITE_ID" --json
```

Keep the token private; do not paste it into public logs. Inspection prints
the signed grant ID, issuer, world and expiry without printing the token
or transport hints. Only an issuer's own local revocation record is
authoritative for that issuer's revocation decision. A valid expiry alone
does not prove a recipient can join.

Invites can be created for 1–1440 minutes. A new admission preparation
requires a currently valid, unrevoked invite issued by the current
authority. Revocation is persisted under the exact signed grant hash,
is idempotent, and survives process restart. It does not revoke other
tokens for the same world.

Revocation and the initial admission promise share a world transaction
lock. If a membership decision has already been durably prepared,
revocation is refused rather than pretending that a pending quorum
decision can be canceled. Resolve the membership decision first. Expiry
does not retroactively erase a signed durable decision either. A
recipient who has already become a member does not need a fresh invite
to use that existing membership.

Desktop: select the world, open **Manage invites**, paste the signed
token and inspect it. The verified receipt shows its world, issuer,
expiry, reuse policy, and locally known status. Revocation requires an
explicit confirmation of the exact grant. Editing the token or changing
worlds invalidates the confirmation. The backend rechecks the selected
world and grant hash before persisting revocation. Bearer tokens are
not copied into activity logs.

## Pending is not admitted

`world join` stages a request. `world status` distinguishes waiting for
signed approval, expired pending invitations, and committed admission
with local synchronization still pending. The accepted signed membership
and complete verified checkpoint determine whether ordinary world
synchronization is available. A staged request cannot fetch private saves. The daemon records a revoked
or expired denial only when the response belongs to the exact outstanding
signed join request and the connection is authenticated as its issuer.
The persisted response is diagnostic, not proof of admission or a new
membership record. A late response for a different request is ignored.
Committed membership takes precedence over the diagnostic. Desktop displays
these pending states and offers a membership refresh; it does not enable
Play merely because a join request was sent.

A fresh invitation from the same issuer may replace an expired or
authenticated-denied pending request for the same signed member role.
Run `world join` with the new token and the same role flag (if any).
Replacement is refused if a membership promise, certificate, accepted
membership/epoch, completed join, replay anchor, or canonical snapshot
exists. The old request is compared under the world transaction lock;
the new signed request has its own nonce, so an old denial cannot apply
to it. No durable admission decision or snapshot is discarded.

## Removal and retained data

Token revocation does **not** remove a committed member. Membership
changes require the established signed membership protocol. Current
replicas authorize new private checkpoint requests against the
accepted signed membership and matching descriptor. They recheck
authenticated membership before new manifest delivery, after manifest
acknowledgement, and before every new blob chunk. A signed membership
commit whose descriptor update has not yet completed denies private
replication rather than trusting the stale descriptor. Removed peers
receive a generic access refusal and may still receive the removal
certificate itself. Existing copies on an offline or removed device
cannot be remotely erased, and data already disclosed cannot be made
secret again. A stale partition cannot immediately know about a newer
membership decision. Do not describe this system as remote data deletion
or retroactive confidentiality.

Reusable grants can request a fresh admission until expiry or revocation,
subject to current membership and approval. Revoking an old grant is not
a permanent ban of its recipient. The issuer's durable local marker is
not a distributed revocation ledger or protection against full data-root
rollback/key cloning. A separate recipient-bound or one-use policy would
need a versioned signed token, durable consumption coordinated with
membership preparation, and explicit legacy/migration behavior. Do not
introduce a client-only one-use checkbox.

### Recipient-bound and one-use policy review

Retain V1 as reusable bearer grants. A future recipient-bound version
would sign the recipient's exact peer identity and public key alongside
the world, issuer, issuer epoch/fence, grant ID, and expiry. Every voter
would need to validate that binding; a UI label is insufficient.
One-use grants additionally require an immutable consumed-grant record
committed with the initial membership preparation, restart-safe replay
of the same admission, and an explicit policy for aborted preparations.
A versioned migration must not reinterpret existing reusable V1 grants
as single-use. No such new protocol is activated by this change.

The CLI revocation, inspection, and denial feedback paths have local
process and durability tests. The authenticated process regression also
exercises reuse of a valid grant, revocation without removing existing
members, an actual joint-consensus leave, removal followed by a new
checkpoint, explicit private status/history denial to the removed
identity, and successful replication to the surviving member. The
removed device retains its verified old bytes. Desktop management is exercised by the browser regression at three
viewport sizes, with native IPC type/build validation. Installed native
GUI and independent-network field acceptance remain.
