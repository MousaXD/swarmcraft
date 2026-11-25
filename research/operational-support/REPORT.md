# Operational support observations and field triage (#55)

Date: 2026-11-25. Scope: Linux local operation and package acceptance.
The build reports version 0.5.1, parent commit 426338c6de294d272631d834b6be325ffbafcfa4
and a **modified** tree. This is actual implementation acceptance, not an
exact clean-commit release attestation.

## Implementation

An independent, opt-in local operational recorder is exposed through
the packaged CLI. It records typed results at real daemon, sidecar and
explicit CLI boundaries, retains at most 128 observations, and exports
only bounded redacted JSON on explicit confirmation. Long-running
daemon operations enqueue without waiting for disk; a separate bounded
worker persists observations. Pausing or clearing advances the recording
revision so older workers cannot recreate history. Support recording
never supplies an authorization input. Existing Desktop action history
is unchanged.

The [operational support contract](../../docs/OPERATIONAL_SUPPORT.md)
defines metrics, retention, percentile interpretation, privacy,
field triage for each incident stage, and separate version/reader/
runtime/package compatibility requirements.

## Real process acceptance

[Local daemon receipt](evidence/local-daemon.json) contains only typed
summary evidence. The real signed three-member fixture ran an authority
and a receiver with separate local identities. No Java/runtime config
or hosting consent was installed; the absent third member could not
supply a fabricated vote.

| Check | Actual result |
| --- | --- |
| Default-off recording and first listener | No observations before opt-in; both actual listeners recorded |
| Lease | Missing quorum refused permit, authenticated second member permitted refresh, disconnection removed permit |
| Receiver-driven replication | 1,048,618 encoded payload bytes, 2,953 ms, one source, one parallel request, no failed requests or resumed blobs in this run |
| Ordinary push | Independent durable finalization recorded in 10 ms; exact payload restored |
| Live game route | No gateway before selection; after normal selection an actual local TCP session refused because host proof was unavailable |
| Offline game route | Unavailable after daemon stop, never a cached positive result |
| Fresh quorum | Three actual collection results in source history; no runtime launch or login inferred |
| Restore and verification | Actual signed checkpoint recovered byte-exact; invalid checkpoint refused; verification recorded |
| Publication | Actual CLI snapshot advanced to checkpoint 4 with verified signature and predecessor; failed source path did not advance; local discovery paused and local refresh observed, no remote listing confirmation |
| Privacy and revision | World, peer, transport, checkpoint, state-root, names, paths and actual gateway address canaries absent; clear/pause while discovery publisher alive stayed empty |
| Cleanup | All fixture processes exited; both local hosting consents absent |

The first integration attempt successfully replicated via ordinary
push, but could not observe a receiver-driven pull. The corrected
fixture explicitly selected controlled pull, then exercised ordinary
push separately. The first nested sandbox attempt lacked a configured
loopback and DNS configuration; the final run used an outer isolated
network namespace with loopback enabled, then a private bubblewrap
root with private temporary storage. No production ancestor ownership
validation or networking gate was relaxed.

Reproduce after local test compilation (Linux with unprivileged user
namespaces, iproute2 and bubblewrap; no host network or private profile):

```sh
cargo test --locked --offline -p swarm-cli --test parallel_replication --no-run
# Set TESTBIN to the test executable Cargo printed, in target/debug/deps.
# Run from repository root. The sandbox binds built binaries read-only.
umask 077
unshare --user --map-root-user --net sh -ec '
  ip link set lo up
  exec bwrap --unshare-user --uid 0 --gid 0 --unshare-pid --die-with-parent \
    --ro-bind /usr /usr --ro-bind /lib /lib --ro-bind /lib64 /lib64 \
    --ro-bind /etc/resolv.conf /etc/resolv.conf --ro-bind /etc/hosts /etc/hosts \
    --proc /proc --dev /dev --tmpfs /tmp --dir /run --dir /home \
    --ro-bind "$1/target/debug" "$1/target/debug" --chdir /tmp \
    --clearenv --setenv PATH /usr/bin --setenv HOME /tmp --setenv TMPDIR /tmp \
    -- /usr/bin/sh -ec "umask 077; exec \"\$@\"" sh "$2" \
    --exact operational_support_reports_actual_replication_lease_denial_restore_and_publication \
    --nocapture --test-threads=1
' sh "$PWD" "$TESTBIN"
```

## Extracted Debian package acceptance

The actual locally built debug Debian package was extracted, and all
four sidecar SHA-256 digests matched the built binaries. The acceptance
harness used the quiescent genuine Java save from #52 (31 files,
2,215,315 bytes). All six checks passed:
default-off/no initialization and explicit consent; redacted actual CLI
and runtime failure observations; byte-exact restore and read-only
report; pause/clear with byte-identical non-support profile and unchanged
withdrawn consent revision, identity and signed checkpoint; unsupported
future reader report without mutation; malformed support-store refusal.

The first package run's final assertion compared the entire live
hosting status, including changing memory telemetry. The corrected
harness compares durable consent and all non-support profile bytes
independently; no production code change or weaker consent rule was
needed. The actual package, four sidecar hashes, six checks and exact
redacted report are in [package receipt](evidence/package-support.json).
Reproduce using an already-built debug package and a genuine
quiescent save, with a fresh output directory:

```sh
unshare --user --map-root-user --net sh -ec '
  ip link set lo up
  exec python3 scripts/acceptance/operational_support.py "$@"
' sh --deb "$DEB" --bin "$BUILD_BIN_DIR" --save "$SAVE" --artifacts "$NEW_ARTIFACTS"
```

## Regression and remaining qualification

- 206 library tests passed, including six support unit tests; two
  existing delegated-cgroup tests ignored, not credited here.
- 51 CLI binary tests passed.
- 14 selected integration regressions passed (profile guard 2, encrypted
  checkpoint backup 1, runtime setup hardening 11).
- New real-process acceptance passed; strict Clippy for library, CLI,
  runtime/host binaries and parallel-replication test passed.
- No hosted CI, main modification, external service or native installer
  execution. The local reports are observations, not independent
  authority or release attestations.

There is no remote support service or fleet SLO dataset. The live
route case is a genuine refusal, not an authenticated Minecraft login.
The recorded runtime prepare failure is a missing-world validation
failure; live Java restore, actual Fabric startup and player-journey
metric capture remain field qualification. Native Windows/macOS
package/report acceptance, actual field engineer exercises and
cross-network mixed-version qualification also remain unverified.
A detached report may be incomplete and cannot prove that no action
occurred or be used as a hosting or recovery certificate.
