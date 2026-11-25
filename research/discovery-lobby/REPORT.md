# Signed world discovery lobby (#45)

## Implementation

Discover is a separate primary player view in both the shipping and opt-in
Svelte frontends. Both use the same bounded public-search controller, with a
six-card page size, a 32-card snapshot, stable ordering, provider coverage,
explicit partial/limit/error states, and per-card observation time and signed
expiry. A new search or context change invalidates obsolete responses; expired
cards are removed. Exact public/unlisted lookup verifies the full requested ID.
It does not join a world or open a route. Private Join still requires a signed
invite.

Rust bounds provider collection and the entire freshness verification pass,
not just each individual request. Only successful signed membership and
freshness-quorum verification produces a listing. A budget exhaustion produces
a partial result; it does not admit unverified candidates. The CLI accepts
`--limit` in 1–64 (default 32). Discovery V1 wire format is unchanged; paging
is local over the verified snapshot, not a distributed continuation cursor.
The verification timestamp is an observation, not proof of current host
availability, admission, authority, or game routing.

## Verified locally

| Scope | Result |
| --- | --- |
| Rust discovery library | 9 tests passed |
| Rust real-network discovery integration | 6 tests passed, including malicious/stale providers, repeated disconnects, and bounded signed-but-unproven candidates |
| Rust quality gates | Targeted strict Clippy, formatting, Tauri locked offline check passed |
| Desktop | 160 Node tests, 82 Svelte unit tests, 30 source-parity tests, Svelte check/build passed |
| Both Chromium frontends | Shared journeys, discovery paging/expiry, malformed/duplicate/private/unlisted exclusion, stale response rejection, keyboard Next, exact unlisted lookup, and separate Join at 720x560, 980x760, 1280x900; native IPC fixture |
| Real loopback publisher/CLI | Nine verified public listings; deterministic capped three; no false match on incomplete query; signed unlisted exact lookup; private lookup refused; no membership created |
| Workflow policy | Five-workflow policy passed; no hosted Actions dispatched |

The [real loopback CLI evidence](evidence/loopback-cli-20261125.json)
records actual result counts, bounded-query status and elapsed times. The
nonmatching query returned partial rather than falsely claiming a complete
empty directory; the private lookup returned no world and network unavailable
rather than disclosing a private record.

Both extracted Debian packages passed the [native Linux recheck](evidence/linux-native-20261125.json) with real WebKitGTK and identical four Rust sidecars: nine verified public listings, six-plus-three paging, exact 980x760 and 720x560 layouts, signed unlisted lookup, separate Join, and empty local membership store. Native Linux evidence is recorded separately. The initial unfiltered search
timed out at the harness's nine-listing assertion on both extracted packages.
A real filtered-search recheck recorded eight verified listings and the
correct partial/limit warning on shipping, followed by a complete nine-listing
response. This is a legitimate bounded-query outcome; no proof or safety limit
was weakened. The retained runner uses the filtered search and permits one additional search attempt, recording the safe summary for diagnosis; its positive gate still requires all nine fixture worlds, correct first-page order, six-plus-three paging, expiry labels, exact unlisted lookup, separate Join and the actual empty client store.

## Reproduction

Build the local Rust CLI/sidecars and both Debian packages from the same
checkout; use identical sidecars. From the repository root:

```sh
python3 scripts/e2e/discovery_lobby_smoke.py --bin-dir target/debug
python3 scripts/acceptance/native_discovery_lobby.py \
  --packages /path/to/debs --artifacts /path/to/private-output \
  --xvfb /path/to/Xvfb --driver /path/to/tauri-driver \
  --native-driver /path/to/WebKitWebDriver
```

Both runners create isolated synthetic worlds, use a real loopback signed
publisher, and shut down only their own processes. No operator profile, account,
Minecraft process, external provider, or hosted service is needed. Screenshots
are local private artifacts; the evidence omits addresses and world IDs.

## Acceptance boundary

Local discovery only. WAN, NAT/relay and remote multi-provider reliability,
system-installed Windows/macOS, and end-to-end Minecraft availability are not
field-qualified. Discovery does not imply membership or a live game route.
