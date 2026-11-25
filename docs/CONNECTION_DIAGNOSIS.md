# Connection diagnosis

Directory lookup, authenticated peer connectivity, host authority proof and Minecraft byte streams are independent checks. None of the first three alone establishes a successful Minecraft login.

## Player actions

- **Join / Have only a World ID?**: Check the exact ID. The result identifies a missing directory, network error, absent announcement, stale or invalid announcement, or a fresh signed announcement. Copy safe directory diagnosis copies only a fixed category and advice, not the World ID, announcement name, address, raw native errors or invite. Editing the ID clears the prior result; a new lookup has a 30-second UI deadline. A result expires after at most 30 seconds or at the signed announcement's expiry, whichever is earlier. A fresh announcement never grants membership.
- **Diagnostics / Selected world connection**: Retry world connection reselects the current loopback gateway and requests independent verification. The status distinguishes a missing gateway, missing authenticated peer link to the accepted authority, unavailable host/quorum/consent/Fabric proof, route timeout, and an authority change while checking. It does not guess which individual host proof was absent. A failed check never publishes a Minecraft address. No cached migration phase is accepted as route proof.
- **Recent Minecraft connection attempt**: Observations from the current gateway distinguish route checking/refusal/timeout, stream opening/admission failure, admitted-stream interruption, an ended session, or a local stream-capacity limit. These are local observations, not evidence of a successful login. They are retained for at most 30 seconds and cleared on gateway replacement or daemon restart; older concurrent sessions cannot overwrite a newer attempt. UI status may expire earlier.
- **Copy safe diagnosis**: copies only allowlisted messages for the currently selected world. It omits host identity, world identity, ports, addresses, raw native errors and bearer credentials. Retry if a report expired. No automatic reconnection is attempted.

## Verification boundary

A usable gateway address requires the existing exact-world fresh signed route and independent authority, membership, quorum, consent and Fabric readiness checks. The renderer's diagnostic text and session observations have no authorizing effect. A previously verified route expires at the existing proof deadline. Directory or stream diagnostics are never a substitute for these checks.

## Automated and field coverage

Rust unit tests exercise bounded gateway observations and non-authorizing status decoding. Network integration tests exercise independently verified game streams and the local negative socket smoke checks an actual refused connection with an unconsented single-peer fixture. Desktop unit/browser tests exercise failure labels, exact-world lookup, expiry, safe copy text, and retry without enabling Play. Svelte's typed IPC view and legacy shipping view share the same directory formatter. Actual installed-client and multi-network player journeys remain field acceptance, not covered by the mock smoke test.
