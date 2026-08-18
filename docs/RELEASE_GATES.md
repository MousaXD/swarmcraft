# SwarmCraft 0.4.0 Release Gates

SwarmCraft changes distributed authority and world durability, so a release candidate must earn more than a successful compile.

## Required candidate evidence

Before merging the integration candidate to `main`:

- current-head CI must pass;
- Release version guard must pass;
- Player journey live acceptance must pass on the final candidate content;
- no unresolved safety review threads may remain;
- quorum/fencing rules must not be weakened to turn intentional fail-closed cases green.

## Normal CI gates

Required coverage includes:

- Rust format, strict Clippy and locked tests on Linux, Windows and macOS;
- RustSec dependency audit and committed lockfile validation;
- hostile network input and handshake hardening;
- snapshot reconstruction, publication ownership/GC/retention race coverage and storage failure injection;
- existing-world import acceptance;
- direct, standby and migration corrupt-sleep fail-closed regressions;
- Host Readiness negative matrix including two-member quorum behavior;
- live join, host lifecycle, migration orchestration, runtime hardening, three-daemon recovery, successor-loss recovery and solo-history conflict handling;
- fuzz smoke and WAN-like QUIC resume impairment;
- Fabric build with embedded Fabric API verification;
- Desktop frontend tests and native packages on Linux, Windows, macOS ARM64 and macOS x86_64.

## Desktop package contract

Every Desktop package must contain all four sidecars declared by Tauri:

- `swarmcraft`
- `swarmcraft-host`
- `swarmcraft-runtime`
- `swarmcraft-import`

Producing a Tauri shell while omitting one of these is a release failure.

## Main snapshot contract

A push to `main` builds the rolling `main-latest` technical-preview release. It must include:

- Linux `.deb`;
- Windows NSIS `.exe`;
- macOS ARM64 `.dmg`;
- macOS x86_64 `.dmg`;
- the exact versioned `swarmcraft-fabric-X.Y.Z.jar`;
- SHA-256 checksum files.

Development signing status must be described accurately.

## Tagged release contract

A `vX.Y.Z` tag builds the same four-sidecar Desktop bundles and the exact versioned Fabric bridge JAR/checksum. Runtime Installer prefers this immutable version tag.

For a just-merged technical-preview main snapshot before its immutable tag is published, Runtime Installer may fall back to `main-latest` only when that release contains the exact requested `swarmcraft-fabric-X.Y.Z.jar` and matching checksum asset. A newer rolling release cannot satisfy an older version request merely because it is `main-latest`.

Application version and wire protocol version remain independent.

## Live Minecraft gate

The live workflow uses official Minecraft/Fabric/Adoptium resolution, a fresh SwarmCraft data directory, explicit EULA acceptance and managed Java. It launches real Minecraft, proves authenticated Fabric readiness, stops through the durability barrier, restarts, restores known world state, and advances canonical snapshots without divergence.

## Intentional YELLOW gates

- **Two-voter crash failover:** Bob alone cannot form majority quorum after Alice disappears. Keep `BlockedByQuorum`.
- **Multi-member wake:** no sleep-bound quorum wake election exists. Keep fail-closed behavior.
- **Public-network certification:** representative NAT/CGNAT/mobile/IPv6/blocked-UDP field evidence remains separate from automated synthetic impairment.
- **Seamless player reconnection:** successor runtime orchestration is implemented, but universal automatic client redirect/reconnect is not yet claimed.

These are documented limitations, not permission to weaken safety.
