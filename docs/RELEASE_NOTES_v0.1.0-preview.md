# SwarmCraft v0.1.0-preview

First public preview of decentralized Minecraft world persistence for Windows and Linux.

Highlights:
- durable Ed25519 peer/world identity and signed canonical records
- content-addressed BLAKE3/Zstd snapshots with bounded-memory streaming storage
- encrypted authenticated libp2p QUIC networking with mDNS, Kademlia, relay, AutoNAT, and DCUtR
- authority fencing, leases, quorum-backed crash recovery, sleep/wake, and stale-peer resynchronization
- authority-mediated signed join/leave membership changes
- Fabric 26.1.2 lifecycle bridge with authenticated loopback IPC and save/shutdown barriers
- Tauri v2 desktop preview with bundled SwarmCraft runtime sidecars
- Windows NSIS and Linux DEB/AppImage release packaging
- real loopback live-join, three-daemon crash recovery, and host-process acceptance tests

Known preview limitation:
If an elected recovery successor dies after durable recovery reservation but before Recovery epoch quorum, the generation may safely stall. This preview favors canonical safety over automatic second-successor liveness in that half-committed recovery round.
