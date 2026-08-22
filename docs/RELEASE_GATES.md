# SwarmCraft 0.4.0 Release Gates

This file records the minimum executable evidence expected before SwarmCraft preview changes are treated as healthy.

It is intentionally stricter than "the code compiles." SwarmCraft changes distributed state, world durability and authority, so process-level failure behavior is part of correctness.

Application version `0.4.0` and wire protocol version `1` remain separate release dimensions.

## Final candidate rule

Before merging the integration candidate to `main`:

- current-head `CI` must pass;
- current-head `Release version guard` must pass;
- current-head `Player journey live acceptance` must pass;
- no unresolved safety review threads may remain;
- the candidate must still be based on the intended `main` head without hidden divergence;
- quorum, fencing, signed-history, runtime-verification or EULA rules must not be weakened to turn a fail-closed case green.

Historical passing runs remain useful evidence, but every final candidate must earn its own green result.

---

## Permanent Rust gates

Required:

- committed `Cargo.lock` remains current;
- Ubuntu format check;
- strict Clippy;
- Rust tests with locked dependencies;
- Windows strict Clippy and tests;
- macOS strict Clippy and tests;
- RustSec dependency audit.

Protocol/storage changes must not bypass these gates merely because a failing test appears unrelated to the edited crate.

---

## Process-level acceptance gates

The normal CI workflow includes real process/network scenarios rather than relying only on unit tests.

Required scenarios include:

### Peer networking hard reconnect

- two independent QUIC/libp2p peers authenticate using signed application identities;
- the restarting peer reloads the same persisted transport identity;
- a replacement connection is allowed to race the dead connection from the previous process;
- the live replacement connection becomes canonical without the stale connection erasing application authentication;
- signed application authentication is re-established after restart;
- authenticated request/response traffic succeeds after the reconnect.

This gate protects the distinction between durable peer identity and transient network connections.

### Hostile input and handshake hardening

- malformed/high-risk network input remains bounded and nonfatal;
- signed application handshake validation rejects invalid or inconsistent peer state;
- wire/request limits remain enforced before unbounded allocation.

### Network impairment and resume

Normal CI also runs an impaired-link QUIC transfer gate:

- 64 MiB is transferred through the real libp2p request/response path;
- loopback traffic is shaped with latency variation, packet loss and a bandwidth limit;
- the sender is hard-restarted every 16 MiB;
- the receiver deliberately commits the final chunk before each restart while the sender loses the acknowledgement;
- the restarted sender reloads the same transport identity and re-authenticates the same application identity;
- `MissingBlobs` resume negotiation returns the receiver's committed offset;
- transfer continues without replaying already committed data;
- every received chunk is checked against the deterministic source payload.

This gate catches reconnect/resume regressions on ordinary pull requests without forcing the full multi-gigabyte profile into every CI job.

### Snapshot swarm reconstruction

- an original peer creates a verified snapshot and replicates it to surviving peers;
- an original/source peer can disappear completely;
- surviving replicas can have asymmetric availability;
- corrupt encoded blob data is rejected by content verification;
- a poisoned partial is discarded when final verification fails;
- a partial blob can resume from another replica holding the same content-addressed blob;
- a new peer reconstructs, finalizes, verifies and restores the exact original world.

### Publication ownership, GC and retention races

- ordinary local snapshot publication participates in GC protection before its manifest becomes durable;
- publication pins are owned by their publication transaction rather than removed by hash alone;
- two concurrent publishers that reference the same blob cannot release one another's protection;
- replica verification rejects decompression beyond the signed uncompressed size before consuming an amplified stream;
- crash-stale coordination state can be recovered without deleting live publication state.

### Storage failure injection

- partial/interrupted persistence paths fail without silently publishing corrupt canonical state;
- deterministic disk/error injection covers critical storage publication paths;
- recovery/retry does not reinterpret corrupted state as valid progress.

### Existing-world import

- source validation requires a usable Minecraft world and explicit compatibility inputs;
- source bytes remain unchanged;
- canonical world state is built and verified in hidden staging;
- failure before final publication leaves no visible half-world;
- retry is safe;
- EULA and machine-local runtime configuration are not imported;
- the imported world re-enters the normal Runtime Wizard + Play flow.

### Corrupt/unreadable sleep state

The final CI must exercise all authority-sensitive entry points:

- direct host launch;
- standby host readiness/launch;
- migration/runtime supervision.

Only a genuine missing sleep record may mean "awake". A present corrupt, unreadable or invalidly signed record must block fail-closed.

### Host Readiness and two-member quorum

The negative readiness matrix must cover runtime, mod, synchronization, reachability, conflict and quorum failures.

A two-voter Alice/Bob world must remain `BlockedByQuorum` after Alice disappears. Bob alone must not be promoted by a one-of-two election shortcut. Explicit authority transfer while both peers are available is a separate supported path.

### Live join and replication

- start independent peer daemons;
- stage a signed join;
- authority accepts canonical membership;
- joined peer receives the current signed snapshot without requiring a reconnect;
- replicated snapshot verifies.

### Host process lifecycle

- restore a verified snapshot into a runtime directory;
- start the Minecraft/Fabric host process;
- complete local IPC handshake;
- exercise save/shutdown barrier behavior;
- commit a final signed snapshot;
- persist sleep state.

### Migration orchestration

- authority transition invokes the shared runtime/migration path rather than a duplicate launch implementation;
- canonical state is restored before successor runtime startup;
- runtime/mod/sleep prerequisites remain fail-closed;
- successor startup reaches authenticated Fabric readiness before it is considered ready.

### Three-daemon hard-kill recovery

- multiple authenticated members share canonical state;
- current authority disappears;
- surviving peers reach safe recovery authority using current quorum/fencing rules;
- stale returning state cannot overwrite the accepted generation;
- replication resumes from canonical history.

This is the positive crash-recovery topology. It deliberately uses enough voting members to preserve quorum.

### Recovery successor disappears

- a recovery candidate begins a durable recovery round;
- that candidate disappears before completing epoch promotion;
- a later strictly higher recovery round on the same canonical base can restore liveness;
- old recovery votes/rounds do not gain authority after the successor changes.

### Solo history

- signed solo policy permits solo advancement when configured;
- compatible returning history is accepted safely;
- independently advanced solo histories are detected as divergent;
- conflicting branches are preserved and never silently merged.

---

## Real clean-machine Minecraft gate

`.github/workflows/player-journey-live.yml` is a separate required final-candidate gate.

It uses a fresh SwarmCraft data directory and official external resolution paths while exercising the candidate Fabric artifact. The workflow must prove:

1. a fresh world begins without EULA acceptance or persisted runtime configuration;
2. setup does not silently accept the Minecraft server EULA;
3. explicit player EULA acceptance is required;
4. compatible managed Java is resolved rather than inherited from the workflow build JVM;
5. official Minecraft/Fabric components and the candidate SwarmCraft bridge are prepared;
6. runtime verification persists the launch configuration;
7. real Minecraft launches through the shared Rust authority/runtime path;
8. authenticated Fabric readiness succeeds and a real `level.dat` exists;
9. known world data can be written;
10. Stop World completes the save/checkpoint/shutdown durability barrier;
11. a new canonical snapshot and durable sleep record are published;
12. backend restart preserves EULA/runtime configuration;
13. a second real launch restores the known world data;
14. a second safe stop advances canonical history again without divergence.

The candidate workflow may inject the freshly built SwarmCraft Fabric JAR because an unpublished candidate cannot yet have an immutable matching release tag. Production/runtime resolution rules remain separately validated by the release artifact contract below.

---

## Multi-gigabyte network soak

The separate `Network Soak` workflow is a permanent transport gate; it runs on networking/storage pull requests and matching `main` changes, plus its scheduled/manual profiles.

Default profile:

- 2 GiB transferred through real QUIC/libp2p request/response messages;
- maximum protocol blob chunk size of 256 KiB;
- 0.2% synthetic packet loss;
- 250 Mbit/s bandwidth shaping;
- light latency/jitter shaping so the job remains volume-focused;
- a hard sender restart every 256 MiB;
- deliberately lost acknowledgement at restart boundaries;
- durable transport identity reload and signed application re-authentication;
- exact committed-offset resume negotiation after restart;
- deterministic byte-for-byte verification;
- uploaded workflow artifacts containing tested commit/profile and output.

Historical first passing default-profile evidence:

- workflow: `Network Soak`;
- run: `31966815821`;
- tested commit: `50a072f0d3e32d6c67d836a9725b5d88078d102c`;
- result: **PASS**.

Manual dispatch can run 1 GiB, 2 GiB or 5 GiB profiles with configurable restart intervals. Green soak evidence proves sustained synthetic transport/reconnect behavior; it does not certify arbitrary residential NAT or carrier networks.

---

## Fabric gate

The supported Fabric bridge must build against the repository's declared Minecraft/Fabric target.

The release artifact must embed the required Fabric API payload under the expected nested-JAR layout. Host lifecycle, save barriers and runtime readiness depend on the bridge; it is not optional test decoration.

---

## Desktop/package gates

CI/native packaging covers the Tauri desktop shell and bundled SwarmCraft runtime sidecars.

Expected package coverage:

- Linux `.deb` + AppImage in normal CI;
- Windows NSIS `.exe` installer;
- macOS Apple Silicon `.dmg`;
- macOS Intel `.dmg`;
- matching runtime sidecars staged for each target/architecture.

Every Desktop package must contain all four Tauri external binaries:

- `swarmcraft`;
- `swarmcraft-host`;
- `swarmcraft-runtime`;
- `swarmcraft-import`.

A package job is only healthy when the installer contains the complete sidecar set, not merely when Tauri produces a shell.

---

## Rolling main snapshot

The `Main Desktop Installers` workflow publishes development installers to the rolling `main-latest` prerelease after its required platform/Fabric jobs succeed.

`main-latest` is a **development snapshot**, not a production-stability promise.

The rolling snapshot must include:

- Linux Debian package and checksum;
- Windows NSIS installer and checksum;
- macOS ARM64 disk image and checksum;
- macOS x86_64 disk image and checksum;
- exact versioned `swarmcraft-fabric-X.Y.Z.jar`;
- checksum for that Fabric JAR.

Main snapshot Desktop packages stage all four sidecars listed above.

Application version metadata must remain coherent across:

- Rust workspace packages;
- desktop Tauri package/config;
- Fabric mod metadata;
- lockfile package versions;
- produced release asset names where applicable.

Wire protocol version is independent and must not be bumped just to match application version numbers.

### Runtime Installer release-source rule

The Runtime Installer prefers the immutable GitHub release tag `vX.Y.Z` for the SwarmCraft Fabric bridge.

For a freshly merged technical-preview `main` before the immutable tag is published, it may fall back to `main-latest` **only** if that rolling release contains the exact requested versioned Fabric JAR and exact checksum asset. A later `main-latest` containing `swarmcraft-fabric-0.5.0.jar` cannot satisfy a request for `0.4.0`.

This closes the bootstrap window without turning `main-latest` into an unversioned trust shortcut.

---

## Tagged release contract

A `vX.Y.Z` tag must build and publish:

- Linux release packages;
- Windows release installer;
- both macOS release architectures;
- the versioned SwarmCraft Fabric bridge JAR and checksum;
- signing-status information/checksums appropriate to each platform.

Tagged Desktop release jobs must stage the same four sidecars as normal CI and the rolling main snapshot.

---

## Release signing

Release workflows can use configured platform signing credentials, but documentation and release notes must distinguish:

- cryptographically signed/notarized production artifacts;
- unsigned or ad-hoc development artifacts.

Never claim Authenticode signing, Apple Developer ID signing or notarization unless the workflow actually performed it successfully for those artifacts.

---

## Storage large-world evidence

The repository has historical large-world streaming evidence:

- GitHub Actions run: `31757348001`;
- tested commit: `886120e4a7b67f8b448541551b8b04fb03366654`;
- command: `cargo test -p swarm-storage release_large_world_streaming_profiles -- --ignored --nocapture`;
- profiles: 1 GiB, 5 GiB and 10 GiB synthetic world files;
- path: streaming snapshot creation, Zstd encoding, content hashing, snapshot commit and streaming verification;
- buffering: bounded storage buffers rather than whole-world materialization.

Repeat or expand this evidence when storage algorithms materially change.

The permanent snapshot-swarm reconstruction gate proves source failover, corruption rejection and cross-replica resume semantics. Publication/GC race tests protect the pre-manifest publication window. The network soak separately proves the QUIC transfer/reconnect path under sustained multi-gigabyte volume and repeated interruptions.

---

## Intentional YELLOW gates

These limitations do **not** block a 0.4.0 technical-preview merge when documented accurately, and they must not be made green by weakening safety.

### Two-voter crash failover

For two voting members, majority quorum is two. If Alice disappears, Bob alone has one vote and must remain `BlockedByQuorum`. Positive automatic crash recovery uses a topology with a surviving quorum, such as Alice/Bob/Carol.

### Multi-member wake

No dedicated sleep-bound quorum wake election exists yet. Multi-member sleeping worlds remain fail-closed. Do not implement first-click-wins wake or repurpose ordinary crash recovery without a transition bound to the durable sleep record/canonical snapshot.

### Seamless player reconnection

The successor Minecraft runtime can start automatically after safe authority transition, but Minecraft clients are not yet universally redirected/reconnected without coordination.

### Public-network certification

Automated direct/relay/DCUtR diagnostics, synthetic impairment and soak testing do not prove every home router, symmetric NAT, CGNAT, mobile carrier, blocked-UDP policy or IPv6 ISP path. Representative field records remain required for those claims.

---

## Gates not yet equivalent to production certification

Green CI does **not** prove all real-world deployment conditions.

Still requiring broader/manual evidence:

- longer-duration and wider-profile network soak campaigns beyond the permanent default profile;
- production-quality parallel multi-source scheduling and longer retention/GC churn campaigns;
- repeated long-duration real-Minecraft crash/host-migration campaigns;
- hardware-specific disk-full/corruption campaigns beyond deterministic failure injection;
- longer hostile/malicious peer campaigns;
- broader fuzz/property testing;
- representative home NAT/CGNAT/mobile/IPv6/blocked-UDP validation;
- production signing/notarization operations;
- seamless automatic Minecraft client reconnection after migration.

See [NETWORK_VALIDATION.md](NETWORK_VALIDATION.md), [FINAL_PLAYER_JOURNEY_ACCEPTANCE.md](FINAL_PLAYER_JOURNEY_ACCEPTANCE.md), and [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).
