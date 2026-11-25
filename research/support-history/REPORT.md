# Opt-in redacted diagnostics and action history (#48)

## Implementation

Both Desktop frontends share a native, device-local, opt-in history of selected
world and networking command outcomes. Recording defaults off, never
backfills raw logs, retains at most 256 completion/interruption records,
and persists across restart. Pausing preserves records; explicit clear
removes them and invalidates in-flight recording revisions. A typed local
world identifier enables selected-world filtering without entering the
export. Native success is a returned command outcome, not a proof of a
committed membership or authority transition.

Explicit preview projects only timestamps, typed action/outcome, Desktop
version/build-source commit and tree status, last reported coarse node path
class, scope and bounded history. No credentials, addresses, IDs, names,
paths, logs, world files or arbitrary error text are included. Native save
binds the exact reviewed bytes to a short-lived token and requests a local
folder; cancel writes nothing. Existing files and the private data root are
refused. Unix files are owner-only. The original raw activity console
remains separate. See [user guidance](../../docs/SUPPORT_DIAGNOSTICS.md).

## Local verification

| Scope | Result |
| --- | --- |
| Desktop Rust | 71 tests passed, including consent/revision, bounds, strict projection, concurrency, path class, private exact-byte save and one-time preview |
| Native Rust tooling | Check, strict Clippy across targets, formatting passed |
| Desktop JavaScript | 173 Node tests passed |
| Svelte | 82 tests, 30 source-parity tests, type/check and build passed |
| Both Chromium frontends | 720x560, 980x760 and 1280x900 browser scenarios passed; review, scope, invalid projection refusal, stale navigation, explicit clear/cancel and native invocation receipts |
| Both extracted Linux packages | 14/14 native checks each; real replica verification and rejected join recorded, default-off/no backfill, scope, secret exclusion, build provenance, native folder cancellation and exact reviewed JSON save with private permissions, pause, restart, clear, unchanged canonical world protocol files |
| Workflow policy | Local policy check passed; no hosted CI dispatch |

[Native evidence](evidence/linux-native-20261125.json) records actual
package and sidecar hashes and boolean checks. Both packages share the
same four native sidecars. Their source base is the preceding commit plus
this milestone's working-tree changes; the report correctly labels the
build modified. Initial native automation did not reliably submit a GTK
location-entry folder path; the final run selects an actual folder in
the isolated home through native controls and succeeds. Synthetic values
and screenshots remain local, not committed.

## Reproduce

Build both local Debian package variants with the same four sidecars.
Use an isolated private output directory, Xvfb, tauri-driver and
WebKitWebDriver; the harness also needs xauth, xdotool and ImageMagick.

```sh
python3 scripts/acceptance/native_support_history.py \
  --packages /path/to/debs --artifacts /path/to/private-output \
  --xvfb /path/to/Xvfb --driver /path/to/tauri-driver \
  --native-driver /path/to/WebKitWebDriver
```

## Qualification boundary

This is a best-effort history of selected Desktop command boundaries, not
a comprehensive daemon/CLI trace or signed audit. The last reported node
path is not live reachability/authorization evidence. Missing Git metadata
is marked unknown; modified builds are not exact-commit attestations.
There is no support collector or automatic upload. Actual Windows/macOS
native dialog/filesystem qualification and native clipboard behavior remain
outside this local checkpoint; the reviewed text supports manual copy.
