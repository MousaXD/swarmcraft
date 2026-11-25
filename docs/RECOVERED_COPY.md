# Recover a retained world as a new private world

When quorum or the original authority is unavailable, a surviving
authorized replica can create a new independent private world without
manufacturing an authority transition. A local checkpoint may be older
than the last save on another device.

## Desktop

1. Select the surviving local world and open Diagnostics → Export and recovery.
2. Choose Create recovered copy.
3. Review the original world ID, snapshot number and exact checkpoint manifest.
4. Confirm Create new private world. Successful completion identifies both
   the old world and the new world ID. Select the recovered world in Worlds.
   Its name starts with `Recovered:`. Runtime installation, EULA acceptance,
   and hosting contribution are separate decisions.

The backend validates the same checkpoint again after confirmation. A
changed selection, unavailable checkpoint, conflicting history or a failed
copy does not authorize a new world. If the outcome cannot be confirmed,
refresh the world list before retrying.

## CLI

```sh
swarmcraft world status "$ORIGINAL_WORLD_ID"
swarmcraft world recover-copy "$ORIGINAL_WORLD_ID" \
  --checkpoint "$CHECKPOINT_MANIFEST_HASH" \
  --confirm-new-identity
```

Use the exact `Checkpoint manifest` value from status, not the state root
or snapshot number. Optional `--name` supplies a suffix for the recovered
world name; `--json` returns the original world, selected checkpoint, new
world, new manifest and state root.

## Safety boundary

- Original world IDs, signed history, control records, checkpoint blobs,
  membership, and configuration are not rewritten.
- The local device must be an authorized member in the retained membership.
  The complete signed historical chain and selected checkpoint data must
  pass verification. The original authority need not be online to sign a
  new replay envelope. This offline verification never grants network replay
  publication or authority over the source world.
- A private new genesis is signed by the local device, with the same
  compatibility manifest and file state root. The new world has a different
  identity, independent membership, disabled solo advancement, and a signed
  description identifying the source world and checkpoint.
- Hosting consent, invitations, source membership, leases, recovery
  certificates, installed runtime paths, and EULA consent are not inherited.
- New-world files are staged and verified before publication. Interrupted
  staging is not a visible playable world.
- Unsupported legacy, pruned, partial or conflicting signed histories are
  denied rather than guessed. Existing explicit export/recover-to-folder
  paths remain available where their own verification succeeds. This is a
  recovery of retained bytes, not evidence of the freshest remote save.

Do not import the copy as the original identity or use it to bypass
quorum. Rejoin the original world through its valid authority/membership
flow if it later becomes available.
