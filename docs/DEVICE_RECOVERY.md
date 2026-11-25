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

V1 currently has no supported authority-initiated retirement command for
an unavailable member. An offline device cannot sign a voluntary leave.
Revoking a known invite blocks that grant, not the lost member's already
committed identity. Do not claim the member is removed or that its access
is revoked until a signed membership transition commits. Do not shrink
quorum by editing a local descriptor.

If quorum cannot be recovered, an authorized surviving replica can create
a separate new private world from an independently verified checkpoint via
[recovered copy](RECOVERED_COPY.md). This preserves retained bytes, not the
original world identity or canonical authority. If no surviving verified
copy exists, there is no guaranteed data recovery.

## Remaining implementation

- Provide an explicitly confirmed current-authority retirement operation
  for a non-authority device. Retain a signed banned roster entry so
  obsolete reusable V1 invitations cannot readmit that exact identity.
  Require the exact current membership hash, epoch and fence, reject
  unresolved competing transitions, and collect durable old/new quorum
  votes. Never count an absent device as a vote or bypass quorum.
- Confirm stale presence, previous authenticated connections, obsolete
  invites and replayed membership cannot reinstate a committed retirement.
  Distinguish staged retirement from committed retirement in the CLI/UI.
- Treat contact-list removal as a local preference, never a signed
  membership transition. No contact registry is activated by this design.
- Do not export private recovery material automatically. Same-identity
  restoration requires a reviewed complete durable-state backup/restore
  protocol, exclusive identity possession, anti-rollback checks, and
  explicit export/import confirmation. Until that exists, new-identity
  replacement is the supported approach; key-only export/import is not.
