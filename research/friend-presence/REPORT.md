# Consent-gated friend presence (#46)

## Implementation

The Friends view in both Desktop variants uses verified local contact identity
pairs and a dedicated privacy section. Adding a contact is not a presence
grant; sharing requires a separate per-contact opt-in and the global switch.
Legacy V1 stores migrate atomically to V2 with all grants off; the settings
read persists the migration under the same lock used for proof generation,
so older publishers reject the new version. Invalid or duplicate identity
records fail closed. Contact mutations advance a durable revision, and a
delayed proof is rejected after a contact changes, even after same-ID re-add.

The presence-only publisher starts explicitly and requires an effective
grant. It never publishes world records or answers world discovery or
freshness-vote requests. Hiding or removing the last grant stops it. The
full world publisher also checks the same effective grant on every response.
Publication hints are reconciled under the contact lock. Probes authenticate
the exact target, correlate request and response, and verify requester,
nonce, identity, signature, and lifetime. Unanswered or refused checks are
not proof of offline status. Positive observations expire within 30 seconds.
Local shared-world information derives from locally known signed membership,
not from a remote profile. Presence does not imply game availability.

Desktop controls call the native CLI through typed Tauri commands; both
frontends share the same controller and display verified native settings
rather than infer consent from a clicked control. Contact removal requires
confirmation and does not mutate signed membership or existing invitations.
Create world invite opens the existing selected-world invitation dialog; it
does not create, send, or bind a bearer invite to a contact automatically.

## Local verification

| Scope | Result |
| --- | --- |
| Rust discovery/contact library | 12 tests passed, including legacy migration, duplicate denial, remove/re-add, and existing concurrent mutation tests |
| Core signed presence | Requester/nonce-bound signature regression passed |
| Real-network integration | Six existing discovery regressions passed; additional real delayed signed presence test passed after remove/re-add |
| Native bridge | Exact peer-ID validation test passed; native check and targeted strict Clippy passed |
| Desktop | 164 Node unit tests, 82 Svelte unit tests, 30 source-parity tests; Svelte check/build |
| Both Chromium frontends | Contact input, hidden/default consent, per-contact and global opt-in, expiry, removal confirmation, late response, invitation navigation, and no horizontal overflow at 720x560, 980x760, and 1280x900 (mocked IPC) |
| Real loopback | Three identities and actual locator with mDNS disabled, signed proof to allowed friend, denial to ungranted and removed/re-added contacts, world metadata refusal, and service stop on hide |
| Native Linux | Both extracted Debian packages, real WebKitGTK and bundled sidecars, actual signed probes in both directions through the configured locator; persisted consent/hide/removal, two viewport sizes, existing invite workflow and unchanged world list |
| Workflow policy | Five workflows accepted; no hosted CI dispatch |

[Loopback CLI evidence](evidence/loopback-cli-20261125.json) records the
actual allowed and denied observations. [Native Linux evidence](evidence/linux-native-20261125.json)
records the final package hashes and individual assertion results. Synthetic
peer IDs and screenshots are retained locally, not committed.

## Reproduce

Build the Rust CLI/runtime/infra and shipping/candidate Debian packages from
the same source, with identical four sidecars. Run:

```sh
python3 scripts/e2e/friend_presence_smoke.py --bin-dir target/debug
python3 scripts/acceptance/native_friends.py \
  --packages /path/to/debs --artifacts /path/to/private-output \
  --xvfb /path/to/Xvfb --driver /path/to/tauri-driver \
  --native-driver /path/to/WebKitWebDriver --infra target/debug/swarmcraft-infra
```

The harness starts only isolated owned services with synthetic profiles and
temporary data. It records boolean checks and package hashes; no invite,
peer identity, private key, address or world data is serialized.

## Qualification boundary

This is locally verified privacy and UI integration, not proof of WAN
reachability, independent-machine presence, system-installed Windows/macOS
or Minecraft availability. The default Desktop listener is loopback.
Operators must configure a reachable private LAN/VPN listen interface and
directory providers. Hiding blocks new proofs, not network transport
observability; cached DHT hints and previously delivered signed observations
can persist until expiry. A deliberate outgoing probe reveals an authenticated
request. Invitations remain manually shared bearer tokens requiring signed
membership approval; no automatic remote invitation delivery is implemented.
