# V1 world visibility, governance and moderation boundary

This is the supported private-play governance baseline, not a public-lobby
launch approval. A directory is a discovery aid; it cannot elect a host,
approve a member, change a save, or veto a world's signed history.

## Visibility is not admission

The initial visibility is part of the creator-signed world configuration.
World creation defaults to private; the operator may explicitly select
`--visibility unlisted` or `--visibility public`.

| Signed visibility | Public browse | Exact-ID discovery | Membership / game access |
| --- | --- | --- | --- |
| Private | No | No, including direct publisher requests | Signed invitation and committed admission |
| Unlisted | No | Yes, with live listing verification | Same |
| Public | Yes, with live listing verification | Yes, with live listing verification | Same |

V1 supports invite-mediated membership only. The reserved open and
approval-required policies are not offered as functional discovery or
admission flows. Joining, voting, hosting, runtime consent, and native
Minecraft account identity remain separate boundaries.

All member-only replication, descriptor, status, checkpoint, and routing
requests retain their application-authenticated signed membership checks,
regardless of visibility. An outsider who knows a world ID receives the
same access-denied response for any visibility; discovery does not issue
invitations or expose snapshot bytes.

## What a discoverable listing discloses

A public or unlisted listing deliberately exposes signed presentation,
bounded compatibility information, world/authority identifiers, and the
canonical generation and checkpoint hashes. V1 live discovery additionally
exchanges the genesis-anchored membership proof (cryptographic member IDs,
public keys, role flags, joint certificates and any bound pending proposal)
with a verifier. It does **not** send bearer invitations, private keys,
runtime credentials, full checkpoint manifests or blob contents through
the discovery endpoint. Unlisted is not secret: anyone with its ID and a
reachable locator can request the listing and proof. Use private visibility
if any of that metadata must stay within the admitted group.

The upgraded publisher rechecks signed configuration, membership, epoch,
fence, compatible descriptor, pending authority transitions and canonical
head at the response boundary. An unchanged announcement is reused; the
publisher reconciles changed local state on a short interval. A listing
must have a fresh nonce-bound vote from its actual announcing authority
as well as the existing current/joint voter majority. A majority of stale
replicas without that authority is insufficient for discovery.

Cached provider hints and previously delivered bytes cannot be revoked
from other machines. A locator may temporarily retain a stale provider
key, or a disconnected replica may still have older state. The updated
client verifies live evidence instead of treating those hints as current
listings. Mixed-version enforcement and hostile old publishers remain
separate rollout concerns.

## Explicit local unlisting

An operator can stop this device's publication and discovery votes without
deleting data or changing canonical configuration:

```bash
swarmcraft world discovery-listing <world-id>
swarmcraft world discovery-listing <world-id> --paused true
swarmcraft world discovery-listing <world-id> --paused false
```

A durable local marker survives publisher restart. Pause is denial-only;
malformed markers fail closed. Resuming never advertises a private world,
authorizes an outsider, or bypasses the usual signed-state checks.
Pausing the current announcing authority prevents new verified listings
from that authority on updated clients. Pausing a replica may reduce the
available discovery quorum. The marker is not a signed world policy or a
global deletion of previous listings, and is not transferred as world
membership when a host changes. Coordinate a group-wide withdrawal with
the other operators and stop any unupgraded publisher.

A general signed visibility edit command is deliberately not activated:
the existing V1 live configuration exchange sends the latest generation,
and an offline replica can miss an intermediate configuration. Before
supporting arbitrary edits or widening publication of an existing private
world, implement bounded, authenticated configuration-history catch-up,
explicit privacy disclosure/confirmation, and mixed-version acceptance.
Never skip predecessor verification to make the edit appear to converge.

## Moderation and roles

- The locally accepted signed authority can initiate supported invitation
  and membership operations. A locally accepted epoch is not a live
  hosting lease. Joint voter majorities are required for membership
  changes; a retired non-authority identity is durably banned only after
  the exact certified retirement. See [device recovery](DEVICE_RECOVERY.md)
  and [invite lifecycle](INVITE_LIFECYCLE.md).
- An active V1 member is also a voter and replica member. Host-candidate
  eligibility is a signed role, not permission to use a device's resources.
  Local hosting consent cannot assign membership or moderation rights.
- A creator is the genesis signer, not a separately persisted moderator
  with permanent authority. A directory operator, friend contact, replica,
  or ordinary Minecraft player has no independent world-admin grant.
- Directory operators can choose not to index or promote a listing. This
  does not revoke membership, ban a Minecraft account, delete remote
  copies or alter world history. Separate signed moderator/owner roles,
  reports, moderation reasons/appeals, and authenticated native account
  bans are future governance/session-policy work. No automatic moderator
  role is inferred from a display label or directory entry.

## Reviewable signed history

```bash
swarmcraft world governance <world-id>
```

The JSON audit verifies the locally retained bounded genesis, config,
membership, authority-transition and checkpoint manifest history. It
reports configuration sequence/hash/signer/visibility, each membership
role change and joint certificate voters, and accepted authority/fence
history. A pending membership vote is distinctly labeled staged rather
than committed. No private key or bearer token is exported. The report
does not grant authority, certify that peers have synchronized, prove
live player-account admission, or attest every retained blob's integrity.
Records do not contain reliable operator-action timestamps or moderation
reasons; the audit exposes signed sequence and actor evidence rather than
inventing them. Incomplete, conflicting, invalid or unsupported histories
fail closed; the report is a local view, not a globally current audit.

Public-lobby rollout still requires the reviewed governance/abuse policy,
native account/session enforcement, safe configuration-history sync and
installed cross-platform validation. Do not interpret successful CLI
or discovery transport tests as public-world release certification.
