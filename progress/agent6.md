# Agent 6 — Friend + Public Discovery

## Status

`IN PROGRESS`

Implementation is complete enough for final validation, but the branch is not READY until the exact post-ledger head passes the required CI matrix.

## Branch / exact head

- Branch: `agent/discovery`
- Last implementation head before this ledger refresh: `a4237bb864b3522bcce5606c4bca14da949a3d48`
- Exact green/final head: pending exact-head CI after fixture correction
- Validation PR: `#45` (`agent/discovery` -> `backup/local-work-20260824`), CI vehicle only; do not merge

## Mission

Implement privacy-aware friend discovery and public/unlisted world discovery without weakening signed membership, visibility, or authority semantics.

## Dependencies read / consumed

- `progress/README.md`
- `progress/agent5.md`
- mandated base `backup/local-work-20260824` / `41c9b5b650aac1e320195f6e1855945f2722abc4`

Agent 5's later connectivity work remains a downstream integration input. Discovery does not invent a competing NAT/reachability authority model.

## Work completed

- Added signed, expiring public/unlisted world announcement support backed by canonical world configuration and authority state.
- Added Kademlia provider namespaces for the public directory, exact world resolution, and friend-presence lookup.
- Added public-world search with compatibility, tag, region, and text filters plus bounded result counts.
- Added exact unlisted-world resolution while keeping private worlds undiscoverable.
- Added local cryptographic friend contacts, friend listing, shared-world calculation, and signed live presence probes.
- Added replay/staleness validation and explicit degraded states for unavailable providers/network paths.
- Added Desktop-facing discovery commands/contracts and tests.
- Hardened discovery transport/request handling after exact-head CI exposed compiler, borrow, formatting, and Clippy defects.
- Boxed `WireResponse::DiscoveryResolved` to avoid inflating the wire-response/network-event enums instead of suppressing `large_enum_variant`.
- Kept the normal daemon request match exhaustive and added an explicit `DISCOVERY_ENDPOINT_REQUIRED` response for discovery-only requests accidentally routed to the normal daemon protocol.

## Contracts / APIs added or changed

- Discovery wire protocol: `/swarmcraft/discovery/1`.
- `WireRequest` discovery variants:
  - `DiscoveryPublic { filter }`
  - `DiscoveryResolve { world_id }`
  - `FriendPresence { expected_peer_id, requester_peer_id, nonce }`
- `WireResponse` discovery variants include bounded public result lists, boxed exact-world resolution, and signed friend presence.
- Public directory/world/friend Kademlia provider keys are separated by explicit namespaces.
- Friend identity is keyed cryptographically by peer ID/public key; display labels are non-authoritative.
- Discovery visibility semantics:
  - PRIVATE: never published or resolved through discovery.
  - UNLISTED: not returned by public browse/search, but may resolve by exact world identifier.
  - PUBLIC: eligible for browse/search and exact resolution.
- Discovery never grants membership or authority.

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
- Desktop Tauri/TypeScript bridge files for discovery commands
- `progress/agent6.md`

## Validation evidence so far

Exact-head CI at `f08028ab157f6bead53dcaabff695cc65338ac91` proved:

- Rust formatting passed on Ubuntu.
- QUIC impairment acceptance passed.
- Fabric server-mod build passed.
- Fuzz smoke passed.
- Earlier discovery-specific compiler/borrow/large-enum failures were gone.
- Desktop frontend tests and Tauri bridge validation passed on Linux.

That run then exposed one shared remaining compile defect: the normal daemon's exhaustive `WireRequest` match did not cover the three discovery-only variants. The same E0004 explained Rust Clippy, Desktop bundled-runtime builds, and the later process-acceptance compile failure across platforms. It was fixed at implementation head `a4237bb864b3522bcce5606c4bca14da949a3d48` with an explicit endpoint-boundary response, not a wildcard arm.

Exact-head CI at `33f9677b58d921fce20de5cda0edd3cf6bdbcfbf` then proved Ubuntu formatting and Clippy and macOS Clippy pass through the daemon boundary fix. Ubuntu and macOS tests both exposed the same deterministic fixture defect: `banned_or_removed_friend_is_not_reported_as_shared_world` constructed an arbitrary `WorldId([6; 32])` that did not match `genesis.world_id()`, correctly causing `WorldMetadataMismatch`. The fixture is being corrected to derive the canonical world ID from the genesis; the production storage invariant is not being weakened.

Do not treat any earlier partial green jobs as exact-final-head proof. The post-fixture SHA must pass fresh CI.

## Decisions / invariants

- Signed canonical world configuration remains the source of truth for visibility and compatibility.
- Discovery records expire and are verified before use; stale historical reachability is not treated as current reachability.
- Friend/contact identity is distinct from membership and authority eligibility.
- Private worlds must not leak through browse, exact resolution, or unnecessary metadata paths.
- Exhaustive wire-request matching is retained so future protocol additions require an explicit ownership decision.
- Storage world identity remains derived from canonical genesis; tests must obey this invariant rather than weakening production validation.
- No lint blanket allows, ignored tests, wildcard match escape hatches, or fabricated green status are acceptable.

## Known issues / blockers

- Final blocker is exact-head CI validation after the canonical-world fixture correction.
- Agent 5 connectivity hints must be consumed during later combined integration without duplicating its reachability authority.

## Handoff for dependent agents

Agent 7 may consume this branch only after this ledger records an exact green SHA as `READY FOR INTEGRATION`. Agent 8 should preserve the visibility/privacy rules, signed-record freshness checks, explicit degraded states, and the discovery-service vs normal-daemon protocol boundary.

## Activity log

- 2026-08-24 — created branch from mandated backup base and began discovery implementation.
- 2026-08-24 — implemented signed world/friend discovery, Kademlia provider lookup, Desktop bridge, and privacy semantics.
- 2026-08-24 — used exact-head GitHub CI to fix compiler errors, borrow conflicts, rustfmt diffs, stale test imports, and Clippy findings.
- 2026-08-24 — structurally boxed exact-world discovery response instead of suppressing enum-size lint.
- 2026-08-24 — `a4237bb864b3522bcce5606c4bca14da949a3d48` — added explicit normal-daemon rejection response for discovery-only wire requests.
- 2026-08-24 — exact-head `33f9677b58d921fce20de5cda0edd3cf6bdbcfbf` reached clean format/Clippy and exposed a cross-platform invalid test fixture; canonical genesis-derived world identity is the required fix.
