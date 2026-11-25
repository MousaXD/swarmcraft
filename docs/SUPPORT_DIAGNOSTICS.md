# Local action history and redacted support reports

SwarmCraft Desktop offers optional, on-device support history in **Tools >
Activity**, also reachable through **Diagnostics > Support history and
redacted report**. This is a convenience record of selected native Desktop
actions, not a signed world audit, security log, or telemetry service.

## Recording and review

- Recording starts off. Select **Enable history** to explicitly opt in.
  Actions performed before opt-in are not reconstructed from any logs.
- Native Rust command boundaries record completion/interruption of selected
  world creation/import, membership requests, invitation actions, replica
  verification/preferences, hosting preferences, runtime preparation,
  play/stop/wake/transfer, checkpoint recovery/export, mod and publication
  actions, and explicit networking start/stop. Routine polling and raw
  command output are not recorded.
- **Returned OK** means only the selected native command returned
  successfully. A staged join, leave, transfer or wake request may still
  need signed membership, quorum, checkpoint or runtime completion.
  **Returned error** omits the error message. **Interrupted** means the
  tracked command future was dropped; an abrupt process loss may leave no
  completion entry.
- History is bounded to the newest 256 entries across all worlds. A
  locally parsed World ID associates a known world action with the selected
  world; it is used only for filtering and never returned in the support
  projection. Some actions, such as unbound join requests, are available
  only in the all-world scope.
- Recording preference and summaries persist in the device's SwarmCraft
  data directory, in the private, versioned `support-history-v1.json` store.
  The store is locked and atomically replaced. Malformed or oversized stores
  fail closed; the application does not export or silently overwrite them.
  History write failures never change the native operation's result.
  The current process can indicate incomplete recording; the history is
  best effort and does not prove an action did or did not occur.
- **Pause recording** prevents new entries but retains existing ones.
  **Clear history… > Delete all history** deletes all saved summaries,
  not worlds or session logs. In-flight actions from an earlier recording
  revision cannot recreate cleared or paused history. Clear does not
  disable an already enabled recorder.

## Explicit export

Choose **Preview redacted report** and review the exact JSON before
sharing. The report is created locally only after this explicit action.
It contains:

| Field | Meaning |
| --- | --- |
| `generated_unix_ms` | Local system time at preview |
| `desktop_build` | Desktop version, build-time Git commit when available, and clean/modified/unknown source-tree status |
| `last_reported_node_path` | Direct, relay, both, limited, unknown or unavailable; a coarse last local node observation, never a fresh game route or host authorization |
| `scope` | All-world actions or selected-world summaries, without any world identifier |
| `history` | Recording state, scoped count, retention/incomplete-recording indicators and allow-listed timestamp/action/outcome/build entries |

The build commit describes the Desktop build's source base. A modified
tree is not an exact commit build; missing Git provenance is marked unknown.
This is not cryptographic package attestation or independent verification
of the bundled sidecar build. The path class is based only on the last
structured local node snapshot, which can be stale; an absent/stopped
Desktop-owned daemon or invalid snapshot yields unavailable.

Raw logs, stdout/stderr, errors, invite and transfer tokens, keys,
credentials, environment variables, paths, IP addresses, peer/world IDs,
names, checkpoint hashes, save data and Minecraft player content are
**not** collected by this projection. The preview token used to bind the
native save is also excluded. This is an allow-list of known typed fields,
not an attempt to regex-redact arbitrary log text.

Select **Save JSON…** to choose a local destination folder through the
native folder chooser. The file has a generated basename, never overwrites
an existing file, and cannot be written into SwarmCraft's private data
directory. Cancel writes nothing. On Unix, new files are created with
owner-only permissions. **Copy JSON** explicitly copies the reviewed
projection; if clipboard access fails, the read-only JSON remains available
for manual copy. Nothing is uploaded, emailed, or otherwise transmitted.

The native preview is short-lived (five minutes) and a save uses the exact
reviewed bytes rather than re-reading logs or accepting arbitrary file
contents from the renderer. Native saves reject a changed recording
revision. Navigation, scope changes, and local clear/pause invalidate the
displayed preview. A cancelled save needs a new preview. Copies already
saved/shared cannot be revoked by clearing local history.

## Raw session output is different

The original Activity console remains under **Raw session output (not
included in support report)**. Explicit detailed world status, compatibility
and member-read actions still open their raw results. This console can
contain sensitive information. Its Clear button clears session presentation
only, not support history. Review raw output separately before sharing it.
Do not send an entire data directory, log file, or world export to support
as a substitute for the redacted report.

No remote support collector, automatic crash-report upload or signed
authority audit is introduced. Native Windows/macOS save dialog and
filesystem permission qualification remain separate from Linux testing.
