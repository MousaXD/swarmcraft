# SwarmCraft real-network validation record

Use one copy of this template per real-machine network scenario. Do not mark a scenario verified from loopback, containers, or hosted CI alone.

## Build under test

- SwarmCraft commit:
- SwarmCraft application version:
- Wire protocol version:
- Date/time UTC:
- Tester(s):

## Scenario

- Matrix scenario: same LAN / home NAT to public peer / two home NATs / symmetric or strict NAT / CGNAT / mobile hotspot / IPv6 / blocked UDP / relay fallback / relay loss
- Expected connection path:
- Actual connection path:
- Bootstrap service used, if any:
- Relay service used, if any:
- Environment class known independently of AutoNAT, if known:

## Endpoint A

- OS and architecture:
- Network type: Ethernet / Wi-Fi / mobile / other
- ISP/carrier class:
- `ConnectivityStateV1` before disruption:
- NAT classification reported by SwarmCraft: unknown / public / private
- IPv4 observed by SwarmCraft: yes / no
- IPv6 observed by SwarmCraft: yes / no
- End-to-end IPv4 path actually tested: yes / no / not tested
- End-to-end IPv6 path actually tested: yes / no / not tested
- Bootstrap configured count:
- Bootstrap connectivity observed: yes / no
- Relay configured count:
- Relay connectivity observed: yes / no
- Hole-punch state:
- Recent connectivity issue kinds:
- Application peer ID:
- Transport peer ID before disruption:
- Public addresses: redact if the record will be public

## Endpoint B

- OS and architecture:
- Network type: Ethernet / Wi-Fi / mobile / other
- ISP/carrier class:
- `ConnectivityStateV1` before disruption:
- NAT classification reported by SwarmCraft: unknown / public / private
- IPv4 observed by SwarmCraft: yes / no
- IPv6 observed by SwarmCraft: yes / no
- End-to-end IPv4 path actually tested: yes / no / not tested
- End-to-end IPv6 path actually tested: yes / no / not tested
- Bootstrap configured count:
- Bootstrap connectivity observed: yes / no
- Relay configured count:
- Relay connectivity observed: yes / no
- Hole-punch state:
- Recent connectivity issue kinds:
- Application peer ID:
- Transport peer ID before disruption:
- Public addresses: redact if the record will be public

## Replication exercise

- World ID:
- Snapshot number:
- Manifest hash:
- State root:
- Encoded bytes transferred:
- Transfer completed: yes / no
- Receiver snapshot verification passed: yes / no
- Restored-world verification passed: yes / no

## Disruption and resume

- Disruption type: process kill / Wi-Fi toggle / cable pull / router reconnect / address change / UDP block / relay loss / other
- Approximate committed offset before disruption:
- Connection path after recovery:
- Endpoint A `ConnectivityStateV1` after recovery:
- Endpoint B `ConnectivityStateV1` after recovery:
- New connectivity issue kinds after disruption:
- Resume offset after recovery:
- Re-authentication succeeded with the same application peer identity: yes / no
- Transport peer identity remained stable for the same installation: yes / no
- Transfer resumed without restarting already committed data: yes / no
- Final snapshot hash/state root unchanged: yes / no

## Safety invariants

Confirm that the transport event did not alter canonical application state:

- World ID unchanged: yes / no
- Membership unchanged except for intentional test actions: yes / no
- Epoch/fencing token changed only if the authority protocol required it: yes / no
- No stale peer was accepted as current authority: yes / no
- No corrupt or unverifiable snapshot was accepted: yes / no
- Relay/bootstrap outage did not create or select canonical history: yes / no

## Evidence

Attach sanitized evidence where possible:

- connectivity-probe output from both endpoints;
- daemon logs from both endpoints;
- connectivity diagnostics before and after disruption;
- relay/bootstrap logs if operated by the tester;
- snapshot verification output;
- timestamps for the disruption and recovery;
- network captures only if they contain no secrets or sensitive payload data.

Never attach private keys, IPC tokens, invitation secrets, or unredacted credentials.

## Result

- Result: PASS / FAIL / INCONCLUSIVE
- Failure or limitation observed:
- Follow-up issue/PR:
- Notes:
