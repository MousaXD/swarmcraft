# SwarmCraft 0.2.0 Release Notes

SwarmCraft 0.2.0 moves the project from its initial preview foundation toward a safer, more usable distributed-world runtime while preserving the core rule that stale, partitioned or failed peers must not silently rewrite canonical history.

## Recovery ballots close the abandoned-successor liveness hole

Crash recovery now uses signed, durable, monotonic recovery ballots instead of permanently locking one next-generation successor.

Each ballot is anchored to the exact accepted generation, canonical snapshot/state root, membership hash and candidate. Voters persist their highest promise before signing. A later successor may supersede an abandoned recovery candidate with a strictly higher round only on the same canonical base. Majority intersection makes the older round stale once an intersecting quorum moves forward.

Recovery epochs carry and persist a quorum certificate. Quorum requirements are not lowered.

## Explicit solo history and conflict safety

Worlds whose signed policy permits solo advancement can represent that history explicitly. Solo branch ancestry/head state is signed and persisted, and player-facing safety state distinguishes solo/unreplicated progress from quorum-backed history.

When peers return, compatible history can be adopted. Independently advanced histories become an explicit conflict and both branches are preserved. SwarmCraft 0.2.0 does not attempt arbitrary Minecraft world merges.

## Per-world compatibility manifests

World configuration includes deterministic execution metadata:

- Minecraft version;
- loader ID and loader version;
- SwarmCraft protocol version;
- Fabric adapter version;
- exact server/client mod and datapack artifact IDs, versions and hashes;
- visibility, authority policy and membership policy;
- signed presentation metadata.

The compatibility fingerprint is canonicalized and anchored to world identity. A replica can remain storage-only when it has not established authority-compatible execution state.

## World visibility and background replicas

The protocol supports private, unlisted and public presentation modes without making a listing service authoritative. Background seeding lets an authorized peer keep replicated world data available while Minecraft is off.

## Player-facing desktop

The Tauri desktop application now leads with a My Worlds dashboard rather than raw daemon controls. World cards surface safety, Minecraft/compatibility information, replica context and the latest checkpoint, with create/join/play/sleep/leave, background seeding, conflict and compatibility flows. Distributed-system internals remain available under Diagnostics.

## Networking diagnostics and hard limits

The network layer retains QUIC/libp2p, mDNS, Kademlia, AutoNAT, DCUtR and relay support and adds structured diagnostic state for local/observed addresses, NAT state, direct/relay connectivity, hole-punch state, selected relay and failure reason.

Additional bounded limits cover recovery certificates, world manifests, presentation tags and existing blob/membership request classes.

## Reproducible and broader CI/release builds

- Root `Cargo.lock` is committed.
- Cargo build/test/clippy paths use `--locked` where appropriate.
- RustSec evaluates the committed dependency graph.
- Linux, Windows and macOS Rust gates are included.
- Desktop workflows cover Linux `.deb`/`.AppImage`, Windows NSIS `.exe`, macOS Apple Silicon `.dmg`, and macOS Intel `.dmg` when runner availability permits.
- Official release workflow also publishes the Fabric bridge jar and SHA-256 files.
- Rolling `main-latest` remains a development snapshot and waits for required platform builds before publication.

## Security and signing readiness

Windows release workflow support can Authenticode-sign when repository certificate secrets are configured. macOS release workflow support can use Developer ID signing/notarization credentials when configured. The workflows explicitly label unsigned/ad-hoc builds when credentials are unavailable and never claim signing/notarization that did not happen.

## Current Minecraft target

- Minecraft Java `26.1.2`
- Fabric Loader `0.19.3`
- Fabric API `0.155.2+26.1.2`
- Java `25+`

A world's signed compatibility manifest may add exact mod/datapack requirements.

## Important limitations

SwarmCraft 0.2.0 remains preview software for small trusted membership sets. It is not anonymous Byzantine consensus. Public/unlisted metadata does not include a centralized public lobby. Automatic third-party mod redistribution is not implemented. Competing solo branches require manual recovery rather than unsafe semantic merging. Real-world NAT environments must be validated according to `docs/NETWORK_VALIDATION.md` before being described as universally verified.

Distributed region simulation is not implemented.

## Upgrade note

Application version 0.2.0 and wire protocol version are separate concepts. This release retains the existing wire protocol version unless an explicit compatibility break requires a protocol bump. New 0.2 wire messages are appended rather than reordering existing postcard enum discriminants.

## Post-0.2.0

Next work includes richer public/friend discovery, licensing-aware automatic modpack acquisition, deeper fuzz/crash/disk-full hardening, broader NAT validation, lower-loss incremental replication, production signing operations, and eventual research into distributed region simulation.
