# Non-authoritative directory moderation (#47)

## Implementation

New public/unlisted worlds may explicitly declare a coarse region and a
two- or three-letter language code. The language is encoded as a reserved
presentation tag in the existing signed configuration. No wire or canonical
serialization change is introduced; private creation rejects listing metadata.
Both Desktop creation paths use this same validation, with optional metadata
cleared when switching to private. Import retains its existing visibility
setting; no retroactive metadata or general signed configuration edit is added.

Discover uses signed metadata to filter public listings. Device-local hiding
is reversible, persistent, and separate from directory suppression. An
operator may suppress a listing only from that publisher's public browse;
exact resolution and freshness voting remain unaffected. Local publication
pause/resume is also available for existing signed public/unlisted worlds,
and does not alter their signed visibility.

Reports reverify an actual public listing and bind the selected signed
configuration hash. Private, unlisted, mismatched and unverifiable targets
are refused. A bounded private policy store saves only a deduplicated report
ID, world ID, config hash, category and verification/recording timestamps.
No report is sent or acted upon automatically. Operators can independently
review a manually supplied bounded report against current public discovery;
the allegation itself remains unverified. The separate operator suppression
decision is reversible. Malformed policy fails browse closed, without
affecting exact lookup. The module never writes canonical world state.

## Local verification

| Scope | Result |
| --- | --- |
| Protocol library | 28 tests passed, including listing metadata |
| CLI moderation library | 5 tests passed (durable policy, bounds, malformed records, strict report envelope, and symlink rejection) |
| Discovery library | 12 tests passed |
| Authenticated discovery integration | 7 real-network tests passed |
| Native command validation | 1 test passed; native cargo check and scoped strict Clippy passed |
| Desktop | 169 Node, 82 Svelte, 30 source-parity tests passed; Svelte check/build |
| Both Chromium frontends | Final 720x560, 980x760, 1280x900 scenario passed; metadata filters, reversible hiding, report cancellation/save/delete, minimal export, duplicate submission guard, exact IPC and keyboard activation |
| Real loopback | Isolated owner/client/reviewing operator, mDNS disabled; all 13 assertions passed, 28 canonical protocol files byte-for-byte unchanged, no membership created |
| Native Linux | Both extracted Debian packages passed 11/11 checks; real WebKitGTK and identical bundled sidecars, signed metadata filtering, publication pause/resume, 980x760 and 720x560 layout, local report/save/export/delete, persisted hide/restore, canonical files and remote policy unchanged |
| Workflow policy | Local policy check; no hosted CI dispatch |

[Loopback CLI evidence](evidence/loopback-cli-20261125.json) records the
actual signed-network and isolation checks. [Native Linux evidence](evidence/linux-native-20261125.json)
records package hashes and per-variant results. The package source base is
the previous checkpoint plus this milestone's working-tree changes. Synthetic
IDs and screenshots are retained locally rather than committed. Native GUI
creation against an external provider catalog was not exercised: CLI creation
was real, and the browser scenario checked GUI creation IPC and private clearing.

## Reproduce

Build the local CLI/runtime and both Debian package variants with the same
four sidecars. Then run:

```sh
python3 scripts/e2e/directory_moderation_smoke.py --bin-dir target/debug
python3 scripts/acceptance/native_directory_moderation.py \\
  --packages /path/to/debs --artifacts /path/to/private-output \\
  --xvfb /path/to/Xvfb --driver /path/to/tauri-driver \\
  --native-driver /path/to/WebKitWebDriver
```

## Qualification boundary

This is a local, non-authoritative directory workflow, not a public registry
or global abuse adjudication service. There is no automated report transport,
central intake/appeals, global suppression, account-level game ban or new
world-admin permission. General signed visibility changes remain gated on
authenticated bounded configuration-history catch-up; a local pause is not
a signed edit. Region/language metadata is self-declared, not an identity or
location attestation. Operators must make moderation decisions separately
and manually share any local allegation. Public rollout, independent
LAN/WAN/NAT verification, native Windows/macOS and actual game-client
qualification remain separate gates.
