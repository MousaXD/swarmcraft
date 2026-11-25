# Encrypted checkpoint backup (V1)

A bundle transports data and signed historical evidence, not authority,
membership, a current save, a private peer identity or a hosting grant.

## Usage

On an admitted device, run:

    swarmcraft world recovery-review WORLD_ID
    swarmcraft backup keygen --destination /private/key-storage/world.key
    swarmcraft backup export WORLD_ID \
      --checkpoint MANIFEST_HASH \
      --key-file /private/key-storage/world.key \
      --destination /existing-parent/checkpoint.scbackup

Record the exact world ID and selected manifest hash independently of the
transport provider. If recovery review reports conflict evidence, add
--acknowledge-conflicts CONFLICT_DIGEST to export. This acknowledges the
reviewed set, not permission to resolve a conflict or host the original.

After successful export, copy the completed encrypted file using any file
transport (including removable media or a cloud storage provider). Keep the
key separate and deliver it securely through a different channel. Never
upload the key alongside the encrypted file.

On the receiving device, confirm the independently recorded pins:

    swarmcraft backup import \
      --expected-world WORLD_ID \
      --expected-checkpoint MANIFEST_HASH \
      --key-file /private/key-storage/world.key \
      --bundle /local-download/checkpoint.scbackup \
      --destination /existing-private-parent/recovered \
      --confirm-data-only

The destination must be new, outside the data root and have an existing
parent. Successful import contains world/ Minecraft data, the original signed
manifest/config and backup-receipt.json. The receipt reports the exact
checkpoint, state root, compatibility contract and byte counts, with
registered_world=false and authority_granted=false. Import never opens
recipient canonical storage, even if it already exists. Use the normal
deliberate Minecraft import workflow to create a separate independent world
with the exact compatibility requirements. Do not treat a restored folder
as original authority or proof of live freshness.

## Security model

- Keys are 32 bytes of OS randomness, not passwords. No password recovery
  or key escrow is provided. Unix key files must be owner-only. On Windows
  use a suitably ACL-protected directory; ACL inspection is not implemented.
- The existing locked RustCrypto XChaCha20-Poly1305 library authenticates
  bounded frames. Each bundle has a fresh random 128-bit nonce prefix and
  monotonically increasing 64-bit frame counter. Header, frame position
  and plaintext length are authenticated. The authenticated empty final
  frame and exact EOF are required.
- Only magic/version, random prefix, record lengths and ciphertext size are
  visible to transport. World IDs, filenames, manifests, governance history
  and payloads are encrypted; the format is not padded.
- Key possession permits decryption, not hosting. Even when a party knows
  the key, the importer verifies the pinned world/genesis, original signed
  epoch/config/membership chains, quorum/transfer certificates, every
  retained signed manifest and predecessor, selected manifest hash,
  portable paths, state root and immutable content hashes. The offline
  proof envelope is deliberately unsigned and is not a replay capability.
- No peer private key, transport key, hosting consent, runtime settings,
  EULA, permit or invitation bearer token is exported. Export requires an
  existing admitted identity and verifies the selected local checkpoint.
  Source state is rechecked before publishing. Import uses only isolated
  temporary storage and never commits a snapshot or installs an epoch.

## Interruption and disk safety

Export writes a private temporary file, syncs it, and publishes the full
file without overwriting an existing path. Unsupported filesystems fail
closed. Truncation, wrong keys, swapped/spliced frames, oversized metadata,
tampering and trailing bytes cannot result in a verified restore.

Import reserves disk space for encoded payloads and the restored tree.
Existing per-write storage admission remains in force. Decoded blobs and
restored files are staged in a private temporary directory on the destination
filesystem. Choose a local private, preferably encrypted disk, not a synced
cloud folder for plaintext restore. Normal failures clean scratch data. A
hard crash may leave a private .swarmcraft-backup-import-* plaintext
directory or temporary key/export file. Inspect and safely remove only
owned abandoned leftovers; no secure-erasure promise is made.

The final directory is created exclusively only after authenticating the
entire bundle and verifying all selected files. A successful command,
present receipt and absence of .swarmcraft-backup-import-incomplete indicate
completion. Never use incomplete output; retry into a new destination.
Unix directory sync is enforced; Windows power-loss and ACL verification
remain field work.

## Format

- 24-byte header: ASCII SCBKUP01, then 16 random nonce bytes.
- Frame: big-endian u32 plaintext length, XChaCha20-Poly1305 ciphertext
  plus 16-byte tag. Plaintext maximum 256 KiB. Nonce = prefix + big-endian
  u64 counter initially zero. AAD = ASCII
  swarmcraft/checkpoint-backup/frame/v1 + NUL + header + counter + length.
- Metadata record: authenticated four-byte length frame and canonical
  postcard bytes in 256 KiB frames (last may be shorter). Order: selection
  (world, manifest hash, optional conflict digest), historical proof, then
  original signed manifests in exact checkpoint order.
- Selected payload: deduplicate by blob hash, reject inconsistent duplicate
  descriptors, stream in ascending hash order, exactly the signed encoded
  size per blob; zero-length blobs need no frame. Only selected checkpoint
  payloads are included, not every historical blob.
- Authenticated zero-length final frame plus exact EOF. V1 limits: existing
  512 KiB proof/history bounds, 16 MiB per manifest, 256 MiB aggregate
  metadata, 1 TiB each encoded/restored payload. Unsupported or oversized
  histories fail closed rather than being truncated.

## Verification and scope

The CLI subprocess regression restores an exact older checkpoint including
compressed, empty, duplicate and multi-frame files; rejects wrong keys,
mismatched pins, interrupted/tampered uploads, appended bytes, unsafe
paths, and overwrite; proves canonical storage remains unchanged. Unit
tests encrypt malicious plaintext with the actual key and verify invalid
signatures and bad blob contents still fail. The historical replay test
also covers a storage-only export after signed transfer, recovery and later
admission.

This is the initial CLI file transport. Desktop controls, optional
Drive/object-store adapters, automated transfer, direct hydration of a
pinned replica from a bundle, and installed cross-platform/large-scale
power-loss acceptance are still open. Do not bypass P2P authority or
publication checks using an imported backup.
