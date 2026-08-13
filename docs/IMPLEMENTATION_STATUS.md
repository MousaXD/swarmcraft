# v0.1.0 Preview Implementation Status

This file maps the executable foundation to the v0.1.0 preview implementation plan.

## Implemented in the foundation slice

- Rust workspace and crate boundaries.
- Protocol version 1 and storage schema version 1.
- Ed25519 peer identity generation and durable private-key storage.
- `PeerId = BLAKE3(public_key)`.
- Canonically encoded `WorldGenesisV1` and deterministic `WorldId`.
- Content-addressed BLAKE3 blob descriptors.
- Zstandard-compressed blob storage.
- Deterministic snapshot manifests and state roots.
- Crash-safe temporary-write + fsync + rename persistence.
- Snapshot verification and corruption detection.
- Snapshot restore / vanilla export.
- Signed snapshot manifests.
- Deterministic authority candidate ranking.
- Monotonic fencing-token rejection of stale/future writes.
- Transport-independent authenticated peer-hello validation.
- Daemon/Fabric IPC message schema including save barriers.
- CLI foundation: init, identity, world create/list/status/snapshot/snapshots/verify/recover/export.
- Windows/Linux Rust CI, clippy, formatting, tests, and RustSec audit.

## Intentionally not claimed as implemented yet

- QUIC transport and connection lifecycle.
- mDNS LAN discovery.
- DHT/bootstrap internet discovery.
- NAT traversal and relay fallback.
- Blob transfer / resumable replication.
- Authority lease runtime and epoch persistence.
- Failure simulator / chaos harness.
- Fabric mod runtime integration.
- Automatic Minecraft launch/restore.
- Graceful or crash host migration.
- Sleep/wake orchestration.
- Tauri desktop UI.
- Windows/Linux installer packaging.

Those require later stages and must not be represented as complete merely because protocol types exist.
