# Fresh private invitation and founderless real-server wake

Scope: Sabathon #1, #5, #6, #7, #9 and the exact pinned Fabric baseline
(#42). This is an explicitly invoked local process gate, not physical
multi-device acceptance or a Minecraft client login.

## Reproduction

Use the compatible Java 25 and Gradle toolchain, the pinned Minecraft
26.1.2 / Fabric Loader 0.19.3 tuple, an actual private LAN interface,
enough host memory for a 512 MiB Minecraft heap under the existing
admission rules, and network access to the official runtime downloads.
Build the exact candidate server bridge and optional Java test probe:

```sh
cd minecraft/fabric
gradle --offline jar clientReconnectProbeClasspath
cd ../..
export SWARMCRAFT_FABRIC_MOD_JAR="$PWD/minecraft/fabric/build/libs/swarmcraft-fabric-0.5.1.jar"
export SWARMCRAFT_RECONNECT_PROBE_CLASSPATH="$(cat minecraft/fabric/build/client-reconnect-test/classpath.txt)"
umask 077
flock /tmp/swarmcraft-sabathon-cargo.lock -c 'CARGO_BUILD_JOBS=2 cargo test --locked --offline --jobs 2 -p swarm-cli --test manual_transfer_process_gate hosted_live::fresh_private_invite_real_sleep_and_founderless_wake -- --ignored --nocapture --test-threads=1'
```

The ignored test creates its own private temporary data directories and
three independent identities. It binds disposable UDP listeners to the
route-selected private LAN address because invitation bootstrap validation
correctly refuses loopback. Game entry remains on each player's loopback
gateway. No public listener, copied world folder, injected genesis or
sleep record, mock Ready, disabled online mode, account credential or
production profile is used. Runtime install uses official artifacts;
the server bridge is the reviewed locally built candidate.

## Invariants exercised

1. A normal CLI Create generates a new private genesis, initial accepted
   epoch and one-member roster. Two genuinely fresh peers obtain signed
   membership through ordinary Invite/Join. The invitation takes
   connectivity from the actual running daemon, not a manually seeded
   diagnostics file or CLI bootstrap override. Host candidacy is explicit
   for the future host; the third peer is storage-only and runs
   `--replica-only`. All three independently verify the checkpoint and
   the two committed joint-membership transitions. No join creates
   hosting consent.
2. The prospective host and founder explicitly install and verify their
   own runtimes, accept the server EULA and opt in to a bounded heap.
   The storage-only peer has no Java runtime or consent. A genuine
   Minecraft status exchange through the verified stable player gateway
   establishes the initial route (not player login).
3. A file marker in the actual running world's save is included in the
   safe stop. The founder, prospective host and storage-only peer must
   all hold the identical signed sleep boundary and verified checkpoint.
   The founder's fresh readiness report must turn green only after both
   exact sleep acknowledgements. The Java reconnect probe must refuse
   admission and no permit may remain.
4. All three daemon processes are stopped. The founder is never restarted.
   The host candidate alone explicitly requests wake; for a bounded
   ten-second observation the old epoch, exact sleep boundary, absent
   runtime readiness and absent permit must remain unchanged.
5. Restart the already admitted storage-only voter using the candidate's
   known private LAN address as a reconnect hint. This is a test
   controlled transport input, not a new membership grant. The two
   surviving peers must obtain the exact sleep-bound recovery certificate
   with their two votes, advance the fencing token once, start the
   previously installed official runtime and restore the saved marker.
   The same player gateway must serve real Minecraft status at the new
   authority. The Java reconnect probe exercises the actual compiled
   client policy at both epochs. The founder's offline data, sleep record
   and old epoch remain untouched. The storage-only voter never obtains a
   permit, consent or runtime.
6. The new host performs a final safe save/stop and the remaining voter
   verifies that signed boundary. The absent founder cannot acknowledge
   it, so green shutdown must remain false. A fresh Java route query must
   refuse a connection. Only test-owned process groups are killed on
   teardown and disposable profiles are removed.

Even a completely passing local gate cannot establish that remote peers
discover one another across WAN/NAT, that the installed Desktop/client
workflow works without a terminal, that a licensed player logged in, or
that an actual in-game inventory marker survived. Those are distinct
field gates. The test does not change authority, consent or route policy.

## Observed results (26 November 2026)

The first attempts exposed errors in the new test itself: private
listener hints may omit the transport peer suffix, and changing hosting
consent invalidates the prior runtime verification. The final fixture
accepts either exact native listener hint format and verifies the
official runtime **after** opting in, before the founder departs.

The earlier diagnostic run used the actual ordinary commands and
passed fresh admission of both peers, initial real Minecraft status
and Java probe, signed all-peer safe sleep, green founder readiness,
and the ten-second single-survivor wake denial. Following explicit
runtime re-verification on the disposable successor, the surviving
majority committed recovery epoch 2; the replacement Fabric runtime
started and its world contained the exact original saved file marker.
However, fresh game route verification repeatedly expired during the
final generation recheck. The strict test failed after 304.04 seconds.
This is partial diagnostic evidence, not a passing acceptance run.

The fix memoizes only the result of successful historical
signature verification in the CLI verifier. Each entry contains the
complete expected peer identity, public key, domain-separated signing
message and 64-byte signature, compared byte for byte. At most 128
entries of at most 64 KiB signing-message bytes are retained in memory
with least-recently-used eviction; oversized input and a poisoned cache
fall back to the original verifier, and invalid signatures never enter
the cache. No persisted cache, certificate validity decision, freshness
vote, host attestation, runtime readiness, membership role, consent or
checkpoint verification result is reused. Semantic and quorum checks,
current durable reads, independent snapshot blob verification, final
generation and consent rechecks, and both original wall-clock and
monotonic deadlines remain mandatory.

The exact-input cache tests reject altered signatures, messages,
expected identity and key, malformed signature length, and exercise
eviction, oversized messages and poisoned-lock fallback. The actual
membership-proof regression corrupts a previously verified historical
vote without changing its valid quorum shape. The durable recovery
regression changes a vote signature in an otherwise valid stored
certificate after warming the verifier; both fail closed. The full
CLI library run passed 210 tests, with three pre-existing delegated
cgroup tests ignored (they require their own explicit test scope).

The next attempted full fresh-world run compiled the native test and
executable, but the unchanged production memory admission refused
before any peer/world was created: 4,511 MiB available versus 4,543 MiB
required. No unrelated application or VM was stopped and no admission
threshold was adjusted. When capacity recovered, the final unmodified production-gate rerun
passed in **173.35 seconds**. The actual status exchanges succeeded at
epochs 1 and 2, the old founder never returned, the headless voter never
hosted, the exact original save marker was restored, and the final
signed sleep replicated with the absent founder's acknowledgement still
missing. Both Java probes correctly refused a sleeping route. All three
disposable data directories were removed. No permission or deadline was
relaxed. This resolves the observed post-wake expiry in the local run,
not every possible high-load or independent-host scenario.

The full CLI library (210 passing, three explicitly scoped cgroup tests
excluded), strict all-target/all-feature CLI Clippy, formatting,
workflow policy and release metadata checks passed. The positive test
used the same server/client/probe JARs as the earlier real transfer
and recovery run. [Retained output and artifact pins](evidence/2026-11-26-fresh-private-wake.txt).
No hosted CI was used. Installed Desktop and authenticated Minecraft
player login, live inventory continuity and physical field qualification
remain separate gates.
