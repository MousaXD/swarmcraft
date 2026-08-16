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

---

## Gates not yet equivalent to production certification

Green CI does **not** prove all real-world deployment conditions.

Still requiring broader/manual evidence:

- repeated long-duration Minecraft crash/host-migration campaigns;
- disk-full and hardware corruption scenarios;
- hostile/malicious peer campaigns;
- broader fuzz/property testing;
- representative home NAT/CGNAT/mobile/IPv6 validation;
- production signing/notarization operations;
- automatic successor Minecraft launch and player reconnection.

See [NETWORK_VALIDATION.md](NETWORK_VALIDATION.md) and [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).
