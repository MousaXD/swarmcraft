# Agent 6 — Friend + Public Discovery

## Status

`READY CANDIDATE`

The implementation head is fully green. This documentation-only ledger refresh must still pass exact-head CI before the branch is promoted to final READY state.

## Branch / validation

- Branch: `agent/discovery`
- Green implementation head: `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`
- Validation PR: `#45` (`agent/discovery` -> `backup/local-work-20260824`), CI vehicle only; do not merge
- Exact-head CI run: `32779349426` — SUCCESS
- Network Soak run: `32779349422` — SUCCESS
- Release version guard run: `32779349498` — SUCCESS

## Mission

Implement privacy-aware friend discovery and public/unlisted world discovery without weakening signed membership, visibility, or authority semantics.

## Dependencies consumed

- `progress/README.md`
- `progress/agent5.md`
- mandated base `backup/local-work-20260824` / `41c9b5b650aac1e320195f6e1855945f2722abc4`

Agent 5 connectivity remains a downstream integration input. Discovery does not invent a competing NAT/reachability authority model.

## Work completed

- Added signed, expiring public/unlisted world announcements backed by canonical world configuration and authority state.
- Added Kademlia provider namespaces for public directory, exact world resolution, and friend-presence lookup.
- Added public-world search with compatibility, tag, region, and text filters plus bounded result counts.
- Added exact unlisted-world resolution while keeping private worlds undiscoverable.
- Added cryptographic friend contacts, friend listing, shared-world calculation, and signed live presence probes.
- Added replay/staleness validation and explicit degraded states for unavailable providers/network paths.
- Added Desktop-facing discovery commands/contracts and tests.
- Boxed `WireResponse::DiscoveryResolved` to avoid inflating wire/network event enums rather than suppressing `large_enum_variant`.
- Kept the normal daemon request match exhaustive and added explicit `DISCOVERY_ENDPOINT_REQUIRED` responses for discovery-only requests sent to the normal daemon protocol.
- Corrected the shared-world fixture to derive `WorldId` from canonical genesis instead of weakening production `WorldMetadataMismatch` validation.

## Contracts / invariants

- Discovery wire protocol: `/swarmcraft/discovery/1`.
- `WireRequest` discovery variants: `DiscoveryPublic`, `DiscoveryResolve`, `FriendPresence`.
- Exact discovery response carries a boxed `WorldAnnouncementV1`.
- Friend identity is cryptographic peer identity; labels are non-authoritative.
- PRIVATE worlds are never discoverable.
- UNLISTED worlds do not appear in browse/search but may resolve by exact world identifier.
- PUBLIC worlds are eligible for browse/search and exact resolution.
- Discovery never grants membership or authority.
- Signed records must be current and verified before use.
- Wire request matching remains exhaustive so new protocol additions require an explicit ownership decision.
- Storage world identity remains derived from canonical genesis.

## Files materially changed

- `crates/swarm-protocol/src/lib.rs`
- `crates/swarm-core/src/lib.rs`
- `crates/swarm-network/src/wire.rs`
- `crates/swarm-network/src/discovery.rs`
- `crates/swarm-network/src/lib.rs`
- `crates/swarm-cli/src/discovery.rs`
- `crates/swarm-cli/src/lib.rs`
- `crates/swarm-cli/src/main.rs`
- `crates/swarm-cli/src/daemon.rs`
- Desktop Tauri/TypeScript discovery bridge files
- `progress/agent6.md`

## Validation evidence

At green implementation head `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`:

- CI `32779349426`: SUCCESS across the configured Linux, macOS, Windows, Desktop/package, acceptance, audit, fuzz, and related jobs.
- Network Soak `32779349422`: SUCCESS.
- Release version guard `32779349498`: SUCCESS.

Earlier exact-head CI was used as the remote Rust toolchain to identify and repair:

- E0308 match-arm type errors in outbound failure handling.
- E0502 friend-store borrow lifetime conflict.
- `large_enum_variant` caused by unboxed exact-world discovery responses.
- rustfmt deviations in discovery code.
- stale `STORAGE_SCHEMA_VERSION` test import.
- Clippy `needless_as_bytes`.
- E0004 non-exhaustive normal-daemon handling after discovery wire variants were introduced.
- invalid test fixture world identity that correctly triggered `WorldMetadataMismatch`.

No lint blanket allows, ignored tests, wildcard ownership escape hatches, arbitrary sleeps, or weakened storage invariants were used.

## Remaining gate

Only this documentation-only ledger descendant needs exact-head CI. If that remains green, Agent 6 is `READY FOR INTEGRATION` and downstream work may consume the branch head.

## Activity log

- 2026-08-24 — implemented signed world/friend discovery, Kademlia lookup, Desktop bridge, and visibility semantics.
- 2026-08-24/25 — iterated against exact-head GitHub CI to repair compiler, borrow, formatting, Clippy, daemon-boundary, and fixture defects.
- 2026-08-25 — `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c` passed CI `32779349426`, Network Soak `32779349422`, and Release Guard `32779349498`.
