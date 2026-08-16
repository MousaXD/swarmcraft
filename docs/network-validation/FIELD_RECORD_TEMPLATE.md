# SwarmCraft real-network validation record

Use one copy of this template per real-machine network scenario. Do not mark a scenario verified from loopback, containers, or hosted CI alone.

## Build under test

- SwarmCraft commit:
- SwarmCraft application version:
- Wire protocol version:
- Date/time UTC:
- Tester(s):

## Scenario

- Matrix scenario: same LAN / home NAT to public peer / two home NATs / CGNAT / mobile hotspot / IPv6 / relay fallback / relay loss
- Expected connection path:
- Actual connection path:
- Bootstrap service used, if any:
- Relay service used, if any:

## Endpoint A

- OS and architecture:
- Network type: Ethernet / Wi-Fi / mobile / other
- ISP/carrier class:
- NAT classification reported by SwarmCraft:
- IPv4 available: yes / no
- IPv6 available: yes / no
- Application peer ID:
- Transport peer ID before disruption:
- Public addresses: redact if the record will be public

## Endpoint B

- OS and architecture:
- Network type: Ethernet / Wi-Fi / mobile / other
- ISP/carrier class:
- NAT classification reported by SwarmCraft:
- IPv4 available: yes / no
- IPv6 available: yes / no
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

- Disruption type: process kill / Wi-Fi toggle / cable pull / router reconnect / address change / relay loss / other
- Approximate committed offset before disruption:
- Connection path after recovery:
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

## Evidence

Attach sanitized evidence where possible:

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
