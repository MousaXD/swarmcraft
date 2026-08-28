# Agent 6 — Discovery

## Recovery status

`INTEGRATED`

- Branch: `agent/discovery`
- Exact live head: `0a72380aebbc6f227957cae733de64dc6f85638c`
- Live ancestry audit: this exact head is an ancestor of `integration/player-launcher-v1` with zero Agent 6 commits left ahead.

## Integrated contract

- Signed public-world announcements over the existing authenticated libp2p/Kademlia architecture.
- `PUBLIC` worlds may appear in public browse/search.
- `UNLISTED` worlds do not appear in browse/search but may be resolved by exact identifier where policy allows.
- `PRIVATE` worlds are not publicly discoverable.
- Discovery never grants membership and does not bypass invite/membership policy.
- Desktop player launcher exposes public browse/search and exact world-ID resolution through Rust-owned discovery APIs.

## Validation evidence

The exact Agent 6 head was previously validated green for CI, Network Soak, and Release Guard and is fully contained in final integration. Final exact-head Network Soak and process acceptance additionally exercise the integrated networking/recovery stack.

Final acceptance is owned by Agent 8; no standalone Agent 6 blocker remains.
