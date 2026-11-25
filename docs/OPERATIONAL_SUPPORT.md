# Operational support and per-version compatibility policy

This is the local field-support contract for Sabathon #55. Observations
help triage the shipped operation boundaries. They never grant membership,
quorum, authority, a hosting permit, EULA acceptance or a playable route.
There is no fleet telemetry collector, remote upload, automatic crash
reporting, production service-level guarantee or new hosted CI job.

## Collect and share only reviewed evidence

Use the compatible CLI from the actual package and the absolute private
data root used by Desktop or the daemon. Collection defaults to off.

```sh
swarmcraft --data-dir "$PROFILE" support status
swarmcraft --data-dir "$PROFILE" support configure --collection on
swarmcraft --data-dir "$PROFILE" support report --confirm
swarmcraft --data-dir "$PROFILE" support report --confirm --world "$WORLD" --live-route
swarmcraft --data-dir "$PROFILE" support configure --collection off
swarmcraft --data-dir "$PROFILE" support clear --confirm
```

The report command prints a bounded redacted JSON object only after
explicit confirmation. It does not initialize the profile, open canonical
storage, select a gateway or start a daemon. The optional live route
request asks the already-running daemon for its normal independent
status check; it is bounded to 24 seconds, can refresh the live
observations, and never opens a Minecraft stream. Without that option,
the report is local-only. An absent daemon is unavailable, not proof of
a lost host. A corrupt/oversized support store refuses export instead
of emitting raw content.

After changing collection preference or clearing history, use the normal
safe shutdown/start procedure for long-running sidecars if recording is
desired again. Never force-kill an active host just to restart recording.
An older collection revision cannot recreate cleared or paused history.
Pause preserves saved observations; clear deletes those observations only.
Neither action alters Desktop action history, raw logs, saves or signed
records. Existing report copies are not revoked by clearing the store.

If sharing with a field engineer, review stdout first. To write a file,
choose a new private filename outside the profile, for example in a
local shell with `umask 077` and `set -o noclobber`:

```sh
swarmcraft --data-dir "$PROFILE" support report --confirm --world "$WORLD" > "$NEW_PRIVATE_REPORT"
```

Do not redirect stderr into the report. Review the saved bytes and send
them only through an authorized support channel. The report contains
timestamps, build metadata, outcome categories, bounded elapsed times and
counts. No raw error messages, addresses, peer/world identifiers, names,
signed hashes of world checkpoints, tokens, credentials, paths, runtime
arguments, keys, player content or save bytes are exported. The report is
an allowlisted typed projection, not a log-sanitization utility. Timestamps
and activity counts may still be sensitive. Do not send the entire profile
or raw logs as a substitute. Desktop's separate [action history](SUPPORT_DIAGNOSTICS.md)
remains available for user action context; it is not merged into this report.

## Metric contract (schema swarmcraft-operational-support-v1)

| Metric | Measured boundary | Useful interpretation |
| --- | --- | --- |
| storage_open | Compatible storage open in init, daemon and direct host | Completed/failed and elapsed ms; incompatible reader preflight may suppress recording altogether. |
| daemon_listener | First actual listener event after daemon initialization starts | A listener is not an authenticated peer connection. |
| daemon_exit | Returned daemon operation result | Abrupt process loss can leave no exit event. |
| replica_pull | Receiver-driven authenticated pull attempt, including exact fenced finalization | Completed/paused/fenced/stalled/storage_pressure/deferred, elapsed ms, bounded payload bytes, resumed blobs, failed requests, source count, max parallel. |
| replica_finalize | Ordinary push or already-present payload: signature check plus durable replica finalization | Not transfer time, not proof a remote peer received the acknowledgment; no invented byte count. |
| lease_permit | Actual local authority permit refresh, quorum loss or canonical fence | Sampled outcome; not a hosting-consent or Minecraft-readiness claim. |
| fresh_quorum | Actual live signed quorum collection | Result and elapsed ms; route admission still requires every other proof. |
| game_route | Daemon route-status checks and missing-gateway response | Verified-at-check or specific refusal category; not a player login. |
| runtime_prepare | Explicit runtime install, repair or verify | Returned result; verify local runtime separately before play. |
| runtime_restore | Actual runtime checkpoint restoration | Result and elapsed ms; not successful Java startup. |
| checkpoint_verify | Explicit local checkpoint verification | Completion is scoped to that operation and retained data. |
| checkpoint_restore / conflict_export | Explicit restore, export or recovered copy; separate conflict evidence export | Outcome, not authority or conflict resolution. |
| checkpoint_publication | CLI checkpoint operation or runtime's final fenced commit | Completed/failed; preserve last signed head and unconfirmed runtime if failed. |
| historical_publication | Completed/deferred canonical historical publication | Staging partial history alone does not constitute publication. |
| discovery_preference / discovery_refresh | Local pause change or local discovery refresh return | No remote directory acceptance or membership proof. |
| backup_export / backup_import | Explicit encrypted backup operation result | Not permission to replace an active identity. |
| update_check / update_stage | Explicit signed-update operation result | Not native installation or release activation. |

The private recorder retains at most 128 observations across all worlds,
a 128 KiB state limit and a 256 KiB report limit. Bounded control-cache
reads are at most 256 KiB each. The nonblocking daemon queue retains at
most 64 pending records; batch writes happen off the event loop. Repeated
lease/route/directory outcomes are sampled at most once per category per
15 seconds per process, not counted as all attempts. Lock contention,
disk pressure, abrupt exit and full queues may lose observations. A
failure to save support data cannot alter the operation's result. The
store has a stable lock and atomic replacement; no arbitrary append-only
raw log is consumed. Zero observations is not a health verdict.

Summaries report counts of retained observations by metric and outcome,
and nearest-rank p50 and p95 of retained timed observations (milliseconds).
Untimed observations do not enter latency statistics. Samples may contain
multiple builds or failure outcomes; never use these as unbiased fleet
latencies, availability percentages, per-world rates or capacity targets.
Transfer counters are limited to 1 TiB payload and 1,000,000 for each count;
durations cap at 24 hours. They are measured per pull, not total network
traffic. Legacy finalization does not report transport duration. Select a
world only for the current cache/route/preference projection; the retained
history is deliberately profile-wide and not attributed to that world.

## Field triage

Use the last accepted checkpoint and existing authoritative backend checks
to make decisions. Unknown, unavailable, stale and incomplete observations
must remain unknown rather than being treated as success.

| Symptom | First diagnosis and safe next action |
| --- | --- |
| Cannot boot | Check the actual installed package, all sidecars, platform and build provenance. Use the read-only `profile check --target current` from the intended compatible package. A missing or incompatible marker in support is a hint, not a complete profile check. Preserve incompatible/corrupt profile bytes; do not delete the reader floor or reinitialize. Compare the Desktop boot action and daemon listener/exit if available. |
| Replication not advancing | Compare local signed head and receipt coverage in the normal world/host readiness view. Check `world replica-status` and `world storage-status` privately. A closed UTC window, paused cache or quota is not a network failure. For storage pressure follow [storage admission](STORAGE_RETENTION_AND_QUOTAS.md); preserve durable partials. A pull stalled for 45 seconds can emit a stall result; reconnect authorized sources using normal peer procedures, not fabricated receipts. |
| Lease / authority unavailable | A fresh readiness cache is limited to 6 seconds. Missing or stale cache is not a safety verdict. The permit heartbeat interval is 1 second and the existing host timeout is 6 seconds. Inspect the accepted membership/epoch, quorum and local contribution state through the normal host-readiness view. Reconnect the actual authorized peers; follow [authority recovery](AUTHORITY_RECOVERY.md) or [unconfirmed shutdown recovery](UNCONFIRMED_SHUTDOWN_RECOVERY.md) if required. Never manually refresh a permit, clear a fence, copy identity keys or promote a minority. |
| Minecraft will not connect | Request the normal live diagnosis for the selected world. Gateway unavailable: select the correct world normally. Peer link unavailable: restore authenticated connectivity. Host proof unavailable, timeout or authority changed: recheck the actual current authority, quorum, consent and verified runtime. No individual proof failure is inferred from the generic category. Recent gateway session state expires in 30 seconds and does not prove login. Follow [connection diagnosis](CONNECTION_DIAGNOSIS.md); never reuse a cached address or bypass the gateway. |
| Restore / backup failed | Preserve the original data root and signed history. Verify the exact retained checkpoint, review conflicts, then export into a new path outside the profile using [recovery review](RECOVERY_REVIEW.md) or [encrypted backups](CHECKPOINT_BACKUP.md). Do not treat an incomplete export or a stale migration phase as recovered data. Only an explicit recovered copy can create a new private identity. |
| Publication failed or listing missing | Distinguish signed durable checkpoint publication from directory discovery. A local paused flag and a successful refresh do not confirm a remote listing. Preserve the last verified checkpoint; follow the appropriate checkpoint/retention or exact-ID discovery diagnosis. Inspect signed visibility and membership, do not disclose private invitations, and never invent a completion or release tag. For update packages follow [signed update delivery](SIGNED_UPDATES.md) and [release gates](RELEASE_GATES.md). |

For escalation, obtain the exact package/version/platform/architecture
and independently verified release provenance (when available), the
approximate incident window, the affected stage, the redacted report and
a short reproduction description. Keep exact checkpoint identifiers and
private paths on the device; compare them locally with authorized peers.
Source commit and tree status describe the build's source base, not an
attested package hash. Modified or unknown builds must not be represented
as exact release builds. Unverified outcomes should be escalated rather
than silently replaced with guessed metrics. Native platform and
multi-machine reproduction remain separate qualification gates.

## Per-version and compatibility policy

Application version, wire protocol, storage schema, profile reader floor,
runtime tuple, package channel, platform and source commit are separate
compatibility dimensions. A version string alone is never sufficient.

| Version / build | Qualification and supported use |
| --- | --- |
| Published 0.5.0 preview | Historical package rehearsal only. No commit-bound provenance manifest; predates reader guard. No downgrade of an initialized profile. |
| Historical published 0.5.1 preview | Reviewed pinned preview assets; also predates reader guard. Same application version as current candidate does not imply identical code, update support, or profile compatibility. No live-profile downgrade. |
| Current 0.5.1 integration candidate | Protocol 1, storage schema 1, reader generation 1. Current reader preflight and locally tested upgrade from the historical packages; not production release approval. Linux evidence is scoped to the exact exercised build/fixture. |
| Future release | Require exact package provenance and native qualification, supported reader/protocol/schema/runtime matrix, current signed-update compatibility and explicit release gates. A changed reader/schema requires reviewed migration rather than a V1 updater bypass. |

Use [profile upgrade and rollback](PROFILE_UPGRADE.md) for exact historic
targets and save-first, offline recovery. Never reconnect a copied identity.
Use the same reviewed build on participating devices until the exact
mixed-version pair has positive authenticated replication, fencing,
recovery and player-journey qualification. Equal protocol numbers do not
certify mixed-version authority or all newer transport capabilities.

The current new-world runtime baseline is Minecraft 26.1.2, Fabric Loader
0.19.3, Java 25. The adapter's broader declared range is not field
qualification. Other adapter-compatible tuples remain unqualified and
not offered for new Desktop worlds. Reject incompatible versions and
validate exact mod/runtime requirements before live use. Low-level
data-only storage/replica operations do not establish Java or Fabric
compatibility. See [runtime matrix](../research/runtime-compatibility/REPORT.md).
Stable and preview updater channels are separately pinned and opt-in;
strictly newer compatible packages only. Native OS trust and normal
save-first installation still apply. No automatic installer or
production trust activation is provided by support.

Update this matrix and metric schema compatibility notes with each
reviewed release. New metric categories must remain fixed and typed;
unknown/malformed older or newer support-store schemas fail closed.
Publish scope-limited evidence with the exact build and qualification
conditions, never a claim based only on a green package build.
