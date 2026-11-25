# Profile upgrade and rollback

Application version, wire protocol, storage schema and local reader generation
are separate compatibility dimensions. The current candidate activates local
reader generation 1 without changing wire protocol 1 or storage schema 1.
Published 0.5.0 and the historical published 0.5.1 preview predate this guard,
even though the latter reports the same application version as the candidate.

## Before upgrading

For explicitly signed update delivery, use the [signed update client and
manual installation contract](SIGNED_UPDATES.md); a successful stage is
not permission to stop an unconfirmed host or replace a running package.

1. Verify the complete package against its release checksums and provenance.
2. Stop hosting through the normal save-first procedure. Wait for the final
   verified checkpoint and close Desktop, daemon, Java and all sidecars.
3. Preserve a complete private, offline copy of the quiescent profile and
   its latest verified checkpoint. Protect the private identity key and
   consent records. Never run two copies of the same identity.
4. With the candidate CLI, run a read-only compatibility check on the intended
   profile (use the actual absolute data-directory path):

   ```sh
   swarmcraft --data-dir "$PROFILE" profile check --target current
   ```

   The check does not create a profile, initialize keys or perform recovery.
   An unsupported schema, protocol, reader floor or malformed marker is an
   error, not an instruction to delete files. Keep the original bytes and
   use a compatible reader.

After a successful preflight, open the profile with the current package.
The first storage open publishes `profile-compatibility.json` atomically
under a stable compatibility lock. No world, signed record, peer identity,
or hosting-consent record is rewritten by activation. Interruption before
publication can be retried; an abandoned staging file is never promoted
as a reader floor. Do not manually remove or edit the floor or its lock.

Verify the saved world and latest consent revision after startup and restart:

```sh
swarmcraft --data-dir "$PROFILE" world list
swarmcraft --data-dir "$PROFILE" world verify "$WORLD"
swarmcraft --data-dir "$PROFILE" world hosting status "$WORLD" --json
```

A withdrawn or absent hosting contribution stays withdrawn or absent.
A successful storage upgrade is not a runtime compatibility certificate,
EULA acceptance, new authority grant or an automatic Minecraft/Fabric upgrade.

## Historical package rollback

Keep the current compatible CLI available for the preflight, even if you
intend to change the installed application. For the historical releases:

```sh
swarmcraft --data-dir "$PROFILE" profile check --target published-0.5.0
swarmcraft --data-dir "$PROFILE" profile check --target published-0.5.1
```

Both targets refuse any initialized profile, including a legacy profile
without the new marker. Only an absent or empty root passes the supported
historical-target preflight. The old executables themselves predate the
guard and cannot be trusted to enforce it. There is no retroactive
interlock in those binaries or automatic OS installer downgrade barrier.

Do not reopen upgraded live data with a historical package. A preserved
pre-upgrade copy is an offline recovery source, not a license to reconnect
an old identity, replay old consent, rewind an accepted epoch or fencing
token, or create simultaneous authorities. Prefer a compatible current
package and verified data-only recovery into a separate fresh identity.
The historical package rollback rehearsal in #50 used separate disposable
profiles, not a live-profile downgrade.

## Verified data-only recovery

Use the compatible reader to verify the signed snapshot and choose the
exact snapshot number:

```sh
swarmcraft --data-dir "$PROFILE" world verify "$WORLD"
swarmcraft --data-dir "$PROFILE" world recover "$WORLD" 1 "$EXPORT"
```

Replace 1 with the verified snapshot number to recover. The output
directory must be outside the profile and not already exist. Preserve the
source; verify the recovered bytes and review the world/runtime/mod tuple
before importing into a new profile and world. An epoch-0 legacy import
may be recoverable as signed data even if its history is not eligible for
a certified encrypted backup; do not manufacture a genesis certificate.

For a supported certified checkpoint, use the encrypted backup procedure:

```sh
swarmcraft --data-dir "$PROFILE" backup keygen --destination "$KEY"
swarmcraft --data-dir "$PROFILE" backup export "$WORLD" --checkpoint "$HASH" \
  --key-file "$KEY" --destination "$BUNDLE"
swarmcraft --data-dir "$NEW_PROFILE" backup import \
  --expected-world "$WORLD" --expected-checkpoint "$HASH" \
  --key-file "$KEY" --bundle "$BUNDLE" --destination "$RESTORE" \
  --confirm-data-only
```

Keep the key private and separate from the bundle. The verified save is
under `$RESTORE/world`, with receipts beside it. Import that save using
the current existing-world import flow, with the exact compatible
Minecraft/Fabric/mod selection and a separate new profile. This creates
a new identity/world and never restores the original authority, consent
or EULA state. A rejected certified export must not be bypassed.

See [local upgrade evidence](../research/profile-upgrade/REPORT.md).
