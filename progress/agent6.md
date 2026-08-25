# Agent 6 — Friend + Public Discovery

## Status

`READY FOR INTEGRATION`

Discovery implementation is complete. Production code has not changed since the green implementation commit `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`; the later Agent 6 branch commits through `9ae21daf188785ac9f757ba4d8c1be75953f80aa` are progress-ledger-only descendants and were also validated successfully.

## Branch / exact head

- Branch: `agent/discovery`
- Exact validated branch head before this final ledger-only promotion: `9ae21daf188785ac9f757ba4d8c1be75953f80aa`
- Exact green implementation head: `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`
- Validation PR: `#45` (`agent/discovery` -> `backup/local-work-20260824`), draft CI vehicle only; DO NOT MERGE
- Historical fixture-trigger PR: `#49` (`ci/discovery-fixture-trigger` -> `agent/discovery`) is closed/unmerged and has no remaining diff because its fixture repair is already present in Agent 6 history
- Historical fixture-trigger branch head: `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`

Agent 7 may consume the latest validated `agent/discovery` descendant. The discovery production implementation itself is exactly the code at `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`; subsequent Agent 6 commits are documentation-only.

## Mission

Implement privacy-aware friend discovery and public/unlisted world discovery without weakening signed membership, visibility, authority, or the existing authenticated libp2p/Kademlia architecture.

## Dependencies to read

- `progress/README.md`
- `progress/agent5.md`

## Dependencies consumed

- `progress/README.md`
- `progress/agent5.md` as present on the Agent 6 branch
- mandated base `backup/local-work-20260824` / `41c9b5b650aac1e320195f6e1855945f2722abc4`

Agent 5 owns automatic invite reachability/bootstrap selection. Agent 6 does not replace or duplicate that contract. Discovery connectivity hints never grant membership or authority.

## Work completed

- Added signed, expiring PUBLIC/UNLISTED world announcements sourced from canonical world configuration and current authority state.
- Added Kademlia provider namespaces for public directory lookup, exact world lookup, and friend-presence lookup.
- Added public-world search with compatibility, tag, approximate-region, text, and bounded-result filters.
- Added exact UNLISTED world resolution while keeping PRIVATE worlds undiscoverable.
- Added cryptographic friend contacts, deterministic friend listing/storage, shared-world calculation, and signed live presence probes.
- Added expiry, signature, identity, nonce/requester binding, and replay/staleness validation.
- Added explicit degraded states for unavailable providers/network paths.
- Added Desktop-facing discovery commands/contracts and tests.
- Boxed `WireResponse::DiscoveryResolved` to avoid inflating wire/network event enums rather than suppressing `large_enum_variant`.
- Kept normal daemon request matching exhaustive and added `DISCOVERY_ENDPOINT_REQUIRED` responses for discovery-only requests sent to the normal daemon protocol.
- Corrected the shared-world fixture to derive `WorldId` from canonical genesis instead of weakening production `WorldMetadataMismatch` validation.

## Contracts / APIs added or changed

- Discovery wire protocol: `/swarmcraft/discovery/1`.
- `WireRequest` discovery variants: `DiscoveryPublic`, `DiscoveryResolve`, `FriendPresence`.
- Exact discovery response carries a boxed `WorldAnnouncementV1`.
- `WorldAnnouncementV1` is signed by the authoritative local SwarmCraft identity and expires after a bounded lifetime.
- Friend presence is signed and bound to expected peer, authenticated requester, nonce, and expiry.
- Friend identity is cryptographic peer identity; labels are non-authoritative local presentation data.
- PUBLIC directory publication is enabled only while at least one locally authoritative PUBLIC world exists.
- Per-world Kademlia publication exists for PUBLIC and UNLISTED worlds to support exact resolution.
- PRIVATE worlds are not announced or published through discovery.
- Discovery results never mutate canonical membership and never grant authority.

### Visibility invariants

- PRIVATE: never published, never returned by browse/search, never returned by exact discovery resolution.
- UNLISTED: never returned by normal public browse/search; may be resolved by exact world identifier.
- PUBLIC: eligible for public directory publication, browse/search, and exact resolution.

## Files changed

Material implementation locations:

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

## Tests and evidence

Exact-head GitHub validation at `9ae21daf188785ac9f757ba4d8c1be75953f80aa`:

- CI `32785666574`: SUCCESS.
  - Rust matrix: Linux, Windows, macOS.
  - `cargo fmt --all -- --check` on Linux.
  - `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` on all Rust matrix targets.
  - `cargo test --workspace --all-features --locked` on all Rust matrix targets.
  - Process-level network/storage/CLI acceptance suite.
  - Network impairment / interrupted QUIC resume gate.
  - Desktop frontend/Tauri bridge tests and native package builds.
  - Rust dependency audit and fuzz smoke.
- Network Soak `32785666580`: SUCCESS (`Interrupted QUIC multi-GiB soak`).
- Release version guard `32785666665`: SUCCESS.

Repository CI does not have a separate `cargo check --workspace` step. Exact-head Clippy and workspace test jobs compile/check the workspace across Linux, Windows, and macOS and passed. A local re-run was not available in the validation environment because direct GitHub DNS access is unavailable; GitHub Actions is therefore the authoritative toolchain evidence.

Targeted discovery evidence covered by the successful workspace suite includes:

- cryptographic friend store / peer-ID and public-key consistency
- duplicate-label safety and deterministic friend ordering
- PRIVATE/UNLISTED exclusion from public browse
- banned/removed friend exclusion from shared-world reporting
- signed announcement verification and forgery rejection
- expired/overlong announcement rejection
- signed PRIVATE announcement rejection
- announcement replay/sequence rejection
- friend-presence requester/nonce binding
- exact-world resolution signature/expiry/privacy checks
- authenticated discovery request boundary

No lint blanket allows, ignored correctness tests, wildcard protocol ownership escape hatches, arbitrary sleeps, centralized discovery service, or weakened storage/authority invariants were introduced.

## Decisions / invariants

- Preserve authenticated libp2p/Kademlia discovery; do not add a centralized friend or world directory service.
- `PeerHello` authentication gates discovery requests at the transport/application identity boundary.
- PUBLIC browse accepts only PUBLIC announcements and re-verifies signed records before display.
- Exact resolution accepts PUBLIC/UNLISTED only and rejects PRIVATE records even if a malicious provider returns one.
- Signed discovery records are freshness-bounded and replay guarded where aggregation can return duplicate/stale records.
- Shared-world reporting requires valid signed membership and non-banned membership for both local peer and friend.
- Friend storage is local, versioned, cryptographic, sorted deterministically, persisted via temp-file + sync + rename, and uses private file permissions on Unix.
- Discovery never grants membership. Canonical signed membership remains authoritative.
- Connectivity/relay reachability never grants authority.
- Agent 5 remains the owner of automatic invite reachability; Agent 6 owns discovery semantics only.

## Known issues / blockers

No Agent 6 blocker remains.

Operational limitations that are intentional/current-architecture constraints rather than Agent 6 defects:

- Discovery availability depends on reachable libp2p/Kademlia peers/bootstrap paths.
- World announcements are deliberately short-lived and require periodic authority refresh.
- Friend presence is deliberately short-lived and is reported as offline/network-unavailable when no authenticated live proof can be obtained.
- Agent 5 integration remains responsible for making normal invite connectivity automatic across NAT/relay conditions.

## Handoff for dependent agents

### Agent 7

Consume `agent/discovery` only from an exact green SHA. The exact validated pre-promotion branch head is:

`9ae21daf188785ac9f757ba4d8c1be75953f80aa`

Production discovery code is unchanged from:

`fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c`

Agent 7 must preserve these semantics:

- PRIVATE: never browseable/resolvable through discovery.
- UNLISTED: exact-ID/invite resolution only; never normal browse/search.
- PUBLIC: browse/search + exact resolution.
- Discovery result is descriptive connectivity/presentation data only; it is never membership or authority.
- Friend presence must remain cryptographically bound to the expected friend identity and live request.
- Do not replace libp2p/Kademlia with a centralized discovery service.
- Do not redesign Agent 5 invite-connectivity ownership.

## Activity log

- 2026-08-24 — implemented signed world/friend discovery, Kademlia lookup, Desktop bridge, and visibility semantics.
- 2026-08-24/25 — iterated against exact-head GitHub CI to repair compiler, borrow, formatting, Clippy, daemon-boundary, and fixture defects.
- 2026-08-25 — `fc52e288730bcdd98eabef3a0eaaf73c7ff92e1c` passed CI `32779349426`, Network Soak `32779349422`, and Release Guard `32779349498`.
- 2026-08-25 — documentation-only descendants advanced Agent 6 to `9ae21daf188785ac9f757ba4d8c1be75953f80aa`; exact-head CI `32785666574`, Network Soak `32785666580`, and Release Guard `32785666665` all passed.
- 2026-08-25 — final validation audit reconfirmed PR #49 is already absorbed/no-diff, verified visibility/friend/public-world contracts, made no production-code changes, and promoted Agent 6 to `READY FOR INTEGRATION`.
