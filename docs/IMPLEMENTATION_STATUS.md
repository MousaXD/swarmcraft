# SwarmCraft 0.4.0 Implementation Status

This document is the repository source of truth for implemented behavior versus known technical-preview limitations. Application version `0.4.0` continues to use wire protocol version `1`.

## Executive summary

SwarmCraft 0.4.0 is an advanced technical preview with the core player journey integrated end to end. Runtime setup, explicit EULA handling, server-mod verification, Host Readiness, existing-world import, safe stop/sleep, and automatic successor Minecraft runtime orchestration are implemented in the shared Rust/Desktop product path.

The remaining gaps are narrower than earlier previews: seamless client reconnection, safe quorum-based multi-member wake, representative public-network certification, and production release/signing operations.

## Implemented

### Identity, storage and replication

- Durable peer/world identity and signed canonical records.
- Content-addressed compressed snapshots with streaming verification and corruption rejection.
- Publication ownership protection, retention/GC coordination, failure injection, and multi-source/source-fallback reconstruction.
- Resumable transfers across reconnects and surviving replicas.

### Networking

- Authenticated libp2p QUIC/TCP paths, Kademlia, mDNS, AutoNAT, DCUtR and relay support.
- Current-path connectivity diagnostics rather than sticky historical-success booleans.
- Bounded hostile-input handling, fuzz smoke and impaired QUIC resume acceptance.

### Authority and migration

- Majority quorum, signed leases, epochs and fencing tokens.
- Quorum-backed crash recovery and stale-authority rejection.
- Shared migration/runtime orchestration used by automatic recovery, manual transfer and supported wake paths.
- Successor Minecraft runtime startup after safe authority transition.
- Three-daemon recovery and recovery-successor failure acceptance.

### Minecraft runtime and lifecycle

- Backend-managed Java/Minecraft/Fabric/Fabric API/SwarmCraft bridge installation.
- Explicit Minecraft EULA acceptance and durable machine-local launch configuration.
- Authenticated Fabric compatibility/readiness handshake.
- Required third-party server-mod metadata/hash verification.
- Fabric save/checkpoint/shutdown barrier before successful Stop World.
- Final signed canonical snapshot and durable sleep record.
- Corrupt/unreadable sleep state fails closed in direct, standby and migration paths.

### Desktop

- Launcher-style Runtime Wizard backed by the Rust sidecar contract.
- Create, join, invite, leave, play, stop/sleep, diagnostics and Host Readiness flows.
- Existing-world import with explicit compatibility metadata and safe backend publication.
- Runtime/mod remediation without reproducing authority logic in JavaScript.

### Packaging

Every native Desktop bundle requires four sidecars:

- `swarmcraft`
- `swarmcraft-host`
- `swarmcraft-runtime`
- `swarmcraft-import`

CI builds Linux `.deb` + AppImage, Windows NSIS, macOS ARM64 `.dmg`, and macOS x86_64 `.dmg`. Main snapshots and tagged releases must stage the same four sidecars. Tagged releases also publish the versioned Fabric bridge JAR and checksum.

## Intentional fail-closed limitations

### Two-voter crash recovery

For two voting members, majority quorum is two. If Alice crashes, Bob alone cannot safely elect himself. `BlockedByQuorum` is correct. A positive automatic crash-recovery topology requires three voting members, or Alice must explicitly transfer authority before leaving.

### Multi-member wake

A dedicated sleep-bound quorum wake election is not implemented. Multi-member sleeping worlds remain blocked rather than using first-click-wins or weakening fencing/quorum.

## Still incomplete

- Seamless automatic Minecraft client reconnection/redirect after authority migration.
- Representative home-router, CGNAT, mobile-carrier, blocked-UDP and independent-ISP IPv6 field certification.
- Automatic redistribution of arbitrary third-party mod JARs.
- Production signing/notarization credentials in every release environment.
- Longer hostile-peer/fuzz/soak campaigns beyond permanent CI profiles.
- Public/friends lobby services and later research features such as distributed simulation.

## Release claim discipline

Safe 0.4.0 claims include managed runtime setup, explicit EULA, existing-world import, verified server-mod readiness, safe stop/sleep, quorum-backed recovery, automatic successor runtime orchestration, signed snapshot replication, and cross-platform technical-preview installers.

Do not claim universal NAT traversal, Byzantine fault tolerance, seamless client reconnection after every crash, safe two-voter crash failover, or multi-member wake until those are genuinely implemented and proven.
