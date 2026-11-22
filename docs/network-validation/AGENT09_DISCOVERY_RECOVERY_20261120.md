# Agent 09: authenticated discovery recovery — 2026-11-20

Scope: issue #113, exact-world resolve and public-world browse over authenticated QUIC.

## Baseline and ancestry

- Main at start: `badcbc6e73b42f37a4287c2c2e03e72e50e832ce`.
- Existing isolated public browse retry: PR #116, `baca6fd9aa98ee4189a40f17fb669717f998336e`.
- Technical-preview candidate PR #89 at `88d69a5c029041567da9ee16e4c5645dd7ef7498` was merged with PR #116 on 2026-11-20; the resulting main SHA was `1ef18fcc373d0e61c0c3fbe1bdb92e1ff264a945`.
- PR #128 was originally stacked on #116. Its **current base is main**, not the already-merged feature branch; only the four scoped discovery/network/test/evidence files differ from main. Historical green PR #89 checks are **not** exact-head evidence for PR #128.

## Specific failure mechanism

Issue #113 reports that macOS discovery lost a live canonical provider after an outbound QUIC stream failure while still rejecting stale and forged responses. PR #116 adds bounded recovery to public browse, but original exact resolve still recorded an outbound failure without resending a pending resolve request.

This changes **only discovery request recovery**:
- Both browse and exact resolve track outbound request IDs with their transport peer IDs; unrequested, wrong-peer, duplicate, and delayed responses do not count as successful discovery results.
- A failed request consumes retry credit only if it matches the tracked request and its peer is an explicit caller-supplied locator. DHT-only peers do not receive application-level retry credit.
- At most two **additional requests per explicit locator**, bounded by the existing eight-second **request-collection phase deadline**. The code has additional independent warmup and candidate-freshness proof timeouts, so eight seconds is **not an end-to-end API latency bound**. When already authenticated, resend on the existing connection; otherwise request a bounded redial using the existing network node's single-flight suppression.
- A collection-phase timeout explicitly reports incomplete discovery (`partial` on browse and `network_unavailable` on unresolved exact-world lookups) rather than a false `empty` or `not_found` outcome.
- Canonical-world authenticity, signature, world identity, current membership, freshness proof, quorum, replay, revoked/banned-member rules, and authority eligibility remain unchanged.

## Fault injection and checks to run

The `discovery_network_freshness` integration test `authenticated_quic_stream_drop_is_observable_for_browse_and_exact_resolve` uses real loopback QUIC and signed application authentication. It intentionally destroys the provider while holding an outbound response stream, asserting an outbound failure for the **same original request ID** for both public browse and exact resolve. This establishes a reproducible transport failure event but does **not**, by itself, prove canonical re-availability after reconnect.

The existing `malicious_and_stale_providers_cannot_win_browse_or_exact_resolve` integration scenario now also makes the live canonical provider deliberately disconnect once during **each** query path without stopping its listener. Both paths must still find that provider after redial/re-authentication while excluding forged and stale announcements. A small `DiscoveryNode::disconnect_peer` method supplies the controlled transport fault.

The `permanently_disconnecting_explicit_provider_is_bounded_and_fails_closed` integration test uses the actual QUIC listener to drop every authenticated discovery request without returning an answer. It requires neither a false empty directory nor a false absent world, and permits no more than three received requests per query (initial plus at most two retries). The standalone retry ledger test independently checks attempted requests for duplicate/misattributed failure events.

The `discovery_retry_fault_injection_rejects_duplicate_wrong_peer_and_dht_failures` unit test exercises the same retry state transition used in the production event loops: a wrong transport peer may not consume the retry, a duplicate failure cannot trigger a new dial, an explicit peer can retry at most twice, and a DHT-only peer cannot spend explicit retry credit.

Run and retain full, unedited logs on all three operating systems:

```bash
cargo test -p swarm-cli --locked discovery_retry_fault_injection -- --nocapture
cargo test -p swarm-cli --locked transient_public_provider_retry_budget -- --nocapture
cargo test -p swarm-cli --locked --test discovery_network_freshness -- --nocapture
cargo test -p swarm-network --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Also require exact final-head Required Validation, Main Desktop Installers, native Linux/Windows/macOS checks and soak. Record the full SHA, runner OS and original Actions job IDs; a pass on a different tree is not certification.

## Explicit outstanding release gates

- The positive recovery integration test is now present for both paths; it remains **unverified until exact-head CI passes**. Do not substitute source inspection or another commit's passing run.
- Repeated Linux/Windows/macOS fault-injection and impaired-network runs are required. They have not yet been independently certified on the newest head.
- The eight-second collection phase does **not** cover explicit-locator warmup or subsequent candidate freshness checks; obtain measured end-to-end latency evidence before promising an overall eight-second user-facing deadline.
- Confirm the injected provider disconnects actually occurred in both phases and that the legitimate canonical result passed the full freshness quorum. Catch any 8-second overall-query timeouts or repeated-disconnect regressions.
- NAT/CGNAT, UDP-blocked networks and deployed relay fallback remain **field validation**, not certified by loopback QUIC.
- No release, V2 authority activation, acceptance of a nonquorum proof or closure of issue #113 follows from this scoped patch alone.
