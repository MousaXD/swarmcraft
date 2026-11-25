# Ownerless permission binding (#65) — local qualification

**Status:** isolated lab verified; no canonical ownerless world, live admission,
production runtime authority, or distributed Minecraft hosting is enabled.

## Contract and scope

The #64 creation verifier checks all founding Ed25519 signatures, independently
expected ownerless world identity, and independently expected compatibility.
The offline `ownerless permission-profile` command derives a versioned pin
`sc65v1:<world hash>:<immutable policy hash>:<compatibility hash>`. It does
not create a world, authorize recovery, or grant a permit. The CLI and
managed V1 supervisor refuse mixed ownerless-lab and V1 IPC configuration
rather than silently falling back to creator-owned authority.

The trusted test launcher hashes the actual server, bridge, probe, and
loader library JAR bytes and signs their manifest commitment in a disposable
genesis; this is a **lab fingerprint**, not a production runtime lock. Its
signed initial state commitment is the reference simulation's initial root,
not the Minecraft save's state root. The Fabric bridge receives the pin in
a process environment variable. The bridge validates the immutable policy
hash and shape; the trusted launcher, not the JVM, verifies the certificate.
No remote or OS attestation is asserted.

The Fabric mixins clamp profiles and command-source permissions; reject
operator grants, stored operator additions, privileged console, RCON,
command blocks, and operator policy changes; and deny all live player
admission. Actual parsed settings require loopback, online mode, secure
profiles, enforced whitelist, disabled RCON/query/management/JMX, and
ordinary operator/function permission levels. Nonempty or unsafe inherited
operator lists fail before listener startup. Only the isolated lab pin
activates these hooks; normal V1 behavior remains unchanged. The test-only
probe is a separate explicit Gradle target, not part of release JARs.

The regional lab uses a separate SQLite application ID and ruleset pin.
Fresh workers must bind the expected world, compatibility and immutable
policy; existing generation and region fencing remains required. Each
receipt contains the pin and a domain-separated session MAC that is checked
inside the atomic local ledger commit. Legacy unbound grants, wrong pins,
retired sessions and privileged operations are refused. Donated compute
grants no governance vote. Two generations use distinct worker processes
and reassigned regions, with exact serial replay of the accepted roots.

## Observed evidence — 26 November 2026

- Minecraft 26.1.2, Fabric Loader 0.19.3, Fabric API 0.155.2+26.1.2,
  Java 25.0.5.1, Gradle 9.5.1. Built the bridge and independent test probe
  locally; no hosted CI. Two restricted JVM launches (the second restores
  the first launch's save) each passed 27 checks; a legacy control passed
  three checks including a real operator grant.
- The test creates synthetic server-side Founder, Successor and Ordinary
  profiles and invokes actual server command/permission APIs, not a mocked
  Fabric implementation. Checked actual game listener and absent RCON
  and management sockets, denied command execution and operator grants,
  and verified empty saved operator lists. Five unsafe startups refused
  before the game listener: wrong pin, mixed IPC, enabled RCON, enabled
  management, inherited operators.
- Four real reference-worker processes in separate Linux network namespaces
  across two generations, two accepted reference ticks matching serial
  replay, 34 refused invalid/privileged requests, clean worker exit.
- Retained [public signed genesis, artifact hashes and local observations](
  ../../research/distributed-simulation/evidence/2026-11-26-ownerless-permission.json).
  The independent evidence checker replays roots, validates the observation
  matrix and rechecks the public creation signatures with the real Rust CLI.
  It cannot independently attest that a recorded process ran or that an
  OS owner has not altered it. Nine negative evidence mutations are tested.
- 88 reference simulation tests; one evidence mutation regression;
  208 CLI library tests passed, three ignored; two real creation/permission
  CLI tests passed; strict targeted CLI Clippy passed. The initial broad CLI
  run under umask 0002 and concurrent JVM load failed 13 existing tests
  (unsafe test directories and a busy support store). The documented
  private umask 077 and serial rerun passed all nonignored tests without
  weakening ownership checks or changing those tests.

## Reproduce (local only)

Use the pinned Java 25 and Gradle 9.5.1 toolchain, official Minecraft
26.1.2 bundled server, and the seven Fabric server-profile dependency JARs
for loader 0.19.3. Verify vendor artifact hashes before use. No installed
worlds or player credentials are needed. Review the Minecraft EULA before
passing the explicit disposable-test consent flag.

```sh
umask 077
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-cli --lib -- --test-threads=1'
flock /tmp/swarmcraft-sabathon-cargo.lock -c \
  'cargo test --locked --offline -p swarm-cli --test ownerless_creation_cli'
(cd minecraft/fabric && gradle build ownerlessProbeJar)
unshare --user --map-root-user --net python3 scripts/e2e/ownerless_permission_smoke.py \
  --java "$JAVA_HOME/bin/java" --cli target/debug/swarmcraft \
  --server /path/to/server.jar --loader-libs /path/to/loader-libs \
  --bridge minecraft/fabric/build/libs/swarmcraft-fabric-0.5.1.jar \
  --probe minecraft/fabric/build/ownerless-probe/ownerless-permission-probe-test-only-0.5.1.jar \
  --accept-eula --output /tmp/new-ownerless-permission-evidence.json
python3 scripts/e2e/test_ownerless_permission_smoke.py
(cd research/distributed-simulation && python3 -m unittest discover -p 'test_*.py')
```

The evidence verifier takes the retained file and an independently expected
`--expected-world scownerless:...` plus `--cli target/debug/swarmcraft`.

## CFT vs BFT and remaining acceptance

These protections govern unmodified supported code under a trusted launcher,
trusted local ledger and honest crash-fault assumptions. They cannot constrain
a malicious host with access to the OS, JVM memory, binaries, process
environment, saved data, or worker secrets. HMAC receipts authenticate
the issuing lab session, not correct execution. The compatibility pin and
Minecraft save are not a canonical accepted head. No real Minecraft
accounts logged in; the auth service was intentionally unreachable in the
private test network. The founder and successor are role-labelled synthetic
profiles on one workstation, not independent hosting machines. The second
JVM restart is not a quorum-authorized hosting transfer. The reference
workers are not Minecraft region servers.

The [#66 reference authority](ownerless-authority.md) verifies durable
local quorum history, roster changes, recovery and reference fencing. Live
canonical authority remains gated; #67 must separate authenticated admission
and worker/validator eligibility. A real founder departure, successor
hosting, playable identity, actual independent-machine distributed Minecraft
and the #63 sustained-throughput qualification remain open. Do not enable
ownerless gameplay or claim Byzantine-resistant execution on this evidence.
