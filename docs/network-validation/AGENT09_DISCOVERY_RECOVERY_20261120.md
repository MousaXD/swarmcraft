# Agent 09: authenticated discovery recovery — 2026-11-20

Scope: issue #113, exact-world resolve and public-world browse over authenticated QUIC.

## Baseline and ancestry

- Main at start: `badcbc6e73b42f37a4287c2c2e03e72e50e832ce`.
- Existing isolated public browse retry: PR #116, `baca6fd9aa98ee4189a40f17fb669717f998336e`.
- Technical-preview combined candidate: PR #89, `88d69a5c029041567da9ee16e4c5645dd7ef7498`.
- This branch stacks on PR #116 rather than reimplementing its original browse fix. Agent 00 must explicitly incorporate and certify this change on the intended release candidate; historical green PR #89 checks are **not** exact-head evidence for this branch.

## Specific failure mechanism

Issue #113 reports that macOS discovery lost a live canonical provider after an outbound QUIC stream failure while still rejecting stale and forged responses. PR #116 adds bounded recovery to public browse, but original exact resolve still recorded an outbound failure without resending a pending resolve request.

This changes **only discovery request recovery**:
- Both browse and exact resolve track outbound request IDs with their transport peer IDs; unrequested, wrong-peer, duplicate, and delayed responses do not count as successful discovery results.
- A failed request consumes retry credit only if it matches the tracked request and its peer is an explicit caller-supplied locator. DHT-only peers do not receive application-level retry credit.
- At most two **additional requests per explicit locator**, with the original eight-second overall discovery-query deadline. When already authenticated, resend on the existing connection; otherwise request a bounded redial using the existing network node's single-flight suppression.
- Canonical-world authenticity, signature, world identity, current membership, freshness proof, quorum, replay, revoked/banned-member rules, and authority eligibility remain unchanged.

## Fault injection and checks to run

The `discovery_network_freshness` integration test `authenticated_quic_stream_drop_is_observable_for_browse_and_exact_resolve` uses real loopback QUIC and signed application authentication. It intentionally destroys the provider while holding an outbound response stream, asserting an outbound failure for the **same original request ID** for both public browse and exact resolve. This establishes a reproducible transport failure event but does **not**, by itself, prove canonical re-availability after reconnect.

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

- End-to-end **positive recovery** of a reachable canonical provider after controlled disconnect and reauthentication, for both public browse and exact resolve, needs an additional full-fidelity test.
- Repeated Linux/Windows/macOS fault-injection and impaired-network runs are required.
- NAT/CGNAT, UDP-blocked networks and deployed relay fallback remain **field validation**, not certified by loopback QUIC.
- No release, V2 authority activation, acceptance of a nonquorum proof or closure of issue #113 follows from this scoped patch alone.
