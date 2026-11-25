# Exact checkpoint and conflict recovery

Recovery is a data operation, not a decision to install an authority. A local
checkpoint may be recovered into a different private world without changing
the original world's ID, membership, accepted epoch, hosting consent, history,
or conflict fences.

## Review and choose

In Desktop, open a world's Diagnostics and choose **Review checkpoints and
conflicts** (also available through **Review conflicts**). The list presents
locally retained certified checkpoint manifests, newest first, and any
preserved signed solo-branch evidence separately. Inspect the full hashes,
signer, epoch, sequence and state root before taking action. The native CLI
verifies the retained certified history and membership; the review does not
promise that all payload bytes are still present. Bytes are verified again
when performing an export or creating a recovered copy.

For CLI operation:

```sh
swarmcraft world recovery-review WORLD_ID
swarmcraft world recovery-export WORLD_ID --checkpoint MANIFEST_HASH --destination /existing-parent/new-export
swarmcraft world recover-copy WORLD_ID --checkpoint MANIFEST_HASH --confirm-new-identity
```

If the review includes a conflict digest, add
`--acknowledge-conflicts CONFLICT_DIGEST` to either action. The acknowledgement
binds the exact currently reviewed signed evidence set. Changed evidence
requires a fresh review; it is not permission to clear a conflict or host the
original. Selecting an older certified checkpoint is deliberate, and does
not roll back the original. Existing authority-affecting operations still
require the current accepted head and no unresolved conflict.

## Divergent branches

A signed solo branch is evidence of a claimed history, not proof of quorum,
canonicality, or availability. Review distinguishes:

- `matching_signed_manifests`: the exact base-to-head signed manifest suffix
  is retained locally and satisfies the branch writer and boundary checks.
  Head payloads must also pass verification before they can be exported.
- `evidence_only`: local files/history cannot be authenticated for this branch
  (for example, the divergent head exists only on another device).
- `unverified_local_history`: matching local history fails branch
  consistency checks. Do not export it as world files.

```sh
swarmcraft world conflict-export WORLD_ID --branch BRANCH_HASH \
  --acknowledge-conflicts CONFLICT_DIGEST --destination /existing-parent/new-branch-export
swarmcraft world conflict-export WORLD_ID --branch BRANCH_HASH \
  --acknowledge-conflicts CONFLICT_DIGEST --evidence-only --destination /existing-parent/new-evidence-export
```

Export each divergent head on a device retaining its exact signed manifests
and verified files. Evidence-only export is an explicit choice and never
produces world files. SwarmCraft does not automatically fetch absent divergent
payloads, choose a winner, or merge competing histories. For an independently
usable copy of a verified solo branch, use the existing deliberate Minecraft
world import on its exported `world/` folder. A certified checkpoint can
instead be copied directly to a new private identity using Recover Copy.
No existing runtime, EULA acceptance, hosting consent, or authority is
inherited.

## Export boundaries

The destination must be a new directory under an existing parent outside the
SwarmCraft data root. Never use an original live world directory. The export
contains a receipt, signed evidence and, when requested and verified, the
selected checkpoint's files in `world/`. Certified exports retain all local
signed manifests and certified-history evidence; branch exports retain the
matching base-to-head manifests and membership proof. They include the selected
head's payload, not every historical payload. The evidence is not an
authority grant or a replay envelope. Evidence exports are not encrypted.

An incomplete export is not a success. If interrupted after the directory was
created, treat any folder containing `.swarmcraft-export-incomplete` as
unusable. Retry into a fresh destination after investigating the failure.
The export path never overwrites an existing directory. The legacy numeric
Recover and latest Export commands remain basic file exports; they also refuse
destinations inside the data root. Do not use them as substitutes for reviewed
conflict choices.

## Verification scope

The local subprocess test uses two isolated stores retaining different signed
heads and the same branch evidence, checks exact restored bytes, confirms
unchanged original histories and conflicts, and checks rejection of stale
acknowledgements, invalid signatures, corrupted payloads, and unsafe
destinations. Local browser tests exercise the Desktop flow at 720x560,
980x760 and 1280x900 with mocked Tauri IPC; actual CLI integration and native
compilation are tested separately. Installed-platform acceptance and obtaining
remote-only divergent payloads remain separate verification/implementation
work. Certified replay does not invent a valid history for unsupported legacy
solo epochs. Portable encrypted backup/import is a separate milestone (#31).
