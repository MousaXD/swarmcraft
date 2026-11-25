# #35 — Local capacity observations

**Date:** 2026-11-25. **Scope:** single Linux workstation, synthetic clients
and storage fixtures, isolated local processes. No production capacity
promise and no hosted CI.

## Real Minecraft simulation

Actual Minecraft 26.1.2 (Java 25), Ryzen 5 5600H (12 logical CPUs),
approximately 13 GiB physical RAM, 1,536 MiB heap limit, JVM active processor
setting 4, view distance 6, simulation distance 4, peaceful survival and
fixed normal-world seed 42035. The workstation also ran unrelated desktop
applications. The Java heap is not the process RSS and the JVM processor
setting is not a hard CPU quota.

One short progressive-world run, 16 seconds per stage, actual compatible
Mineflayer clients (protocol 26.1). All requested clients were listed by
the real server at both measurement boundaries. Each loaded client moved
in-game; at eight clients the smallest displacement was 19.5 blocks.

| Connected clients | Actual ticks | Observed TPS | Java CPU % | Peak sampled Java RSS (MiB) | Saved world bytes |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 322 | 20.061 | 9.2 | 763.9 | 2,241,181 |
| 1 | 323 | 20.138 | 173.4 | 841.0 | 8,309,285 |
| 2 | 321 | 20.001 | 79.1 | 925.0 | 9,826,158 |
| 4 | 321 | 20.001 | 107.5 | 979.3 | 12,610,821 |
| 8 | 321 | 19.999 | 66.9 | 1,025.4 | 13,541,398 |

The server's sampled average tick durations at eight clients were
5.7, 4.8 and 4.9 ms (50 ms target); this is not a per-tick tail-latency
distribution. Short wall-clock intervals and catch-up may produce an
observed rate marginally above 20. The terrain generated during earlier
stages remains cached, so falling CPU usage with more clients is not
evidence that adding players reduces simulation cost. The initial parse
attempt was stopped cleanly after detecting the server's changed console
wording; it produced no capacity samples and is not included.

An independent fresh-world run used two longer 30-second stages and
the enhanced traffic/CPU/memory sampler:

| Clients | Actual ticks / seconds | Observed TPS | Median sampled MSPT | Java CPU % | Peak Java RSS (MiB) | Peak clients RSS (MiB) | Saved world bytes |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 12 | 601 / 30.046 | 20.003 | 6.3 | 201.8 | 914.6 | 393.8 | 13,539,638 |
| 16 | 601 / 30.065 | 19.990 | 7.9 | 110.3 | 1,397.6 | 443.6 | 19,126,326 |

At 16 clients every client moved (minimum displacement 30.3 blocks),
all 16 slots remained occupied at the start and end, and minimum
available host RAM was 1,965.7 MiB. Both stages completed with clean
server shutdown. The configured 16-slot test ceiling was reached; the
machine's absolute maximum was not established.

| Clients | Total client RX after all spawned (bytes) | Total TX (bytes) | Peak sampled aggregate RX (bytes/s) | Peak sampled aggregate TX (bytes/s) |
| ---: | ---: | ---: | ---: | ---: |
| 12 | 16,767,258 | 216,033 | 2,595,252 | 7,672 |
| 16 | 12,299,396 | 272,856 | 2,214,270 | 9,170 |

These are one-second local TCP socket samples after all clients spawn,
not WAN bandwidth or transport-level peak saturation. The 12-client
stage generates terrain that is reused by the 16-client stage.
The exact game JAR SHA-256 was
`cd47e7c38328f64768fd17af8fcd8b22496b40b63d4ffee81e71ae059fedcb42`.
Both runs used OpenJDK 25.0.4.1. The structured outputs and selected
server log lines are retained under `evidence/`.

## Direct and relayed authenticated game substreams

One measured local run, same request/echo workload: 256 x 4,096 bytes,
one MiB acknowledged application payload each way, including stream open.

| Path | Elapsed (ms) | Acknowledged request bytes |
| --- | ---: | ---: |
| IPv4 QUIC | 157.617 | 1,048,576 |
| IPv6 QUIC | 162.067 | 1,048,576 |
| Direct TCP/Noise | 125.558 | 1,048,576 |
| Relayed TCP/Noise | 317.817 | 1,048,576 |

All three direct and both relay tests passed. The relay-only test verifies
authenticated identities, no direct fallback, byte integrity, loss of
authentication after outage, and fresh replacement relay authentication.
With two independent circuits and a configured 262,144-byte encrypted
circuit cap, the bulk circuit acknowledged 126,976 request bytes before
closure; the other circuit remained usable. Bidirectional payload,
encryption and framing overhead explain why request bytes alone are not
the configured circuit budget. No WAN, public relay, NAT or actual
Minecraft-through-relay capacity claim is made.

## Replica count and world size

Production V1 storage APIs, deterministic synthetic region-shaped files
with mixed random/compressible content, three separate destination stores
on the same physical disk. Each destination was reopened, verified and
restored exactly; no additional simulation process was started.

| Input world | Verified destinations | Cumulative payload bytes | Cumulative logical destination bytes | Cumulative verified copy (ms) |
| ---: | ---: | ---: | ---: | ---: |
| 1 MiB | 1 | 787,614 | 787,711 | 31.805 |
| 1 MiB | 2 | 1,575,228 | 1,575,422 | 62.233 |
| 1 MiB | 3 | 2,362,842 | 2,363,133 | 94.481 |
| 8 MiB | 1 | 6,301,037 | 6,301,134 | 135.447 |
| 8 MiB | 2 | 12,602,074 | 12,602,268 | 262.311 |
| 8 MiB | 3 | 18,903,111 | 18,903,402 | 386.215 |
| 32 MiB | 1 | 25,202,281 | 25,202,378 | 431.921 |
| 32 MiB | 2 | 50,404,562 | 50,404,756 | 1,412.236 |
| 32 MiB | 3 | 75,606,843 | 75,607,134 | 2,440.626 |

Capture times: 40.104, 151.732 and 550.649 ms for 1, 8 and 32 MiB,
respectively. Restore durations for the three 32 MiB destinations:
312.685, 782.629 and 765.733 ms (including byte-for-byte comparison).
Local storage copy does not measure network latency, remote disk,
independent failure domains, or simultaneous gameplay. Fixture signatures
are not authority proofs. Existing authenticated daemon replication
evidence is tracked separately under #32.

## Voter capacity and remaining field gates

The positive three- and five-daemon recovery scenarios were **skipped**:
safe hosting admission required 4,543 MiB available for a selected
512 MiB heap plus reserves, but the workstation had 3,466 MiB available
when the tests ran. The tests return successfully after explicitly logging
this skip; this is not positive recovery timing evidence. The admission
rule was not changed.

Separately, the signed discovery freshness verification path was measured
with real Ed25519 signatures, a strict majority and the production
freshness validator. Each row is the median of five samples in an
unoptimized local Rust test build. Insufficient-majority and repeated
proofs were rejected in every trial.

| Membership voters | Majority | Encoded votes (bytes) | Median signing (ms) | Median verification (ms) |
| ---: | ---: | ---: | ---: | ---: |
| 3 | 2 | 799 | 0.433 | 59.489 |
| 5 | 3 | 1,198 | 0.647 | 68.915 |
| 9 | 5 | 1,996 | 1.072 | 87.039 |
| 17 | 9 | 3,592 | 1.915 | 120.483 |

All seven existing discovery freshness regression tests also passed.
This is local cryptographic proof processing, not remote voter
collection latency, durable failover or a replacement for the skipped
live recovery tests.

No number of voters or replicas increases single-host Minecraft
simulation capacity. Full authenticated Fabric/gateway player load,
modded and diverse terrain workloads, longer independently repeated
trials, concurrent network replication during gameplay, cross-machine
recovery and WAN relay saturation remain separate verification work.
