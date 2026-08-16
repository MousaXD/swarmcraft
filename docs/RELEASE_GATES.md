# SwarmCraft 0.2.x Release Gates

This file records the minimum executable evidence expected before SwarmCraft preview changes are treated as healthy.

It is intentionally stricter than "the code compiles." SwarmCraft changes distributed state, world durability and authority, so process-level failure behavior is part of correctness.

## Latest verified main CI baseline

For the 0.2.1 frontend/status baseline:

- GitHub Actions workflow: `CI`
- Run: `31960938193`
- Commit: `e70cbab011470909d0427ecda1e51bc320cda87a`
- Result: **PASS**

That run passed Rust gates on Linux, Windows and macOS, the Fabric build, RustSec, process-level acceptance scenarios and native desktop package jobs.

A later commit must earn its own green CI result; this record is evidence, not a waiver.

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

- an original peer creates a verified snapshot and replicates it to two peers;
- the original peer disappears completely;
- one surviving replica is missing a blob;
- another surviving replica contains a same-size but corrupt encoded blob;
- the corrupt source is rejected by content verification;
- a failed final blob verification discards the poisoned partial file so another source can retry from offset zero;
- a partial blob begun from one surviving replica can resume from another replica holding the same content-addressed blob;
- a fourth peer reconstructs, finalizes, verifies and restores the exact original world from the surviving replicas.

This is the permanent executable form of the roadmap's three-peer to fourth-peer reconstruction criterion.

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

### Three-daemon hard-kill recovery

- multiple authenticated members share canonical state;
- current authority disappears;
- surviving peers reach safe recovery authority using current quorum/fencing rules;
- stale returning state cannot overwrite the accepted generation;
- replication resumes from canonical history.

### Recovery successor disappears

- a recovery candidate begins a durable recovery round;
- that candidate disappears before completing epoch promotion;
- a later strictly higher recovery round on the same canonical base can restore liveness;
- old recovery votes/rounds do not gain authority after the successor changes.

This scenario closes the known v0.1 preview liveness limitation. It is now a permanent regression gate.

### Solo history

- signed solo policy permits solo advancement;
- compatible returning history is accepted safely;
- independently advanced solo histories are detected as divergent;
- conflicting branches are preserved and never silently merged.

---

## Multi-gigabyte network soak

The separate `Network Soak` workflow is a permanent Phase 1 transport gate for networking/storage changes and also runs weekly.

Default profile:

- 2 GiB transferred through real QUIC/libp2p request/response messages;
- maximum protocol blob chunk size of 256 KiB;
- 0.2% synthetic packet loss;
- 250 Mbit/s bandwidth shaping;
- light latency/jitter shaping so the job remains volume-focused rather than becoming thousands of artificial sequential RTTs;
- a hard sender restart every 256 MiB;
- deliberately lost acknowledgement at every restart boundary;
- durable transport identity reload and signed application re-authentication;
- exact committed-offset resume negotiation after every restart;
- deterministic byte-for-byte chunk verification;
- uploaded workflow artifacts containing the tested commit/profile, qdisc configuration and test output.

The first passing default-profile evidence added with this gate is:

- workflow: `Network Soak`;
- run: `31966815821`;
- tested commit: `50a072f0d3e32d6c67d836a9725b5d88078d102c`;
- result: **PASS**.

Manual dispatch can run 1 GiB, 2 GiB or 5 GiB profiles with configurable restart intervals. Green soak evidence proves sustained synthetic transport/reconnect behavior; it does not certify arbitrary residential NAT or carrier networks.

---

## Fabric gate

The supported Fabric bridge must build against the repository's declared Minecraft/Fabric target.

The bridge is not optional test decoration. Host lifecycle, save barriers and authority-permit behavior depend on it.

---

## Desktop/package gates

CI/native packaging covers the Tauri desktop shell and bundled SwarmCraft runtime sidecars.

Expected package coverage:

- Linux native package build, including `.deb` and configured AppImage/release targets;
- Windows NSIS `.exe` installer;
- macOS Apple Silicon package;
- macOS Intel package when the configured runner is available;
- runtime sidecars staged for the matching platform/architecture.

A package job is only healthy when the installer contains the expected SwarmCraft runtime sidecars, not merely when Tauri produces an empty shell.

---

## Rolling main snapshot

The `Main Desktop Installers` workflow publishes development installers to the rolling `main-latest` prerelease after its required platform build policy is satisfied.

`main-latest` is a **development snapshot**, not a production-stability promise.

Application version metadata must remain coherent across:

- Rust workspace packages;
- desktop Tauri package/config;
- Fabric mod metadata;
- lockfile package versions;
- produced installer names where applicable.

Wire protocol version is independent and must not be bumped just to match application version numbers.

---

## Release signing

Release workflows can use configured platform signing credentials, but documentation and release notes must distinguish:

- cryptographically signed/notarized production artifacts;
- unsigned or ad-hoc development artifacts.

Never claim Authenticode signing, Apple Developer ID signing or notarization unless the workflow actually performed it successfully for those artifacts.

---

## Storage large-world evidence

The repository has historical large-world streaming evidence:

- GitHub Actions run: `31757348001`
- Tested commit: `886120e4a7b67f8b448541551b8b04fb03366654`
- Command: `cargo test -p swarm-storage release_large_world_streaming_profiles -- --ignored --nocapture`
- Profiles: 1 GiB, 5 GiB and 10 GiB synthetic world files
- Path: streaming snapshot creation, Zstd encoding, content hashing, snapshot commit and streaming verification
- Buffering: bounded storage buffers rather than whole-world materialization

Repeat or expand this evidence when storage algorithms materially change.

The permanent snapshot-swarm reconstruction gate proves source failover, corruption rejection and cross-replica resume semantics. The network soak separately proves that the QUIC transfer/reconnect path survives sustained multi-gigabyte volume and repeated interruptions.

---

## Gates not yet equivalent to production certification

Green CI does **not** prove all real-world deployment conditions.

Still requiring broader/manual evidence:

- longer-duration and wider-profile network soak campaigns beyond the permanent default profile;
- production-quality parallel multi-source scheduling and retention/GC policy;
- repeated long-duration Minecraft crash/host-migration campaigns;
- disk-full and hardware corruption scenarios;
- hostile/malicious peer campaigns;
- broader fuzz/property testing;
- representative home NAT/CGNAT/mobile/IPv6 validation;
- production signing/notarization operations;
- automatic successor Minecraft launch and player reconnection.

See [NETWORK_VALIDATION.md](NETWORK_VALIDATION.md) and [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).
