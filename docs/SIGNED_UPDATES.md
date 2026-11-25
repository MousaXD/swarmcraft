# Signed update delivery (V1)

The packaged `swarmcraft` CLI supports explicit signed update checks and
staging. Desktop does not automatically check, download, install, restart,
or replace its own binaries. All profiles default to **off**. No production
trust key, online index, or automatic installation has been activated by
the implementation. An update cannot grant hosting consent, EULA,
membership, authority or a lease.

## Client operation

Use the CLI shipped alongside the running Desktop package and the intended
absolute local data directory. Obtain the channel's Ed25519 public key
(64 lowercase hex characters) and independently authenticated SHA-256
fingerprint of its 32 raw bytes through a trusted out-of-band procedure.
Never trust a key supplied by an update feed or a package. Stable and preview
must have different keys. The first opt-in requires both key and fingerprint.

```sh
swarmcraft --data-dir "$PROFILE" update status
swarmcraft --data-dir "$PROFILE" update configure --channel stable \
  --public-key "$PUBLIC_KEY_HEX" --expected-fingerprint "$AUTHENTICATED_FINGERPRINT"
swarmcraft --data-dir "$PROFILE" update check --online
swarmcraft --data-dir "$PROFILE" update stage --online
swarmcraft --data-dir "$PROFILE" update verify-staged
swarmcraft --data-dir "$PROFILE" update configure --channel off
```

To explicitly choose the preview channel, use `--channel preview` with
that channel's distinct authenticated key and fingerprint. Once pinned,
select the channel again without resupplying the key. Opting out preserves
both pins and replay history. A changed pin requires a reviewed rotation
procedure, not deletion of the local trust state. Concurrent mutating
operations are serialized by a stable lock; an in-flight operation must
exit before changing the preference. An offline or failed check does not
silently fall back to an older accepted offer.

For offline delivery, use `--feed-dir "$REVIEWED_FEED"` instead of
`--online`. The directory must contain `update-manifest.json`,
`update-manifest.sig` and (for staging) the exact signed package filename.
There is no network request for offline delivery. The online index is
explicitly selected and constrained to the repository's release download
path and bounded HTTPS redirects to GitHub's release-asset hosts. No
arbitrary URL, credentials, or user-supplied shell command is accepted.
The fixed indexes are:

- Stable: release tag `swarmcraft-updates-stable`.
- Preview: release tag `swarmcraft-updates-preview`.

The package bytes are taken from the exact separately signed release tag,
not the mutable index or legacy `main-latest`. Index or transport failure
does not mean an update is available. Expired manifests are rejected.

The client selects its compiled OS/architecture. Linux x86-64 uses Debian
by default; use `--linux-format appimage` for an AppImage installation
(including on re-verification). Native Windows x86-64 NSIS, macOS arm64
and macOS x86-64 DMG are represented separately. Unsupported platforms,
missing formats, equal/older application versions, or different protocol,
storage schema or reader generation are refused. V1 is deliberately
limited to compatible in-place reader/schema upgrades, not a migration
or downgrade mechanism.

A successful `check` authenticates the manifest and signed package
descriptor only. It does **not** verify downloaded package bytes.
A successful `stage` or `verify-staged` also verifies the full byte count
and SHA-256, and emits the exact staged path. Staging uses private files,
bounded reads (64 KiB manifest, 1 GiB package), an atomic same-filesystem
directory publication, and a best-effort 256 MiB free-space reserve.
A corrupt or interrupted stage is never returned as approved. Staged
packages are not executable on Unix. Old stages remain separate; remove
only reviewed obsolete staging artifacts while no update is running.
Do not erase `updates/state.json` to bypass trust or replay protection.

## Before manual installation

1. Review the release version, source commit, signed package target, and
   release notes. A preview is not a production qualification.
2. Stop hosting save-first and wait for the final verified checkpoint.
   Close Desktop, daemon, Java and all sidecars. Preserve a private offline
   copy of the quiescent profile and its latest verified checkpoint. Never
   reconnect two copies of one identity.
3. Run `profile check --target current` on the intended profile, then
   `update verify-staged` immediately before using the native installer.
   Do not install if either fails. Use the exact returned package path;
   do not substitute an older staged file or a separately downloaded copy.
4. Use the native package manager/installer, not a shell command derived
   from the manifest. On Windows stable, independently verify the exact
   expected Authenticode signer, trusted timestamp and OS trust; on macOS
   stable, verify the expected Developer ID team, notarization, stapling,
   Gatekeeper and architecture. Preview Windows and macOS packages do not
   inherit stable production signing claims. On Debian, review native
   package identity/version/architecture before installation; an AppImage
   must be the reviewed correct-format artifact. The detached update
   signature is not a replacement for platform-native signing/trust.
5. Reopen only the compatible updated package, verify identity, world
   checkpoint and latest consent revision. On a failed upgrade preserve
   the current profile and follow [profile recovery](PROFILE_UPGRADE.md).
   Historical binaries cannot retroactively enforce the reader-floor guard.

There is no automatic installer execution, OS package replacement,
uninstall, key rotation, or Desktop settings panel in V1. Offline staging
was locally exercised on Linux; actual native install replacement,
production Windows/macOS OS trust verification, public HTTPS delivery and
key custody/rotation require separate authorized rollout. A signed package
is only as trustworthy as the independently pinned signing key and
authorized release review. Local same-account tampering, loss of replay
state and a dishonest system clock are not defeated by a signature alone.

## Publisher procedure (offline)

Use the existing [release gates](RELEASE_GATES.md) first. The updater
is not a new publication bypass. Sign only the exact reviewed asset
inventory, including `release-provenance.json`, checksums and native
signing receipts for stable Windows/macOS packages. For stable the
provenance channel is `production`; preview must use `preview`.
Debian package metadata is read with `dpkg-deb -f` (no packaged code or
maintainer script execution). The AppImage must carry the expected
x86-64 ELF type-2 header. Native receipts are byte-bound release-job
evidence, not independent OS trust checks on the signing machine.

An authorized release operator keeps the two distinct private keys in
offline custody outside all source trees and delivery directories.
The local signer uses Python 3, OpenSSL Ed25519 and dpkg-deb; the private
key file must be singly linked and owner-private on Unix. Provide the
reviewed expected fingerprint, commit, version and compatibility tuple.
Use a strictly increasing sequence per channel, never reuse a sequence
for different manifest bytes, and use a validity period of at most 30 days.
Keep the previous signed index/sequence in operator records.

```sh
python3 scripts/ci-release/sign_updates.py \
  --artifacts "$VERIFIED_RELEASE_ASSETS" --output "$NEW_INDEX_DIRECTORY" \
  --key "$OFFLINE_CHANNEL_PRIVATE_KEY" --expected-fingerprint "$AUTHENTICATED_FINGERPRINT" \
  --channel stable --expected-commit "$REVIEWED_COMMIT" \
  --expected-version "$REVIEWED_VERSION" --minimum-client-version "$MINIMUM_READER_APP_VERSION" \
  --sequence "$NEXT_CHANNEL_SEQUENCE" --valid-days 7 \
  --protocol-version 1 --storage-schema-version 1 --reader-generation 1
```

The signer writes only the two signed index files. It does not publish
a tag, upload anything, create a workflow, or sign an executable via
Authenticode or Developer ID. The manifest signs the exact JSON bytes
with domain `SwarmCraft update manifest v1\0`. Each detached package
signature independently binds channel, version, commit, immutable
release tag, target, filename, byte count and SHA-256 using the distinct
package-signature domain. The client validates both and streams the
actual payload. Signing a metadata digest alone is not native OS signing.

Only after the immutable production `vX.Y.Z` or preview
`main-snapshot-<SHA>` release has passed the normal release gates and its
actual bytes have been verified may the authorized operator publish the
matching index pair on the selected channel tag. No updater-specific
workflow is required. A mismatched index/signature pair fails closed
during replacement; never publish test keys or fixture packages. Actual
key provisioning, index publication, native install acceptance and
rotation/revocation are explicitly pending authorization.

See [local acceptance evidence](../research/signed-updates/REPORT.md).
