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
Committed membership takes precedence over the diagnostic.

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

The CLI revocation, inspection, and denial feedback paths have local
process and durability tests. The authenticated process regression also
exercises reuse of a valid grant, revocation without removing existing
members, an actual joint-consensus leave, removal followed by a new
checkpoint, explicit private status/history denial to the removed
identity, and successful replication to the surviving member. The
removed device retains its verified old bytes. Dedicated Desktop
management and independently networked field validation remain.
