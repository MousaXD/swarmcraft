# SwarmCraft v0.1.0-preview release gates

This file records executable release evidence for the preview branch.

## Storage large-world soak

- Result: PASS
- GitHub Actions run: 31757348001
- Tested commit: 886120e4a7b67f8b448541551b8b04fb03366654
- Command: `cargo test -p swarm-storage release_large_world_streaming_profiles -- --ignored --nocapture`
- Profiles: 1 GiB, 5 GiB, and 10 GiB synthetic world files
- Path: streaming snapshot creation, Zstd blob encoding, content hashing, snapshot commit, and streaming verification
- Buffering: bounded 1 MiB storage buffers; no whole-world or whole-blob materialization is required by the streaming path

## Permanent CI gates

The normal PR workflow must also pass before merge:

- Ubuntu Rust format, strict Clippy, and tests
- Windows Rust strict Clippy and tests
- real loopback live-join plus immediate snapshot replication test
- real three-daemon hard-kill recovery plus stale-peer resynchronization test
- real host-process IPC / restore / final-snapshot / sleep test
- Fabric server mod build
- RustSec dependency audit
- Linux DEB and AppImage builds with bundled runtime sidecars
- Windows NSIS build with bundled runtime sidecars

## Known preview limitation

If an elected recovery successor dies after durable recovery reservation but before Recovery epoch quorum, the generation may safely stall. The preview favors canonical safety over liveness and does not automatically skip to a second successor within that half-committed recovery round.
