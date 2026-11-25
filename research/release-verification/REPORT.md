# Sabathon #50 - published preview and production signing boundary

Verified 2026-11-25, read-only against existing GitHub releases. No new release,
tag, hosted workflow, or main update was performed.

## Actual downloaded assets

- Pinned preview 0.5.1, commit `aaffac22e32547983f1fd558af6eb32c0da4917b`:
  [11 downloaded assets](evidence/pinned-20261125.json), 104,277,947 bytes.
  SHA-256 and size agree with GitHub's uploaded asset metadata; all five
  package checksums and the source/version/platform manifest agree.
- Previous 0.5.0 `main-latest`, tag target
  `682c78f522aff53a59720de6c864e4e536eea778`:
  [10 downloaded assets](evidence/legacy-20261125.json), 102,481,494 bytes.
  All five package checksums agree. Legacy publication has no commit-bound
  provenance manifest; the tag target and checked bytes are the limit of proof.
- Both Fabric JARs report the expected actual mod version and server
  environment, and both Debian packages report the expected version and
  amd64 architecture. Published inventory is Debian amd64, Windows x64
  setup, macOS arm64 and x86_64 images, and Fabric JAR. No preview AppImage,
  mobile or other platform distribution is claimed. The production graph
  additionally requires Linux AppImage.
- Release inventories and tag targets were checked before and after download.
  This proves availability and integrity at audit time, not that an
  administrator can never mutate a tag or release in the future.

## Actual package rollback rehearsal

[Native Linux evidence](evidence/native-linux-20261125.json): extracted the
two actual downloaded Debian packages with all four bundled sidecars and ran
each via real WebKitGTK in a private Xvfb session. Created one private world
per isolated version, verified each historical CLI's signed snapshot,
and checked identity, world list, state root and retained postcard file
hashes across legacy -> pinned -> legacy -> pinned. All four stages passed
and owned package processes were closed between stages. The first harness
attempt used the newer CLI's checkpoint output label; the published CLIs
instead emit the verified snapshot label. The corrected harness checked
that actual historical output and retained canonical hashes. No live game,
shared-profile schema migration, system installer replacement or live-data
downgrade was attempted. Those are separate #52 acceptance gates.

## Production gate

The existing manually approved production job checks signing credentials and
platform-native signatures. The strengthened publication boundary now rejects
generic signing text declarations and requires exact commit/platform/package
size/SHA-256-bound JSON receipts. The Windows job independently checks the
staged installer's Authenticode trust, all signatures, timestamp, and expected
certificate thumbprint. Each macOS job validates the final DMG stapling,
mounts the image read-only, verifies the included app's Developer ID/team,
strict deep code signature, app ticket, Gatekeeper assessment and architecture,
then checks the image bytes have not changed. The publisher checks receipts
and records their digests in release provenance. This is not independent
publisher-side OS trust verification or a cryptographically signed receipt.

Offline negative tests exercise missing, malformed, wrong-byte, wrong-commit,
wrong-platform and incomplete signing evidence, and policy tests require
the verification steps before upload. Bash syntax was checked. No Windows or
macOS native signing run was possible on this Linux workstation, so actual
production signing/notarization remains blocked pending authorized native
execution, release credentials and exact-release approval. Do not treat
these offline tests or unsigned preview downloads as a production certificate.
